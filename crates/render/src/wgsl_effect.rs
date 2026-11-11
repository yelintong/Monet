//! A general interface for **independent WGSL effects**.
//!
//! Monet renders effects on the CPU (tiny-skia), so this module owns the parts that are
//! host-independent: loading a `.wgsl` file from disk, scanning its entry points, bindings and
//! uniform layouts, exposing a small set of **generic engine exports**, discovering the extra
//! parameters a shader needs, and packing engine data + user values into uniform bytes. A GPU backend
//! (or the CPU renderer) consumes those bytes; nothing here depends on egui, wgpu or winit.
//!
//! Contract
//! --------
//! * `WgslEffects::load` reads a local `.wgsl` file and scans it. A failed load never replaces a
//!   working effect, so a broken edit can't leave the engine with half a shader.
//! * Generic engine exports (`GENERIC_EXPORTS`) are named data slots the engine fills from the
//!   document/view. Any uniform field with one of those names is wired to the slot automatically
//!   and gets **no** input control.
//! * Every other uniform field is an "extra entry": `params()` returns a control spec for it, so
//!   when an effect is added the UI can build its input fields from the shader instead of from
//!   hard-coded panel code. `set_param` changes a value and `uniform_bytes` packs it.
//! * Optional `// @fx ...` metadata in the WGSL refines the defaults:
//!   `@fx data <field> = <expr>`, `@fx param <field> <min> <max> <step> <default> "<label>" [map]`,
//!   `@fx hide <field>...`, `@fx group <field> <group>`, `@fx exports <name>...`.
//! * Expressions support numbers, `v` (the control's value), export names, `PI`/`E`,
//!   `+ - * / ( )` and `min/max/clamp/abs/sqrt/floor/ceil`.
//!
//! On wasm there is no filesystem: `Source::Path` returns [`Error::Unsupported`] and callers pass
//! the source text with `Source::Inline` instead, so the rest of the interface is identical.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Uniform fields the engine supplies itself. They are not user parameters, so no input control is
/// generated for them. Hosts can register any of these (and their own names) with `set_export`.
pub const GENERIC_EXPORTS: &[&str] = &[
    "quad",
    "nodeSize",
    "nodeSizePx",
    "regionOriginPx",
    "canvasSize",
    "resolution",
    "dpr",
    "scale",
    "time",
    "pointer",
    "cornerRadii",
    "tileUV",
    "tileOffset",
    "tileExpansionScale",
    "nodeSpaceCenterPoint",
    "maskTextureSize",
    "nodeToTileSpace",
    "tileToNodeSpace",
    "frameRotation",
    "bevelSize",
    "directComposite",
    "textureMaskDirectComposite",
    "hasUniformCorners",
    "cornerSmoothings",
    "tintColor",
    "directCompositeBackgroundColor",
    "sceneTex",
    "sceneSamp",
    "blurIn",
    "blurSamp",
];

/// Entry points the engine always knows how to drive (fullscreen triangle, quad, blit, blur, tile).
/// Anything else is an extra entry of the effect.
pub const GENERIC_ENTRIES: &[&str] = &["vs_full", "vs_quad", "fs_blit", "fs_tile", "fs_blurH", "fs_blurV"];

/// Generic resource bindings the engine provides for every effect.
pub const GENERIC_BINDINGS: &[&str] = &["sceneTex", "sceneSamp", "blurIn", "blurSamp", "U", "F", "T"];

/// Why loading or packing an effect failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Io(String),
    Parse(String),
    NotLoaded(String),
    Unsupported(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(s) | Error::Parse(s) | Error::Unsupported(s) => f.write_str(s),
            Error::NotLoaded(s) => write!(f, "效果 {s} 尚未加载：请先填本地 .wgsl 路径并点「加载」"),
        }
    }
}

impl std::error::Error for Error {}

/// Where an effect's WGSL source comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A local `.wgsl` file. Desktop/server only; `reload` re-reads it so edits take effect.
    Path(PathBuf),
    /// Source text injected by the host (wasm, tests, control channel).
    Inline(String),
}

/// Named engine data slots: the generic exits an effect uses to read data from the engine.
#[derive(Clone, Debug, Default)]
pub struct EngineExports {
    slots: BTreeMap<String, Vec<f64>>,
}

impl EngineExports {
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<Vec<f64>>) {
        self.slots.insert(name.into(), value.into());
    }
    pub fn scalar(&mut self, name: impl Into<String>, value: f64) {
        self.slots.insert(name.into(), vec![value]);
    }
    pub fn get(&self, name: &str) -> Option<&[f64]> {
        self.slots.get(name).map(Vec::as_slice)
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.slots.keys().map(String::as_str)
    }
}

/// One input control derived from a shader field.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamSpec {
    /// Uniform path, e.g. `U.refractionIntensity`.
    pub field: String,
    pub label: String,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub default: f64,
    /// Expression turning the UI value `v` into the value written to the uniform.
    pub map: String,
    pub group: String,
    /// True when the scanner discovered this field rather than an `@fx param` declaration.
    pub auto: bool,
    /// True for `@fx host`: no uniform write, the host consumes `param_value` itself.
    pub host: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub stage: String,
    pub name: String,
    pub generic: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub group: u32,
    pub binding: u32,
    pub name: String,
    pub ty: String,
    pub uniform: bool,
    pub generic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scalar {
    F32,
    I32,
    U32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub ty: String,
    pub offset: usize,
    pub align: usize,
    pub size: usize,
    pub components: usize,
    pub scalar: Scalar,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UniformLayout {
    pub struct_name: Option<String>,
    pub bare: bool,
    pub size: usize,
    pub fields: Vec<Field>,
}

/// `<min>, <max>, <step>, <default>, <label>, <map>` of an `@fx param` / `@fx host` declaration.
type ParamDecl = (f64, f64, f64, f64, String, String);

/// Optional declarations found in `// @fx ...` comments.
#[derive(Clone, Debug, Default)]
pub struct Meta {
    pub id: String,
    pub label: String,
    pub exports: Vec<String>,
    pub data: Vec<(String, String)>,
    pub params: BTreeMap<String, ParamDecl>,
    pub hides: BTreeSet<String>,
    pub groups: BTreeMap<String, String>,
    pub hosts: BTreeMap<String, ParamDecl>,
}

/// The result of scanning one WGSL source.
#[derive(Clone, Debug)]
pub struct Scan {
    pub id: String,
    pub label: String,
    pub meta: Meta,
    pub entries: Vec<Entry>,
    pub bindings: Vec<Binding>,
    pub uniforms: BTreeMap<String, UniformLayout>,
    pub generic_entries: Vec<String>,
    pub extra_entries: Vec<String>,
    pub extra_bindings: Vec<String>,
    /// Fields that are neither a generic export nor hidden: the ones the UI builds controls for.
    pub extra_fields: Vec<String>,
}

#[derive(Clone, Debug)]
struct Loaded {
    source: Source,
    code: String,
    scan: Scan,
    params: BTreeMap<String, f64>,
}

/// The engine's WGSL effect registry. Add an effect with `define`, load it with `load`, read its
/// discovered controls with `params`, and pack its uniforms with `uniform_bytes`.
#[derive(Clone, Debug, Default)]
pub struct WgslEffects {
    effects: BTreeMap<String, Loaded>,
    exports: EngineExports,
    status: String,
    last_error: Option<String>,
}

// ---------------------------------------------------------------------------
//  WGSL scanning
// ---------------------------------------------------------------------------

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn round_up(n: usize, a: usize) -> usize {
    if a == 0 {
        return n;
    }
    n.div_ceil(a) * a
}

fn type_info(ty: &str) -> Result<(usize, usize, usize, Scalar), Error> {
    let t: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
    match t.as_str() {
        "f32" => Ok((4, 4, 1, Scalar::F32)),
        "i32" => Ok((4, 4, 1, Scalar::I32)),
        "u32" => Ok((4, 4, 1, Scalar::U32)),
        _ => {
            if let Some(rest) = t.strip_prefix("vec") {
                let mut it = rest.chars();
                let n = it.next().and_then(|c| c.to_digit(10)).unwrap_or(0) as usize;
                let scalar = if t.ends_with("i32>") {
                    Scalar::I32
                } else if t.ends_with("u32>") {
                    Scalar::U32
                } else {
                    Scalar::F32
                };
                if (2..=4).contains(&n) {
                    let align = if n == 2 { 8 } else { 16 };
                    return Ok((align, n * 4, n, scalar));
                }
            }
            if let Some(rest) = t.strip_prefix("mat") {
                let mut it = rest.chars();
                let cols = it.next().and_then(|c| c.to_digit(10)).unwrap_or(0) as usize;
                let rows = it.nth(1).and_then(|c| c.to_digit(10)).unwrap_or(0) as usize;
                if (2..=4).contains(&cols) && (2..=4).contains(&rows) {
                    let align = if rows == 2 { 8 } else { 16 };
                    let col = round_up(rows * 4, align);
                    return Ok((align, cols * col, cols * rows, Scalar::F32));
                }
            }
            Err(Error::Parse(format!("unsupported WGSL uniform type: {ty}")))
        }
    }
}

fn layout_fields(members: &[(String, String)]) -> Result<UniformLayout, Error> {
    let mut offset = 0usize;
    let mut fields = Vec::new();
    for (name, ty) in members {
        let (align, size, components, scalar) = type_info(ty)?;
        offset = round_up(offset, align);
        fields.push(Field { name: name.clone(), ty: ty.clone(), offset, align, size, components, scalar });
        offset = offset.saturating_add(size);
    }
    Ok(UniformLayout { struct_name: None, bare: false, size: round_up(offset, 16), fields })
}

fn parse_structs(code: &str) -> Result<BTreeMap<String, UniformLayout>, Error> {
    let mut out = BTreeMap::new();
    let bytes = code.as_bytes();
    let mut i = 0usize;
    while let Some(rel) = code[i..].find("struct") {
        let start = i + rel;
        // `struct` must be a whole word.
        if start > 0 && is_ident(code.as_bytes()[start - 1] as char) {
            i = start + 6;
            continue;
        }
        let mut p = start + 6;
        p += code[p..].chars().take_while(|c| c.is_whitespace()).map(char::len_utf8).sum::<usize>();
        let name_start = p;
        while p < bytes.len() && is_ident(bytes[p] as char) {
            p += 1;
        }
        let name = &code[name_start..p];
        while p < bytes.len() && (bytes[p] as char).is_whitespace() {
            p += 1;
        }
        if p >= bytes.len() || bytes[p] != b'{' {
            i = start + 6;
            continue;
        }
        let body_start = p + 1;
        let Some(close_rel) = code[body_start..].find('}') else { break };
        let body = &code[body_start..body_start + close_rel];
        let clean: String = body.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
        let mut members = Vec::new();
        for part in clean.split([',', ';']) {
            if let Some(colon) = part.find(':') {
                let field_name: String = part[..colon].chars().filter(|c| is_ident(*c)).collect();
                let ty = part[colon + 1..].trim().to_string();
                if !field_name.is_empty() && !ty.is_empty() {
                    members.push((field_name, ty));
                }
            }
        }
        let mut layout = layout_fields(&members)?;
        layout.struct_name = Some(name.to_string());
        out.insert(name.to_string(), layout);
        i = body_start + close_rel + 1;
    }
    Ok(out)
}

fn parse_meta(code: &str) -> Meta {
    let mut meta = Meta::default();
    for line in code.lines() {
        let Some(at) = line.find("// @fx ") else { continue };
        let rest = line[at + 7..].trim();
        let (key, tail) = match rest.split_once(char::is_whitespace) {
            Some((k, t)) => (k, t.trim()),
            None => (rest, ""),
        };
        match key {
            "id" => meta.id = tail.to_string(),
            "label" => meta.label = tail.trim_matches('"').to_string(),
            "exports" => meta.exports.extend(tail.split_whitespace().map(String::from)),
            "hide" => meta.hides.extend(tail.split_whitespace().map(String::from)),
            "group" => {
                let mut it = tail.split_whitespace();
                if let (Some(field), Some(group)) = (it.next(), it.next()) {
                    meta.groups.insert(field.to_string(), group.to_string());
                }
            }
            "data" => {
                if let Some((field, expr)) = tail.split_once('=') {
                    meta.data.push((field.trim().to_string(), expr.trim().to_string()));
                }
            }
            "param" | "host" => {
                if let Some(spec) = parse_param(tail) {
                    if key == "host" {
                        meta.hosts.insert(spec.0.clone(), spec.1);
                    } else {
                        meta.params.insert(spec.0.clone(), spec.1);
                    }
                }
            }
            _ => {}
        }
    }
    meta
}

/// `@fx param <field> <min> <max> <step> <default> "<label>" [map]`
fn parse_param(tail: &str) -> Option<(String, ParamDecl)> {
    let mut it = tail.splitn(6, char::is_whitespace).filter(|s| !s.is_empty());
    let field = it.next()?.to_string();
    let _ty = it.next()?;
    let min = it.next()?.parse().ok()?;
    let max = it.next()?.parse().ok()?;
    let step = it.next()?.parse().ok()?;
    let rest = it.next().unwrap_or("");
    let (default, label, map) = if let Some(open) = rest.find('"') {
        let default = rest[..open].trim().parse().ok()?;
        let after = &rest[open + 1..];
        let close = after.find('"')?;
        let label = after[..close].to_string();
        let map = after[close + 1..].trim().to_string();
        (default, label, map)
    } else {
        (rest.parse().ok()?, String::new(), String::new())
    };
    Some((field, (min, max, step, default, label, map)))
}

fn parse_entries(code: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for (stage, marker) in [("vertex", "@vertex"), ("fragment", "@fragment"), ("compute", "@compute")] {
        let mut i = 0;
        while let Some(rel) = code[i..].find(marker) {
            let start = i + rel;
            let tail = &code[start + marker.len()..];
            if let Some(fn_rel) = tail.find("fn ") {
                let p = start + marker.len() + fn_rel + 3;
                let name: String = code[p..].chars().take_while(|c| is_ident(*c)).collect();
                if !name.is_empty() {
                    out.push(Entry { stage: stage.to_string(), name: name.clone(), generic: GENERIC_ENTRIES.contains(&name.as_str()) });
                }
            }
            i = start + marker.len();
        }
    }
    out
}

fn parse_number(code: &str, from: usize, prefix: &str) -> Option<(u32, usize)> {
    let p = from + prefix.len();
    let digits: String = code[p..].chars().take_while(|c| c.is_ascii_digit()).collect();
    let v = digits.parse().ok()?;
    Some((v, p + digits.len()))
}

fn parse_bindings(code: &str, structs: &BTreeMap<String, UniformLayout>) -> Result<(Vec<Binding>, BTreeMap<String, UniformLayout>), Error> {
    let mut bindings = Vec::new();
    let mut uniforms = BTreeMap::new();
    let mut i = 0usize;
    while let Some(rel) = code[i..].find("@group(") {
        let start = i + rel;
        let Some((group, after_group)) = parse_number(code, start, "@group(") else { break };
        let Some(binding_rel) = code[after_group..].find("@binding(") else { break };
        let binding_at = after_group + binding_rel;
        if binding_at > start + 80 {
            i = start + 8;
            continue;
        }
        let Some((binding, after_binding)) = parse_number(code, binding_at, "@binding(") else { break };
        let rest = &code[after_binding..];
        let Some(var_rel) = rest.find("var") else { break };
        let var_at = after_binding + var_rel;
        let uniform = code[var_at..].starts_with("var<uniform>");
        let name_at = var_at + if uniform { 13 } else { 3 };
        let name: String = code[name_at..].chars().skip_while(|c| c.is_whitespace()).take_while(|c| is_ident(*c)).collect();
        let Some(colon_rel) = code[name_at..].find(':') else { break };
        let ty_at = name_at + colon_rel + 1;
        let semi = code[ty_at..].find(';').unwrap_or(80).min(80);
        let ty = code[ty_at..ty_at + semi].trim().to_string();
        if name.is_empty() || ty.is_empty() {
            i = start + 8;
            continue;
        }
        bindings.push(Binding { group, binding, name: name.clone(), ty: ty.clone(), uniform, generic: GENERIC_BINDINGS.contains(&name.as_str()) });
        if uniform {
            let mut layout = if let Some(s) = structs.get(&ty) {
                s.clone()
            } else {
                let mut bare = layout_fields(&[(name.clone(), ty.clone())])?;
                bare.bare = true;
                bare
            };
            layout.struct_name = Some(ty.clone());
            uniforms.insert(name, layout);
        }
        i = ty_at + semi + 1;
    }
    Ok((bindings, uniforms))
}

/// Scan WGSL source without a GPU. Pure text parsing, safe on every target.
pub fn scan_source(code: &str) -> Result<Scan, Error> {
    let structs = parse_structs(code)?;
    let (bindings, uniforms) = parse_bindings(code, &structs)?;
    let entries = parse_entries(code);
    let meta = parse_meta(code);

    let bound: BTreeSet<String> = meta
        .data
        .iter()
        .map(|(f, _)| f.split('.').take(2).collect::<Vec<_>>().join("."))
        .chain(meta.params.keys().cloned())
        .chain(meta.hosts.keys().cloned())
        .collect();
    let mut extra_fields = Vec::new();
    for (var, layout) in &uniforms {
        for f in &layout.fields {
            let path = if layout.bare { var.clone() } else { format!("{var}.{}", f.name) };
            let generic = GENERIC_EXPORTS.iter().any(|g| *g == f.name);
            if generic || bound.contains(&path) || meta.hides.contains(&path) || f.size > 16 {
                continue;
            }
            extra_fields.push(path);
        }
    }

    Ok(Scan {
        id: meta.id.clone(),
        label: meta.label.clone(),
        meta,
        generic_entries: entries.iter().filter(|e| e.generic).map(|e| e.name.clone()).collect(),
        extra_entries: entries.iter().filter(|e| !e.generic).map(|e| e.name.clone()).collect(),
        extra_bindings: bindings.iter().filter(|b| !b.generic).map(|b| b.name.clone()).collect(),
        entries,
        bindings,
        uniforms,
        extra_fields,
    })
}

// ---------------------------------------------------------------------------
//  Expression evaluation (no eval, no dependencies)
// ---------------------------------------------------------------------------

struct Expr<'a> {
    toks: Vec<String>,
    pos: usize,
    v: f64,
    exports: &'a EngineExports,
}

fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
            let mut t = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                t.push(chars[i]);
                i += 1;
            }
            out.push(t);
            continue;
        }
        out.push(c.to_string());
        i += 1;
    }
    out
}

impl<'a> Expr<'a> {
    fn peek(&self) -> &str {
        self.toks.get(self.pos).map(String::as_str).unwrap_or("")
    }
    fn take(&mut self) -> String {
        let t = self.peek().to_string();
        self.pos += 1;
        t
    }
    fn primary(&mut self) -> Result<f64, Error> {
        let t = self.take();
        if t == "-" {
            return Ok(-self.primary()?);
        }
        if t == "+" {
            return self.primary();
        }
        if t == "(" {
            let v = self.sum()?;
            if self.peek() == ")" {
                self.pos += 1;
            }
            return Ok(v);
        }
        if let Ok(n) = t.parse::<f64>() {
            return Ok(n);
        }
        if t == "v" {
            return Ok(self.v);
        }
        if t == "PI" {
            return Ok(std::f64::consts::PI);
        }
        if t == "E" {
            return Ok(std::f64::consts::E);
        }
        if self.peek() == "(" {
            self.pos += 1;
            let mut args = Vec::new();
            if self.peek() != ")" {
                loop {
                    args.push(self.sum()?);
                    if self.peek() == "," {
                        self.pos += 1;
                        continue;
                    }
                    break;
                }
            }
            if self.peek() == ")" {
                self.pos += 1;
            }
            let a = |i: usize| args.get(i).copied().unwrap_or(0.0);
            return Ok(match t.as_str() {
                "min" => a(0).min(a(1)),
                "max" => a(0).max(a(1)),
                "clamp" => a(0).max(a(1)).min(a(2)),
                "abs" => a(0).abs(),
                "sqrt" => a(0).max(0.0).sqrt(),
                "floor" => a(0).floor(),
                "ceil" => a(0).ceil(),
                _ => return Err(Error::Parse(format!("unknown function: {t}"))),
            });
        }
        // An export followed by a swizzle (`nodeSize.y`, `quad.w`) selects one component. This is
        // what lets a 0..100 control express a ratio of the component's own size instead of an
        // absolute pixel value.
        if let Some((name, swizzle)) = t.split_once('.') {
            if let Some(vals) = self.exports.get(name) {
                let idx = match swizzle {
                    "x" | "r" => 0,
                    "y" | "g" => 1,
                    "z" | "b" => 2,
                    "w" | "a" => 3,
                    _ => return Err(Error::Parse(format!("bad export swizzle: {swizzle}"))),
                };
                return Ok(vals.get(idx).copied().unwrap_or(0.0));
            }
        }
        if let Some(vals) = self.exports.get(&t) {
            return Ok(vals.first().copied().unwrap_or(0.0));
        }
        Err(Error::Parse(format!("unknown name in expression: {t}")))
    }
    fn product(&mut self) -> Result<f64, Error> {
        let mut x = self.primary()?;
        while self.peek() == "*" || self.peek() == "/" {
            let op = self.take();
            let y = self.primary()?;
            if op == "*" {
                x *= y;
            } else if y.abs() > 1e-12 {
                x /= y;
            }
        }
        Ok(x)
    }
    fn sum(&mut self) -> Result<f64, Error> {
        let mut x = self.product()?;
        while self.peek() == "+" || self.peek() == "-" {
            let op = self.take();
            let y = self.product()?;
            if op == "+" {
                x += y;
            } else {
                x -= y;
            }
        }
        Ok(x)
    }
}

pub fn eval_expr(expr: &str, v: f64, exports: &EngineExports) -> Result<f64, Error> {
    if expr.trim().is_empty() {
        return Ok(v);
    }
    let mut e = Expr { toks: tokenize(expr), pos: 0, v, exports };
    let value = e.sum()?;
    if !value.is_finite() {
        return Err(Error::Parse(format!("non-finite result: {expr}")));
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
//  Registry
// ---------------------------------------------------------------------------

impl WgslEffects {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a generic engine data slot (or a custom one) that effects can read.
    pub fn set_export(&mut self, name: impl Into<String>, value: impl Into<Vec<f64>>) {
        self.exports.set(name, value);
    }
    pub fn exports(&self) -> &EngineExports {
        &self.exports
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
    pub fn effect_ids(&self) -> Vec<String> {
        self.effects.keys().cloned().collect()
    }

    /// Declare an effect before loading it. `id` is the name the UI and commands use.
    pub fn define(&mut self, id: &str, source: Source) {
        if !self.effects.contains_key(id) {
            self.effects.insert(id.to_string(), Loaded { source, code: String::new(), scan: empty_scan(id), params: BTreeMap::new() });
        }
    }

    /// Load (or reload) an effect from a local `.wgsl` file. Transactional: a parse error or a
    /// missing file leaves the previously loaded effect untouched.
    pub fn load(&mut self, id: &str, source: Source) -> Result<(), Error> {
        let code = read_source(&source)?;
        let scan = scan_source(&code)?;
        let mut params = BTreeMap::new();
        for spec in param_specs(&scan) {
            let old = self.effects.get(id).and_then(|l| l.params.get(&spec.field)).copied();
            params.insert(spec.field, old.unwrap_or(spec.default));
        }
        self.effects.insert(id.to_string(), Loaded { source, code, scan, params });
        self.last_error = None;
        self.status = format!("{id}: {} bytes, {} controls", self.effects[id].code.len(), self.effects[id].params.len());
        Ok(())
    }

    /// Re-read the current source (a `.wgsl` file edited on disk takes effect here).
    pub fn reload(&mut self, id: &str) -> Result<(), Error> {
        let source = self.effects.get(id).map(|l| l.source.clone()).ok_or_else(|| Error::NotLoaded(id.to_string()))?;
        self.load(id, source)
    }

    pub fn set_path(&mut self, id: &str, path: impl Into<PathBuf>) {
        if let Some(l) = self.effects.get_mut(id) {
            l.source = Source::Path(path.into());
        } else {
            self.define(id, Source::Path(path.into()));
        }
    }

    pub fn scan(&self, id: &str) -> Option<&Scan> {
        self.effects.get(id).map(|l| &l.scan)
    }
    /// Raw WGSL source, for a GPU backend that compiles it into its own pipeline.
    pub fn source_code(&self, id: &str) -> Option<&str> {
        self.effects.get(id).map(|l| l.code.as_str())
    }
    pub fn source(&self, id: &str) -> Option<&Source> {
        self.effects.get(id).map(|l| &l.source)
    }

    /// Controls the UI builds when this effect is added: declared params, host params, and every
    /// extra field the scanner found that is not a generic engine export.
    pub fn params(&self, id: &str) -> Vec<ParamSpec> {
        self.effects.get(id).map(|l| param_specs(&l.scan)).unwrap_or_default()
    }
    pub fn param_value(&self, id: &str, field: &str) -> Option<f64> {
        self.effects.get(id).and_then(|l| l.params.get(field)).copied()
    }
    pub fn set_param(&mut self, id: &str, field: &str, value: f64) -> Result<(), Error> {
        let l = self.effects.get_mut(id).ok_or_else(|| Error::NotLoaded(id.to_string()))?;
        if !l.params.contains_key(field) {
            return Err(Error::Parse(format!("unknown parameter: {field}")));
        }
        // The HTML demo's sliders are bounded (`min`/`max` on `<input type=range>`); keep the
        // stored UI value inside the same bounds so a typed/scrubbed value can never leave the
        // declared 0..100 (or 1..100) scale.
        let clamped = param_specs(&l.scan).into_iter().find(|p| p.field == field)
            .map_or(value, |p| if p.min <= p.max { value.clamp(p.min, p.max) } else { value });
        l.params.insert(field.to_string(), clamped);
        Ok(())
    }

    /// The value after the parameter's `map` expression: what actually lands in the uniform, and
    /// what a CPU renderer should use so the visible effect stays in step with the shader.
    pub fn param_uniform_value(&self, id: &str, field: &str) -> Result<f64, Error> {
        let l = self.effects.get(id).ok_or_else(|| Error::NotLoaded(id.to_string()))?;
        let spec = param_specs(&l.scan).into_iter().find(|p| p.field == field)
            .ok_or_else(|| Error::Parse(format!("unknown parameter: {field}")))?;
        let v = l.params.get(field).copied().unwrap_or(spec.default);
        eval_expr(&spec.map, v, &self.exports)
    }

    /// Pack one uniform buffer with the engine-wide exports.
    pub fn uniform_bytes(&self, id: &str, var: &str) -> Result<Vec<u8>, Error> {
        self.uniform_bytes_for(id, var, &self.exports)
    }

    /// Pack one uniform buffer with a caller-supplied export table. A material channel uses this
    /// to give each node its own geometry (size, position, zoom) without a single global table.
    pub fn uniform_bytes_for(&self, id: &str, var: &str, exports: &EngineExports) -> Result<Vec<u8>, Error> {
        let l = self.effects.get(id).ok_or_else(|| Error::NotLoaded(id.to_string()))?;
        let layout = l.scan.uniforms.get(var).ok_or_else(|| Error::Parse(format!("no uniform {var}")))?;
        let mut buf = vec![0u8; layout.size.max(16)];
        for (path, expr) in &l.scan.meta.data {
            if path.split('.').next() != Some(var) {
                continue;
            }
            let value = eval_expr(expr, 0.0, exports)?;
            write_field(&mut buf, layout, path, &[value], &l.scan)?;
        }
        for spec in param_specs(&l.scan) {
            if spec.host || spec.field.split('.').next() != Some(var) {
                continue;
            }
            let v = l.params.get(&spec.field).copied().unwrap_or(spec.default);
            let value = eval_expr(&spec.map, v, exports)?;
            write_field(&mut buf, layout, &spec.field, &[value], &l.scan)?;
        }
        // A field with an explicit `@fx param`/`@fx host` declaration owns its value: the engine's
        // generic export must not overwrite what the control's `map` expression just wrote.
        let declared: Vec<String> = param_specs(&l.scan).into_iter().filter(|p| !p.auto)
            .map(|p| p.field).collect();
        // Generic fields whose name matches an engine export are filled automatically.
        for f in &layout.fields {
            let path = if layout.bare { var.to_string() } else { format!("{var}.{}", f.name) };
            if !GENERIC_EXPORTS.contains(&f.name.as_str()) {
                continue;
            }
            if l.scan.meta.data.iter().any(|(p, _)| p == &path) || declared.iter().any(|p| p == &path) {
                continue;
            }
            if let Some(vals) = exports.get(&f.name) {
                write_field(&mut buf, layout, &path, vals, &l.scan)?;
            }
        }
        Ok(buf)
    }

    /// A serialisable summary for the control channel and tests.
    pub fn snapshot(&self, id: &str) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let Some(l) = self.effects.get(id) else { return out };
        out.insert("id".into(), id.into());
        out.insert("bytes".into(), l.code.len().to_string());
        out.insert("generic_entries".into(), l.scan.generic_entries.join(","));
        out.insert("extra_entries".into(), l.scan.extra_entries.join(","));
        out.insert("extra_fields".into(), l.scan.extra_fields.join(","));
        out.insert("params".into(), l.params.len().to_string());
        out.insert("uniforms".into(), l.scan.uniforms.keys().cloned().collect::<Vec<_>>().join(","));
        out.insert("status".into(), self.status.clone());
        out
    }
}

fn empty_scan(id: &str) -> Scan {
    Scan {
        id: id.to_string(),
        label: String::new(),
        meta: Meta::default(),
        entries: Vec::new(),
        bindings: Vec::new(),
        uniforms: BTreeMap::new(),
        generic_entries: Vec::new(),
        extra_entries: Vec::new(),
        extra_bindings: Vec::new(),
        extra_fields: Vec::new(),
    }
}

fn field_of<'a>(layout: &'a UniformLayout, path: &str, var: &str) -> Result<(&'a Field, String), Error> {
    let parts: Vec<&str> = path.split('.').collect();
    let field = if layout.bare { layout.fields.first() } else { parts.get(1).and_then(|n| layout.fields.iter().find(|f| f.name == *n)) };
    let f = field.ok_or_else(|| Error::Parse(format!("unknown uniform field: {path} (var {var})")))?;
    let swizzle = parts.get(2).copied().unwrap_or("").to_string();
    Ok((f, swizzle))
}

fn write_field(buf: &mut [u8], layout: &UniformLayout, path: &str, values: &[f64], _scan: &Scan) -> Result<(), Error> {
    let var = path.split('.').next().unwrap_or("");
    let (f, swizzle) = field_of(layout, path, var)?;
    let vals: Vec<f64> = if values.is_empty() { vec![0.0] } else { values.to_vec() };
    let mut indices: Vec<usize> = (0..f.components).collect();
    if !swizzle.is_empty() {
        indices.clear();
        for ch in swizzle.chars() {
            let idx = match ch {
                'x' | 'r' => 0,
                'y' | 'g' => 1,
                'z' | 'b' => 2,
                'w' | 'a' => 3,
                _ => return Err(Error::Parse(format!("bad swizzle: {swizzle}"))),
            };
            indices.push(idx);
        }
    }
    for (k, idx) in indices.iter().enumerate() {
        let at = f.offset.saturating_add(idx * 4);
        if at + 4 > buf.len() {
            continue;
        }
        let x = *vals.get(k % vals.len()).unwrap_or(&0.0);
        let bytes = match f.scalar {
            Scalar::F32 => (x as f32).to_le_bytes(),
            Scalar::I32 => (x as i32).to_le_bytes(),
            Scalar::U32 => (x as u32).to_le_bytes(),
        };
        if let Some(dst) = buf.get_mut(at..at + 4) {
            dst.copy_from_slice(&bytes);
        }
    }
    Ok(())
}

fn param_specs(scan: &Scan) -> Vec<ParamSpec> {
    let mut specs = Vec::new();
    let mut seen = BTreeSet::new();
    for (field, (min, max, step, default, label, map)) in &scan.meta.params {
        specs.push(ParamSpec {
            field: field.clone(),
            label: if label.is_empty() { field.clone() } else { label.clone() },
            min: *min,
            max: *max,
            step: *step,
            default: *default,
            map: map.clone(),
            group: scan.meta.groups.get(field).cloned().unwrap_or_else(|| "default".into()),
            auto: false,
            host: false,
        });
        seen.insert(field.clone());
    }
    for (field, (min, max, step, default, label, map)) in &scan.meta.hosts {
        specs.push(ParamSpec {
            field: field.clone(),
            label: if label.is_empty() { field.clone() } else { label.clone() },
            min: *min,
            max: *max,
            step: *step,
            default: *default,
            map: map.clone(),
            group: scan.meta.groups.get(field).cloned().unwrap_or_else(|| "default".into()),
            auto: false,
            host: true,
        });
        seen.insert(field.clone());
    }
    for path in &scan.extra_fields {
        if seen.contains(path) {
            continue;
        }
        let int_field = scan.uniforms.iter().any(|(_, l)| {
            l.fields.iter().any(|f| format!("{}.{}", path.split('.').next().unwrap_or(""), f.name) == *path && f.scalar != Scalar::F32)
        });
        // Like the HTML demo's `<input type=range>` rows: scanned fields use an intuitive 0..100
        // (integers 1..100, angles 0..360) scale instead of raw 0..1 shader values, and `map`
        // turns that UI value into the number actually written to the uniform. An explicit
        // `@fx param` declaration always overrides these defaults.
        let lower = path.rsplit('.').next().unwrap_or(path).to_ascii_lowercase();
        let angle_field = lower.contains("angle");
        let (min, max, step, default, map) = if int_field {
            (1.0, 100.0, 1.0, 1.0, String::new())
        } else if angle_field {
            (0.0, 360.0, 1.0, 0.0, "v*PI/180".to_string())
        } else {
            (0.0, 100.0, 1.0, 0.0, "v/100".to_string())
        };
        specs.push(ParamSpec {
            field: path.clone(),
            label: path.clone(),
            min, max, step, default, map,
            group: scan.meta.groups.get(path).cloned().unwrap_or_else(|| "default".into()),
            auto: true,
            host: false,
        });
        seen.insert(path.clone());
    }
    specs
}

#[cfg(not(target_arch = "wasm32"))]
fn read_source(source: &Source) -> Result<String, Error> {
    match source {
        Source::Path(p) => std::fs::read_to_string(p).map_err(|e| Error::Io(format!("{}: {e}", p.display()))),
        Source::Inline(s) => Ok(s.clone()),
    }
}

#[cfg(target_arch = "wasm32")]
fn read_source(source: &Source) -> Result<String, Error> {
    match source {
        Source::Path(p) => Err(Error::Unsupported(format!("no filesystem on wasm: {}", p.display()))),
        Source::Inline(s) => Ok(s.clone()),
    }
}

/// A host can point the engine at a default effect file with `FIGCRAFT_WGSL_EFFECT`, so a WGSL
/// effect loads at startup without typing a path. On wasm there is no environment to read.
pub fn default_effect_path() -> Option<PathBuf> {
    std::env::var_os("FIGCRAFT_WGSL_EFFECT").map(PathBuf::from)
}

/// The engine-wide registry. The UI, commands and the renderer all talk to this one instance.
pub fn shared() -> &'static Mutex<WgslEffects> {
    static INSTANCE: OnceLock<Mutex<WgslEffects>> = OnceLock::new();
    INSTANCE.get_or_init(|| Mutex::new(WgslEffects::new()))
}

/// Convenience for hosts that keep the effect file at a fixed path.
pub fn load_from_path(id: &str, path: &Path) -> Result<(), Error> {
    let mut registry = shared().lock().unwrap_or_else(|e| e.into_inner());
    registry.load(id, Source::Path(path.to_path_buf()))
}
