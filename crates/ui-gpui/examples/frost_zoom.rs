//! Does the frost keep its strength when the canvas is zoomed in?
//!
//! A material's blur radius is in **document** units, so the mip level it asks for is
//! `log2(frost × dpr)` and grows with the zoom. This puts a striped backdrop behind a glass pane
//! and measures how much of the stripe pattern survives at `frost = 0` (none should) and
//! `frost = 20` (almost nothing should) at several device scales.
//!
//! `cargo run -p figcraft-ui-gpui --example frost_zoom --features gpu`

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, NodeKind};
use figcraft_render::View;
use figcraft_ui_gpui::wgsl_runner::{passes_for, WgslRunner};

const CW: u32 = 1157;
const CH: u32 = 1666;

fn doc_with(frost: f64, scale: f64) -> Document {
    let glass = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/wgsl/glass.wgsl");
    let mut doc = Document::default();
    let page = doc.pages[0];

    // Stripes whose **device** period is a constant 64 px at this scale, so the measurement means
    // the same thing at every zoom.
    let step = 64.0 / scale;
    let inks = ["000000", "FFFFFF"];
    for i in 0..80 {
        let x0 = (i as f64) * step;
        let r = doc
            .create(page, NodeKind::Rectangle, "stripe", x0, 0.0, x0 + step, 4000.0)
            .expect("stripe");
        doc.get_mut(r).expect("stripe node").fills =
            vec![Paint::solid(Color::from_hex(inks[i % 2]).unwrap())];
    }

    // The pane is far larger than the viewport at every scale: the whole view is interior, so only
    // the frost can make the stripes fade.
    let pane = doc
        .create(page, NodeKind::Rectangle, "Pane", 0.0, 0.0, 20000.0, 20_000.0)
        .expect("pane");
    {
        let n = doc.get_mut(pane).expect("pane node");
        n.fills = vec![Paint::solid(Color::from_hex("00FF00").unwrap())];
        let binding = format!(
            "{{\"path\":\"{}\",\"params\":{{\"u.frost\":{frost}}}}}",
            glass.replace('\\', "\\\\")
        );
        n.plugin_data.insert("figcraft.wgslMaterial".to_string(), binding);
    }
    doc
}

fn main() {
    let Some(mut runner) = WgslRunner::new() else {
        println!("no GPU adapter");
        return;
    };

    for scale in [1.75_f64, 6.0, 30.1] {
        for frost in [0.0_f64, 20.0] {
            let doc = doc_with(frost, scale);
            let page = doc.pages[0];
            let view = View { origin: (4000.0, 4000.0), scale, width: CW, height: CH };
            let passes = passes_for(&doc, page, &view, 0.0);
            if passes.is_empty() {
                println!("scale={scale} frost={frost}: no pass");
                continue;
            }
            // One hard vertical edge at the middle of the canvas: black left, white right.
            let mut scene = vec![255u8; (CW * CH * 4) as usize];
            for y in 0..CH as usize {
                for x in 0..CW as usize {
                    let v: u8 = if x < CW as usize / 2 { 0 } else { 255 };
                    let i = (y * CW as usize + x) * 4;
                    scene[i] = v;
                    scene[i + 1] = v;
                    scene[i + 2] = v;
                    scene[i + 3] = 255;
                }
            }
            let Some(out) = runner.composite(&scene, CW, CH, &passes) else {
                println!("scale={scale} frost={frost}: composite failed");
                continue;
            };
            // The 10%..90% transition width along the middle row is the blur radius, read directly.
            let y = CH as usize / 2;
            let lum = |x: usize| f64::from(out[(y * CW as usize + x) * 4 + 1]);
            let (lo, hi) = (25.5_f64, 229.5_f64);
            let mut first = None;
            let mut last = None;
            for x in 0..CW as usize {
                let v = lum(x);
                if first.is_none() && v > lo {
                    first = Some(x);
                }
                if v < hi {
                    last = Some(x);
                }
            }
            let width = match (first, last) {
                (Some(a), Some(b)) if b > a => b - a,
                _ => 0,
            };
            println!(
                "scale={scale:5.2} dpr={scale:5.2} frost={frost:5.1}  edge_10_90={width:4} px  (ideal radius = frost*dpr = {:.0} px)",
                frost * scale
            );
        }
    }
}
