//! WebM/AV1 animation export: frames are encoded by `rav1e` (pure Rust, no `asm`/nasm) and
//! wrapped in a small Matroska (EBML) writer so the result plays in Chrome, Edge, Firefox and
//! current Safari, in `<video>` tags and most editors. AV1-in-WebM is the only fully pure-Rust
//! route (VP8/VP9 have no maintained pure-Rust encoder; H.264 needs a C++ library).
//!
//! Ported from figcraft/desigma `desigma-export::anim::webm` (PR#1624 family), adapted to this
//! tree's [`super::motion_export::Animation`] (already evaluates Motion and returns a fixed-size
//! premultiplied Pixmap per frame). Rate control is constant quantizer: [`WebmOptions::quality`]
//! (1..=100) maps to a rav1e quantizer of `255 - 2*quality`, so flat UI animations stay small.
//! Frames are encoded in display order (low latency), a key frame every two seconds, with Cues
//! for seeking. **No UI dependency**: driven by the `motion.export` command.

use rav1e::prelude::*;

use super::motion_export::Animation;
use crate::{Result, bad};

/// WebM/AV1 encoder options.
#[derive(Clone, Debug, PartialEq)]
pub struct WebmOptions {
    /// Frames per second, 1..=120.
    pub fps: u32,
    /// Picture quality 1..=100 (quantizer `255 - 2*quality`; default 85 → 85).
    pub quality: u8,
    /// rav1e speed preset 0 (slowest) ..= 10 (fastest). Default 9 ≈ real time at 720p.
    pub speed: u8,
    /// Colour transparent parts are flattened onto (AV1-in-WebM here has no alpha).
    pub background: [u8; 3],
    /// Encoder threads (0 = one per core, at most 8).
    pub threads: usize,
    /// Tiles to split the picture into for parallel encoding (0 = auto).
    pub tiles: usize,
}

impl Default for WebmOptions {
    fn default() -> Self {
        Self { fps: 60, quality: 85, speed: 9, background: [255, 255, 255], threads: 0, tiles: 0 }
    }
}

impl WebmOptions {
    pub fn quantizer(&self) -> usize {
        255 - 2 * self.quality.clamp(1, 100) as usize
    }
}

// ---------------------------------------------------------------------------------- EBML

fn size_vint(n: u64) -> Vec<u8> {
    let mut len = 1;
    while len < 8 && n >= (1u64 << (7 * len)) - 1 {
        len += 1;
    }
    let mut out = vec![0u8; len];
    for (i, b) in out.iter_mut().enumerate() {
        *b = (n >> (8 * (len - 1 - i))) as u8;
    }
    out[0] |= 1 << (8 - len);
    out
}

fn elem(id: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(id.len() + 9 + payload.len());
    out.extend_from_slice(id);
    out.extend_from_slice(&size_vint(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

fn uint_bytes(v: u64, min_len: usize) -> Vec<u8> {
    let mut len = min_len.max(1);
    while len < 8 && (v >> (8 * len)) != 0 {
        len += 1;
    }
    (0..len).rev().map(|i| (v >> (8 * i)) as u8).collect()
}

fn uint(id: &[u8], v: u64) -> Vec<u8> {
    elem(id, &uint_bytes(v, 1))
}

fn string(id: &[u8], s: &str) -> Vec<u8> {
    elem(id, s.as_bytes())
}

fn float64(id: &[u8], v: f64) -> Vec<u8> {
    elem(id, &v.to_be_bytes())
}

struct Block {
    ts_ms: u64,
    key: bool,
    data: Vec<u8>,
}

fn ebml_header() -> Vec<u8> {
    let mut p = Vec::new();
    p.extend(uint(&[0x42, 0x86], 1)); // EBMLVersion
    p.extend(uint(&[0x42, 0xF7], 1)); // EBMLReadVersion
    p.extend(uint(&[0x42, 0xF2], 4)); // EBMLMaxIDLength
    p.extend(uint(&[0x42, 0xF3], 8)); // EBMLMaxSizeLength
    p.extend(string(&[0x42, 0x82], "webm")); // DocType
    p.extend(uint(&[0x42, 0x87], 4)); // DocTypeVersion
    p.extend(uint(&[0x42, 0x85], 2)); // DocTypeReadVersion
    elem(&[0x1A, 0x45, 0xDF, 0xA3], &p)
}

fn seek(id: &[u8], position: u64) -> Vec<u8> {
    let mut p = elem(&[0x53, 0xAB], id);
    p.extend(elem(&[0x53, 0xAC], &uint_bytes(position, 4)));
    elem(&[0x4D, 0xBB], &p)
}

/// Builds the whole file. `blocks` are in presentation order; the first is a key frame.
fn mux(w: u32, h: u32, fps: u32, av1c: &[u8], blocks: &[Block]) -> Vec<u8> {
    let frame_ms = 1000.0 / fps as f64;
    let duration = blocks.len() as f64 * frame_ms;

    let mut info = uint(&[0x2A, 0xD7, 0xB1], 1_000_000); // TimestampScale: 1 ms
    info.extend(float64(&[0x44, 0x89], duration));
    info.extend(string(&[0x4D, 0x80], "figcraft"));
    info.extend(string(&[0x57, 0x41], "figcraft"));
    let info = elem(&[0x15, 0x49, 0xA9, 0x66], &info);

    let mut colour = uint(&[0x55, 0xB1], 1); // MatrixCoefficients: BT.709
    colour.extend(uint(&[0x55, 0xB9], 1)); // Range: broadcast (limited)
    colour.extend(uint(&[0x55, 0xBA], 1)); // TransferCharacteristics: BT.709
    colour.extend(uint(&[0x55, 0xBB], 1)); // Primaries: BT.709
    let mut video = uint(&[0xB0], w as u64);
    video.extend(uint(&[0xBA], h as u64));
    video.extend(elem(&[0x55, 0xB0], &colour));
    let mut track = uint(&[0xD7], 1); // TrackNumber
    track.extend(uint(&[0x73, 0xC5], 1)); // TrackUID
    track.extend(uint(&[0x83], 1)); // TrackType: video
    track.extend(uint(&[0x9C], 0)); // FlagLacing
    track.extend(string(&[0x86], "V_AV1"));
    track.extend(elem(&[0x63, 0xA2], av1c));
    track.extend(uint(&[0x23, 0xE3, 0x83], (frame_ms * 1e6).round() as u64)); // DefaultDuration
    track.extend(elem(&[0xE0], &video));
    let tracks = elem(&[0x16, 0x54, 0xAE, 0x6B], &elem(&[0xAE], &track));

    // clusters: one per key frame (and at most ~20 s long so block offsets fit in 16 bits)
    let mut clusters: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        let start = blocks[i].ts_ms;
        let mut body = uint(&[0xE7], start);
        let mut j = i;
        while j < blocks.len() && (j == i || (!blocks[j].key && blocks[j].ts_ms - start < 20_000)) {
            let b = &blocks[j];
            let mut sb = vec![0x81]; // track 1
            sb.extend_from_slice(&((b.ts_ms - start) as i16).to_be_bytes());
            sb.push(if b.key { 0x80 } else { 0x00 });
            sb.extend_from_slice(&b.data);
            body.extend(elem(&[0xA3], &sb));
            j += 1;
        }
        clusters.push((start, elem(&[0x1F, 0x43, 0xB6, 0x75], &body)));
        i = j;
    }

    // layout: SeekHead, Info, Tracks, Clusters, Cues (positions relative to the segment payload)
    let seekhead_len = {
        let mut p = seek(&[0x15, 0x49, 0xA9, 0x66], 0);
        p.extend(seek(&[0x16, 0x54, 0xAE, 0x6B], 0));
        p.extend(seek(&[0x1C, 0x53, 0xBB, 0x6B], 0));
        elem(&[0x11, 0x4D, 0x9B, 0x74], &p).len()
    };
    let info_pos = seekhead_len as u64;
    let tracks_pos = info_pos + info.len() as u64;
    let mut pos = tracks_pos + tracks.len() as u64;
    let mut cues = Vec::new();
    for (ts, c) in &clusters {
        let mut tp = uint(&[0xF7], 1);
        tp.extend(uint(&[0xF1], pos));
        let mut cp = uint(&[0xB3], *ts);
        cp.extend(elem(&[0xB7], &tp));
        cues.extend(elem(&[0xBB], &cp));
        pos += c.len() as u64;
    }
    let cues = elem(&[0x1C, 0x53, 0xBB, 0x6B], &cues);
    let mut sh = seek(&[0x15, 0x49, 0xA9, 0x66], info_pos);
    sh.extend(seek(&[0x16, 0x54, 0xAE, 0x6B], tracks_pos));
    sh.extend(seek(&[0x1C, 0x53, 0xBB, 0x6B], pos));
    let seekhead = elem(&[0x11, 0x4D, 0x9B, 0x74], &sh);
    debug_assert_eq!(seekhead.len(), seekhead_len);

    let mut seg = seekhead;
    seg.extend(info);
    seg.extend(tracks);
    for (_, c) in clusters {
        seg.extend(c);
    }
    seg.extend(cues);

    let mut out = ebml_header();
    out.extend(elem(&[0x18, 0x53, 0x80, 0x67], &seg));
    out
}

// ------------------------------------------------------------------------------ colour

/// BT.709 limited-range planes (4:2:0) of opaque `rgb`.
fn to_yuv420(rgb: &[u8], w: usize, h: usize) -> ([Vec<u8>; 3], usize) {
    let cw = w.div_ceil(2);
    let ch = h.div_ceil(2);
    let mut y = vec![0u8; w * h];
    let mut u = vec![128u8; cw * ch];
    let mut v = vec![128u8; cw * ch];
    for (i, px) in rgb.chunks_exact(3).enumerate() {
        let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        y[i] = (16.0 + (0.2126 * r + 0.7152 * g + 0.0722 * b) * (219.0 / 255.0)).round().clamp(16.0, 235.0) as u8;
    }
    for cy in 0..ch {
        for cx in 0..cw {
            let (mut r, mut g, mut b, mut n) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for dy in 0..2 {
                for dx in 0..2 {
                    let (x, yy) = ((cx * 2 + dx).min(w - 1), (cy * 2 + dy).min(h - 1));
                    let p = (yy * w + x) * 3;
                    r += rgb[p] as f32;
                    g += rgb[p + 1] as f32;
                    b += rgb[p + 2] as f32;
                    n += 1.0;
                }
            }
            let (r, g, b) = (r / n, g / n, b / n);
            let k = 224.0 / 255.0;
            u[cy * cw + cx] = (128.0 + (-0.1146 * r - 0.3854 * g + 0.5 * b) * k).round().clamp(16.0, 240.0) as u8;
            v[cy * cw + cx] = (128.0 + (0.5 * r - 0.4542 * g - 0.0458 * b) * k).round().clamp(16.0, 240.0) as u8;
        }
    }
    ([y, u, v], cw)
}

/// Flatten a premultiplied-RGBA Pixmap onto `bg` and return straight RGB bytes (AV1 has no alpha).
fn flatten_rgb(pm: &figcraft_render::Pixmap, bg: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity((pm.width() * pm.height() * 3) as usize);
    for px in pm.data().chunks_exact(4) {
        let inv = 255 - px[3] as u32;
        for c in 0..3 {
            out.push((px[c] as u32 + (bg[c] as u32 * inv + 127) / 255).min(255) as u8);
        }
    }
    out
}

/// Strip a leading temporal delimiter OBU (Matroska stores temporal units without them).
fn strip_temporal_delimiter(data: &[u8]) -> &[u8] {
    if data.len() >= 2 && data[0] == 0x12 && data[1] == 0x00 { &data[2..] } else { data }
}

/// Encode the animation as an AV1 WebM. Returns the file bytes.
pub fn encode_webm(anim: &Animation, o: &WebmOptions) -> Result<Vec<u8>> {
    let cmd = "motion.export";
    let fps = o.fps.clamp(1, 120);
    let times = anim.frame_times(fps);
    let total = times.len() as u32;
    let (w, h) = anim.size();
    let (wu, hu) = (w as usize, h as usize);
    if wu == 0 || hu == 0 || wu % 2 == 1 || hu % 2 == 1 {
        return Err(bad(cmd, "WebM needs an even, non-zero picture size; lower the scale"));
    }

    let mut enc = EncoderConfig::with_speed_preset(o.speed.min(10));
    enc.width = wu;
    enc.height = hu;
    enc.bit_depth = 8;
    enc.chroma_sampling = ChromaSampling::Cs420;
    enc.time_base = Rational::new(1, fps as u64);
    enc.quantizer = o.quantizer();
    enc.min_quantizer = 0;
    enc.low_latency = true;
    enc.min_key_frame_interval = 0;
    enc.max_key_frame_interval = (fps as u64 * 2).max(1);
    enc.pixel_range = PixelRange::Limited;
    enc.color_description = Some(ColorDescription {
        color_primaries: ColorPrimaries::BT709,
        transfer_characteristics: TransferCharacteristics::BT709,
        matrix_coefficients: MatrixCoefficients::BT709,
    });
    enc.speed_settings.scene_detection_mode = SceneDetectionSpeed::None;
    let threads = if o.threads == 0 { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).min(8) } else { o.threads };
    enc.tiles = if o.tiles > 0 { o.tiles } else { (wu * hu / (640 * 360)).clamp(1, threads.next_power_of_two()) };
    let cfg = Config::new().with_encoder_config(enc).with_threads(threads);
    let mut ctx: Context<u8> = cfg.new_context().map_err(|e| bad(cmd, format!("AV1 encoder: {e}")))?;
    let av1c = ctx.container_sequence_header();

    let mut blocks: Vec<Block> = Vec::with_capacity(times.len());
    let take = |ctx: &mut Context<u8>, blocks: &mut Vec<Block>| -> Result<()> {
        loop {
            match ctx.receive_packet() {
                Ok(p) => {
                    let ts_ms = (p.input_frameno as f64 * 1000.0 / fps as f64).round() as u64;
                    blocks.push(Block { ts_ms, key: p.frame_type == FrameType::KEY, data: strip_temporal_delimiter(&p.data).to_vec() });
                }
                Err(EncoderStatus::Encoded) => continue,
                Err(EncoderStatus::NeedMoreData) | Err(EncoderStatus::LimitReached) => return Ok(()),
                Err(e) => return Err(bad(cmd, format!("AV1 encoder: {e}"))),
            }
        }
    };
    let mut sent = 0u32;
    for &t in &times {
        let pm = anim.render(t).ok_or_else(|| bad(cmd, "a frame failed to render"))?;
        let rgb = flatten_rgb(&pm, o.background);
        let ([py, pu, pv], cw) = to_yuv420(&rgb, wu, hu);
        let mut frame = ctx.new_frame();
        frame.planes[0].copy_from_raw_u8(&py, wu, 1);
        frame.planes[1].copy_from_raw_u8(&pu, cw, 1);
        frame.planes[2].copy_from_raw_u8(&pv, cw, 1);
        ctx.send_frame(frame).map_err(|e| bad(cmd, format!("AV1 encoder: {e}")))?;
        sent += 1;
        take(&mut ctx, &mut blocks)?;
    }
    ctx.flush();
    take(&mut ctx, &mut blocks)?;
    if blocks.len() as u32 != sent || blocks.is_empty() {
        return Err(bad(cmd, format!("AV1 encoder produced {} packets for {sent} frames", blocks.len())));
    }
    blocks.sort_by_key(|b| b.ts_ms);
    let _ = total;
    Ok(mux(w, h, fps, &av1c, &blocks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::anim::motion::TimelineSettings;
    use figcraft_doc::anim::motion::{AnimProperty, AnimValue};
    use figcraft_doc::geom::Vec2;
    use figcraft_doc::{Document, NodeId, NodeKind};

    fn tiny_animated_doc() -> (Document, NodeId) {
        let mut d = Document::new("t");
        let page = d.first_page().unwrap();
        let frame = d.create(page, NodeKind::Frame, "F", 0.0, 0.0, 16.0, 16.0).unwrap();
        let dot = d.create(frame, NodeKind::Rectangle, "Dot", 2.0, 2.0, 8.0, 8.0).unwrap();
        let n = d.node_mut(dot).unwrap();
        let tr = n.motion.ensure_track(AnimProperty::Opacity);
        tr.set_key(0, AnimValue::Scalar(1.0), None);
        tr.set_key(200, AnimValue::Scalar(0.0), None);
        let pos = n.motion.ensure_track(AnimProperty::Position);
        pos.set_key(0, AnimValue::Vec2(Vec2::new(2.0, 2.0)), None);
        pos.set_key(200, AnimValue::Vec2(Vec2::new(6.0, 2.0)), None);
        d.timelines.insert(frame, TimelineSettings { duration_ms: 200, fps: 10, looping: true });
        (d, frame)
    }

    /// EBML structural sanity: a real WebM header, a Segments/Tracks/Cues, and the AV1 codec id.
    #[test]
    fn webm_mux_writes_a_valid_ebml_file() {
        let (d, frame) = tiny_animated_doc();
        let anim = Animation::new(&d, frame, 1.0).unwrap();
        let o = WebmOptions { fps: 5, speed: 10, threads: 1, tiles: 1, ..Default::default() };
        let webm = encode_webm(&anim, &o).unwrap();
        // EBML magic + DocType "webm" + Segment id.
        assert_eq!(&webm[0..4], &[0x1A, 0x45, 0xDF, 0xA3]);
        assert!(webm.windows(4).any(|w| w == b"webm"));
        assert!(webm.windows(4).any(|w| w == [0x18, 0x53, 0x80, 0x67]), "Segment element");
        assert!(webm.windows(5).any(|w| w == b"V_AV1"), "AV1 codec id");
        assert!(webm.len() > 200, "real encoded frames, not just a header");
    }

    #[test]
    fn quantizer_maps_quality_to_rav1e_range() {
        let mut o = WebmOptions::default();
        o.quality = 100;
        assert_eq!(o.quantizer(), 55);
        o.quality = 1;
        assert_eq!(o.quantizer(), 253);
        o.quality = 85;
        assert_eq!(o.quantizer(), 85);
    }

    #[test]
    fn odd_size_is_refused_before_allocation() {
        let (mut d, frame) = tiny_animated_doc();
        // Make the frame an odd width: 17 px at scale 1.
        if let Some(n) = d.node_mut(frame) {
            n.width = 17.0;
        }
        let anim = Animation::new(&d, frame, 1.0).unwrap();
        let o = WebmOptions { fps: 5, speed: 10, threads: 1, tiles: 1, ..Default::default() };
        assert!(encode_webm(&anim, &o).is_err(), "4:2:0 AV1 requires even dimensions");
    }
}
