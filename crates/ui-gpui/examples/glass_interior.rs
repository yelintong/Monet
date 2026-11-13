//! What does the glass material actually paint **inside** a pane that is much larger than the
//! viewport (i.e. every edge far off-screen)? This renders through the same whole-page compositor
//! the canvas uses, with no UI in the way, and prints the pixels it produced.
//!
//! `cargo run -p figcraft-ui-gpui --example glass_interior --features gpu`

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, NodeKind};
use figcraft_render::View;
use figcraft_ui_gpui::wgsl_runner::{passes_for, WgslRunner};

const CW: u32 = 1157;
const CH: u32 = 1666;

fn main() {
    let glass_path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/wgsl/glass.wgsl");
    println!("glass.wgsl: {glass_path}");

    let mut doc = Document::default();
    let page = doc.pages[0];

    // A distinctive backdrop: solid red, 800x600, so any copy of it is easy to recognise.
    let red = doc
        .create(page, NodeKind::Rectangle, "Backdrop", 0.0, 0.0, 800.0, 600.0)
        .expect("backdrop");
    doc.get_mut(red).expect("backdrop node").fills =
        vec![Paint::solid(Color::from_hex("FF0000").unwrap())];

    // The pane: same box, glass material, and a **green** fill so a frame that fell back to the
    // plain fill is impossible to confuse with a frame that composited the material.
    let pane = doc
        .create(page, NodeKind::Rectangle, "Pane", 0.0, 0.0, 800.0, 600.0)
        .expect("pane");
    {
        let n = doc.get_mut(pane).expect("pane node");
        n.fills = vec![Paint::solid(Color::from_hex("00FF00").unwrap())];
        let binding = format!(
            "{{\"path\":\"{}\",\"params\":{{}}}}",
            glass_path.replace('\\', "\\\\")
        );
        n.plugin_data.insert("figcraft.wgslMaterial".to_string(), binding);
    }

    let Some(mut runner) = WgslRunner::new() else {
        println!("no GPU adapter — cannot composite");
        return;
    };

    let cases: &[(&str, (f64, f64), f64)] = &[
        // viewport centred in the pane: every edge is ~ (400-19)*scale px away
        ("interior (edges far off-screen)", (390.0, 290.0), 30.1),
        // pane's left edge on screen
        ("left edge on screen", (0.0, 290.0), 30.1),
        // reference: whole pane visible at 100%
        ("whole pane at 1.75x", (0.0, 0.0), 1.75),
    ];

    for (label, (ox, oy), scale) in cases {
        let view = View { origin: (*ox, *oy), scale: *scale, width: CW, height: CH };
        let passes = passes_for(&doc, page, &view, 0.0);
        println!("\n== {label} @ scale {scale} == passes={}", passes.len());
        for p in &passes {
            println!(
                "   node={:?} rect_px={:?} node_px=({:.0},{:.0}) origin_px=({:.1},{:.1}) fields={}",
                p.node,
                p.rect_px,
                p.node_px.0,
                p.node_px.1,
                p.origin_px.0,
                p.origin_px.1,
                p.field_bytes.is_some(),
            );
            for (var, bytes) in &p.uniforms {
                if var == "u" && bytes.len() >= 56 {
                    let f = |at: usize| {
                        f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
                    };
                    println!(
                        "   u: depth={:.2} disp={:.2} frost={:.2} splay={:.2} dpr={:.3} nodeSizePx=({:.0},{:.0}) origin=({:.1},{:.1})",
                        f(20), f(24), f(28), f(32), f(36), f(40), f(44), f(48), f(52)
                    );
                }
            }
        }
        // The CPU scene the compositor reads as its backdrop: white canvas with the red rect
        // painted where the view maps it. The pane's own fill is NOT in it (the renderer digs the
        // hole), so a pixel that comes back red is a copy of the backdrop and a pixel that comes
        // back green is the pane's plain fill (i.e. the material never ran).
        let mut scene = vec![255u8; (CW * CH * 4) as usize];
        {
            let xf = view.transform();
            let (sx0, sy0) = {
                let p = xf * figcraft_geom::Point::new(0.0, 0.0);
                (p.x, p.y)
            };
            let (sx1, sy1) = {
                let p = xf * figcraft_geom::Point::new(800.0, 600.0);
                (p.x, p.y)
            };
            let x0 = sx0.max(0.0) as i64;
            let y0 = sy0.max(0.0) as i64;
            let x1 = (sx1.min(f64::from(CW))) as i64;
            let y1 = (sy1.min(f64::from(CH))) as i64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y as usize) * CW as usize + x as usize) * 4;
                    // BGRA: red
                    scene[i] = 0;
                    scene[i + 1] = 0;
                    scene[i + 2] = 255;
                    scene[i + 3] = 255;
                }
            }
        }
        let Some(rgba) = runner.composite(&scene, CW, CH, &passes) else {
            println!("   composite returned None");
            continue;
        };
        let at = |x: usize, y: usize| {
            let i = (y * CW as usize + x) * 4;
            (rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3])
        };
        let (cx, cy) = (CW as usize / 2, CH as usize / 2);
        println!("   centre        = {:?}", at(cx, cy));
        println!("   quarter in    = {:?}", at(cx / 2, cy / 2));
        println!("   near left     = {:?}", at(20, cy));
        let mut mn = 255u8;
        let mut mx = 0u8;
        for x in 0..CW as usize {
            let (r, g, _b, _a) = at(x, cy);
            mn = mn.min(g);
            mx = mx.max(g);
        }
        println!("   centre row green range = {mn}..{mx}  (flat == it is a plain copy)");
    }
}
