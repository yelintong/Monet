// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
//
// 2026-11-08 用户确认 P0-1 落地蓝图（上一轮交付的对比报告，本轮「继续」）后授权本次解冻：
// 第 3 步要求给标签条补 Home / Sun / Moon / Play / Win* / LiveDisconnected 共 8 个变体，
// 并按上游 #1729 的 20/32/12/26pt 图标规范使用。本次**只新增枚举变体与其绘制分支**，
// 三条不变量、既有 70 个变体的绘制与所有控件行为一律未动。
//
// 2026-11-08 二次解冻（用户指令原文）：
//   「还有你当前这个 SVG 的渲染东西有 bug，我建议你也去抄一下 kit」
//   前置约束：「注意我是让你抄它的，不是让你把整个库搬过来」
//   → 按外部组件库 `gpui-kit` 的 Icon 公开契约（IconName → SVG 资源路径 → `.text_color()`）
//     在**本地**实现，不引入 gpui-kit / gpui_component 依赖。
// ============================================================================
//! FigCraft's icon set, rendered as real SVG assets through GPUI's SVG pipeline.
//!
//! GPUI 0.2.1 **does** have an SVG element (`svg()` / `Svg`) and `Window::paint_svg()`; the old
//! header claimed otherwise. The previous implementation hand-built every glyph out of filled
//! triangles via `PathBuilder::fill()`: circles were 16/22-segment polygons, diagonal strokes
//! were two triangles with visible seams, and `PathBuilder` curve fills are broken on 0.2.1
//! (see `canvas/painter.rs`). That is the rendering bug this module fixes.
//!
//! Contract copied from `gpui-kit`'s `Icon` (implementation, not the crate):
//! - `IconName`-like enum → a stable asset path `icons/<kebab-name>.svg`;
//! - the bytes come from an `AssetSource` registered with `Application::with_assets()`
//!   (kit's `.data(include_bytes!(...))` / `.path(...)` equivalence);
//! - colour is applied with `.text_color()` (kit's `.text_color(cx.theme().red)`), i.e. GPUI
//!   renders the SVG into an alpha mask and tints it with the given `Hsla`;
//! - size is `.with_size(px(n))` over a 16x16 viewBox (here `.w().h().flex_none()`).
//!
//! The geometry table below is ported from `ui-egui/src/icons/draw.rs` on a 16x16 grid. It is
//! unchanged: the helpers now emit SVG primitives in grid units instead of pixel triangles.
//! `ox/oy/s` are kept (0, 0, 16) so every match arm stays byte-for-byte the same.

use std::{borrow::Cow, sync::OnceLock};

use gpui::{
    App, AssetSource, Bounds, Hsla, IntoElement, Pixels, Point, Result, SharedString,
    TransformationMatrix, Window, prelude::*, px, svg,
};

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
    /// A frame outline with four inward arrowheads — "fit to contents" (`object.resizeToFit`).
    FitContents,
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
    // ── 标签条（上游 `ui-egui/src/tabs.rs`，P0-1）──────────────────────────────────────────
    /// Home tab: a house.
    Home,
    /// Light/dark theme toggle: the sun (shown in dark theme).
    Sun,
    /// Light/dark theme toggle: the moon (shown in light theme).
    Moon,
    /// A presentation tab's play triangle.
    Play,
    /// Frameless title bar: minimise.
    WinMinimize,
    /// Frameless title bar: maximise.
    WinMaximize,
    /// Frameless title bar: restore.
    WinRestore,
    /// A live session whose connection dropped.
    LiveDisconnected,
}

impl Icon {
    /// 全部图标，按枚举声明顺序（kit `IconName` 的等价物）。
    /// 图标资源表、`AssetSource::list()`、单测都以这张表为准。
    pub const ALL: &'static [Icon] = &[
        Icon::Plus,
        Icon::Minus,
        Icon::Close,
        Icon::Check,
        Icon::ChevronDown,
        Icon::ChevronUp,
        Icon::ChevronLeft,
        Icon::ChevronRight,
        Icon::Search,
        Icon::Dot,
        Icon::Variable,
        Icon::LinkOn,
        Icon::LinkOff,
        Icon::Section,
        Icon::Panel,
        Icon::Menu,
        Icon::Settings,
        Icon::Trash,
        Icon::Rectangle,
        Icon::Ellipse,
        Icon::Frame,
        Icon::FitContents,
        Icon::Text,
        Icon::Component,
        Icon::Instance,
        Icon::Page,
        Icon::Image,
        Icon::ViewGrid,
        Icon::Detach,
        Icon::AlignLeft,
        Icon::AlignHCenter,
        Icon::AlignRight,
        Icon::AlignTop,
        Icon::AlignVCenter,
        Icon::AlignBottom,
        Icon::DistributeH,
        Icon::Rotate,
        Icon::FlipH,
        Icon::FlipV,
        Icon::Angle,
        Icon::AbsolutePosition,
        Icon::Move,
        Icon::Hand,
        Icon::Scale,
        Icon::Slice,
        Icon::Line,
        Icon::Arrow,
        Icon::Polygon,
        Icon::Star,
        Icon::Pen,
        Icon::Pencil,
        Icon::ShapeBuilder,
        Icon::TextOnPath,
        Icon::Comment,
        Icon::Actions,
        Icon::Panels,
        Icon::Eye,
        Icon::Animation,
        Icon::Code,
        Icon::Group,
        Icon::Vector,
        Icon::ComponentSet,
        Icon::IconKind,
        Icon::Slot,
        Icon::Lock,
        Icon::Unlock,
        Icon::EyeOff,
        Icon::Union,
        Icon::Subtract,
        Icon::Intersect,
        Icon::Exclude,
        Icon::Home,
        Icon::Sun,
        Icon::Moon,
        Icon::Play,
        Icon::WinMinimize,
        Icon::WinMaximize,
        Icon::WinRestore,
        Icon::LiveDisconnected,
    ];

    /// 资源名（kebab-case，对应 `icons/<name>.svg`）。
    pub fn asset_name(self) -> &'static str {
        match self {
            Icon::Plus => "plus",
            Icon::Minus => "minus",
            Icon::Close => "close",
            Icon::Check => "check",
            Icon::ChevronDown => "chevron-down",
            Icon::ChevronUp => "chevron-up",
            Icon::ChevronLeft => "chevron-left",
            Icon::ChevronRight => "chevron-right",
            Icon::Search => "search",
            Icon::Dot => "dot",
            Icon::Variable => "variable",
            Icon::LinkOn => "link-on",
            Icon::LinkOff => "link-off",
            Icon::Section => "section",
            Icon::Panel => "panel",
            Icon::Menu => "menu",
            Icon::Settings => "settings",
            Icon::Trash => "trash",
            Icon::Rectangle => "rectangle",
            Icon::Ellipse => "ellipse",
            Icon::Frame => "frame",
            Icon::FitContents => "fit-contents",
            Icon::Text => "text",
            Icon::Component => "component",
            Icon::Instance => "instance",
            Icon::Page => "page",
            Icon::Image => "image",
            Icon::ViewGrid => "view-grid",
            Icon::Detach => "detach",
            Icon::AlignLeft => "align-left",
            Icon::AlignHCenter => "align-h-center",
            Icon::AlignRight => "align-right",
            Icon::AlignTop => "align-top",
            Icon::AlignVCenter => "align-v-center",
            Icon::AlignBottom => "align-bottom",
            Icon::DistributeH => "distribute-h",
            Icon::Rotate => "rotate",
            Icon::FlipH => "flip-h",
            Icon::FlipV => "flip-v",
            Icon::Angle => "angle",
            Icon::AbsolutePosition => "absolute-position",
            Icon::Move => "move",
            Icon::Hand => "hand",
            Icon::Scale => "scale",
            Icon::Slice => "slice",
            Icon::Line => "line",
            Icon::Arrow => "arrow",
            Icon::Polygon => "polygon",
            Icon::Star => "star",
            Icon::Pen => "pen",
            Icon::Pencil => "pencil",
            Icon::ShapeBuilder => "shape-builder",
            Icon::TextOnPath => "text-on-path",
            Icon::Comment => "comment",
            Icon::Actions => "actions",
            Icon::Panels => "panels",
            Icon::Eye => "eye",
            Icon::Animation => "animation",
            Icon::Code => "code",
            Icon::Group => "group",
            Icon::Vector => "vector",
            Icon::ComponentSet => "component-set",
            Icon::IconKind => "icon-kind",
            Icon::Slot => "slot",
            Icon::Lock => "lock",
            Icon::Unlock => "unlock",
            Icon::EyeOff => "eye-off",
            Icon::Union => "union",
            Icon::Subtract => "subtract",
            Icon::Intersect => "intersect",
            Icon::Exclude => "exclude",
            Icon::Home => "home",
            Icon::Sun => "sun",
            Icon::Moon => "moon",
            Icon::Play => "play",
            Icon::WinMinimize => "win-minimize",
            Icon::WinMaximize => "win-maximize",
            Icon::WinRestore => "win-restore",
            Icon::LiveDisconnected => "live-disconnected",
        }
    }

    /// kit `Icon::empty().path("icons/my-brand-logo.svg")` 的等价物：图标 → 资源路径。
    pub fn asset_path(self) -> SharedString {
        let asset = &table()[self as usize];
        debug_assert_eq!(asset.icon, self, "Icon 判别值与资源表顺序不一致");
        asset.path.clone()
    }
}

/// 一个图标的静态资源（路径 + 生成好的 SVG 字节）。
struct IconAsset {
    icon: Icon,
    name: &'static str,
    path: SharedString,
    svg: Vec<u8>,
}

/// 只在首次访问时把 79 个图标编译成 SVG；之后 `AssetSource::load()` 零拷贝返回。
static TABLE: OnceLock<Vec<IconAsset>> = OnceLock::new();

fn table() -> &'static Vec<IconAsset> {
    TABLE.get_or_init(|| {
        Icon::ALL
            .iter()
            .map(|&icon| {
                let name = icon.asset_name();
                let path = SharedString::from(format!("icons/{name}.svg"));
                let svg = svg_canvas(icon).to_svg().into_bytes();
                IconAsset { icon, name, path, svg }
            })
            .collect()
    })
}

/// 本地 `AssetSource`：把 `icons/<name>.svg` 映射到内存里的 SVG 字节。
///
/// 这就是 kit 里 `.data(include_bytes!("search.svg"))` / `.path("icons/...")` 背后的机制。
/// GPUI 的默认资源源是 `()`（什么都不加载），所以**必须**在 `Application` 上注册本类型，
/// 否则 `svg()` 会静默不画。项目里的 `img(ImageSource::Render(...))` 不经过 `AssetSource`，
/// 不受影响。
pub struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let Some(name) = path
            .strip_prefix("icons/")
            .and_then(|rest| rest.strip_suffix(".svg"))
        else {
            return Ok(None);
        };
        Ok(table()
            .iter()
            .find(|a| a.name == name)
            .map(|a| Cow::Borrowed(a.svg.as_slice())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        if path.is_empty() || path == "icons" || path == "icons/" {
            Ok(table().iter().map(|a| a.path.clone()).collect())
        } else {
            Ok(Vec::new())
        }
    }
}

/// 16x16 viewBox 的矢量图元收集器。坐标都是设计网格单位（0..16）。
#[derive(Default)]
struct SvgCanvas {
    shapes: Vec<String>,
}

impl SvgCanvas {
    fn polygon(&mut self, pts: &[(f32, f32)]) {
        if pts.len() < 3 {
            return;
        }
        let d = pts
            .iter()
            .enumerate()
            .map(|(i, (x, y))| format!("{}{:.3} {:.3}", if i == 0 { "M" } else { "L" }, x, y))
            .collect::<Vec<_>>()
            .join(" ");
        self.shapes.push(format!("<path d=\"{d} Z\"/>"));
    }

    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
        self.polyline(&[(x0, y0), (x1, y1)], false, w);
    }

    fn polyline(&mut self, pts: &[(f32, f32)], closed: bool, w: f32) {
        if pts.len() < 2 {
            return;
        }
        let mut d = String::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            d.push_str(&format!("{}{:.3} {:.3} ", if i == 0 { "M" } else { "L" }, x, y));
        }
        if closed {
            d.push('Z');
        }
        // Round caps/joins replace the two-triangle strokes: no seams, no polygonal corners.
        self.shapes.push(format!(
            "<path d=\"{d}\" fill=\"none\" stroke=\"#000\" stroke-width=\"{w:.3}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>"
        ));
    }

    fn disc(&mut self, cx: f32, cy: f32, r: f32) {
        self.shapes
            .push(format!("<circle cx=\"{cx:.3}\" cy=\"{cy:.3}\" r=\"{:.3}\"/>", r.max(0.0)));
    }

    fn ring(&mut self, cx: f32, cy: f32, r: f32, w: f32) {
        self.shapes.push(format!(
            "<circle cx=\"{cx:.3}\" cy=\"{cy:.3}\" r=\"{:.3}\" fill=\"none\" stroke=\"#000\" stroke-width=\"{w:.3}\"/>",
            r.max(0.0)
        ));
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let (x, w) = ordered(x0, x1);
        let (y, h) = ordered(y0, y1);
        self.shapes.push(format!(
            "<rect x=\"{x:.3}\" y=\"{y:.3}\" width=\"{w:.3}\" height=\"{h:.3}\"/>"
        ));
    }

    fn outline_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
        let (x, wd) = ordered(x0, x1);
        let (y, h) = ordered(y0, y1);
        self.shapes.push(format!(
            "<rect x=\"{x:.3}\" y=\"{y:.3}\" width=\"{wd:.3}\" height=\"{h:.3}\" fill=\"none\" stroke=\"#000\" stroke-width=\"{w:.3}\"/>"
        ));
    }

    fn to_svg(self) -> String {
        // GPUI's SvgRenderer keeps only the alpha channel, so the ink colour here is irrelevant;
        // `.text_color()` / `paint_svg(color)` supplies the actual tint.
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\" fill=\"#000\" stroke=\"none\">{}</svg>",
            self.shapes.join("")
        )
    }
}

fn ordered(a: f32, b: f32) -> (f32, f32) {
    if a <= b { (a, b - a) } else { (b, a - b) }
}

/// 把一个图标的几何表翻译成 SVG 图元。`ox/oy/s` 固定为 (0, 0, 16)：设计网格直接就是
/// viewBox 单位，下面 380 行 match 分支与 ui-egui 图标表保持一致。
fn svg_canvas(icon: Icon) -> SvgCanvas {
    let ox = 0.0_f32;
    let oy = 0.0_f32;
    let s = 16.0_f32;
    let mut p = SvgCanvas::default();
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
        Icon::FitContents => {
            // A frame outline with four inward-pointing arrowheads (shrink the frame to
            // wrap its children). Mirrors egui `Icon::FitContents` → `object.resizeToFit`.
            outline_rect(&mut p, ox, oy, s, 2.5, 2.5, 13.5, 13.5, 1.3);
            tri(&mut p, ox, oy, s, 2.5, 5.5, 5.5, 2.5, 5.5, 5.5); // top-left → centre
            tri(&mut p, ox, oy, s, 13.5, 5.5, 10.5, 2.5, 10.5, 5.5); // top-right → centre
            tri(&mut p, ox, oy, s, 2.5, 10.5, 5.5, 13.5, 5.5, 10.5); // bottom-left → centre
            tri(&mut p, ox, oy, s, 13.5, 10.5, 10.5, 13.5, 10.5, 10.5); // bottom-right → centre
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
        // ── 标签条（P0-1）：与上面同一套 16 单位网格与原语 ────────────────────────────────
        Icon::Home => {
            poly_fill(&mut p, ox, oy, s, &[(1.5, 8.0), (8.0, 2.0), (14.5, 8.0)]);
            outline_rect(&mut p, ox, oy, s, 3.5, 8.0, 12.5, 14.0, 1.6);
            rect(&mut p, ox, oy, s, 7.0, 10.5, 9.0, 14.0);
        }
        Icon::Sun => {
            disc(&mut p, ox, oy, s, 8.0, 8.0, 3.4);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let (c, sn) = (a.cos(), a.sin());
                thick_line(&mut p, ox, oy, s, 8.0 + c * 5.2, 8.0 + sn * 5.2, 8.0 + c * 7.0, 8.0 + sn * 7.0, 1.5);
            }
        }
        Icon::Moon => {
            // Outer crescent + the short inner edge; a subtractive shape made of two arcs.
            arc(&mut p, ox, oy, s, 8.0, 8.0, 5.4, 0.42 * std::f32::consts::PI, 1.58 * std::f32::consts::PI, 2.0, 22);
            arc(&mut p, ox, oy, s, 11.2, 8.0, 4.4, 0.60 * std::f32::consts::PI, 1.40 * std::f32::consts::PI, 1.4, 16);
        }
        Icon::Play => {
            poly_fill(&mut p, ox, oy, s, &[(4.5, 2.5), (4.5, 13.5), (13.5, 8.0)]);
        }
        Icon::WinMinimize => {
            thick_line(&mut p, ox, oy, s, 3.0, 8.0, 13.0, 8.0, 1.6);
        }
        Icon::WinMaximize => {
            outline_rect(&mut p, ox, oy, s, 3.0, 3.0, 13.0, 13.0, 1.6);
        }
        Icon::WinRestore => {
            outline_rect(&mut p, ox, oy, s, 3.0, 5.5, 10.5, 13.0, 1.5);
            thick_line(&mut p, ox, oy, s, 5.5, 5.5, 5.5, 3.0, 1.5);
            thick_line(&mut p, ox, oy, s, 5.5, 3.0, 13.0, 3.0, 1.5);
            thick_line(&mut p, ox, oy, s, 13.0, 3.0, 13.0, 10.5, 1.5);
            thick_line(&mut p, ox, oy, s, 10.5, 10.5, 13.0, 10.5, 1.5);
        }
        Icon::LiveDisconnected => {
            ring(&mut p, ox, oy, s, 8.0, 8.0, 5.0, 1.6, 22);
            thick_line(&mut p, ox, oy, s, 4.0, 12.0, 12.0, 4.0, 2.0);
        }
    }
    p
}

/// Paint `icon` centred in `bounds`, scaled to the shorter side. The entry point for callers that
/// draw directly into a `canvas` paint closure (a swatch's image/pattern glyph), which is why it
/// goes through `Window::paint_svg` instead of the `svg()` element.
pub(crate) fn draw_centred(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    icon: Icon,
    color: gpui::Rgba,
    cx: &App,
) {
    let w: f32 = bounds.size.width.into();
    let h: f32 = bounds.size.height.into();
    let side = w.min(h).max(1.0);
    let ox = f32::from(bounds.origin.x) + (w - side) / 2.0;
    let oy = f32::from(bounds.origin.y) + (h - side) / 2.0;
    let square = Bounds {
        origin: Point::new(px(ox), px(oy)),
        size: gpui::size(px(side), px(side)),
    };
    let _ = window.paint_svg(
        square,
        icon.asset_path(),
        TransformationMatrix::unit(),
        Hsla::from(color),
        cx,
    );
}

// ── 绘制原语：签名与旧版一致，输出从「像素三角形」换成「SVG 图元」──────────────────
fn tri(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32) {
    let k = s / 16.0;
    p.polygon(&[(ox + x0 * k, oy + y0 * k), (ox + x1 * k, oy + y1 * k), (ox + x2 * k, oy + y2 * k)]);
}

fn thick_line(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
    let k = s / 16.0;
    p.line(ox + x0 * k, oy + y0 * k, ox + x1 * k, oy + y1 * k, w * k);
}

fn disc(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32) {
    let k = s / 16.0;
    p.disc(ox + cx * k, oy + cy * k, r * k);
}

fn rect(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, x0: f32, y0: f32, x1: f32, y1: f32) {
    let k = s / 16.0;
    p.rect(ox + x0 * k, oy + y0 * k, ox + x1 * k, oy + y1 * k);
}

fn outline_rect(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, x0: f32, y0: f32, x1: f32, y1: f32, w: f32) {
    let k = s / 16.0;
    p.outline_rect(ox + x0 * k, oy + y0 * k, ox + x1 * k, oy + y1 * k, w * k);
}

fn diamond(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32) {
    let k = s / 16.0;
    let (x, y) = (ox + cx * k, oy + cy * k);
    let rr = r * k;
    p.polygon(&[(x, y - rr), (x + rr, y), (x, y + rr), (x - rr, y)]);
}

fn outline_diamond(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32, w: f32) {
    let k = s / 16.0;
    let (x, y) = (ox + cx * k, oy + cy * k);
    let rr = r * k;
    p.polyline(&[(x, y - rr), (x + rr, y), (x, y + rr), (x - rr, y)], true, w * k);
}

fn poly_fill(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, pts: &[(f32, f32)]) {
    let k = s / 16.0;
    let v: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (ox + x * k, oy + y * k)).collect();
    p.polygon(&v);
}

fn stroke_path(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, pts: &[(f32, f32)], closed: bool, w: f32) {
    let k = s / 16.0;
    let v: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (ox + x * k, oy + y * k)).collect();
    p.polyline(&v, closed, w * k);
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

#[allow(clippy::too_many_arguments)]
fn arc(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32, a0: f32, a1: f32, w: f32, n: usize) {
    let pts = arc_pts(cx, cy, r, a0, a1, n);
    stroke_path(p, ox, oy, s, &pts, false, w);
}

/// A true SVG circle replaces the 22-segment polygonal ring.
fn ring(p: &mut SvgCanvas, ox: f32, oy: f32, s: f32, cx: f32, cy: f32, r: f32, w: f32, _n: usize) {
    let k = s / 16.0;
    p.ring(ox + cx * k, oy + cy * k, r * k, w * k);
}

/// Draw `icon` in `color` at `size` px, as an element.
///
/// kit 契约：`Icon::new(name).with_size(px(20.)).text_color(theme.red)` →
/// `svg().path("icons/<name>.svg").w().h().flex_none().text_color(...)`.
/// 调用点（`icon::paint(icon, rgba(t.icon), 14.0)`）一行都不用改。
pub fn paint(icon: Icon, color: impl Into<gpui::Rgba>, size: f32) -> impl IntoElement {
    let rgba: gpui::Rgba = color.into();
    svg()
        .path(icon.asset_path())
        .w(px(size))
        .h(px(size))
        .flex_none()
        .text_color(Hsla::from(rgba))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_icon_emits_a_well_formed_svg() {
        let t = table();
        assert_eq!(t.len(), Icon::ALL.len());
        assert_eq!(t.len(), 79, "Icon 变体数变了：请同步 ALL / asset_name / 本断言");
        let mut names = HashSet::new();
        for a in t {
            assert!(names.insert(a.name), "duplicate asset name {}", a.name);
            let s = std::str::from_utf8(&a.svg).expect("svg is utf8");
            assert!(s.starts_with("<svg "), "{}: {}", a.name, s);
            assert!(s.ends_with("</svg>"), "{}", a.name);
            assert!(s.contains("viewBox=\"0 0 16 16\""), "{}", a.name);
            assert!(!s.contains("NaN") && !s.contains("inf"), "{}: {}", a.name, s);
            assert!(
                s.contains("<path") || s.contains("<circle") || s.contains("<rect"),
                "{} has no shapes: {}",
                a.name,
                s
            );
        }
    }

    #[test]
    fn asset_source_resolves_every_icon_path() {
        let assets = IconAssets;
        for a in table() {
            assert_eq!(a.icon.asset_path(), a.path);
            let got = assets.load(&a.path).unwrap().expect("load");
            assert_eq!(&*got, a.svg.as_slice(), "{}", a.name);
        }
        assert!(assets.load("icons/not-a-real-icon.svg").unwrap().is_none());
        assert!(assets.load("other/x.svg").unwrap().is_none());
        assert_eq!(assets.list("icons").unwrap().len(), Icon::ALL.len());
        assert!(assets.list("other").unwrap().is_empty());
    }

    /// GPUI 的 `SvgRenderer` 用 usvg 0.45.1 把 SVG 渲成 alpha mask；解析失败时 `paint_svg`
    /// 只 `log_err()` 然后静默不画。这里用同一个解析器把 79 个图标全部过一遍。
    #[test]
    fn every_icon_svg_parses_with_gpui_s_usvg() {
        for a in table() {
            let parsed = usvg::Tree::from_data(&a.svg, &usvg::Options::default());
            assert!(parsed.is_ok(), "{}: {:?}", a.name, parsed.err());
            let tree = parsed.unwrap();
            assert_eq!(tree.size().width(), 16.0, "{} viewBox width", a.name);
            assert_eq!(tree.size().height(), 16.0, "{} viewBox height", a.name);
        }
    }

    #[test]
    fn common_glyphs_use_real_svg_primitives() {
        let svg = |i: Icon| String::from_utf8(svg_canvas(i).to_svg().into_bytes()).unwrap();
        assert!(svg(Icon::WinMaximize).contains("<rect"), "maximise is a real outline rect");
        let ring = svg(Icon::LiveDisconnected);
        assert!(ring.contains("<circle") && ring.contains("fill=\"none\""), "ring is a circle, not 22 triangles");
        assert!(!svg(Icon::Plus).contains("<circle"), "plus stays two strokes");
    }
}
