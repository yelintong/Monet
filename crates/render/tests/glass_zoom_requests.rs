//! Material `ShadeRequest` geometry across zooms: the banded renderer must produce the **same**
//! window as a single render at every zoom (one request per node, not one per band). The live
//! canvas goes through `Retained::render` → `render_view` → `render_bands` → `band_spans`, so a
//! mismatch here is what turns one pane into N overlapping requests.
//!
//! Visibility itself (does the raster get drawn?) is covered by `material_per_layer.rs`; this file
//! only pins the request geometry.

use std::collections::HashMap;
use std::sync::Arc;

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_render::shade::{ShadeBoard, ShaderSpec};
use figcraft_render::{render_view, render_view_single, Options, View};

fn make_doc() -> (Document, NodeId) {
    let mut doc = Document::new("zoom-repro");
    let page = doc.pages[0];
    // A material node at the page origin so it is always in the view's top-left at any zoom.
    let n = doc
        .create(page, NodeKind::Rectangle, "Glass", 0.0, 0.0, 200.0, 400.0)
        .unwrap();
    (doc, n)
}

fn board_for(node: NodeId) -> Arc<ShadeBoard> {
    let mut specs = HashMap::new();
    specs.insert(node, Arc::new(ShaderSpec::new("material:glass", "glass.wgsl")));
    Arc::new(ShadeBoard::new(specs))
}

fn requests_for(board: &ShadeBoard, node: NodeId) -> Vec<figcraft_render::shade::ShadeRequest> {
    board.take_requests().into_iter().filter(|r| r.node == node).collect()
}

#[test]
fn glass_request_geometry_across_zooms() {
    for &z in &[0.3_f64, 1.0, 2.3, 6.75] {
        let (doc, node) = make_doc();
        let board = board_for(node);
        let view = View { origin: (0.0, 0.0), scale: z, width: 1200, height: 800 };

        // Single render (no banding).
        board.take_requests();
        let _ = render_view_single(&doc, doc.pages[0], &view, &Options { shade: Some(board.clone()), ..Default::default() });
        let single = requests_for(&board, node);

        // Banding render.
        board.take_requests();
        let _ = render_view(&doc, doc.pages[0], &view, &Options { shade: Some(board.clone()), ..Default::default() });
        let banded = requests_for(&board, node);

        println!(
            "\n=== scale {z} ===  node_px expected ~ ({:.1},{:.1})",
            200.0 * z, 400.0 * z
        );
        println!("  SINGLE ({}) requests:", single.len());
        for r in &single {
            println!(
                "    size=({},{}), origin_px=({:.1},{:.1}), node_px=({:.1},{:.1}), scale={:.3}",
                r.size.0, r.size.1, r.origin_px.0, r.origin_px.1, r.node_px.0, r.node_px.1, r.scale
            );
        }
        println!("  BANDED ({}) requests:", banded.len());
        for r in &banded {
            println!(
                "    size=({},{}), origin_px=({:.1},{:.1}), node_px=({:.1},{:.1}), scale={:.3}",
                r.size.0, r.size.1, r.origin_px.0, r.origin_px.1, r.node_px.0, r.node_px.1, r.scale
            );
        }

        assert_eq!(banded.len(), 1, "scale {z}: one material node must produce exactly one request, not one per band");
        let s = &single[0];
        let b = &banded[0];
        println!(
            "  PRIMARY single size=({},{}) origin=({:.1},{:.1}) | banded size=({},{}) origin=({:.1},{:.1})",
            s.size.0, s.size.1, s.origin_px.0, s.origin_px.1, b.size.0, b.size.1, b.origin_px.0, b.origin_px.1,
        );
        assert_eq!(s.size, b.size, "banding must produce the same primary window as single render at scale {z}");
        assert!(
            (s.origin_px.0 - b.origin_px.0).abs() < 2.0 && (s.origin_px.1 - b.origin_px.1).abs() < 2.0,
            "banding origin_px must match single render at scale {z}: {:?} vs {:?}",
            s.origin_px, b.origin_px
        );
    }
}
