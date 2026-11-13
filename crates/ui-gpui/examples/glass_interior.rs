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
    let glass = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/wgsl/glass.wgsl");
    let scale = 30.1; // 1720%
    let origin = (0.0_f64, 0.0_f64);
    // The pane's left edge, 300 device px into the viewport. Its interior is to the right.
    let pane_left_doc = 300.0 / scale;

    let Some(mut runner) = WgslRunner::new() else { println!("no GPU adapter"); return };

    // The figure being refracted sits **left of the pane's edge**, i.e. exactly the strip the lens
    // pulls in. Two cases: the same strip, once inside the viewport canvas and once outside it.
    // Device-pixel placement makes the two cases directly comparable: the second figure is the
    // first one moved past the canvas' right edge, so it was never rasterized.
    for (label, dev_x0, dev_x1) in [
        ("figure at device x 480..540  (inside the window)", 480.0, 540.0),
        ("figure at device x 1500..1560 (outside the window)", 1500.0, 1560.0),
    ] {
        let (fig_x0, fig_x1) = (dev_x0 / scale, dev_x1 / scale);
        let mut doc = Document::default();
        let page = doc.pages[0];
        let fig = doc.create(page, NodeKind::Rectangle, "fig", fig_x0, 0.0, fig_x1, 4000.0).expect("fig");
        doc.get_mut(fig).expect("fig n").fills = vec![Paint::solid(Color::from_hex("FF0000").unwrap())];
        let pane = doc.create(page, NodeKind::Rectangle, "Pane", pane_left_doc, 0.0, 40_000.0, 40_000.0).expect("pane");
        {
            let n = doc.get_mut(pane).expect("pane n");
            n.fills = vec![Paint::solid(Color::from_hex("00FF00").unwrap())];
            let binding = format!(
                "{{\"path\":\"{}\",\"params\":{{\"u.frost\":0,\"u.dispersion\":0,\"u.refraction\":1,\"u.depth\":50}}}}",
                glass.replace('\\', "\\\\")
            );
            n.plugin_data.insert("figcraft.wgslMaterial".to_string(), binding);
        }
        let view = View { origin, scale, width: CW, height: CH };
        let passes = passes_for(&doc, page, &view, 0.0);
        for pass in &passes {
            for (var, bytes) in &pass.uniforms {
                if var == "u" && bytes.len() >= 40 {
                    let f = |at: usize| f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
                    println!(
                        "   u.depth={:.2} doc  dpr={:.2}  => max sample offset {:.0} px  (canvas {}x{}, window {:?})",
                        f(20), f(36), f(20) * f(36), CW, CH, pass.rect_px
                    );
                }
            }
        }

        // CPU scene exactly as the renderer builds it: the canvas **is** the viewport, so a node
        // whose device rect lies outside 0..CW is simply not in it.
        let mut scene = vec![255u8; (CW * CH * 4) as usize];
        for y in 0..CH as usize {
            for x in 0..CW as usize {
                let doc_x = origin.0 + (x as f64) / scale;
                if doc_x >= fig_x0 && doc_x < fig_x1 {
                    let i = (y * CW as usize + x) * 4;
                    scene[i] = 0; scene[i + 1] = 0; scene[i + 2] = 255;
                }
            }
        }
        let Some(out) = runner.composite(&scene, CW, CH, &passes) else { println!("{label}: failed"); continue };
        // Scan the row just inside the pane's left edge for the pulled-in red.
        let y = CH as usize / 2;
        let mut red = 0usize;
        let mut max_red = 0u8;
        for x in 300..700 {
            let i = (y * CW as usize + x) * 4;
            let (g, r) = (i32::from(out[i + 1]), i32::from(out[i + 2]));
            if r > 40 && r > g + 20 {
                red += 1;
                max_red = max_red.max(r as u8);
            }
        }
        println!("{label}");
        println!("   passes={} red_pulled_inside_pane={red} px (max R={max_red})", passes.len());
    }
}
