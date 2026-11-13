//! Per-node WGSL material binding: data model, fixed-directory discovery, and persistence.
//!
//! A binding is stored in `figcraft_doc::Node::plugin_data` under [`BINDING_KEY`] as a compact JSON
//! string, so it rides along with the document (save / load / undo / dirty) without touching
//! `EffectKind` (whose exhaustive matches span doc / pdf / fig + penpot export / vectordrawable).
//! External `.wgsl` files stay outside the repo: the document only holds a path reference.
//!
//! The GPU channel (`gpu.rs`, `--features gpu`) reads a binding through [`read_binding`] and feeds
//! `figcraft_render::wgsl_effect` to build the material pipeline. This module is intentionally
//! free of `wgpu` so the panel can list files / edit the binding even when the GPU backend is off.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use figcraft_doc::{Document, Node, NodeId};
use figcraft_render::wgsl_effect as fx;
use serde_json::{Value, json};

/// `plugin_data` key holding the compact JSON of a [`WgslMaterialBinding`].
pub const BINDING_KEY: &str = "figcraft.wgslMaterial";

/// Environment variable pointing at the folder scanned for `.wgsl` effects.
pub const DIR_ENV: &str = "FIGCRAFT_WGSL_DIR";

/// One node's material choice: the shader file plus the user's parameter values (0..100 scale,
/// matching `wgsl_effect::ParamSpec`). Kept as plain data so it can live in `plugin_data`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WgslMaterialBinding {
    /// Local path to the `.wgsl` file (absolute, or as the user picked it).
    pub path: String,
    /// Control values keyed by the uniform path, e.g. `U.gain`.
    pub params: BTreeMap<String, f64>,
}

impl WgslMaterialBinding {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into(), params: BTreeMap::new() }
    }

    /// Attach control values (`field` → 0..100 UI value), returning `self` for chaining.
    pub fn with_params(mut self, params: BTreeMap<String, f64>) -> Self {
        self.params = params;
        self
    }

    /// Encode to the compact string stored in `plugin_data`.
    pub fn to_json(&self) -> String {
        let params: BTreeMap<&str, f64> = self.params.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        json!({ "path": self.path, "params": params }).to_string()
    }

    /// Decode a binding from the string stored in `plugin_data`. Malformed data yields `None`
    /// rather than panicking, so a hand-edited document can't take the panel down.
    pub fn from_json(s: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(s).ok()?;
        let path = v.get("path")?.as_str()?.to_string();
        let mut params = BTreeMap::new();
        if let Some(o) = v.get("params").and_then(Value::as_object) {
            for (k, val) in o {
                if let Some(n) = val.as_f64() {
                    params.insert(k.clone(), n);
                }
            }
        }
        Some(Self { path, params })
    }

    /// A stable registry id for a path. The GPU channel and the parameter panel must agree on it.
    pub fn effect_id(&self) -> String {
        effect_id(&self.path)
    }
}

/// The registry id `wgsl_effect::shared()` uses for a given file path.
pub fn effect_id(path: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    format!("material:{:x}", h.finish())
}

/// Read the binding of one node, if any.
pub fn read_binding(doc: &Document, id: NodeId) -> Option<WgslMaterialBinding> {
    let n: &Node = doc.node(id)?;
    let s = n.plugin_data.get(BINDING_KEY)?;
    WgslMaterialBinding::from_json(s)
}

/// Read the binding of the first selected node that has one.
pub fn read_binding_of(doc: &Document, ids: &[NodeId]) -> Option<WgslMaterialBinding> {
    ids.iter().find_map(|id| read_binding(doc, *id))
}

/// Build the `node.set` params that write a binding. `None` removes it (writes JSON `null`).
pub fn write_binding_params(binding: Option<&WgslMaterialBinding>) -> Value {
    let value = binding.map(WgslMaterialBinding::to_json);
    json!({ "props": { "pluginData": { BINDING_KEY: value } } })
}

/// Make sure the shader at `path` is scanned into the shared registry, and return its registry id.
///
/// Both the panel (which needs [`fx::ParamSpec`]s to build input rows) and the GPU channel (which
/// needs the source + uniform layout) go through this so they agree on one id per path. A parse
/// error leaves the registry untouched and is reported through `WgslEffects::last_error`, never
/// panicking — a half-written `.wgsl` on disk can't take the panel down.
pub fn ensure_loaded(path: &str) -> String {
    let id = effect_id(path);
    let mut reg = fx::shared().lock().unwrap_or_else(|e| e.into_inner());
    if reg.scan(&id).is_none() {
        let _ = reg.load(&id, fx::Source::Path(std::path::PathBuf::from(path)));
    }
    id
}

/// The user-facing controls declared by a shader, seeded with their declared defaults.
///
/// Returns an empty list for a missing / unparseable file; callers simply render no param rows.
pub fn params_for(path: &str) -> Vec<fx::ParamSpec> {
    let id = ensure_loaded(path);
    let reg = fx::shared().lock().unwrap_or_else(|e| e.into_inner());
    reg.params(&id)
}

/// A fresh binding for `path` with every control at its declared default (host controls excluded:
/// they are consumed by the host, not written by the user).
pub fn binding_for(path: &str) -> WgslMaterialBinding {
    let params = params_for(path).into_iter().filter(|p| !p.host).map(|p| (p.field, p.default)).collect();
    WgslMaterialBinding::new(path).with_params(params)
}

/// Pack one node's control values into its `U` uniform **in isolation**.
///
/// The shared registry is global, so writing a node's params into it would let two nodes that use
/// the same shader bleed values into each other. Instead clone the registry (`WgslEffects: Clone`),
/// apply this binding's params to the clone, and pack from the clone. Returns the registry id, the
/// packed `U` bytes, and the `frost` host-control value the offscreen blur chain needs.
///
/// `set_param` clamps to each control's min/max and ignores stale keys from an older shader
/// revision, so a hand-edited document can't produce an out-of-range uniform.
pub fn pack_node_uniforms(binding: &WgslMaterialBinding, exports: &fx::EngineExports) -> (String, Vec<u8>, f64) {
    let id = binding.effect_id();
    ensure_loaded(&binding.path);
    let mut local = fx::shared().lock().unwrap_or_else(|e| e.into_inner()).clone();
    for (field, value) in &binding.params {
        let _ = local.set_param(&id, field, *value);
    }
    let u = local.uniform_bytes_for(&id, "U", exports).unwrap_or_default();
    // `frost` is a host control consumed by the blur chain, not a uniform write. Its params key is
    // the full uniform path (`U.frost`), so resolve by suffix — `set_param` above already applied
    // the user's edit to the clone; `param_value` then reads the edited value or the seeded default.
    let frost = local.params(&id).into_iter().find(|p| p.field.ends_with("frost")).and_then(|p| local.param_value(&id, &p.field)).unwrap_or(0.0);
    (id, u, frost)
}

/// One discoverable shader in the fixed directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WgslFile {
    /// Display name (file stem, or file name when stems collide).
    pub name: String,
    /// Full local path handed to `wgsl_effect::Source::Path`.
    pub path: PathBuf,
}

/// The folder scanned for `.wgsl` files: `$FIGCRAFT_WGSL_DIR`, else `<cwd>/assets/wgsl`.
pub fn material_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(DIR_ENV) {
        let p = PathBuf::from(dir);
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    PathBuf::from("assets").join("wgsl")
}

/// List `.wgsl` files in [`material_dir`], sorted by name. The directory may not exist yet (an
/// unset-up install); that simply yields an empty list rather than an error.
pub fn scan_material_dir() -> Vec<WgslFile> {
    scan_dir(&material_dir())
}

/// List `.wgsl` files in an explicit directory, sorted by name. Symlinked entries are skipped.
pub fn scan_dir(dir: &Path) -> Vec<WgslFile> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<WgslFile> = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wgsl")) {
            continue;
        }
        // Avoid following links into the user's machine from a folder they didn't intend.
        if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
            continue;
        }
        if !p.is_file() {
            continue;
        }
        let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned());
        files.push(WgslFile { name, path: p });
    }
    files.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    // Disambiguate equal stems ("a.fractal.wgsl" vs "a.blur.wgsl" both stem "a"): use file name.
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for f in &files {
        *counts.entry(f.name.clone()).or_default() += 1;
    }
    for f in &mut files {
        if counts.get(&f.name).copied().unwrap_or(0) > 1 {
            f.name = f.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| f.name.clone());
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_round_trips_through_json() {
        let mut b = WgslMaterialBinding::new("D:/shaders/glass.wgsl");
        b.params.insert("U.gain".into(), 42.5);
        b.params.insert("U.frost".into(), 0.0);
        let s = b.to_json();
        let back = WgslMaterialBinding::from_json(&s).expect("valid binding");
        assert_eq!(back, b);
    }

    #[test]
    fn malformed_binding_is_ignored() {
        assert!(WgslMaterialBinding::from_json("not json").is_none());
        assert!(WgslMaterialBinding::from_json(r#"{"params":{}}"#).is_none(), "path required");
        assert!(WgslMaterialBinding::from_json(r#"{"path":"a","params":{"x":"bad"}}"#).is_some(), "bad values skipped");
    }

    #[test]
    fn write_params_use_the_reserved_key() {
        let b = WgslMaterialBinding::new("x.wgsl");
        let v = write_binding_params(Some(&b));
        assert!(v["props"]["pluginData"][BINDING_KEY].is_string());
        let v = write_binding_params(None);
        assert!(v["props"]["pluginData"][BINDING_KEY].is_null(), "None removes the binding");
    }

    #[test]
    fn scan_filters_sorts_and_disambiguates() {
        let tmp = std::env::temp_dir().join(format!("figcraft_wgsl_scan_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("z.wgsl"), "// z").unwrap();
        std::fs::write(tmp.join("a.wgsl"), "// a").unwrap();
        std::fs::write(tmp.join("notes.txt"), "ignore").unwrap();
        std::fs::create_dir_all(tmp.join("sub.wgsl")).unwrap(); // a directory with a .wgsl name
        let got = scan_dir(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);
        assert_eq!(got.len(), 2, "only real .wgsl files, no dirs or .txt");
        assert_eq!(got[0].name, "a");
        assert_eq!(got[1].name, "z");
    }

    const SHADER: &str = r#"
// @fx id t
// @fx label Test
// @fx param U.gain float 0 100 1 50 "Gain" v/100
// @fx host U.frost float 0 100 1 20 "Frost" v/100
struct U { gain : f32, frost : f32, };
@group(0) @binding(0) var<uniform> U : U;
@vertex fn vs_full(@builtin(vertex_index) vi : u32) -> vec4<f32> { return vec4<f32>(0.0); }
@fragment fn fs_glass() -> @location(0) vec4<f32> { return vec4<f32>(U.gain); }
"#;

    #[test]
    fn binding_for_seeds_declared_defaults_and_skips_hosts() {
        let tmp = std::env::temp_dir().join(format!("figcraft_wgsl_bind_{}.wgsl", std::process::id()));
        std::fs::write(&tmp, SHADER).unwrap();
        let b = binding_for(&tmp.to_string_lossy());
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(b.path, tmp.to_string_lossy());
        assert_eq!(b.params.get("U.gain"), Some(&50.0), "declared param seeded with its default");
        assert!(!b.params.contains_key("U.frost"), "host controls are consumed by the host, not stored");
    }

    #[test]
    fn pack_node_uniforms_isolates_params_and_reads_host_controls() {
        let tmp = std::env::temp_dir().join(format!("figcraft_wgsl_apply_{}.wgsl", std::process::id()));
        std::fs::write(&tmp, SHADER).unwrap();
        let path = tmp.to_string_lossy().into_owned();
        let mut b = binding_for(&path);
        b.params.insert("U.gain".into(), 80.0);
        let (id, bytes, frost) = pack_node_uniforms(&b, &fx::EngineExports::default());
        let _ = std::fs::remove_file(&tmp);
        let gain = f32::from_le_bytes(bytes[0..4].try_into().unwrap());
        assert!((gain - 0.8).abs() < 1e-6, "80 -> map v/100 -> 0.8 lands in the uniform");
        assert!((frost - 20.0).abs() < 1e-6, "host control frost raw 0..100 value still readable for the blur chain");
        // Isolation: the shared registry is untouched by this node's edit.
        let reg = fx::shared().lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(reg.param_value(&id, "U.gain"), Some(50.0), "global registry keeps its default");
    }
}
