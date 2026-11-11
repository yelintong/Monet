//! Tests for the general WGSL effect interface. All shader text here is synthetic, written for
//! these tests; nothing is copied from any other product.

use crate::wgsl_effect::{self as w, Error, Source};

const SHADER: &str = r#"
// @fx id demo
// @fx label Demo Effect
// @fx exports nodeSize dpr
// @fx param U.gain float 0 100 1 50 "Gain" v/100
// @fx hide U.pad
struct Params {
  nodeSize : vec2<f32>,
  gain : f32,
  pad : f32,
  extra : f32,
};
@group(0) @binding(0) var<uniform> U : Params;
@group(0) @binding(1) var sceneTex : texture_2d<f32>;
@vertex fn vs_full(@builtin(vertex_index) vi : u32) -> vec4<f32> { return vec4<f32>(0.0); }
@fragment fn fs_effect(@location(0) uv : vec2<f32>) -> @location(0) vec4<f32> { return vec4<f32>(U.gain); }
"#;

fn f32_at(buf: &[u8], at: usize) -> f32 {
    let b: [u8; 4] = buf.get(at..at + 4).and_then(|s| s.try_into().ok()).unwrap_or([0; 4]);
    f32::from_le_bytes(b)
}

#[test]
fn scan_finds_entries_bindings_and_uniform_layout() -> Result<(), Error> {
    let scan = w::scan_source(SHADER)?;
    assert_eq!(scan.id, "demo");
    assert_eq!(scan.label, "Demo Effect");
    assert_eq!(scan.generic_entries, vec!["vs_full"]);
    assert_eq!(scan.extra_entries, vec!["fs_effect"]);
    assert!(scan.extra_bindings.is_empty(), "sceneTex is a generic binding");
    let u = scan.uniforms.get("U").ok_or_else(|| Error::Parse("no U".into()))?;
    assert_eq!(u.size, 32, "vec2(8) + gain(4) + pad(4) + extra(4) = 20, rounded to 32");
    let offsets: Vec<(&str, usize)> = u.fields.iter().map(|f| (f.name.as_str(), f.offset)).collect();
    assert_eq!(offsets, vec![("nodeSize", 0), ("gain", 8), ("pad", 12), ("extra", 16)]);
    assert_eq!(scan.extra_fields, vec!["U.extra"], "only the unbound, unhidden field");
    Ok(())
}

#[test]
fn extra_fields_become_controls_and_declared_params_keep_their_spec() -> Result<(), Error> {
    let mut fx = w::WgslEffects::new();
    fx.define("demo", Source::Inline(SHADER.into()));
    fx.load("demo", Source::Inline(SHADER.into()))?;
    let params = fx.params("demo");
    let gain = params.iter().find(|p| p.field == "U.gain").ok_or_else(|| Error::Parse("no gain".into()))?;
    assert!(!gain.auto);
    assert_eq!(gain.label, "Gain");
    assert_eq!((gain.min, gain.max, gain.step, gain.default), (0.0, 100.0, 1.0, 50.0));
    assert_eq!(gain.map, "v/100");
    let extra = params.iter().find(|p| p.field == "U.extra").ok_or_else(|| Error::Parse("no extra".into()))?;
    assert!(extra.auto, "scanner-discovered field gets a control without any metadata");
    Ok(())
}

#[test]
fn engine_exports_and_user_values_are_packed_at_wgsl_offsets() -> Result<(), Error> {
    let mut fx = w::WgslEffects::new();
    fx.define("demo", Source::Inline(SHADER.into()));
    fx.load("demo", Source::Inline(SHADER.into()))?;
    fx.set_export("nodeSize", vec![640.0, 360.0]);
    fx.set_param("demo", "U.gain", 80.0)?;
    let buf = fx.uniform_bytes("demo", "U")?;
    assert_eq!(buf.len(), 32);
    assert_eq!(f32_at(&buf, 0), 640.0, "generic export nodeSize.x");
    assert_eq!(f32_at(&buf, 4), 360.0, "generic export nodeSize.y");
    assert!((f32_at(&buf, 8) - 0.8).abs() < 1e-6, "gain 80 -> map v/100 -> 0.8");
    assert_eq!(f32_at(&buf, 12), 0.0, "hidden pad stays zero");
    fx.set_param("demo", "U.extra", 7.0)?;
    let buf2 = fx.uniform_bytes("demo", "U")?;
    // Scanned controls use the HTML demo's 0..100 scale with a v/100 map, not raw shader units.
    assert!((f32_at(&buf2, 16) - 0.07).abs() < 1e-6, "auto control 7 -> 0.07");
    Ok(())
}

#[test]
fn expressions_use_v_exports_and_functions() -> Result<(), Error> {
    let mut ex = w::EngineExports::default();
    ex.scalar("cw", 1070.0);
    ex.scalar("gw", 640.0);
    ex.scalar("dpr", 2.0);
    let a = w::eval_expr("max(1,v)*(cw/gw)", 30.0, &ex)?;
    assert!((a - 50.15625).abs() < 1e-9);
    assert!((w::eval_expr("v*PI/180", 315.0, &ex)? - 5.497787143782138).abs() < 1e-9);
    assert!((w::eval_expr("v*dpr", 40.0, &ex)? - 80.0).abs() < 1e-9);
    assert!((w::eval_expr("", 3.5, &ex)? - 3.5).abs() < 1e-9);
    assert!(w::eval_expr("nope + 1", 1.0, &ex).is_err());
    // Ratio against the component's own size (the demo's cw/gw), via export swizzles.
    ex.set("canvasSize", vec![1070.0, 620.0]);
    ex.set("nodeSize", vec![640.0, 360.0]);
    assert!((w::eval_expr("max(1,v)*canvasSize.x/nodeSize.x", 30.0, &ex)? - 50.15625).abs() < 1e-9);
    assert_eq!(w::eval_expr("nodeSize.y", 0.0, &ex)?, 360.0);
    assert!(w::eval_expr("canvasSize.q", 0.0, &ex).is_err());
    Ok(())
}

#[test]
fn a_failed_load_keeps_the_working_effect() -> Result<(), Error> {
    let mut fx = w::WgslEffects::new();
    fx.define("demo", Source::Inline(SHADER.into()));
    fx.load("demo", Source::Inline(SHADER.into()))?;
    let before = fx.snapshot("demo");
    let broken = "@group(0) @binding(0) var<uniform> U : NotAType;\n@fragment fn fs_x() -> f32 { return 1.0; }";
    assert!(fx.load("demo", Source::Inline(broken.into())).is_err());
    assert_eq!(fx.snapshot("demo"), before, "transactional: no half-loaded shader");
    assert_eq!(fx.param_value("demo", "U.gain"), Some(50.0));
    assert_eq!(f32_at(&fx.uniform_bytes("demo", "U")?, 0), 0.0);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn reload_picks_up_an_edited_local_wgsl_file() -> Result<(), Error> {
    let path = std::env::temp_dir().join(format!("figcraft_wgsl_effect_{}.wgsl", std::process::id()));
    std::fs::write(&path, SHADER).map_err(|e| Error::Io(e.to_string()))?;
    let mut fx = w::WgslEffects::new();
    fx.define("demo", Source::Path(path.clone()));
    let first = fx.load("demo", Source::Path(path.clone()));
    let edited = format!("{SHADER}\n// edited\n");
    std::fs::write(&path, &edited).map_err(|e| Error::Io(e.to_string()))?;
    let second = fx.reload("demo");
    let _ = std::fs::remove_file(&path);
    first?;
    second?;
    let snap = fx.snapshot("demo");
    let bytes: usize = snap.get("bytes").and_then(|s| s.parse().ok()).unwrap_or(0);
    assert_eq!(bytes, edited.len(), "reload re-reads the file on disk");
    assert_eq!(fx.params("demo").len(), 2);
    Ok(())
}

#[test]
fn a_full_mixed_uniform_block_keeps_wgsl_alignments() -> Result<(), Error> {
    // Synthetic struct with the same type sequence as a full glass uniform: two mat3x3, five
    // vec4, a vec2, then a run of scalars and i32s. No product code or assets involved.
    let src = r#"
struct Block {
  a : mat3x3<f32>, b : mat3x3<f32>, c : vec4<f32>, d : vec4<f32>, e : vec4<f32>,
  f : vec4<f32>, g : vec4<f32>, h : vec2<f32>, i : f32, j : i32, k : f32,
  l : f32, m : f32, n : f32, o : f32, p : f32, q : f32, r : f32,
  s : f32, t : i32, u : i32, v : f32,
};
@group(0) @binding(0) var<uniform> U : Block;
@fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }
"#;
    let scan = w::scan_source(src)?;
    let u = scan.uniforms.get("U").ok_or_else(|| Error::Parse("no U".into()))?;
    assert_eq!(u.size, 240, "2*48 + 5*16 + 8 + 16*4 = 240");
    let at = |name: &str| u.fields.iter().find(|f| f.name == name).map(|f| f.offset);
    assert_eq!(at("a"), Some(0));
    assert_eq!(at("b"), Some(48), "mat3x3 columns are 16-byte aligned");
    assert_eq!(at("h"), Some(176));
    assert_eq!(at("j"), Some(188));
    assert_eq!(at("k"), Some(192));
    assert_eq!(at("n"), Some(204));
    assert_eq!(at("v"), Some(236));
    assert_eq!(scan.extra_entries, vec!["fs_main"]);
    // Matrices are skipped; the scalar fields are all discovered as controls.
    assert!(scan.extra_fields.len() >= 14, "every scalar/vec field without a generic name gets a control");
    Ok(())
}

#[test]
fn shared_registry_is_one_engine_wide_instance() -> Result<(), Error> {
    {
        let mut fx = w::shared().lock().unwrap_or_else(|e| e.into_inner());
        fx.define("shared_demo", Source::Inline(SHADER.into()));
        fx.load("shared_demo", Source::Inline(SHADER.into()))?;
        fx.set_export("nodeSize", vec![100.0, 50.0]);
    }
    let fx = w::shared().lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(f32_at(&fx.uniform_bytes("shared_demo", "U")?, 0), 100.0);
    assert!(fx.snapshot("shared_demo").contains_key("extra_entries"));
    Ok(())
}

/// Controls behave like the HTML demo's bounded `<input type=range>` rows: values outside the
/// declared range are clamped, and a scanned field is a 0..100 scale mapped to 0..1 in the UBO.
#[test]
fn controls_clamp_to_declared_min_max() -> Result<(), Error> {
    let mut fx = w::WgslEffects::new();
    fx.define("demo", Source::Inline(SHADER.into()));
    fx.load("demo", Source::Inline(SHADER.into()))?;
    let extra = fx.params("demo").into_iter().find(|p| p.field == "U.extra")
        .ok_or_else(|| Error::Parse("no extra".into()))?;
    assert_eq!((extra.min, extra.max, extra.step), (0.0, 100.0, 1.0));
    assert_eq!(extra.map, "v/100");

    fx.set_param("demo", "U.gain", 500.0)?;
    assert_eq!(fx.param_value("demo", "U.gain"), Some(100.0), "above max is clamped");
    fx.set_param("demo", "U.gain", -20.0)?;
    assert_eq!(fx.param_value("demo", "U.gain"), Some(0.0), "below min is clamped");
    fx.set_param("demo", "U.extra", 100.0)?;
    let buf = fx.uniform_bytes("demo", "U")?;
    assert!((f32_at(&buf, 8)).abs() < 1e-6, "gain 0 -> 0");
    assert!((f32_at(&buf, 16) - 1.0).abs() < 1e-6, "extra 100 -> v/100 -> 1.0");
    Ok(())
}

const ANGLE_SHADER: &str = r#"
struct P {
  lightAngle : f32,
  amount : f32,
  count : i32,
};
@group(0) @binding(0) var<uniform> U : P;
@vertex fn vs_full() -> vec4<f32> { return vec4<f32>(0.0); }
@fragment fn fs_x() -> @location(0) vec4<f32> { return vec4<f32>(U.lightAngle); }
"#;

/// Scanned fields get the three intuitive scales: 0..100 normalised, 1..100 for integers and
/// 0..360 degrees (mapped to radians) for angles.
#[test]
fn scanned_controls_get_html_style_scales() -> Result<(), Error> {
    let mut fx = w::WgslEffects::new();
    fx.define("a", Source::Inline(ANGLE_SHADER.into()));
    fx.load("a", Source::Inline(ANGLE_SHADER.into()))?;
    let params = fx.params("a");
    let angle = params.iter().find(|p| p.field == "U.lightAngle").unwrap();
    assert_eq!((angle.min, angle.max), (0.0, 360.0));
    assert_eq!(angle.map, "v*PI/180");
    let amount = params.iter().find(|p| p.field == "U.amount").unwrap();
    assert_eq!((amount.min, amount.max), (0.0, 100.0));
    assert_eq!(amount.map, "v/100");
    let count = params.iter().find(|p| p.field == "U.count").unwrap();
    assert_eq!((count.min, count.max, count.default), (1.0, 100.0, 1.0));
    assert_eq!(count.map, "");
    Ok(())
}

const OVERRIDE_SHADER: &str = r#"
// @fx param U.dpr float 0 100 1 50 "Dpr" v/100
struct P { dpr : f32, other : f32 };
@group(0) @binding(0) var<uniform> U : P;
@vertex fn vs_full() -> vec4<f32> { return vec4<f32>(0.0); }
@fragment fn fs_x() -> @location(0) vec4<f32> { return vec4<f32>(U.dpr); }
"#;

/// A declared `@fx param` owns its field even when the engine also exports a generic value for it.
#[test]
fn declared_param_wins_over_generic_export() -> Result<(), Error> {
    let mut fx = w::WgslEffects::new();
    fx.define("o", Source::Inline(OVERRIDE_SHADER.into()));
    fx.load("o", Source::Inline(OVERRIDE_SHADER.into()))?;
    fx.set_export("dpr", vec![2.0]);
    let buf = fx.uniform_bytes("o", "U")?;
    assert!((f32_at(&buf, 0) - 0.5).abs() < 1e-6, "declared 50 -> v/100 -> 0.5, not the export's 2.0");
    Ok(())
}
