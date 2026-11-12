//! Motion animation export: render a frame (or a whole page) of a Motion timeline frame by frame
//! and encode it as an animated **GIF** or a **PNG sequence in a zip**.
//!
//! Ported from figcraft/desigma `desigma-export::anim` (PR#1624 family), adapted to this tree:
//! frames are evaluated with [`figcraft_doc::anim::motion::evaluate`] and drawn with the same CPU
//! renderer as static exports ([`figcraft_render::render_node`]), so an exported frame is
//! pixel-identical to a PNG export of that moment. Encoding uses the workspace `image` (GIF) and
//! `zip` crates. **No UI dependency**: exposed through the `motion.export` engine command so the
//! CLI, control channel, MCP and tests all share one path (docs/architecture.md "one path").
//!
//! WebM/AV1 export is intentionally not included here — it needs the `rav1e` encoder and is a
//! separate, larger step.

use std::io::{Cursor, Write};

use base64::Engine as _;
use figcraft_doc::Document;
use figcraft_doc::anim::motion::{TimelineSettings, evaluate};
use figcraft_doc::NodeId;
use figcraft_geom::Rect;
use figcraft_render::{Pixmap, View};
use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, RgbaImage};
use serde_json::{Value, json};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::{Result, Session, bad};
use crate::params::*;

/// Largest side of an exported animation in pixels.
pub const MAX_SIDE: u32 = 8192;
/// Most frames one export may contain (10 minutes at 60 fps is 36000).
pub const MAX_FRAMES: u32 = 40_000;
/// Largest GIF dimension the encoder handles comfortably.
const GIF_MAX_FPS: u32 = 50;

/// Output formats of a motion export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimFormat {
    Gif,
    /// AV1-in-WebM (pure Rust `rav1e`).
    Webm,
    /// A zip of numbered PNG files.
    PngZip,
}

impl AnimFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gif => "GIF",
            Self::Webm => "WebM (AV1)",
            Self::PngZip => "PNG frames",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Webm => "webm",
            Self::PngZip => "zip",
        }
    }
    /// `gif`, `webm`, `png`, `zip` / `png-zip` (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "gif" => Some(Self::Gif),
            "webm" | "av1" => Some(Self::Webm),
            "png" | "zip" | "png-zip" | "pngzip" => Some(Self::PngZip),
            _ => None,
        }
    }
}

/// A node (frame, layer, or a whole page) and its timeline, ready to be drawn at any time.
pub struct Animation {
    base: Document,
    node: NodeId,
    page: NodeId,
    timeline: TimelineSettings,
    /// Fixed output view: union bounds over the whole timeline, so a size/position animation can
    /// never clip a frame or change its dimensions between frames.
    view: View,
    size: (u32, u32),
}

impl Animation {
    /// Prepares the export of `node` at `scale` (1.0 = 100%). The timeline is the one stored for
    /// the **frame/page itself** when present, else its page's — so two frames on one page can be
    /// exported with independent timelines (the "same container = one animation group" rule).
    pub fn new(doc: &Document, node: NodeId, scale: f64) -> Option<Self> {
        let n = doc.node(node)?;
        let is_page = n.is_page();
        let page = if is_page { node } else { doc.page_of(node)? };
        let scale = if scale.is_finite() { scale.clamp(0.05, 16.0) } else { 1.0 };
        // Per-frame timeline first: a frame's own entry wins over the page's.
        let timeline = doc
            .timelines
            .get(&node)
            .or_else(|| doc.timelines.get(&page))
            .copied()
            .unwrap_or_default();
        let mut bounds: Option<Rect> = None;
        let samples = 24u32;
        for i in 0..=samples {
            let t = timeline.duration_ms as f64 * i as f64 / samples as f64;
            let evaluated = evaluate(doc, page, t);
            if let Some(b) = figcraft_render::export_bounds(&evaluated, node) {
                bounds = Some(match bounds {
                    None => b,
                    Some(prev) => prev.union(b),
                });
            }
        }
        let b = bounds?;
        let w = (b.width() * scale).ceil().max(1.0);
        let h = (b.height() * scale).ceil().max(1.0);
        if w > MAX_SIDE as f64 || h > MAX_SIDE as f64 {
            return None;
        }
        let view = View { origin: (b.x0, b.y0), scale, width: w as u32, height: h as u32 };
        Some(Self { base: doc.clone(), node, page, timeline, view, size: (w as u32, h as u32) })
    }

    pub fn timeline(&self) -> TimelineSettings {
        self.timeline
    }
    /// Output size in pixels (constant over the animation).
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
    /// Whether anything on the page is animated.
    pub fn is_animated(&self) -> bool {
        figcraft_doc::anim::motion::animated_nodes(&self.base, self.page)
            .into_iter()
            .any(|id| self.base.node(id).is_some_and(|n| !n.motion.is_empty()))
    }

    /// Timeline times of the frames of an export at `fps`. A looping timeline gets the frames
    /// `0 .. duration` (the last leads back into the first); a non-looping one adds a final frame
    /// at the end so the animation ends on its last state.
    pub fn frame_times(&self, fps: u32) -> Vec<f64> {
        frame_times(&self.timeline, fps)
    }

    /// Draws the animation at `t_ms` on a transparent background as a premultiplied Pixmap of a
    /// constant size. Motion is baked into a scratch document first, then drawn through a fixed
    /// view (union bounds over the timeline), so position/size animation never clips or resizes a
    /// frame — the same guarantee desigma's `Animation::render` gives.
    pub fn render(&self, t_ms: f64) -> Option<Pixmap> {
        let evaluated = evaluate(&self.base, self.page, t_ms);
        let mut out = Pixmap::new(self.view.width, self.view.height)?;
        figcraft_render::render_nodes_into(&evaluated, &[self.node], &self.view, &mut out);
        Some(out)
    }
}

/// See [`Animation::frame_times`].
pub fn frame_times(timeline: &TimelineSettings, fps: u32) -> Vec<f64> {
    let fps = fps.clamp(1, TimelineSettings::MAX_FPS);
    let d = timeline.duration_ms.max(1) as f64;
    let step = 1000.0 / fps as f64;
    let n = ((d / step).round() as u32).clamp(1, MAX_FRAMES);
    let mut out: Vec<f64> = (0..n).map(|i| (i as f64 * step).min(d)).collect();
    if !timeline.looping && out.last().is_none_or(|l| *l < d - 0.5) {
        out.push(d);
    }
    out
}

/// Encode premultiplied Pixmap frames as an animated GIF with the given per-frame delays (ms).
pub fn encode_gif(frames: &[Pixmap], delays_ms: &[u32], repeating: bool) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        if repeating {
            enc.set_repeat(Repeat::Infinite).map_err(|e| bad("motion.export", format!("gif repeat: {e}")))?;
        }
        let mut imgs = Vec::with_capacity(frames.len());
        for (pix, ms) in frames.iter().zip(delays_ms.iter()) {
            let (w, h) = (pix.width(), pix.height());
            let img = RgbaImage::from_raw(w, h, figcraft_render::to_rgba8(pix))
                .ok_or_else(|| bad("motion.export", "gif frame size mismatch"))?;
            imgs.push(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms((*ms).max(10), 1)));
        }
        enc.encode_frames(imgs).map_err(|e| bad("motion.export", format!("gif encode: {e}")))?;
    }
    Ok(out)
}

/// Pack Pixmap frames into a zip of numbered PNG files (`base_0001.png`, ...).
pub fn write_png_zip(frames: &[Pixmap], base: &str) -> Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zw = ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let digits = frames.len().to_string().len().max(4);
        for (i, pix) in frames.iter().enumerate() {
            let name = format!("{base}_{:0digits$}.png", i + 1);
            let png = figcraft_render::encode_png(pix).map_err(|e| bad("motion.export", format!("png encode: {e}")))?;
            zw.start_file(name, opts).map_err(|e| bad("motion.export", format!("zip entry: {e}")))?;
            zw.write_all(&png).map_err(|e| bad("motion.export", format!("zip write: {e}")))?;
        }
        zw.finish().map_err(|e| bad("motion.export", format!("zip finish: {e}")))?;
    }
    Ok(buf.into_inner())
}

/// The `motion.export` command: no document write, no undo — it only reads and encodes.
pub fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "motion.export";
    let ids = req_targets(cmd, s, p)?;
    let node = ids[0];
    let format = match string(p, "format").as_deref() {
        None | Some("gif") => AnimFormat::Gif,
        Some(f) => AnimFormat::parse(f).ok_or_else(|| bad(cmd, "format: gif|webm|png|zip|png-zip"))?,
    };
    let scale = num(p, "scale").unwrap_or(1.0);
    let fps = num(p, "fps").unwrap_or(0.0) as u32;
    let quality = num(p, "quality").unwrap_or(85.0) as u8;
    let speed = num(p, "speed").unwrap_or(9.0) as u8;
    let anim = Animation::new(s.doc()?, node, scale).ok_or_else(|| bad(cmd, "nothing to export (no node, no bounds, or too large)"))?;
    if !anim.is_animated() {
        return Err(bad(cmd, "the layer has no Motion animation to export"));
    }
    // GIF falls back to a sane frame rate so its palette stays legible; PNG zip / WebM keep
    // the requested (or timeline) rate, with WebM allowing up to its own 120 fps cap.
    let fps = if fps == 0 {
        anim.timeline().fps
    } else if format == AnimFormat::Gif {
        fps.min(GIF_MAX_FPS)
    } else {
        fps.min(120)
    };
    let times = anim.frame_times(fps);
    if times.len() as u32 > MAX_FRAMES {
        return Err(bad(cmd, format!("too many frames ({} > {MAX_FRAMES}); lower fps or duration", times.len())));
    }
    let size = anim.size();
    // WebM is streamed: `encode_webm` renders each frame itself. Rendering them up front first
    // would double the (already slow) AV1 work, so only Gif/PngZip pre-build a frame Vec.
    let (bytes, ext) = match format {
        AnimFormat::Webm => {
            let o = super::motion_webm::WebmOptions { fps, quality, speed, ..Default::default() };
            (super::motion_webm::encode_webm(&anim, &o)?, "webm")
        }
        _ => {
            let mut frames = Vec::with_capacity(times.len());
            for t in &times {
                frames.push(anim.render(*t).ok_or_else(|| bad(cmd, "a frame failed to render"))?);
            }
            match format {
                AnimFormat::Gif => {
                    let step = (1000.0 / fps as f64).round().max(1.0) as u32;
                    (encode_gif(&frames, &vec![step; frames.len()], anim.timeline().looping)?, "gif")
                }
                AnimFormat::PngZip => {
                    let name = s.doc()?.node(node).map(|n| n.name.clone()).unwrap_or_else(|| "animation".into());
                    (write_png_zip(&frames, &name)?, "zip")
                }
                AnimFormat::Webm => return Err(bad(cmd, "webm is streamed, handled above")),
            }
        }
    };
    Ok(json!({
        "format": ext,
        "label": format.label(),
        "data": base64::engine::general_purpose::STANDARD.encode(&bytes),
        "frames": times.len(),
        "fps": fps,
        "width": size.0,
        "height": size.1,
        "loop": anim.timeline().looping,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::{NodeKind, anim::motion::{AnimProperty, AnimValue, Ease}};
    use figcraft_doc::geom::Vec2;

    fn animated_doc() -> (Document, NodeId, NodeId) {
        let mut d = Document::new("t");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "Card", 0.0, 0.0, 24.0, 24.0).unwrap();
        let id = d.create(frame, NodeKind::Rectangle, "Dot", 2.0, 2.0, 8.0, 8.0).unwrap();
        {
            let n = d.node_mut(id).unwrap();
            let tr = n.motion.ensure_track(AnimProperty::Opacity);
            tr.set_key(0, AnimValue::Scalar(0.0), None);
            tr.set_key(500, AnimValue::Scalar(1.0), Some(Ease::Linear));
            let pos = n.motion.ensure_track(AnimProperty::Position);
            pos.set_key(0, AnimValue::Vec2(Vec2::new(2.0, 2.0)), None);
            pos.set_key(500, AnimValue::Vec2(Vec2::new(14.0, 2.0)), None);
        }
        d.timelines.insert(frame, TimelineSettings { duration_ms: 500, fps: 10, looping: true });
        (d, frame, id)
    }

    #[test]
    fn frame_times_loop_vs_one_shot() {
        let tl = TimelineSettings { duration_ms: 1000, fps: 10, looping: true };
        let t = frame_times(&tl, 10);
        assert_eq!(t.len(), 10, "looping: 0..duration excludes the wrap-around frame");
        assert!((t[0] - 0.0).abs() < 1e-6 && (t[9] - 900.0).abs() < 1e-6);
        let tl = TimelineSettings { duration_ms: 1000, fps: 10, looping: false };
        let t = frame_times(&tl, 10);
        assert_eq!(t.len(), 11, "one-shot adds a final frame at duration");
        assert!((t[10] - 1000.0).abs() < 1e-6);
    }

    #[test]
    fn animation_uses_the_frames_own_timeline_and_evaluates_motion() {
        let (d, frame, _) = animated_doc();
        let anim = Animation::new(&d, frame, 1.0).unwrap();
        assert!(anim.is_animated());
        assert_eq!(anim.timeline().duration_ms, 500, "the frame's own timeline wins over the page's");
        assert_eq!(anim.size(), (24, 24));
        let first = anim.render(0.0).unwrap();
        let mid = anim.render(250.0).unwrap();
        assert_eq!(first.data().len(), 24 * 24 * 4);
        assert_ne!(figcraft_render::to_rgba8(&first), figcraft_render::to_rgba8(&mid), "the Position track must actually move between frames");
    }

    #[test]
    fn gif_and_png_zip_encode_real_frames() {
        let (d, frame, _) = animated_doc();
        let anim = Animation::new(&d, frame, 1.0).unwrap();
        let times = anim.frame_times(10);
        let frames: Vec<Pixmap> = times.iter().map(|t| anim.render(*t).unwrap()).collect();
        let gif = encode_gif(&frames, &vec![100; frames.len()], true).unwrap();
        assert_eq!(&gif[0..6], b"GIF89a", "a real animated GIF");
        assert!(gif.len() > 100);
        let zip = write_png_zip(&frames, "Card").unwrap();
        assert_eq!(&zip[0..2], b"PK", "a real zip");
        let mut zs = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
        assert_eq!(zs.len(), frames.len());
        assert!(zs.by_name("Card_0001.png").is_ok());
    }

    #[test]
    fn motion_export_command_produces_a_gif() {
        use serde_json::json;
        let (d, frame, _) = animated_doc();
        let mut s = Session::with_document(d);
        let v = s.execute("motion.export", &json!({"id": frame.0, "format": "gif", "fps": 10, "scale": 1.0})).unwrap();
        assert_eq!(v["format"], "gif");
        assert_eq!(v["loop"], true);
        assert_eq!(v["width"], 24);
        assert_eq!(v["frames"], 5, "looping 500ms at 10fps");
        let raw = base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().unwrap()).unwrap();
        assert_eq!(&raw[0..6], b"GIF89a");
    }

    #[test]
    fn motion_export_command_produces_a_webm() {
        use serde_json::json;
        let (d, frame, _) = animated_doc();
        let mut s = Session::with_document(d);
        // 24x24 (even, 4:2:0-safe), 5 looping frames; speed 10 keeps the test quick. The command
        // must stream AV1 itself (no up-front frame Vec) and return a decodable EBML/WebM + meta.
        let v = s.execute("motion.export", &json!({"id": frame.0, "format": "webm", "fps": 10, "speed": 10, "scale": 1.0})).unwrap();
        assert_eq!(v["format"], "webm");
        assert_eq!(v["label"], "WebM (AV1)");
        assert_eq!(v["width"], 24);
        assert_eq!(v["height"], 24);
        assert_eq!(v["frames"], 5, "looping 500ms at 10fps");
        assert_eq!(v["fps"], 10);
        assert_eq!(v["loop"], true);
        let raw = base64::engine::general_purpose::STANDARD.decode(v["data"].as_str().unwrap()).unwrap();
        assert_eq!(&raw[0..4], &[0x1A, 0x45, 0xDF, 0xA3], "a real EBML/WebM file");
        assert!(raw.windows(5).any(|w| w == b"V_AV1"), "AV1 codec id");
        assert!(raw.len() > 200, "real encoded AV1 frames, not just a header");
    }

    #[test]
    fn export_refuses_unanimated_and_junk_params() {
        use serde_json::json;
        let mut d = Document::new("t");
        let page = d.first_page().unwrap();
        let id = d.create(page, NodeKind::Rectangle, "Still", 0.0, 0.0, 10.0, 10.0).unwrap();
        let mut s = Session::with_document(d);
        assert!(s.execute("motion.export", &json!({"id": id.0})).is_err(), "no animation");
        assert!(s.execute("motion.export", &json!({"id": id.0, "format": "mp4"})).is_err(), "unsupported format refused");
    }
}
