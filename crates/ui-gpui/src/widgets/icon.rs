// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// ============================================================================
//! FigCraft's icon set, drawn in code as filled vector glyphs.
//!
//! GPUI 0.2.1 has no SVG-string element and no icon font, and its `Path` is an atlas-backed triangle
//! fan, so each glyph is composed from filled triangles. The grid is the 16×16 design box; `size`
//! scales it. Callers pass an `Icon` and a colour; the element draws whatever it knows and falls
//! back to a neutral dot for the rest.

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, PathBuilder, Pixels, Point, Style, Window, px,
};

/// 让旧的 `push_triangle` 调用继续可用：`Path` 的手写三角面路径在 0.2.1 里 bounds 从 (0,0) 起算，
/// 常被剔除而**画不出来**；改走仓库里已验证可用的 `PathBuilder::fill()` + `add_polygon`。
trait TriPush {
    fn push_triangle(&mut self, xy: (Point<Pixels>, Point<Pixels>, Point<Pixels>), st: (Point<f32>, Point<f32>, Point<f32>));
}

impl TriPush for PathBuilder {
    fn push_triangle(&mut self, xy: (Point<Pixels>, Point<Pixels>, Point<Pixels>), _st: (Point<f32>, Point<f32>, Point<f32>)) {
        self.add_polygon(&[xy.0, xy.1, xy.2], true);
    }
}
use gpui::{DefiniteLength, Length, Size};

/// Every icon FigCraft's UI can show. The names mirror `crates/ui-egui/src/icons.rs` so the two
/// front-ends share one vocabulary; glyphs are drawn for the common ones and fall back otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Plus,
    Minus,
    Close,
    Check,
    ChevronDown,
    ChevronUp,
    ChevronLeft,
    ChevronRight,
    Search,
    Dot,
    Variable,
    LinkOn,
    LinkOff,
    Section,
    Panel,
    Menu,
    Settings,
    Trash,
    /// Filled rectangle (the `Rectangle` node kind).
    Rectangle,
    /// Filled ellipse (the `Ellipse` node kind).
    Ellipse,
    /// Outlined rectangle (a `Frame` / `Group` node kind).
    Frame,
    /// The letter "T" (a `Text` node kind).
    Text,
    /// Filled diamond (a `Component` / `ComponentSet` node kind).
    Component,
    /// Outlined diamond (an `Instance` node kind).
    Instance,
    /// Outlined rectangle with a folded corner (the `Page` node kind).
    Page,
    /// A picture frame with a sun and a hill (the `Image` fill).
    Image,
    /// A 3×3 tile grid (the `Pattern` fill).
    ViewGrid,
    /// A broken link (unbind a variable).
    Detach,
    /// The six alignment glyphs (`align.left` … `align.bottom`) and the distribute one.
    AlignLeft,
    AlignHCenter,
    AlignRight,
    AlignTop,
    AlignVCenter,
    AlignBottom,
    DistributeH,
    /// Rotate 90°, flip horizontally / vertically, and the rotation field's angle mark.
    Rotate,
    FlipH,
    FlipV,
    Angle,
    /// "Ignore auto layout" (absolute position) — four corner brackets.
    AbsolutePosition,
    // ── The bottom toolbar's marks (`ui-egui/src/toolbar.rs`) ────────────────────────────────
    //
    // The tool glyphs, the mode switcher's four marks and the Actions bolt. Geometry is ported
    // from `ui-egui/src/icons/draw.rs`'s own 16-unit table, drawn here from this crate's filled
    // primitives (`tri` / `thick_line` / `disc` / `rect` / `poly_fill` / `stroke_path`).
    /// The arrow pointer (`ToolId::Move`).
    Move,
    /// An open hand (`ToolId::Hand`).
    Hand,
    /// A frame with a diagonal resize arrow (`ToolId::Scale`).
    Scale,
    /// A knife (`ToolId::Slice`).
    Slice,
    /// A diagonal line (`ToolId::Line`).
    Line,
    /// A line with a head (`ToolId::Arrow`).
    Arrow,
    /// A pentagon (`ToolId::Polygon`).
    Polygon,
    /// A five-point star (`ToolId::Star`).
    Star,
    /// A pen nib (`ToolId::Pen`).
    Pen,
    /// A pencil (`ToolId::Pencil`, the Draw mode mark).
    Pencil,
    /// Two overlapping rings with the lens filled (`ToolId::ShapeBuilder`).
    ShapeBuilder,
    /// A small T over the curve it follows (`ToolId::TextOnPath`).
    TextOnPath,
    /// A speech bubble (`ToolId::Comment`).
    Comment,
    /// The Actions bolt.
    Actions,
    /// Three columns (the Design panel's button).
    Panels,
    /// An almond eye with its pupil (the view-only badge).
    Eye,
    /// A motion path with two keyframes (the Animation mode mark).
    Animation,
    /// Angle brackets (the Inspect mode mark).
    Code,
    /// Outlined rectangle with a nested outline (the `Group` node kind).
    Group,
    /// A triangle silhouette (the `Vector` node kind).
    Vector,
    /// Diamond inside an outline rectangle (the `ComponentSet` node kind).
    ComponentSet,
    /// A diamond with a centre dot (the `Icon` node kind).
    IconKind,
    /// Outlined rectangle with a centre dot (a frame that is a component slot).
    Slot,
    /// A padlock (the layer-lock toggle).
    Lock,
    /// An open padlock (the layer-unlock toggle).
    Unlock,
    /// An almond with a diagonal slash (the layer-hidden toggle).
    EyeOff,
    /// Two overlapping filled discs (boolean `Union`).
    Union,
    /// A filled disc behind an outlined one (boolean `Subtract`).
    Subtract,
    /// Two rings with their shared lens filled (boolean `Intersect`).
    Intersect,
    /// Two rings only (boolean `Exclude`).
    Exclude,
}

/// An icon drawn as a filled vector glyph.
pub struct IconGlyph {
    icon: Icon,
    color: gpui::Rgba,
    size: f32,
}

impl IconGlyph {
    /// A `size`×`size` glyph in `color`.
    pub fn new(icon: Icon, color: impl Into<gpui::Rgba>, size: f32) -> Self {
        Self { icon, color: color.into(), size }
    }
}

impl Element for IconGlyph {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _gid: Option<&GlobalElementId>,
        _iid: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let s = px(self.size);
        let style = Style {
            size: Size { width: Length::Definite(DefiniteLength::from(s)), height: Length::Definite(DefiniteLength::from(s)) },
            ..Style::default()
        };
        (window.request_layout(style, std::iter::empty(), cx), ())
    }

    fn prepaint(
        &mut self,
        _gid: Option<&GlobalElementId>,
        _iid: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _rl: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _gid: Option<&GlobalElementId>,
        _iid: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _rl: &mut Self::RequestLayoutState,
        _pp: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        draw(window, bounds, self.icon, self.color);
    }
}

impl IntoElement for IconGlyph {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

/// Paint `icon` centred in `bounds` with `color`.
fn draw(window: &mut Window, bounds: Bounds<Pixels>, icon: Icon, color: gpui::Rgba) {
    let o = bounds.origin;
    let ox = f32::from(o.x);
    let oy = f32::from(o.y);
    let s: f32 = bounds.size.width.into();
    let mut p = PathBuilder::fill();
    match icon {
        Icon::Plus => {
            thick_line(&mut p, ox, oy, s, 8.0, 3.0, 8.0, 13.0, 2.0);
            thick_line(&mut p, ox, oy, s, 3.0, 8.0, 13.0, 8.0, 2.0);
        }
        Icon::Minus => {
            thick_line(&mut p, ox, oy, s, 3.0, 8.0, 13.0, 8.0, 2.0);
        }
        Icon::Close => {
            thick_line(&mut p, ox, oy, s, 4.0, 4.0, 12.0, 12.0, 2.0);
            thick_line(&mut p, ox, oy, s, 12.0, 4.0, 4.0, 12.0, 2.0);
        }
        Icon::Check => {
            thick_line(&mut p, ox, oy, s, 3.5, 8.5, 6.5, 11.5, 2.0);
            thick_line(&mut p, ox, oy, s, 6.5, 11.5, 12.5, 4.5, 2.0);
        }
        Icon::ChevronDown => {
            thick_line(&mut p, ox, oy, s, 4.0, 6.0, 8.0, 10.0, 2.0);
            thick_line(&mut p, ox, oy, s, 8.0, 10.0, 12.0, 6.0, 2.0);
        }
        Icon::ChevronUp => {
            thick_line(&mut p, ox, oy, s, 4.0, 10.0, 8.0, 6.0, 2.0);
            thick_line(&mut p, ox, oy, s, 8.0, 6.0, 12.0, 10.0, 2.0);
        }
        Icon::ChevronLeft => {
            thick_line(&mut p, ox, oy, s, 10.0, 4.0, 6.0, 8.0, 2.0);
            thick_line(&mut p, ox, oy, s, 6.0, 8.0, 10.0, 12.0, 2.0);
        }
        Icon::ChevronRight => {
            thick_line(&mut p, ox, oy, s, 6.0, 4.0, 10.0, 8.0, 2.0);
            thick_line(&mut p, ox, oy, s, 10.0, 8.0, 6.0, 12.0, 2.0);
        }
        Icon::Search => {
            disc(&mut p, ox, oy, s, 7.0, 7.0, 4.5);
            thick_line(&mut p, ox, oy, s, 10.5, 10.5, 13.5, 13.5, 2.0);
        }
        Icon::Dot | Icon::Variable | Icon::LinkOn | Icon::LinkOff | Icon::Panel | Icon::Menu | Icon::Settings | Icon::Trash => {
            disc(&mut p, ox, oy, s, 8.0, 8.0, 3.5);
        }
        Icon::Section => {
            // A dashed-outline section with its solid title strip (`ui-egui`: a rect at 2.5,5.5 →
            // 13.5,13.5 under a filled 2,2 → 9,6 tab).
            outline_rect(&mut p, ox, oy, s, 2.5, 5.5, 13.5, 13.5, 1.4);
            rect(&mut p, ox, oy, s, 2.0, 2.0, 9.0, 6.0);
        }
        Icon::Rectangle => {
            rect(&mut p, ox, oy, s, 3.0, 3.0, 13.0, 13.0);
        }
        Icon::Ellipse => {
            disc(&mut p, ox, oy, s, 8.0, 8.0, 5.5);
        }
        Icon::Frame => {
            outline_rect(&mut p, ox, oy, s, 3.0, 3.0, 13.0, 13.0, 2.0);
        }
        Icon::Text => {
            thick_line(&mut p, ox, oy, s, 8.0, 3.0, 8.0, 13.0, 2.0);
            thick_line(&mut p, ox, oy, s, 4.0, 3.0, 12.0, 3.0, 2.0);
        }
        Icon::Component => {
            diamond(&mut p, ox, oy, s, 8.0, 8.0, 5.5);
        }
        Icon::Instance => {
            outline_diamond(&mut p, ox, oy, s, 8.0, 8.0, 5.5, 2.0);
        }
        Icon::Page => {
            outline_rect(&mut p, ox, oy, s, 3.0, 3.0, 13.0, 13.0, 2.0);
            thick_line(&mut p, ox, oy, s, 9.0, 3.0, 13.0, 7.0, 2.0);
        }
        Icon::Image => {
            outline_rect(&mut p, ox, oy, s, 2.5, 3.0, 13.5, 13.0, 1.5);
            disc(&mut p, ox, oy, s, 6.0, 6.5, 1.5);
            tri(&mut p, ox, oy, s, 3.5, 12.0, 7.5, 7.0, 11.5, 12.0);
            tri(&mut p, ox, oy, s, 9.0, 12.0, 11.5, 8.5, 13.0, 12.0);
        }
        Icon::Detach => {
            // Two halves of a link, pulled apart: the upper-left and lower-right halves.
            thick_line(&mut p, ox, oy, s, 3.0, 9.0, 6.5, 5.5, 1.8);
            thick_line(&mut p, ox, oy, s, 6.5, 12.0, 3.0, 15.0, 1.8);
            thick_line(&mut p, ox, oy, s, 13.0, 7.0, 9.5, 10.5, 1.8);
            thick_line(&mut p, ox, oy, s, 9.5, 4.0, 13.0, 1.0, 1.8);
        }
        Icon::AlignLeft => {
            thick_line(&mut p, ox, oy, s, 3.0, 2.0, 3.0, 14.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 5.0, 11.0, 5.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 10.0, 9.0, 10.0, 1.6);
        }
        Icon::AlignHCenter => {
            thick_line(&mut p, ox, oy, s, 8.0, 2.0, 8.0, 14.0, 1.6);
            thick_line(&mut p, ox, oy, s, 2.0, 5.0, 14.0, 5.0, 1.6);
            thick_line(&mut p, ox, oy, s, 4.5, 10.0, 11.5, 10.0, 1.6);
        }
        Icon::AlignRight => {
            thick_line(&mut p, ox, oy, s, 13.0, 2.0, 13.0, 14.0, 1.6);
            thick_line(&mut p, ox, oy, s, 5.0, 5.0, 13.0, 5.0, 1.6);
            thick_line(&mut p, ox, oy, s, 7.0, 10.0, 13.0, 10.0, 1.6);
        }
        Icon::AlignTop => {
            thick_line(&mut p, ox, oy, s, 2.0, 3.0, 14.0, 3.0, 1.6);
            thick_line(&mut p, ox, oy, s, 5.0, 3.0, 5.0, 11.0, 1.6);
            thick_line(&mut p, ox, oy, s, 10.0, 3.0, 10.0, 9.0, 1.6);
        }
        Icon::AlignVCenter => {
            thick_line(&mut p, ox, oy, s, 2.0, 8.0, 14.0, 8.0, 1.6);
            thick_line(&mut p, ox, oy, s, 5.0, 2.0, 5.0, 14.0, 1.6);
            thick_line(&mut p, ox, oy, s, 10.0, 4.5, 10.0, 11.5, 1.6);
        }
        Icon::AlignBottom => {
            thick_line(&mut p, ox, oy, s, 2.0, 13.0, 14.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 5.0, 5.0, 5.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 10.0, 7.0, 10.0, 13.0, 1.6);
        }
        Icon::DistributeH => {
            thick_line(&mut p, ox, oy, s, 3.5, 3.0, 3.5, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 8.0, 3.0, 8.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 12.5, 3.0, 12.5, 13.0, 1.6);
        }
        Icon::Rotate => {
            thick_line(&mut p, ox, oy, s, 4.0, 11.0, 4.0, 6.5, 1.6);
            thick_line(&mut p, ox, oy, s, 4.0, 6.5, 6.5, 4.0, 1.6);
            thick_line(&mut p, ox, oy, s, 6.5, 4.0, 10.5, 4.0, 1.6);
            thick_line(&mut p, ox, oy, s, 10.5, 4.0, 12.0, 5.5, 1.6);
            thick_line(&mut p, ox, oy, s, 10.2, 2.2, 12.4, 5.0, 1.6);
            thick_line(&mut p, ox, oy, s, 12.4, 5.0, 9.4, 5.6, 1.6);
        }
        Icon::FlipH => {
            thick_line(&mut p, ox, oy, s, 8.0, 2.0, 8.0, 14.0, 1.6);
            thick_line(&mut p, ox, oy, s, 6.5, 5.0, 3.0, 8.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 8.0, 6.5, 11.0, 1.6);
            thick_line(&mut p, ox, oy, s, 6.5, 11.0, 6.5, 5.0, 1.6);
            thick_line(&mut p, ox, oy, s, 9.5, 5.0, 13.0, 8.0, 1.6);
            thick_line(&mut p, ox, oy, s, 13.0, 8.0, 9.5, 11.0, 1.6);
            thick_line(&mut p, ox, oy, s, 9.5, 11.0, 9.5, 5.0, 1.6);
        }
        Icon::FlipV => {
            thick_line(&mut p, ox, oy, s, 2.0, 8.0, 14.0, 8.0, 1.6);
            thick_line(&mut p, ox, oy, s, 5.0, 6.5, 8.0, 3.0, 1.6);
            thick_line(&mut p, ox, oy, s, 8.0, 3.0, 11.0, 6.5, 1.6);
            thick_line(&mut p, ox, oy, s, 11.0, 6.5, 5.0, 6.5, 1.6);
            thick_line(&mut p, ox, oy, s, 5.0, 9.5, 8.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 8.0, 13.0, 11.0, 9.5, 1.6);
            thick_line(&mut p, ox, oy, s, 11.0, 9.5, 5.0, 9.5, 1.6);
        }
        Icon::Angle => {
            thick_line(&mut p, ox, oy, s, 3.0, 13.0, 13.0, 3.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 13.0, 12.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 10.0, 6.0, 13.0, 1.6);
        }
        Icon::AbsolutePosition => {
            thick_line(&mut p, ox, oy, s, 3.0, 7.0, 3.0, 3.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 3.0, 7.0, 3.0, 1.6);
            thick_line(&mut p, ox, oy, s, 9.0, 3.0, 13.0, 3.0, 1.6);
            thick_line(&mut p, ox, oy, s, 13.0, 3.0, 13.0, 7.0, 1.6);
            thick_line(&mut p, ox, oy, s, 13.0, 9.0, 13.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 13.0, 13.0, 9.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 7.0, 13.0, 3.0, 13.0, 1.6);
            thick_line(&mut p, ox, oy, s, 3.0, 13.0, 3.0, 9.0, 1.6);
        }
        Icon::ViewGrid => {
            for r in 0..3 {
                for c in 0..3 {
                    let x0 = 3.0 + c as f32 * 3.6;
                    let y0 = 3.0 + r as f32 * 3.6;
                    rect(&mut p, ox, oy, s, x0, y0, x0 + 2.8, y0 + 2.8);
                }
            }
        }
        // ── The bottom toolbar's marks ────────────────────────────────────────────────────────
        Icon::Move => {
            // The pointer: one silhouette, the tail separate from the head.
            poly_fill(&mut p, ox, oy, s, &[(4.0, 2.0), (4.0, 13.6), (6.9, 10.9), (8.8, 14.8), (10.6, 14.0), (8.7, 10.2), (12.6, 10.2)]);
        }
        Icon::Hand => {
            // Four fingers over a palm, then the thumb: the original's arcs become short
            // segments (a filled path builder has no stroked curves).
            for (x, top) in [(4.5, 5.0), (6.5, 3.5), (8.5, 4.0), (10.5, 5.5)] {
                thick_line(&mut p, ox, oy, s, x, 8.0, x, top + 1.0, 1.5);
                arc(&mut p, ox, oy, s, x + 0.75, top + 1.0, 0.75, std::f32::consts::PI, std::f32::consts::TAU, 1.5, 4);
                thick_line(&mut p, ox, oy, s, x + 1.5, top + 1.0, x + 1.5, 8.0, 1.5);
            }
            stroke_path(
                &mut p,
                ox,
                oy,
                s,
                &[(12.5, 8.0), (12.5, 10.0), (11.5, 12.0), (9.5, 13.0), (7.0, 12.4), (3.0, 9.4), (3.4, 8.6), (4.5, 8.5)],
                false,
                1.5,
            );
        }
        Icon::Scale => {
            outline_rect(&mut p, ox, oy, s, 2.5, 7.5, 8.5, 13.5, 1.5);
            thick_line(&mut p, ox, oy, s, 6.0, 10.0, 13.0, 3.0, 1.5);
            stroke_path(&mut p, ox, oy, s, &[(9.5, 2.5), (13.5, 2.5), (13.5, 6.5)], false, 1.5);
        }
        Icon::Slice => {
            stroke_path(&mut p, ox, oy, s, &[(13.5, 2.5), (5.0, 8.0), (8.0, 11.0)], true, 1.4);
            thick_line(&mut p, ox, oy, s, 2.8, 13.2, 5.9, 10.1, 2.6);
        }
        Icon::Line => thick_line(&mut p, ox, oy, s, 2.5, 13.5, 13.5, 2.5, 1.6),
        Icon::Arrow => {
            thick_line(&mut p, ox, oy, s, 2.5, 13.5, 13.0, 3.0, 1.6);
            stroke_path(&mut p, ox, oy, s, &[(7.5, 2.5), (13.5, 2.5), (13.5, 8.5)], false, 1.6);
        }
        Icon::Polygon => stroke_path(&mut p, ox, oy, s, &[(8.0, 2.5), (13.5, 12.5), (2.5, 12.5)], true, 1.6),
        Icon::Star => {
            let pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 5.0;
                    let r = if i % 2 == 0 { 6.0 } else { 2.7 };
                    (8.0 + r * a.cos(), 8.6 + r * a.sin())
                })
                .collect();
            stroke_path(&mut p, ox, oy, s, &pts, true, 1.5);
        }
        Icon::Pen => {
            stroke_path(&mut p, ox, oy, s, &[(8.0, 2.0), (12.5, 8.5), (10.5, 13.5), (5.5, 13.5), (3.5, 8.5)], true, 1.5);
            thick_line(&mut p, ox, oy, s, 8.0, 2.0, 8.0, 7.0, 1.5);
            disc(&mut p, ox, oy, s, 8.0, 8.8, 1.2);
        }
        Icon::Pencil => {
            stroke_path(&mut p, ox, oy, s, &[(2.5, 13.5), (3.2, 10.3), (10.5, 3.0), (13.0, 5.5), (5.7, 12.8)], true, 1.4);
            thick_line(&mut p, ox, oy, s, 9.0, 4.5, 11.5, 7.0, 1.4);
        }
        Icon::ShapeBuilder => {
            // Two overlapping rings with the lens they share filled.
            let (r, d) = (4.5f32, 2.0f32);
            let a = (d / r).acos();
            ring(&mut p, ox, oy, s, 8.0 - d, 8.0, r, 1.4, 24);
            ring(&mut p, ox, oy, s, 8.0 + d, 8.0, r, 1.4, 24);
            let mut lens: Vec<(f32, f32)> = arc_pts(8.0 - d, 8.0, r, -a, a, 12);
            lens.extend(arc_pts(8.0 + d, 8.0, r, std::f32::consts::PI - a, std::f32::consts::PI + a, 12));
            poly_fill(&mut p, ox, oy, s, &lens);
        }
        Icon::TextOnPath => {
            thick_line(&mut p, ox, oy, s, 4.5, 3.0, 11.5, 3.0, 1.5);
            thick_line(&mut p, ox, oy, s, 8.0, 3.0, 8.0, 8.5, 1.5);
            stroke_path(&mut p, ox, oy, s, &[(2.0, 13.5), (5.0, 11.5), (8.0, 10.8), (11.0, 11.5), (14.0, 13.5)], false, 1.4);
        }
        Icon::Comment => {
            // A speech bubble: a ring with the tail at the bottom left.
            ring(&mut p, ox, oy, s, 8.5, 7.5, 5.0, 1.5, 28);
            stroke_path(&mut p, ox, oy, s, &[(6.0, 12.2), (2.5, 13.5), (4.4, 10.6)], true, 1.2);
        }
        Icon::Actions => {
            poly_fill(&mut p, ox, oy, s, &[(9.0, 2.0), (3.5, 9.0), (7.5, 9.0), (6.5, 14.0), (12.5, 7.0), (8.5, 7.0)]);
        }
        Icon::Panels => {
            outline_rect(&mut p, ox, oy, s, 2.5, 3.5, 13.5, 12.5, 1.5);
            thick_line(&mut p, ox, oy, s, 5.5, 4.0, 5.5, 12.0, 1.5);
            thick_line(&mut p, ox, oy, s, 10.5, 4.0, 10.5, 12.0, 1.5);
        }
        Icon::Eye => {
            // An almond (two arcs meeting at the corners) around the pupil.
            let (r, cy, half) = (7.0f32, 7.5f32, 5.5f32);
            let a = (half / r).asin();
            let d = r * a.cos();
            let mut e: Vec<(f32, f32)> = arc_pts(8.0, cy + d, r, -std::f32::consts::FRAC_PI_2 - a, -std::f32::consts::FRAC_PI_2 + a, 10);
            e.extend(arc_pts(8.0, cy - d, r, std::f32::consts::FRAC_PI_2 - a, std::f32::consts::FRAC_PI_2 + a, 10));
            stroke_path(&mut p, ox, oy, s, &e, true, 1.3);
            ring(&mut p, ox, oy, s, 8.0, cy, 2.0, 1.2, 16);
        }
        Icon::Animation => {
            stroke_path(&mut p, ox, oy, s, &[(2.0, 13.0), (4.5, 9.0), (8.0, 6.0), (11.5, 4.0), (14.0, 3.5)], false, 1.4);
            poly_fill(&mut p, ox, oy, s, &[(4.5, 6.5), (7.0, 9.0), (4.5, 11.5), (2.0, 9.0)]);
            poly_fill(&mut p, ox, oy, s, &[(11.5, 1.5), (14.0, 4.0), (11.5, 6.5), (9.0, 4.0)]);
        }
        Icon::Code => {
            stroke_path(&mut p, ox, oy, s, &[(5.5, 4.0), (2.0, 8.0), (5.5, 12.0)], false, 1.5);
            stroke_path(&mut p, ox, oy, s, &[(10.5, 4.0), (14.0, 8.0), (10.5, 12.0)], false, 1.5);
            thick_line(&mut p, ox, oy, s, 9.0, 3.0, 7.0, 13.0, 1.4);
        }
        Icon::Group => {
            // A nested outline: the classic "group" mark (container of containers).
            outline_rect(&mut p, ox, oy, s, 3.0, 3.0, 13.0, 13.0, 1.5);
            outline_rect(&mut p, ox, oy, s, 5.5, 5.5, 11.5, 11.5, 1.2);
        }
        Icon::Vector => {
            poly_fill(&mut p, ox, oy, s, &[(8.0, 2.0), (14.0, 13.5), (2.0, 13.5)]);
        }
        Icon::ComponentSet => {
            outline_rect(&mut p, ox, oy, s, 2.5, 2.5, 13.5, 13.5, 1.3);
            diamond(&mut p, ox, oy, s, 8.0, 8.0, 4.5);
        }
        Icon::IconKind => {
            diamond(&mut p, ox, oy, s, 8.0, 8.0, 4.5);
            disc(&mut p, ox, oy, s, 8.0, 8.0, 1.5);
        }
        Icon::Slot => {
            outline_rect(&mut p, ox, oy, s, 3.0, 3.0, 13.0, 13.0, 1.5);
            disc(&mut p, ox, oy, s, 8.0, 8.0, 1.6);
        }
        Icon::Lock => {
            outline_rect(&mut p, ox, oy, s, 5.0, 7.0, 11.0, 13.0, 1.4);
            arc(&mut p, ox, oy, s, 8.0, 5.5, 2.3, std::f32::consts::PI, std::f32::consts::TAU, 1.3, 10);
        }
        Icon::Unlock => {
            outline_rect(&mut p, ox, oy, s, 5.0, 7.0, 11.0, 13.0, 1.4);
            arc(&mut p, ox, oy, s, 9.2, 6.0, 2.3, std::f32::consts::PI, std::f32::consts::TAU, 1.3, 10);
        }
        Icon::EyeOff => {
            let (r, cy, half) = (7.0f32, 7.5f32, 5.5f32);
            let a = (half / r).asin();
            let d = r * a.cos();
            let mut e: Vec<(f32, f32)> = arc_pts(8.0, cy + d, r, -std::f32::consts::FRAC_PI_2 - a, -std::f32::consts::FRAC_PI_2 + a, 10);
            e.extend(arc_pts(8.0, cy - d, r, std::f32::consts::FRAC_PI_2 - a, std::f32::consts::FRAC_PI_2 + a, 10));
            stroke_path(&mut p, ox, oy, s, &e, true, 1.3);
            ring(&mut p, ox, oy, s, 8.0, cy, 2.0, 1.2, 16);
            thick_line(&mut p, ox, oy, s, 2.5, 13.5, 13.5, 2.5, 1.6);
        }
        Icon::Union => {
            disc(&mut p, ox, oy, s, 6.0, 8.0, 4.0);
            disc(&mut p, ox, oy, s, 10.0, 8.0, 4.0);
        }
        Icon::Subtract => {
            disc(&mut p, ox, oy, s, 6.0, 8.0, 4.0);
            ring(&mut p, ox, oy, s, 10.0, 8.0, 4.0, 1.3, 18);
        }
        Icon::Intersect => {
            let (r, d) = (4.5f32, 2.0f32);
            let a = (d / r).acos();
            ring(&mut p, ox, oy, s, 8.0 - d, 8.0, r, 1.4, 24);
            ring(&mut p, ox, oy, s, 8.0 + d, 8.0, r, 1.4, 24);
            let mut lens: Vec<(f32, f32)> = arc_pts(8.0 - d, 8.0, r, -a, a, 12);
            lens.extend(arc_pts(8.0 + d, 8.0, r, std::f32::consts::PI - a, std::f32::consts::PI + a, 12));
            poly_fill(&mut p, ox, oy, s, &lens);
        }
        Icon::Exclude => {
            let r = 4.0f32;
            ring(&mut p, ox, oy, s, 6.0, 8.0, r, 1.4, 20);
            ring(&mut p, ox, oy, s, 10.0, 8.0, r, 1.4, 20);
        }
    }
    if let Ok(path) = p.build() {
        window.paint_path(path, color);
    }
}

/// Paint `icon` centred in `bounds`, scaled to the shorter side. The entry point for callers that
/// draw outside an `IconGlyph` element (a swatch's image/pattern glyph).
pub(crate) fn draw_centred(window: &mut Window, bounds: Bounds<Pixels>, icon: Icon, color: gpui::Rgba) {
    let w: f32 = bounds.size.width.into();
    let h: f32 = bounds.size.height.into();
    let side = w.min(h).max(1.0);
    let ox = f32::from(bounds.origin.x) + (w - side) / 2.0;
    let oy = f32::from(bounds.origin.y) + (h - side) / 2.0;
    let square = Bounds { origin: Point::new(px(ox), px(oy)), size: gpui::size(px(side), px(side)) };
    draw(window, square, icon, color);
}

/// Add a filled triangle `(x0,y0) (x1,y1) (x2,y2)` (local 0..16, scaled by `s`) as one triangle.
fn tri(p: &mut PathBuilder, ox: f32, oy: f32, s: f32, x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32) {
    let k = s / 16.0;
    let a = Point::new(px(ox + x0 * k), px(oy + y0 * k));
    let b = Point::new(px(ox + x1 * k), px(oy + y1 * k));
    let c = Point::new(px(ox + x2 * k), px(oy + y2 * k));
    let z = Point::new(0.0_f32, 0.0_f32);
    p.push_triangle((a, b, c), (z, z, z));
}

/// Add a thick segment `p0→p1` (local 0..16 box, scaled by `s`, offset by `o`) as two triangles.
fn thick_line(p: &mut PathBuilder, ox: f32, oy: f32, s: f32, x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
    let k = s / 16.0;
    let ax = ox + x0 * k;
    let ay = oy + y0 * k;
    let bx = ox + x1 * k;
    let by = oy + y1 * k;
    let dx = bx - ax;
    let dy = by - ay;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let nx = -dy / len * (w * k / 2.0);
    let ny = dx / len * (w * k / 2.0);
    let c = Point::new(px(bx + nx), px(by + ny));
    let d = Point::new(px(bx - nx), px(by - ny));
    let e = Point::new(px(ax + nx), px(ay + ny));
    let f = Point::new(px(ax - nx), px(ay - ny));
    let z = Point::new(0.0_f32, 0.0_f32);
    p.push_triangle((e, c, d), (z, z, z));
    p.push_triangle((e, d, f), (z, z, z));
}

/// Add a filled disc centred at `(cx,cy)` radius `r` (local 0..16, scaled by `s`) as a triangle fan.
fn disc(p: &mut PathBuilder, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32) {
    let k = s / 16.0;
    let cxp = ox + cx * k;
    let cyp = oy + cy * k;
    let rr = r * k;
    let n = 16;
    let z = Point::new(0.0_f32, 0.0_f32);
    let c = Point::new(px(cxp), px(cyp));
    for i in 0..n {
        let a0 = (i as f32 / n as f32) * std::f32::consts::TAU;
        let a1 = ((i + 1) as f32 / n as f32) * std::f32::consts::TAU;
        let p0 = Point::new(px(cxp + rr * a0.cos()), px(cyp + rr * a0.sin()));
        let p1 = Point::new(px(cxp + rr * a1.cos()), px(cyp + rr * a1.sin()));
        p.push_triangle((c, p0, p1), (z, z, z));
    }
}

/// Add a filled axis-aligned rectangle `[x0,y0]..[x1,y1]` (local 0..16, scaled by `s`) as two triangles.
fn rect(p: &mut PathBuilder, ox: f32, oy: f32, s: f32, x0: f32, y0: f32, x1: f32, y1: f32) {
    let k = s / 16.0;
    let ax = ox + x0 * k;
    let ay = oy + y0 * k;
    let bx = ox + x1 * k;
    let by = oy + y1 * k;
    let z = Point::new(0.0_f32, 0.0_f32);
    p.push_triangle((Point::new(px(ax), px(ay)), Point::new(px(bx), px(ay)), Point::new(px(bx), px(by))), (z, z, z));
    p.push_triangle((Point::new(px(ax), px(ay)), Point::new(px(bx), px(by)), Point::new(px(ax), px(by))), (z, z, z));
}

/// Add an outlined axis-aligned rectangle `[x0,y0]..[x1,y1]` from four thick segments.
fn outline_rect(p: &mut PathBuilder, ox: f32, oy: f32, s: f32, x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
    thick_line(p, ox, oy, s, x0, y0, x1, y0, w);
    thick_line(p, ox, oy, s, x1, y0, x1, y1, w);
    thick_line(p, ox, oy, s, x1, y1, x0, y1, w);
    thick_line(p, ox, oy, s, x0, y1, x0, y0, w);
}

/// Add a filled diamond centred at `(cx,cy)` with half-extent `r` (local 0..16, scaled by `s`).
fn diamond(p: &mut PathBuilder, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32) {
    let k = s / 16.0;
    let top = Point::new(px(ox + cx * k), px(oy + (cy - r) * k));
    let right = Point::new(px(ox + (cx + r) * k), px(oy + cy * k));
    let bot = Point::new(px(ox + cx * k), px(oy + (cy + r) * k));
    let left = Point::new(px(ox + (cx - r) * k), px(oy + cy * k));
    let z = Point::new(0.0_f32, 0.0_f32);
    p.push_triangle((top, right, bot), (z, z, z));
    p.push_triangle((top, bot, left), (z, z, z));
}

/// Add an outlined diamond (four thick segments around the diamond).
fn outline_diamond(p: &mut PathBuilder, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32, w: f32) {
    let top = (cx, cy - r);
    let right = (cx + r, cy);
    let bot = (cx, cy + r);
    let left = (cx - r, cy);
    thick_line(p, ox, oy, s, top.0, top.1, right.0, right.1, w);
    thick_line(p, ox, oy, s, right.0, right.1, bot.0, bot.1, w);
    thick_line(p, ox, oy, s, bot.0, bot.1, left.0, left.1, w);
    thick_line(p, ox, oy, s, left.0, left.1, top.0, top.1, w);
}

/// Add a filled polygon through `pts` (local 0..16, scaled by `s`) as one path.
fn poly_fill(p: &mut PathBuilder, ox: f32, oy: f32, s: f32, pts: &[(f32, f32)]) {
    let k = s / 16.0;
    let v: Vec<Point<Pixels>> = pts.iter().map(|&(x, y)| Point::new(px(ox + x * k), px(oy + y * k))).collect();
    if v.len() >= 3 {
        p.add_polygon(&v, true);
    }
}

/// Add a stroked polyline through `pts` as chained thick segments (`P::poly` in the original).
fn stroke_path(p: &mut PathBuilder, ox: f32, oy: f32, s: f32, pts: &[(f32, f32)], closed: bool, w: f32) {
    for pair in pts.windows(2) {
        thick_line(p, ox, oy, s, pair[0].0, pair[0].1, pair[1].0, pair[1].1, w);
    }
    if closed
        && pts.len() > 2
        && let (Some(a), Some(b)) = (pts.first(), pts.last())
    {
        thick_line(p, ox, oy, s, b.0, b.1, a.0, a.1, w);
    }
}

/// The points along an arc (grid units), `n` segments from `a0` to `a1` radians.
fn arc_pts(cx: f32, cy: f32, r: f32, a0: f32, a1: f32, n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * (i as f32 / n as f32);
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// Add a stroked arc as `n` thick segments (a filled path builder has no stroked curves).
#[allow(clippy::too_many_arguments)]
fn arc(p: &mut PathBuilder, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32, a0: f32, a1: f32, w: f32, n: usize) {
    stroke_path(p, ox, oy, s, &arc_pts(cx, cy, r, a0, a1, n), false, w);
}

/// Add a whole outlined circle.
fn ring(p: &mut PathBuilder, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32, w: f32, n: usize) {
    arc(p, ox, oy, s, cx, cy, r, 0.0, std::f32::consts::TAU, w, n);
}

/// Draw `icon` in `color` at `size` px, as an element.
pub fn paint(icon: Icon, color: impl Into<gpui::Rgba>, size: f32) -> impl IntoElement {
    IconGlyph::new(icon, color, size)
}
