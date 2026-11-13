//! 画布提示带（auto layout 的 padding / spacing）：垫在子层**下面**铺的色块与斜线。
//!
//! 为什么放在渲染里而不是叠加层：带子的边缘和子层的边缘是同一条线。若带子画在贴图之上，
//! 子层在贴图里的抗锯齿边缘会把**画框底色**混进边界那一列（半物体半白），带子只盖到一半，
//! 于是看着像一条白细线。画在子层之前，子层的边缘就是和带子混色，边界只有一次合成。
//!
//! 坐标一律是**页面坐标**；图案周期/线宽也按页面单位给，调用方自己用 zoom 换算成屏幕像素。

use figcraft_geom::{Rect, Shape};
use tiny_skia::{FillRule, Paint, Path, PathBuilder, Pixmap};

use crate::Band;
use crate::draw::{ts_path, ts_transform};

/// 铺一块带子：底色 + 斜线。
pub(crate) fn paint(page_xf: figcraft_geom::Affine, pix: &mut Pixmap, b: &Band) {
    let r = Rect::new(b.rect[0], b.rect[1], b.rect[2], b.rect[3]);
    if !(r.width() > 0.0 && r.height() > 0.0 && r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite()) {
        return;
    }
    if b.fill[3] > 0
        && let Some(tp) = ts_path(&r.to_path(0.1))
    {
        fill_path(pix, &tp, b.fill, page_xf);
    }
    if let Some(s) = &b.stripe {
        stripes(page_xf, pix, r, s);
    }
}

fn fill_path(pix: &mut Pixmap, path: &Path, rgba: [u8; 4], page_xf: figcraft_geom::Affine) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]);
    paint.anti_alias = true;
    pix.fill_path(path, &paint, FillRule::Winding, ts_transform(page_xf), None);
}

/// 45°/135° 斜线：线心 `x + y = n·period + (ox+oy)`（m>0）或 `y - x = n·period - (ox-oy)`（m<0），
/// 每条裁进带子里再填——线段有半宽会戳出带子，多边形不会。
fn stripes(page_xf: figcraft_geom::Affine, pix: &mut Pixmap, r: Rect, s: &crate::Stripe) {
    let period = s.period;
    let half = s.width / 2.0;
    if !(period.is_finite() && period > 0.0 && half.is_finite() && half >= 0.0) {
        return;
    }
    let off = if s.m > 0.0 { s.origin[0] + s.origin[1] } else { -(s.origin[0] - s.origin[1]) };
    let (lo, hi) = if s.m > 0.0 { (r.x0 + r.y0 - off, r.x1 + r.y1 - off) } else { (r.y0 - r.x1 - off, r.y1 - r.x0 - off) };
    // 多算两头：只擦到一个角的条纹也要画。
    let reach = 2.0 * half * std::f64::consts::SQRT_2;
    let start = ((lo - reach) / period).floor() * period;
    let mut c = start;
    // 上限保护：周期小到离谱时不至于画出几万条。
    let mut guard = 0;
    while c <= hi + reach && guard < 4096 {
        guard += 1;
        if let Some(poly) = stripe_polygon(s.m, c + off, half, r) {
            fill_poly(pix, &poly, s.color, page_xf);
        }
        c += period;
    }
}

fn fill_poly(pix: &mut Pixmap, pts: &[[f64; 2]], rgba: [u8; 4], page_xf: figcraft_geom::Affine) {
    let Some((first, rest)) = pts.split_first() else { return };
    let mut b = PathBuilder::new();
    b.move_to(first[0] as f32, first[1] as f32);
    for p in rest {
        b.line_to(p[0] as f32, p[1] as f32);
    }
    b.close();
    if let Some(path) = b.finish() {
        fill_path(pix, &path, rgba, page_xf);
    }
}

/// 一条斜线落在 `r` 里的精确形状（3~6 边形）；None：这条线没穿过带子。
fn stripe_polygon(m: f32, c: f64, half: f64, r: Rect) -> Option<Vec<[f64; 2]>> {
    let (dx, dy) = if m > 0.0 { (1.0f64, -1.0f64) } else { (1.0, 1.0) };
    let len = (dx * dx + dy * dy).sqrt();
    let (ux, uy) = (dx / len, dy / len);
    let (nx, ny) = (-uy, ux);
    let (cx, cy) = (r.x0 + r.width() / 2.0, r.y0 + r.height() / 2.0);
    let (px, py) = if m > 0.0 {
        let t = (c - cx - cy) / 2.0;
        (cx + t, cy + t)
    } else {
        let t = (c - (cy - cx)) / 2.0;
        (cx - t, cy + t)
    };
    let big = (r.width() + r.height()) * 2.0 + len * 4.0;
    let quad = [
        [px + ux * big + nx * half, py + uy * big + ny * half],
        [px + ux * big - nx * half, py + uy * big - ny * half],
        [px - ux * big - nx * half, py - uy * big - ny * half],
        [px - ux * big + nx * half, py - uy * big + ny * half],
    ];
    let poly = clip_to_rect(&quad, r);
    (poly.len() >= 3).then_some(poly)
}

/// Sutherland–Hodgman：把凸多边形按矩形四条边裁一遍。
fn clip_to_rect(poly: &[[f64; 2]], r: Rect) -> Vec<[f64; 2]> {
    let (x0, y0, x1, y1) = (r.x0, r.y0, r.x1, r.y1);
    let mut p = poly.to_vec();
    p = clip_edge(&p, |q| q[0] >= x0, |a, b| [x0, a[1] + (b[1] - a[1]) * ((x0 - a[0]) / (b[0] - a[0]))]);
    p = clip_edge(&p, |q| q[0] <= x1, |a, b| [x1, a[1] + (b[1] - a[1]) * ((x1 - a[0]) / (b[0] - a[0]))]);
    p = clip_edge(&p, |q| q[1] >= y0, |a, b| [a[0] + (b[0] - a[0]) * ((y0 - a[1]) / (b[1] - a[1])), y0]);
    clip_edge(&p, |q| q[1] <= y1, |a, b| [a[0] + (b[0] - a[0]) * ((y1 - a[1]) / (b[1] - a[1])), y1])
}

fn clip_edge(poly: &[[f64; 2]], keep: impl Fn([f64; 2]) -> bool, cut: impl Fn([f64; 2], [f64; 2]) -> [f64; 2]) -> Vec<[f64; 2]> {
    if poly.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let (ka, kb) = (keep(a), keep(b));
        if ka {
            out.push(a);
        }
        if ka != kb {
            out.push(cut(a, b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band_rect() -> Rect {
        Rect::new(10.0, 20.0, 90.0, 44.0)
    }

    /// 每条斜线都裁进带子里：所有顶点落在带子内（两端不出界、也不缺口）。
    #[test]
    fn every_stripe_stays_inside_the_band() {
        let r = band_rect();
        for m in [1.0f32, -1.0] {
            let mut painted = 0;
            for k in -20..20 {
                let c = k as f64 * 8.485281374238571;
                if let Some(poly) = stripe_polygon(m, c, 1.0, r) {
                    painted += 1;
                    assert!(poly.len() >= 3);
                    for q in &poly {
                        assert!(q[0] >= r.x0 - 1e-6 && q[0] <= r.x1 + 1e-6 && q[1] >= r.y0 - 1e-6 && q[1] <= r.y1 + 1e-6, "{q:?} pokes out of {r:?}");
                    }
                }
            }
            assert!(painted > 5, "the band is hatched on both diagonals");
        }
    }

    /// 完全在带子外的线不产生形状；只擦过一个角的会留下一个小三角形。
    #[test]
    fn a_stripe_that_misses_the_band_draws_nothing() {
        let r = band_rect();
        assert!(stripe_polygon(1.0, 1000.0, 1.0, r).is_none());
        assert!(stripe_polygon(-1.0, -1000.0, 1.0, r).is_none());
        let corner = stripe_polygon(1.0, r.x0 + r.y0, 1.0, r).expect("a sliver at the corner");
        assert_eq!(corner.len(), 3);
    }

    /// 空带/离谱参数不画、不炸。
    #[test]
    fn absurd_bands_are_safe() {
        let mut pix = Pixmap::new(16, 16).expect("pixmap");
        let tf = figcraft_geom::Affine::IDENTITY;
        let base = Band { owner: figcraft_doc::NodeId(1), rect: [0.0, 0.0, 0.0, 0.0], fill: [255, 0, 0, 40], stripe: None };
        paint(tf, &mut pix, &base);
        let nan = Band { rect: [f64::NAN, 0.0, 1.0, 1.0], ..base.clone() };
        paint(tf, &mut pix, &nan);
        let huge = Band {
            rect: [0.0, 0.0, 16.0, 16.0],
            stripe: Some(crate::Stripe { m: 1.0, color: [0, 0, 255, 150], period: 1e-9, width: 1.0, origin: [0.0, 0.0] }),
            ..base.clone()
        };
        paint(tf, &mut pix, &huge);
        let zero = Band {
            rect: [0.0, 0.0, 16.0, 16.0],
            stripe: Some(crate::Stripe { m: -1.0, color: [0, 0, 255, 150], period: 0.0, width: 1.0, origin: [0.0, 0.0] }),
            ..base
        };
        paint(tf, &mut pix, &zero);
        // 只画了底色：有任何像素被改过就说明没崩着乱画。
        assert!(pix.data().chunks_exact(4).any(|p| p[3] > 0));
    }
}
