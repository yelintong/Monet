// Deterministic probe: bind the solid.wgsl material to a rectangle and run the universal material
// pass builder (`passes_for`) at several device scales, then composite it on the GPU and count the
// magenta pixels. The whole thing is window-free, so we can see — without a screenshot — whether the
// shaded raster survives and at which scale it goes missing.
//
// Run: cargo run -p figcraft-ui-gpui --example solid_probe --features gpu
//
// `passes_for` itself needs no GPU, so even on a machine without an adapter we still get the
// geometry / binding diagnostics for every scale, which is the half most likely to hide a bug.

use figcraft_doc::{Document, NodeKind};
use figcraft_geom::Point;
use figcraft_render::View;
use figcraft_ui_gpui::wgsl_runner::{passes_for, WgslRunner};

fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/solid.wgsl");
    println!("solid.wgsl: {path}");

    let mut doc = Document::default();
    let page = doc.pages[0];
    // A 220×140 rectangle at the origin, no fill (the per-layer path drops the fill anyway).
    let r = doc
        .create(page, NodeKind::Rectangle, "R", 0.0, 0.0, 220.0, 140.0)
        .expect("rect");
    // The material binding is stored in `plugin_data["figcraft.wgslMaterial"]` as a JSON string.
    // Windows paths carry backslashes, which MUST be escaped for the JSON to parse — the app itself
    // writes this through `WgslMaterialBinding::to_json` (serde escapes automatically); a hand-rolled
    // `format!` with a raw path produces `"C:\Users\..."` and `from_json` silently rejects it.
    let binding = format!(
        "{{\"path\":\"{}\",\"params\":{{}}}}",
        path.replace('\\', "\\\\")
    );
    doc.get_mut(r)
        .expect("rect node")
        .plugin_data
        .insert("figcraft.wgslMaterial".to_string(), binding);

    // ---- Ground-truth diagnostics (no GPU needed) ----
    println!("page={page:?} r={r:?}");
    println!("subtree(page)={:?}", doc.subtree(page).into_iter().collect::<Vec<_>>());
    println!("page.visible={:?}", doc.node(page).map(|n| n.visible));
    println!("r.visible={:?}", doc.node(r).map(|n| n.visible));
    println!("r.parent={:?}", doc.node(r).map(|n| n.parent));
    let b = doc.bounds_of(&[r]);
    println!("bounds(r)={:?}", b);
    println!(
        "plugin_data(r)={:?}",
        doc.node(r).map(|n| n.plugin_data.keys().collect::<Vec<_>>())
    );
    if let Some(json) = doc.node(r).and_then(|n| n.plugin_data.get("figcraft.wgslMaterial")) {
        println!("binding json ({} bytes): {}", json.len(), json);
    }

    let canvas_w = 3000u32;
    let canvas_h = 2200u32;

    // ---- Per-scale geometry / pass diagnostics (no GPU needed) ----
    for &sc in &[0.5_f64, 1.0, 2.0, 3.5, 6.0, 11.8] {
        let origin = (110.0 - (canvas_w as f64 / 2.0) / sc, 70.0 - (canvas_h as f64 / 2.0) / sc);
        let view = View { origin, scale: sc, width: canvas_w, height: canvas_h };
        let passes = passes_for(&doc, page, &view, 0.0);
        if passes.is_empty() {
            // Replicate the guards to find which one trips.
            let rb = doc.bounds_of(&[r]).unwrap();
            let (cw, ch) = (f64::from(canvas_w), f64::from(canvas_h));
            let xf = view.transform();
            let p0 = xf * Point::new(rb.x0, rb.y0);
            let p1 = xf * Point::new(rb.x1, rb.y1);
            let (x0, y0) = (p0.x, p0.y);
            let (x1, y1) = (p1.x, p1.y);
            let (nx0, ny0) = (x0.min(x1), y0.min(y1));
            let (nx1, ny1) = (x0.max(x1), y0.max(y1));
            let (sx0, sy0) = (nx0.max(0.0), ny0.max(0.0));
            let (sx1, sy1) = (nx1.min(cw), ny1.min(ch));
            let (px0, py0) = (sx0.floor(), sy0.floor());
            let (px1, py1) = (sx1.ceil().min(cw), sy1.ceil().min(ch));
            println!(
                "scale={sc:.3} -> NO PASS  mapped=({x0:.1},{y0:.1})-({x1:.1},{y1:.1})  clipped=({sx0:.1},{sy0:.1})-({sx1:.1},{sy1:.1})  wholepx=({px0:.0},{py0:.0})-({px1:.0},{py1:.0})  w={} h={}",
                rb.width(), rb.height()
            );
            continue;
        }
        let p = &passes[0];
        println!(
            "scale={sc:.3} rect_px={:?} node_px={:?} origin_px={:?} radii_px={:?} uniforms={} field_bytes={:?}",
            p.rect_px, p.node_px, p.origin_px, p.radii_px, p.uniforms.len(), p.field_bytes.as_ref().map(|v| v.len())
        );
    }

    // ---- GPU composite + magenta count (needs an adapter) ----
    let mut runner = match WgslRunner::new() {
        Some(r) => r,
        None => {
            eprintln!("no GPU adapter — skipping composite; geometry diagnostics above are the useful part");
            return;
        }
    };

    let scene = vec![120u8; (canvas_w * canvas_h * 4) as usize];
    for &sc in &[0.5_f64, 1.0, 2.0, 3.5, 6.0, 11.8] {
        let origin = (110.0 - (canvas_w as f64 / 2.0) / sc, 70.0 - (canvas_h as f64 / 2.0) / sc);
        let view = View { origin, scale: sc, width: canvas_w, height: canvas_h };
        let passes = passes_for(&doc, page, &view, 0.0);
        if passes.is_empty() {
            println!("scale={sc:.3} -> NO PASS (composite skipped)");
            continue;
        }
        let Some(out) = runner.composite(&scene, canvas_w, canvas_h, &passes) else {
            println!("scale={sc:.3} -> composite returned None");
            continue;
        };
        let mut magenta = 0usize;
        let mut drawn = 0usize;
        for i in (0..out.len()).step_by(4) {
            let (rr, gg, bb, aa) = (out[i], out[i + 1], out[i + 2], out[i + 3]);
            if aa > 10 {
                drawn += 1;
                if rr > 200 && gg < 70 && bb > 200 {
                    magenta += 1;
                }
            }
        }
        let win = (passes[0].rect_px[2] as f64 * passes[0].rect_px[3] as f64) as f64;
        println!(
            "scale={sc:.3} device={:?} drawn_px={} magenta_px={} ({:.1}% of window, {:.1}% of drawn)",
            passes[0].rect_px, drawn, magenta, 100.0 * magenta as f64 / win.max(1.0), 100.0 * magenta as f64 / drawn.max(1) as f64
        );
    }
}
