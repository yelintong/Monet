// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// ----------------------------------------------------------------------------
// 2026-11-06 解冻（用户指令）：「照搬EGUI还原底部工具栏功能…检查下拉菜单是否由template衍生，
// 并改template禁用样式作为下拉菜单的普通样式，并去掉背景颜色」。
// 本文件因此按指令重写：
//   * 下拉按钮**不再自己拼一个 div**，改为走 `input/template.rs` 的 `Input` 模板（lead | edit |
//     trail 三槽一次布局），与文本框/数字框/颜色行同源；
//   * 普通态取模板的**禁用态样式**（`bare`：无填充、无壳），即**去掉背景颜色**；
//   * hover 只加一圈绝对定位的 1pt 发丝（不参与布局、不改变底色），egui 的 `field_hover_border`。
// 以上三条是本次改动的目标，后续不要退回「自己画背景」的写法。
// ----------------------------------------------------------------------------
// 2026-11-08 解冻（用户指令）：「这个下拉弹窗的这个跟随按钮、宽度取按钮真实宽度难道不能
// 封装成下拉弹窗通用能力吗」。原先每个调用点各自包一层 canvas 探针、各自算宽度，
// 而且探针每帧随元素树重建 → 点击后根本不会再更新。本次把能力收进控件：
//   * `Dropdown::menu(key)` 登记一个稳定 key，prepaint 每帧把自身矩形写进**跨帧**注册表；
//   * `MenuAnchor` 是触发按钮与弹窗共享的句柄，`CanvasView::open_dropdown_menu` 是唯一开窗入口
//     （宽度取按钮真实宽度、下方 4pt、跟随都由入口统一负责）。
// 以上不变量（模板同源 / bare 常态 / 统一网格）依旧不得破坏。
// ============================================================================
//! The dropdown button — the [`Input`] template in its **bare** (no-fill) state.
//!
//! The design's `Field / Disabled` look is this control's *normal* look: no background, no shell,
//! only the lead, the label and the trailing chevron, with one hairline while hovered. That is the
//! template's `bare` path — the same layout the text/number/colour fields are built on, so the
//! lead slot, the value's inset and the chevron all sit on the design's 24 pt rhythm instead of
//! being hand-positioned. Metrics (`ui-egui`'s `widgets::dropdown`):
//!
//! * **lead** — a 16 pt glyph or a 9 pt colour dot, in the template's 24 pt lead slot;
//! * **label** — the editable middle, elided when the dropdown is narrower than its text;
//! * **chevron** — 12 pt, in the template's 24 pt trail slot (flush with the field's right edge).

use crate::canvas::geom::Color32;
use crate::widgets::icon::{Icon, paint as paint_icon};
use crate::widgets::input::template::{Input, Parts};
use crate::widgets::{LEAD_PAD, RADIUS, SLOT, TEXT_PT, pt, rgba, tokens};
use gpui::{App, Bounds, MouseButton, MouseDownEvent, Pixels, SharedString, Window, div, prelude::*};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

// ── 下拉弹窗的通用锚点（用户 2026-11-08：跟随按钮 + 宽度取按钮真实宽度，收口成通用能力）────
// 右栏 / 动画弹窗每帧重建元素树，探针若随元素一起新建，点击后就不会再更新。所以按一个**稳定
// key** 放进跨帧注册表：按钮每帧 prepaint 写、菜单每帧 render 读，二者共享同一个 cell。
thread_local! {
    static ANCHORS: RefCell<HashMap<String, Rc<Cell<Option<Bounds<Pixels>>>>>> = RefCell::new(HashMap::new());
    /// 当前打开的下拉菜单的 `(key, 点击时的 x)`：只有它对应的按钮写探针，同 key 的其它按钮
    /// 不会互相覆盖（滚动只改 y，x 不变；不同列的下拉 x 天然不同）。
    static ACTIVE: RefCell<Option<(String, f32)>> = RefCell::new(None);
}

/// 触发按钮与它弹出的菜单共享的锚点句柄（Clone 很便宜，内部就是一个 `Rc`）。
#[derive(Clone)]
pub struct MenuAnchor {
    key: String,
    live: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl MenuAnchor {
    /// 取（或建）某个按钮的锚点；同一个 key 永远拿到同一个跨帧 cell。
    pub fn new(key: impl Into<String>) -> Self {
        let key = key.into();
        let live = ANCHORS.with(|m| m.borrow_mut().entry(key.clone()).or_insert_with(|| Rc::new(Cell::new(None))).clone());
        Self { key, live }
    }
    /// 按钮当前矩形（窗口坐标）；`None` 表示还没画过。
    pub fn rect(&self) -> Option<Bounds<Pixels>> { self.live.get() }
    /// 点击那一帧先把 `on_click` 拿到的矩形作为初值，之后由 prepaint 持续刷新。
    pub fn seed(&self, b: Bounds<Pixels>) { self.live.set(Some(b)); }
    pub fn key(&self) -> &str { &self.key }
    pub(crate) fn cell(&self) -> Rc<Cell<Option<Bounds<Pixels>>>> { self.live.clone() }
}

/// `MenuAnchor::new` 的便捷写法。
pub fn menu_anchor(key: impl Into<String>) -> MenuAnchor {
    MenuAnchor::new(key)
}

/// 开关「当前活动的下拉菜单」：只有活动 key（且 x 对得上）的按钮每帧写探针。
pub fn set_active_menu(key: Option<(String, f32)>) {
    ACTIVE.with(|a| *a.borrow_mut() = key);
}
fn active_matches(key: &str, x: f32) -> bool {
    ACTIVE.with(|a| matches!(&*a.borrow(), Some((k, sx)) if k == key && (sx - x).abs() < 1.0))
}

/// The lead a dropdown may carry (`ui-egui`'s `DropLead`).
#[derive(Clone, Copy)]
pub enum DropLead {
    /// A 14 pt glyph.
    Icon(Icon),
    /// A 4.5 pt-radius colour dot.
    Dot(Color32),
}

/// The click callback: it receives the button's own rect (the menu's anchor).
pub type OnClick = Rc<dyn Fn(gpui::Bounds<Pixels>, &mut Window, &mut App)>;

/// A dropdown-looking button showing `label`, `width` points wide.
pub struct Dropdown {
    label: SharedString,
    width: f32,
    /// `true`：填满所在的网格单元（`w_full`），不按像素定宽——面板里**不允许手算列宽**。
    fill: bool,
    lead: Option<DropLead>,
    on_click: Option<OnClick>,
    /// 下拉弹窗的锚点 key；登记后每帧上报自身矩形，菜单跟随 + 与按钮同宽。
    menu_key: Option<String>,
}

impl Dropdown {
    /// `ui-egui::dropdown`: `width` is required (`col * 2.0 + 8.0` in the panel), the lead optional.
    pub fn new(label: impl Into<SharedString>, width: f32, lead: Option<DropLead>) -> Self {
        Self { label: label.into(), width, fill: false, lead, on_click: None, menu_key: None }
    }

    /// 登记为一个「下拉弹窗」的触发按钮。key 必须**跨帧稳定**（同一个按钮每帧传同一个字符串）；
    /// 开窗走 `CanvasView::open_dropdown_menu`，宽度 / 下方 4pt / 跟随都由那里统一处理。
    pub fn menu(mut self, key: impl Into<String>) -> Self {
        self.menu_key = Some(key.into());
        self
    }

    /// 填满所在网格单元（`w_full`）：列宽由统一网格的 `flex_1` 决定，调用方不传宽度。
    pub fn fill(mut self) -> Self {
        self.fill = true;
        self
    }

    /// Called on a click, with **the button's own rect** — `ui-egui` passes the field's `Response`
    /// to `menu_popup`, whose width and position both come from that rect, not from the pointer.
    pub fn on_click(mut self, f: impl Fn(gpui::Bounds<Pixels>, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl IntoElement for Dropdown {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        let t = tokens();
        let width = self.width;
        let on_click = self.on_click;
        // 每帧新建的 hit 只服务「本次点击」；跨帧的那个在注册表里，由 `MenuAnchor` 持有。
        let hit = Rc::new(Cell::new(None::<Bounds<Pixels>>));
        let hit_click = hit.clone();
        let anchor = self.menu_key.map(MenuAnchor::new);

        // ONE template: lead | edit | trail, measured by `layout` (no window needed — the slots'
        // widths are the design's 24 pt cell and an 8 pt empty lead).
        let input = Input::new(width).bare(true);
        let lead_w = if self.lead.is_some() { SLOT } else { LEAD_PAD };
        let frame = input.layout(lead_w, SLOT);

        let lead = self.lead.map(|lead| match lead {
            DropLead::Icon(i) => div().flex_none().size(pt(16.0)).child(paint_icon(i, rgba(t.text_secondary), 16.0)).into_any_element(),
            // A 9 pt dot centred in the 24 pt slot, as the original draws it.
            DropLead::Dot(c) => div().flex_none().size(pt(9.0)).rounded(pt(5.0)).bg(rgba(c)).into_any_element(),
        });
        let label = div()
            .w_full()
            .h_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(pt(TEXT_PT))
            .text_color(rgba(t.text))
            .child(self.label.clone())
            .into_any_element();
        let chevron = div().flex_none().size(pt(12.0)).child(paint_icon(Icon::ChevronDown, rgba(t.text_secondary), 12.0)).into_any_element();

        let mut el = input.paint(&frame, Parts { lead, edit: Some(label), trail: Some(chevron), ..Default::default() });
        el = if self.fill { el } else { el.w(pt(width)) };
        // The permanent hairline: an absolute overlay, so it neither repaints a background nor takes
        // part in layout (the shell's own 1 pt border would shrink the content box by 1). `用户
        // 2026-11-06`：普通态也要有描边（像选中态那样常态化显示），所以常态就是 `border_strong`，
        // 不再透明；hover 时换成 `field_hover_border`。背景色由 `bare` 模板已去掉，这里不动。
        el = el
            .group("dropdown-bare")
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .rounded(pt(RADIUS))
                    .border_1()
                    .border_color(rgba(t.border_strong))
                    .group_hover("dropdown-bare", |s| s.border_color(rgba(t.field_hover_border))),
            )
            // The anchor: the card's width and position both come from this rect.
            .child(
                gpui::canvas(
                    move |bounds, _window, _cx| {
                        hit.set(Some(bounds));
                        // 只有当前打开的那个菜单的按钮写跨帧探针：滚动 / 面板变宽 / 弹窗拖动
                        // 后，`Popup::Menu` 每帧 render 都能读到按钮的最新矩形。
                        if let Some(a) = &anchor {
                            if active_matches(&a.key, f32::from(bounds.origin.x)) {
                                a.live.set(Some(bounds));
                            }
                        }
                    },
                    |_bounds, _, _window, _cx| {},
                )
                .absolute()
                .size_full(),
            )
            .on_mouse_down(MouseButton::Left, move |_e: &MouseDownEvent, window, cx| {
                if let (Some(f), Some(rect)) = (on_click.as_ref(), hit_click.get()) {
                    f(rect, window, cx);
                }
            });
        el
    }
}

/// `ui-egui::dropdown`: a dropdown with an optional **icon** lead.
pub fn dropdown(label: impl Into<SharedString>, width: f32, lead: Option<Icon>) -> Dropdown {
    dropdown_lead(label, width, lead.map(DropLead::Icon))
}

/// `ui-egui::dropdown_lead`: the same button with an icon **or a colour dot** before the label.
pub fn dropdown_lead(label: impl Into<SharedString>, width: f32, lead: Option<DropLead>) -> Dropdown {
    Dropdown::new(label, width, lead)
}
