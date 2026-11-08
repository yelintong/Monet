//! 无显示环境下的画布预览：把画布实际会画的内容合成成一张 PNG。
//!
//! 复用**画布自身的绘制指令**（`canvas::show` 记录下来的 `Painter.ops`），
//! 所以这张图真实反映画布的排版、颜色与叠加层位置。
//! 唯一不包含的是矢量文字（文字由 GPUI 文本元素渲染，进程外无法光栅化）。

use figcraft_render::{Retained, View};
use image::{ImageEncoder, RgbaImage};

use crate::app::{CanvasApp, sample_doc};
use crate::canvas::geom::*;
use crate::canvas::painter::{Op, Painter};
use crate::canvas::{draw_overlays, presented_image_rect};
use crate::theme::doc_c32;

const W: u32 = 1280;
const H: u32 = 860;

/// 写出 `path`（PNG）。等价于真机窗口里画布的内容（不含矢量文字）。
pub fn export_preview(path: &str) -> Result<(), String> {
    let mut app = CanvasApp::new(sample_doc());
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(W as f32, H as f32));
    app.canvas.rect = rect;

    // 与 show() 相同的视口同步 + 首次 fit。
    {
        let Ok(ds) = app.session.ds_mut() else { return Err("no document".into()) };
        ds.view.vw = f64::from(rect.width());
        ds.view.vh = f64::from(rect.height());
        ds.view.rulers = true;
        ds.zoom_to_fit();
    }

    let (doc, page) = {
        let Ok(ds) = app.session.ds() else { return Err("no document".into()) };
        // 选中第一个子层，让预览里能看到选中框/手柄/尺寸标注。
        (ds.doc.clone(), ds.page)
    };
    if let Some(first) = doc.node(page).and_then(|n| n.children.first().copied()) {
        app.session.set_selection(vec![first]);
    }
    let origin = app.session.ds().map(|d| d.view.origin()).unwrap_or((0.0, 0.0));
    let zoom = app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0);
    let view = View { origin, scale: zoom, width: W, height: H };

    let opts = figcraft_render::Options { lod: true, magnified_image_pixels: true, ..Default::default() };
    let mut retained = Retained::default();
    let (view, pix, _work) = retained.render(&doc, page, &view, &opts).ok_or_else(|| "render failed".to_string())?;

    // 画布底：页面底色。
    let bg = match doc.node(page).map(|n| &n.kind) {
        Some(figcraft_doc::NodeKind::Page { background, .. }) => doc_c32(*background),
        _ => Color32::from_rgb(0x1E, 0x1E, 0x1E),
    };
    app.canvas.page = bg;
    let mut buf = vec![0u8; (W * H * 4) as usize];
    fill(&mut buf, W, H, 0, 0, W as i32, H as i32, bg);

    // 贴图：按 presented_image_rect 重投影到屏幕。
    let tl = app.to_screen(view.origin.0, view.origin.1);
    let r = presented_image_rect(tl, &view, zoom, 1.0, false);
    blit_scaled(&mut buf, W, H, pix.data(), pix.width(), pix.height(), r);

    // 叠加层：同一套记录指令。
    let t = app.theme;
    let mut painter = Painter::new(rect);
    draw_overlays(&mut app, &mut painter, &t);

    for op in &painter.ops {
        replay(&mut buf, W, H, op);
    }

    write_png(path, &buf)
}

// ------------------------------------------------------------------ CPU 合成

fn write_png(path: &str, buf: &[u8]) -> Result<(), String> {
    let img = RgbaImage::from_raw(W, H, buf.to_vec()).ok_or_else(|| "buffer size mismatch".to_string())?;
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let writer = std::io::BufWriter::new(file);
    image::codecs::png::PngEncoder::new(writer).write_image(img.as_raw(), W, H, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())
}

fn blend(buf: &mut [u8], w: u32, h: u32, x: i32, y: i32, c: Color32) {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return;
    }
    let a = u16::from(c.0[3]);
    if a == 0 {
        return;
    }
    let i = ((y as u32 * w + x as u32) * 4) as usize;
    if a == 255 {
        buf[i] = c.0[0];
        buf[i + 1] = c.0[1];
        buf[i + 2] = c.0[2];
        buf[i + 3] = 255;
        return;
    }
    let inv = 255 - a;
    for k in 0..3 {
        buf[i + k] = ((u16::from(c.0[k]) * a + u16::from(buf[i + k]) * inv) / 255) as u8;
    }
    buf[i + 3] = 255;
}

fn fill(buf: &mut [u8], w: u32, h: u32, x0: i32, y0: i32, x1: i32, y1: i32, c: Color32) {
    for y in y0.max(0)..y1.min(h as i32) {
        for x in x0.max(0)..x1.min(w as i32) {
            blend(buf, w, h, x, y, c);
        }
    }
}

fn line(buf: &mut [u8], w: u32, h: u32, a: Pos2, b: Pos2, stroke: &Stroke) {
    let steps = (b.x - a.x).abs().max((b.y - a.y).abs()).max(1.0) as i32;
    let half = (stroke.width / 2.0).max(0.5);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let (x, y) = (a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        let x0 = (x - half).floor() as i32;
        let y0 = (y - half).floor() as i32;
        let x1 = (x + half).ceil() as i32;
        let y1 = (y + half).ceil() as i32;
        fill(buf, w, h, x0, y0, x1, y1, stroke.color);
    }
}

fn circle(buf: &mut [u8], w: u32, h: u32, c: Pos2, radius: f32, fill_c: Color32, stroke: &Stroke) {
    let r0 = radius.ceil() as i32;
    let inner = (radius - stroke.width / 2.0).max(-1.0);
    let outer = radius + stroke.width / 2.0;
    for dy in -r0..=r0 {
        for dx in -r0..=r0 {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            let (x, y) = (c.x as i32 + dx, c.y as i32 + dy);
            if fill_c.a() > 0 && radius > 0.0 && d <= radius {
                blend(buf, w, h, x, y, fill_c);
            }
            if stroke.width > 0.0 && stroke.color.a() > 0 && d >= inner && d <= outer {
                blend(buf, w, h, x, y, stroke.color);
            }
        }
    }
}

fn poly(buf: &mut [u8], w: u32, h: u32, pts: &[Pos2], closed: bool, fill_c: Color32, stroke: &Stroke) {
    if fill_c.a() > 0 && pts.len() >= 3 {
        let miny = pts.iter().map(|p| p.y).fold(f32::MAX, f32::min).max(0.0) as i32;
        let maxy = pts.iter().map(|p| p.y).fold(f32::MIN, f32::max).min(h as f32) as i32;
        for y in miny..=maxy {
            let yc = y as f32 + 0.5;
            let mut xs: Vec<f32> = Vec::new();
            let n = pts.len();
            for i in 0..n {
                let a = pts[i];
                let b = pts[(i + 1) % n];
                if (a.y <= yc && b.y > yc) || (b.y <= yc && a.y > yc) {
                    let t = (yc - a.y) / (b.y - a.y);
                    xs.push(a.x + (b.x - a.x) * t);
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in xs.chunks(2) {
                if pair.len() == 2 {
                    let (x0, x1) = (pair[0].round() as i32, pair[1].round() as i32);
                    fill(buf, w, h, x0, y, x1, y + 1, fill_c);
                }
            }
        }
    }
    if stroke.width > 0.0 && stroke.color.a() > 0 {
        let n = if closed { pts.len() } else { pts.len().saturating_sub(1) };
        for i in 0..n {
            line(buf, w, h, pts[i], pts[(i + 1) % pts.len()], stroke);
        }
    }
}

fn replay(buf: &mut [u8], w: u32, h: u32, op: &Op) {
    match op {
        Op::Rect { rect, radius, fill: f, stroke, kind } => {
            let (r, bw) = match kind {
                StrokeKind::Inside => (*rect, stroke.width),
                StrokeKind::Middle => (rect.expand(stroke.width / 2.0), stroke.width),
                StrokeKind::Outside => (rect.expand(stroke.width), stroke.width),
            };
            if !r.is_positive() {
                return;
            }
            let (x0, y0, x1, y1) = (r.min.x as i32, r.min.y as i32, r.max.x.ceil() as i32, r.max.y.ceil() as i32);
            let rad = f32::from(radius.nw.max(radius.ne).max(radius.sw).max(radius.se));
            if f.a() > 0 {
                if rad > 0.5 {
                    let c = r.center();
                    poly(buf, w, h, &rounded(r, rad), true, *f, &Stroke::NONE);
                    let _ = c;
                } else {
                    fill(buf, w, h, x0, y0, x1, y1, *f);
                }
            }
            if bw > 0.0 && stroke.color.a() > 0 {
                let s = Stroke::new(bw, stroke.color);
                if rad > 0.5 {
                    poly(buf, w, h, &rounded(r, rad), true, Color32::TRANSPARENT, &s);
                } else {
                    fill(buf, w, h, x0, y0, x1, y0 + bw.ceil() as i32, s.color);
                    fill(buf, w, h, x0, y1 - bw.ceil() as i32, x1, y1, s.color);
                    fill(buf, w, h, x0, y0, x0 + bw.ceil() as i32, y1, s.color);
                    fill(buf, w, h, x1 - bw.ceil() as i32, y0, x1, y1, s.color);
                }
            }
        }
        Op::Line { a, b, stroke } => line(buf, w, h, *a, *b, stroke),
        Op::Circle { center, radius, fill: f, stroke } => circle(buf, w, h, *center, *radius, *f, stroke),
        Op::Poly { points, closed, fill: f, stroke } => poly(buf, w, h, points, *closed, *f, stroke),
    }
}

fn rounded(r: Rect, rad: f32) -> Vec<Pos2> {
    let rad = rad.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let mut out = Vec::new();
    let centers = [
        (pos2(r.min.x + rad, r.min.y + rad), std::f32::consts::PI),
        (pos2(r.max.x - rad, r.min.y + rad), std::f32::consts::PI * 1.5),
        (pos2(r.max.x - rad, r.max.y - rad), 0.0),
        (pos2(r.min.x + rad, r.max.y - rad), std::f32::consts::FRAC_PI_2),
    ];
    for (c, start) in centers {
        for i in 0..=6 {
            let a = start + i as f32 / 6.0 * std::f32::consts::FRAC_PI_2;
            out.push(pos2(c.x + rad * a.cos(), c.y + rad * a.sin()));
        }
    }
    out
}

fn blit_scaled(buf: &mut [u8], w: u32, h: u32, src: &[u8], sw: u32, sh: u32, dest: Rect) {
    if sw == 0 || sh == 0 || !dest.is_positive() {
        return;
    }
    let x0 = dest.min.x.floor().max(0.0) as i32;
    let y0 = dest.min.y.floor().max(0.0) as i32;
    let x1 = dest.max.x.ceil().min(w as f32) as i32;
    let y1 = dest.max.y.ceil().min(h as f32) as i32;
    for y in y0..y1 {
        for x in x0..x1 {
            let u = ((x as f32 - dest.min.x) / dest.width() * sw as f32) as u32;
            let v = ((y as f32 - dest.min.y) / dest.height() * sh as f32) as u32;
            let (u, v) = (u.min(sw - 1), v.min(sh - 1));
            let si = ((v * sw + u) * 4) as usize;
            let c = Color32([src[si], src[si + 1], src[si + 2], src[si + 3]]);
            blend(buf, w, h, x, y, c);
        }
    }
}
