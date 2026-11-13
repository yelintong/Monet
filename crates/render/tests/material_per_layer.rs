//! Does the **per-layer** external-material path actually deliver the material's pixels, and is that
//! stable across zoom? This is the channel the live canvas uses by default (`FIGCRAFT_SHADE_LAYERED`
//! on); the whole-page compositor is exercised separately by `examples/solid_probe`.
//!
//! The per-layer path is a two-frame handshake: `shader_layer` runs *before* the GPU has anything,
//! so frame 1 enqueues a `ShadeRequest` and draws nothing; the backend computes the raster and
//! `board.put`s it; frame 2 must then hit `board.ready(node, key)` and paint it. Two things can
//! break that: (a) the key changes between frames so `ready` never hits, or (b) the renderer drops
//! the returned pixmap. Either way the pane is invisible — exactly the reported "glass vanishes".
//!
//! No GPU is needed: we stand in for the backend by putting an opaque magenta raster under the
//! request's own key, then assert frame 2 paints magenta over the node at every zoom.

use std::collections::HashMap;
use std::sync::Arc;

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_render::shade::{premultiplied, ShadeBoard, Shaded, ShaderSpec};
use figcraft_render::{render_view, to_rgba8, Options, View};

fn make_doc() -> (Document, NodeId) {
    let mut doc = Document::new("per-layer");
    let page = doc.pages[0];
    // 200×400 at the page origin, no fill / no children: the material is the only paint.
    let n = doc.create(page, NodeKind::Rectangle, "Solid", 0.0, 0.0, 200.0, 400.0).unwrap();
    doc.get_mut(n).unwrap().fills.clear();
    (doc, n)
}

fn board_for(node: NodeId) -> Arc<ShadeBoard> {
    let mut specs = HashMap::new();
    // The path/effect id are irrelevant offline — we never invoke a GPU, only the request lifecycle.
    specs.insert(node, Arc::new(ShaderSpec::new("material:solid", "solid.wgsl")));
    Arc::new(ShadeBoard::new(specs))
}

fn magenta_of(w: u32, h: u32) -> tiny_skia::Pixmap {
    let bytes = {
        let mut v = vec![0u8; (w as usize) * (h as usize) * 4];
        for p in v.chunks_exact_mut(4) {
            p.copy_from_slice(&[255, 0, 255, 255]);
        }
        v
    };
    premultiplied(&bytes, w, h).expect("magenta raster")
}

fn count_magenta(rgba: &[u8]) -> usize {
    let mut n = 0;
    for p in rgba.chunks_exact(4) {
        if p[0] > 200 && p[1] < 70 && p[2] > 200 {
            n += 1;
        }
    }
    n
}

#[test]
fn per_layer_material_is_painted_on_the_second_frame_at_every_zoom() {
    // Device scales: 0.3 (30% × dpr≈1.0), 1.0, 2.3, 6.75 (the zooms from the bug report).
    for &z in &[0.3_f64, 1.0, 2.3, 6.75] {
        let (doc, node) = make_doc();
        let board = board_for(node);
        // A view large enough to force the banded path (width*height > 300_000).
        let view = View { origin: (0.0, 0.0), scale: z, width: 1200, height: 800 };
        let opts = Options { shade: Some(board.clone()), lod: false, ..Default::default() };

        // Frame 1: registers the request(s); nothing is drawn yet.
        let frame1 = render_view(&doc, doc.pages[0], &view, &opts).expect("frame 1");
        let reqs: Vec<_> = board.take_requests().into_iter().filter(|r| r.node == node).collect();
        let f1_magenta = count_magenta(&to_rgba8(&frame1));
        assert!(!reqs.is_empty(), "scale {z}: the per-layer path must enqueue a material request");

        // Stand in for the GPU backend: install an opaque magenta raster under each request's key.
        let mut keys = Vec::new();
        for r in &reqs {
            keys.push((r.key, r.size));
            board.put(node, Shaded { key: r.key, pixmap: magenta_of(r.size.0, r.size.1) });
        }

        // Frame 2 must hit the cache and paint the material over the node's device rect.
        let frame2 = render_view(&doc, doc.pages[0], &view, &opts).expect("frame 2");
        let f2_magenta = count_magenta(&to_rgba8(&frame2));

        // The node may be taller/wider than the view: only the visible part is on screen.
        let node_w = (200.0 * z).round() as usize;
        let node_h = (400.0 * z).round() as usize;
        let vis_w = node_w.min(view.width as usize);
        let vis_h = node_h.min(view.height as usize);
        let vis_area = vis_w * vis_h;
        println!(
            "scale {z}: requests={} keys={:?} frame1_magenta={f1_magenta} frame2_magenta={f2_magenta} visible_area≈{vis_area}:{:?}",
            reqs.len(),
            keys,
            (vis_w, vis_h),
        );

        assert_eq!(f1_magenta, 0, "scale {z}: frame 1 has no raster yet, so nothing shows");
        assert!(
            f2_magenta > 0,
            "scale {z}: the material raster (installed under the request key) must be painted on frame 2 — \
             a zero here means `ready` missed (key drifted) or the returned pixmap was dropped"
        );
        // The material covers the node's whole visible device rect, so it should fill ~vis_area pixels.
        assert!(
            f2_magenta >= vis_area * 9 / 10,
            "scale {z}: expected the material to cover the node's visible part (~{vis_area} px), got {f2_magenta}"
        );
    }
}

/// The same, but rendered without banding — isolates "is the miss/hit contract right?" from "is the
/// band splitter mangling the request?".
#[test]
fn per_layer_material_is_painted_without_banding_too() {
    let (doc, node) = make_doc();
    let board = board_for(node);
    let view = View { origin: (0.0, 0.0), scale: 1.0, width: 400, height: 300 };
    let opts = Options { shade: Some(board.clone()), lod: false, ..Default::default() };

    let _ = figcraft_render::render_view_single(&doc, doc.pages[0], &view, &opts).expect("frame 1");
    let reqs: Vec<_> = board.take_requests().into_iter().filter(|r| r.node == node).collect();
    assert_eq!(reqs.len(), 1, "one unbanded render, one request");
    for r in &reqs {
        board.put(node, Shaded { key: r.key, pixmap: magenta_of(r.size.0, r.size.1) });
    }
    let pix = figcraft_render::render_view_single(&doc, doc.pages[0], &view, &opts).expect("frame 2");
    let m = count_magenta(&to_rgba8(&pix));
    println!("unbanded frame2_magenta={m}");
    assert!(m > 0, "unbanded: frame 2 must paint the material");
}

/// **The live path.** The canvas does not call `render_view` directly: it goes through `Retained`,
/// whose incremental cache re-renders only what changed and hands back `Work::Reused` when it thinks
/// nothing did. That reuse decision does not know a material raster arrived in between — so if the
/// second frame comes back "reused", the pane is drawn from the pre-material image and the glass never
/// appears even though the raster exists. This mirrors the two frames the worker runs.
#[test]
fn retained_two_frames_still_show_the_material() {
    use std::sync::Arc;

    for &(scale, w, h) in &[(0.525_f64, 1157u32, 1666u32), (11.812, 1157, 1666)] {
        let (mut doc, node) = make_doc();
        // The live test node: 220×140 at (80,80), so it is not hugging the view's corner.
        {
            let n = doc.get_mut(node).unwrap();
            n.transform = figcraft_geom::Affine::translate((80.0, 80.0));
            n.fills.clear();
            n.width = 220.0;
            n.height = 140.0;
        }
        let doc = Arc::new(doc);
        let page = doc.pages[0];
        let board = board_for(node);
        let view = View { origin: (0.0, 0.0), scale, width: w, height: h };
        let opts = Options { shade: Some(board.clone()), lod: true, ..Default::default() };

        let mut ret = figcraft_render::Retained::default();
        // Frame 1: request only.
        let f1_magenta = {
            let (_, pix, work) = ret.render(&doc, page, &view, &opts).expect("frame 1");
            println!("scale {scale}: frame1 work={work:?}");
            count_magenta(&to_rgba8(pix))
        };
        let reqs: Vec<_> = board.take_requests().into_iter().filter(|r| r.node == node).collect();
        assert!(!reqs.is_empty(), "scale {scale}: frame 1 must request");
        for r in &reqs {
            board.put(node, Shaded { key: r.key, pixmap: magenta_of(r.size.0, r.size.1) });
        }
        // Frame 2, WITHOUT clearing: the reuse cache thinks nothing changed and hands back the
        // pre-material image. This is the bug — and why an idle canvas never shows the material.
        let (f2_magenta, work2) = {
            let (_, pix, work) = ret.render(&doc, page, &view, &opts).expect("frame 2");
            (count_magenta(&to_rgba8(pix)), work)
        };
        println!("scale {scale}: frame1_magenta={f1_magenta} frame2_magenta={f2_magenta} work2={work2:?}");
        assert_eq!(f1_magenta, 0, "scale {scale}: frame 1 has no raster yet");
        if f2_magenta == 0 {
            assert!(matches!(work2, figcraft_render::Work::Reused), "scale {scale}: a hidden raster must coincide with a reused frame");
        }

        // Frame 3 after `clear()` — what `rasterize` now does once it produced rasters: the frame is
        // re-rasterised and `shader_layer` hits the raster, so the material is really drawn.
        ret.clear();
        let (f3_magenta, work3) = {
            let (_, pix, work) = ret.render(&doc, page, &view, &opts).expect("frame 3");
            (count_magenta(&to_rgba8(pix)), work)
        };
        println!("scale {scale}: after clear frame3_magenta={f3_magenta} work3={work3:?}");
        assert!(
            f3_magenta > 0,
            "scale {scale}: after clearing the reuse cache the redraw must paint the material — \
             this is the `rasterize` fix path (frame {work3:?})"
        );
    }
}
