//! 相邻同色形状之间的接缝：是**光栅器自身的抗锯齿合成**，不是画布/贴图的问题。
//!
//! 两个共边的实色矩形，各自带 1px 抗锯齿画到同一张不透明底图上。共用边落在半个像素上时，
//! 左块在该列覆盖 a，右块覆盖 1-a；两次 SourceOver 合成后这列的颜色是
//! `c*(a + (1-a) - a(1-a)) + bg*a(1-a)`，比实色浅 `a(1-a)` 倍——a=0.5 时最明显，
//! 也就是那条 1px、约 20% 偏亮的线。边正好落在整像素上时 a=1 或 0，接缝消失。
//!
//! 这个测试把结论钉住：同样的输入，**只用 tiny-skia** 就有同样的接缝。

use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect as TsRect, Shader, Transform};

const W: u32 = 64;
const H: u32 = 16;
/// 共用的那条边在设备像素里的位置：.5 就是落在半像素上。
const SHARED: f32 = 32.5;

fn fill(color: Color) -> Paint<'static> {
    Paint { shader: Shader::SolidColor(color), anti_alias: true, ..Default::default() }
}

fn rect(x0: f32, x1: f32) -> tiny_skia::Path {
    let mut b = PathBuilder::new();
    b.push_rect(TsRect::from_ltrb(x0, 0.0, x1, H as f32).expect("rect"));
    b.finish().expect("path")
}

/// 两个共边同色矩形，共边在 `shared` 处。
fn two_rects(shared: f32) -> Pixmap {
    let mut pix = Pixmap::new(W, H).expect("pixmap");
    pix.fill(Color::WHITE);
    let green = Color::from_rgba8(0x2F, 0xA3, 0x6B, 255);
    pix.fill_path(&rect(0.0, shared), &fill(green), FillRule::Winding, Transform::identity(), None);
    pix.fill_path(&rect(shared, W as f32), &fill(green), FillRule::Winding, Transform::identity(), None);
    pix
}

fn column(pix: &Pixmap, x: u32) -> (u8, u8, u8) {
    let i = (y_mid() * W + x) as usize * 4;
    let d = pix.data();
    (d[i], d[i + 1], d[i + 2])
}

fn y_mid() -> u32 {
    H / 2
}

const GREEN: (u8, u8, u8) = (0x2F, 0xA3, 0x6B);

#[test]
fn a_shared_edge_on_a_pixel_boundary_has_no_seam() {
    let pix = two_rects(32.0);
    for x in 0..W {
        assert_eq!(column(&pix, x), GREEN, "column {x} is the fill");
    }
}

#[test]
fn a_shared_edge_between_pixels_leaves_a_one_pixel_seam() {
    let pix = two_rects(SHARED);
    // 共边落在 32.5：只有跨在边上的那一列（32）被两边各覆盖一半，
    // 合成后偏亮一点点；两侧其余列都是实色。
    let seam = column(&pix, 32);
    assert_ne!(seam, GREEN, "the seam is there: {seam:?}");
    // 偏亮的幅度就是被底色透出来的那点：每通道都比实色更接近白色。
    for (g, w) in [(seam.0, GREEN.0), (seam.1, GREEN.1), (seam.2, GREEN.2)] {
        assert!(g > w, "lighter than the fill: {seam:?} vs {GREEN:?}");
    }
    // 而且只有这一列：接缝**不扩散**（贴图呈现没有把它糊成一片）。
    assert_eq!(column(&pix, 31), GREEN);
    assert_eq!(column(&pix, 33), GREEN);
}

#[test]
fn one_path_for_both_rects_has_no_seam_whatever_the_scale() {
    let mut pix = Pixmap::new(W, H).expect("pixmap");
    pix.fill(Color::WHITE);
    let mut b = PathBuilder::new();
    b.push_rect(TsRect::from_ltrb(0.0, 0.0, SHARED, H as f32).expect("left"));
    b.push_rect(TsRect::from_ltrb(SHARED, 0.0, W as f32, H as f32).expect("right"));
    let both = b.finish().expect("path");
    pix.fill_path(&both, &fill(Color::from_rgba8(0x2F, 0xA3, 0x6B, 255)), FillRule::Winding, Transform::identity(), None);
    // 同一条路径里两块拼成一块，共用边不产生任何中间列。
    for x in 0..W {
        assert_eq!(column(&pix, x), GREEN, "column {x} is the fill");
    }
}

// ---------------------------------------------------------------------------
// 端到端：走真实的 `draw_children`，两个相邻同色矩形并成一条路径。
// ---------------------------------------------------------------------------

use figcraft_color::{Color as FgColor, Paint as FgPaint};
use figcraft_doc::{Document, NodeKind};

use crate::{Options, View, render_view};

/// 页面里两个共边的同色矩形，共边在页面 x = 50 处。
fn adjacent_rects() -> (Document, figcraft_doc::NodeId) {
    let mut d = Document::new("seam");
    let page = d.first_page().expect("page");
    let green = FgColor::from_rgba8(0x2F, 0xA3, 0x6B, 255);
    for (name, x, w) in [("left", 0.0, 50.0), ("right", 50.0, 50.0)] {
        let id = d.create(page, NodeKind::Rectangle, name, x, 0.0, w, 20.0).expect("rect");
        d.get_mut(id).expect("node").fills = vec![FgPaint::solid(green)];
    }
    (d, page)
}

fn pixel(pix: &tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
    let i = (y * pix.width() + x) as usize * 4;
    let d = pix.data();
    (d[i], d[i + 1], d[i + 2])
}

#[test]
fn adjacent_rects_of_one_colour_leave_no_seam_at_a_fractional_offset() {
    let (d, page) = adjacent_rects();
    // 视图偏移半个像素：共边落在设备像素 49.5，正是以前露出 1px 亮线的位置。
    let v = View { origin: (0.5, 0.0), scale: 1.0, width: 100, height: 20 };
    let pix = render_view(&d, page, &v, &Options::default()).expect("render");
    for y in [5, 10, 15] {
        for x in 2..98 {
            assert_eq!(pixel(&pix, x, y), GREEN, "({x}, {y}) is the fill, with no seam");
        }
    }
}

const RED: (u8, u8, u8) = (0xC8, 0x33, 0x33);
const BLUE: (u8, u8, u8) = (0x33, 0x66, 0xCC);

fn solid(d: &mut Document, parent: figcraft_doc::NodeId, name: &str, x: f64, y: f64, w: f64, h: f64, rgb: (u8, u8, u8)) -> figcraft_doc::NodeId {
    let id = d.create(parent, NodeKind::Rectangle, name, x, y, w, h).expect("rect");
    d.get_mut(id).expect("node").fills = vec![FgPaint::solid(FgColor::from_rgba8(rgb.0, rgb.1, rgb.2, 255))];
    id
}

/// 两个共边同色矩形，中间夹着一个**不重叠**的别色图层：并边不该被它挡住，接缝照样要消失。
#[test]
fn a_layer_between_two_rects_that_touches_neither_blocks_no_merge() {
    let mut d = Document::new("seam");
    let page = d.first_page().expect("page");
    solid(&mut d, page, "left", 0.0, 0.0, 50.0, 20.0, GREEN);
    // 画在两者之间，但离它们的共用边（页面 x = 50）很远：既没挡住左块也没挡住右块。
    solid(&mut d, page, "between", 150.0, 40.0, 20.0, 20.0, BLUE);
    solid(&mut d, page, "right", 50.0, 0.0, 50.0, 20.0, GREEN);
    let v = View { origin: (0.5, 0.0), scale: 1.0, width: 100, height: 20 };
    let pix = render_view(&d, page, &v, &Options::default()).expect("render");
    for y in [5, 10, 15] {
        for x in 2..98 {
            assert_eq!(pixel(&pix, x, y), GREEN, "({x}, {y}) is the fill, with no seam");
        }
    }
}

/// 夹在中间的那层如果压在右块上，右块原本盖着它；并边会把右块提到它下面，绘制顺序就变了
/// ——这种情况必须放弃并边。
#[test]
fn a_layer_between_two_rects_that_it_paints_over_blocks_the_merge() {
    let mut d = Document::new("seam");
    let page = d.first_page().expect("page");
    solid(&mut d, page, "left", 0.0, 0.0, 50.0, 20.0, GREEN);
    // 画在两者之间、并且压住右块：右块在它上面。
    solid(&mut d, page, "between", 80.0, 0.0, 10.0, 20.0, RED);
    solid(&mut d, page, "right", 50.0, 0.0, 50.0, 20.0, GREEN);
    let v = View { origin: (0.5, 0.0), scale: 1.0, width: 100, height: 20 };
    let pix = render_view(&d, page, &v, &Options::default()).expect("render");
    // 右块盖着红块的地方仍是绿：并边若把右块提前画，这里会露出红色。
    for x in [81, 85, 89] {
        assert_eq!(pixel(&pix, x, 10), GREEN, "({x}) keeps its place above the red layer");
    }
    // 红块只在右块之外露出来……它整块都在右块下面，所以一行全绿。
    assert_eq!(pixel(&pix, 10, 10), GREEN);
}

/// 分处两个 Group 的同色矩形：Group 自己什么都不画，两者的子层落在同一张图上，也要并边。
#[test]
fn rects_in_different_plain_containers_leave_no_seam() {
    let mut d = Document::new("seam");
    let page = d.first_page().expect("page");
    let a = d.create(page, NodeKind::Group, "a", 0.0, 0.0, 50.0, 20.0).expect("group");
    let b = d.create(page, NodeKind::Group, "b", 50.0, 0.0, 50.0, 20.0).expect("group");
    solid(&mut d, a, "left", 0.0, 0.0, 50.0, 20.0, GREEN);
    solid(&mut d, b, "right", 0.0, 0.0, 50.0, 20.0, GREEN);
    let v = View { origin: (0.5, 0.0), scale: 1.0, width: 100, height: 20 };
    let pix = render_view(&d, page, &v, &Options::default()).expect("render");
    for y in [5, 10, 15] {
        for x in 2..98 {
            assert_eq!(pixel(&pix, x, y), GREEN, "({x}, {y}) is the fill, with no seam");
        }
    }
}
