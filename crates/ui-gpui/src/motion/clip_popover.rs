// ============================================================================
// Clip 设置浮窗 —— `ui-egui/src/motion/clip_popover.rs` 的 GPUI 移植。
//
// 窗口外壳**直接用** `widgets::popover::Popover`（240/264 宽、40 pt header、标题插槽、
// X、13 圆角、阴影、拖标题栏、贴右栏左缘），本文件只填 title / body 两个插槽。
// body 每行仍然只用 `field_row` / `field_row_full` 交给 `PanelSection::child`。
// ============================================================================
//! The clip's floating settings window over the canvas. It is a `Popover` with a `title_text`
//! header and a `PanelSection` body; the view owns open/close and the dragged position.

use figcraft_i18n::tr_noop;
use figcraft_i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

use figcraft_doc::anim::motion::{Clip, ClipKind, ClipMode, Ease};
use figcraft_doc::{Document, NodeId};
use gpui::{AnyElement, App, Entity, IntoElement, Point, SharedString, Size, Window, div, prelude::*, px};
use crate::widgets::{pt, rgba, tokens};

use crate::motion::{Act, MotionUi, Queue};
use crate::widgets::dropdown::dropdown;
use crate::widgets::input::Slot;

use crate::widgets::input::number_field::number_field;
use crate::widgets::input::text_field::TextField;
use crate::widgets::menu::MenuRow;
use crate::widgets::popover::{HEADER_H, ITEM_GAP, Open as PopoverOpen, popover_body, popover_window, title_text};

pub const WIDTH: f32 = 264.0;

/// Seven retained numeric fields, paired on the unified grid (Delay | Duration,
/// Amount | From X, From Y | To X, To Y | blank).
pub struct ClipFields {
    fields: Vec<Entity<TextField>>,
    /// The clip as of this frame; a commit clones it, changes one value and pushes `SetClip`.
    draft: Rc<RefCell<Option<Clip>>>,
    node: Option<NodeId>,
    id: u32,
}

const NUM_LABELS: [&str; 7] = [tr_noop("Delay"), tr_noop("Duration"), "Amount", "From X", "From Y", "To X", "To Y"];

/// egui `amount_unit` / `amount_label`：不同动画类型的幅度单位（° / % / 距离）。
fn amount_unit(kind: ClipKind) -> (f32, &'static str) {
    match kind {
        ClipKind::Position => (1.0, ""),
        ClipKind::Rotation => (1.0, "°"),
        _ => (100.0, "%"),
    }
}

fn amount_label(kind: ClipKind) -> &'static str {
    match kind {
        ClipKind::Position => "Distance",
        ClipKind::Rotation => "Angle",
        ClipKind::Opacity => tr("Opacity"),
        ClipKind::Scale => tr("Scale"),
        ClipKind::Size => tr("Size"),
    }
}

/// 当前 clip 下 7 个数值框的标签、可见性和显示倍率。
/// egui 的 Custom 模式才显示 From/To，且只有 Position/Scale/Size 是 vec2（X+Y 两列）。
fn labels_for(c: &Clip) -> ([String; 7], [bool; 7], f32) {
    let (f, suffix) = amount_unit(c.kind);
    let vec2 = matches!(c.kind, ClipKind::Position | ClipKind::Scale | ClipKind::Size);
    let custom = c.mode == ClipMode::Custom;
    let mut labels: [String; 7] = NUM_LABELS.map(str::to_string);
    labels[2] = format!("{}{}", amount_label(c.kind), if suffix.is_empty() { String::new() } else { format!(" {suffix}") });
    let mut visible = [false; 7];
    visible[0] = true; // Delay
    visible[1] = true; // Duration
    visible[2] = true; // Amount
    if custom {
        visible[3] = true; // From X
        visible[4] = vec2; // From Y
        visible[5] = true; // To X
        visible[6] = vec2; // To Y
    }
    (labels, visible, f)
}

impl ClipFields {
    pub fn new(cx: &mut App, queue: &Queue) -> Self {
        let draft = Rc::new(RefCell::new(None::<Clip>));
        let mut fields = Vec::with_capacity(NUM_LABELS.len());
        for (i, label) in NUM_LABELS.iter().enumerate() {
            let d = draft.clone();
            let q = queue.clone();
            // egui `number_field_mixed(..., Lead::None, ...)`：label 由外层 `field_line` 的第 1 列
            // 画（x=4），框内 lead 留空，否则同一行出现两遍文字。width=0 → 填满 124 pt control 槽。
            let field = number_field(cx, 0.0, None, "", 1.0, Slot::None);
            field.update(cx, |tf, _| {
                tf.set_on_commit(move |s, _window, _cx| {
                    let Ok(v) = s.trim().parse::<f64>() else { return };
                    let Some(mut c) = d.borrow().as_ref().cloned() else { return };
                    // 显示值 = 模型值 * factor（egui amount_unit），提交时反算回去。
                    let f = CLIP_FACTOR.with(|c| c.get()) as f64;
                    let val = v / f;
                    match i {
                        0 => c.delay_ms = v.max(0.0) as u32,
                        1 => c.duration_ms = v.max(1.0) as u32,
                        2 => c.amount = val as f32,
                        3 => c.from.x = val,
                        4 => c.from.y = val,
                        5 => c.to.x = val,
                        _ => c.to.y = val,
                    }
                    if let Some((node, id)) = CLIP_TARGET.with(|c| *c.borrow()) {
                        q.push(Act::SetClip(node, id, Box::new(c)));
                    }
                });
            });
            fields.push(field);
        }
        Self { fields, draft, node: None, id: 0 }
    }

    /// Refresh every field from the currently open clip.
    pub fn sync(&mut self, cx: &mut App, doc: &Document, ui: &MotionUi) {
        let Some((node, id)) = ui.clip_popover else {
            *self.draft.borrow_mut() = None;
            return;
        };
        self.node = Some(node);
        self.id = id;
        CLIP_TARGET.with(|c| *c.borrow_mut() = Some((node, id)));
        let Some(c) = doc.node(node).and_then(|n| n.motion.clips.iter().find(|c| c.id == id)).cloned() else {
            *self.draft.borrow_mut() = None;
            return;
        };
        *self.draft.borrow_mut() = Some(c.clone());
        let (_labels, _visible, factor) = labels_for(&c);
        CLIP_FACTOR.with(|f| f.set(factor));
        let values = [
            c.delay_ms as f64,
            c.duration_ms as f64,
            c.amount as f64 * factor as f64,
            c.from.x * factor as f64,
            c.from.y * factor as f64,
            c.to.x * factor as f64,
            c.to.y * factor as f64,
        ];
        for (f, v) in self.fields.iter().zip(values) {
            f.update(cx, |tf, _| {
                // lead 恒为 `Slot::None`（构造时定死）：label 由外层 `field_line` 画，框内不再重复。
                tf.set_value(format!("{v}"));
            });
        }
    }

    fn field(&self, i: usize) -> Entity<TextField> {
        self.fields[i].clone()
    }
}

thread_local! {
    /// 当前 clip 的显示倍率（`amount_unit`）；'static 提交闭包读它。
    static CLIP_FACTOR: std::cell::Cell<f32> = const { std::cell::Cell::new(1.0) };
    /// The `(node, clip id)` the number fields commit to. The fields are retained entities whose
    /// callbacks are `'static`, so they cannot hold the frame's `(node, id)` directly.
    static CLIP_TARGET: RefCell<Option<(NodeId, u32)>> = const { RefCell::new(None) };
    /// 头部栏拖动：与浮窗 / 尺寸分隔条共用 `hold_drag` 状态机。
    /// 与 easing/timeline **同构**：模块自持、根视图的 move 推进、up 收尾。
    /// （不走 `drag_router`：它用 move 事件的 `pressed_button == Left` 判断「仍按住」，
    ///   而 GPUI 的 move 事件通常不带该按钮 → 第一个 move 就被当成松手 → 拖一下就卡。）
    static CLIP_DRAG: RefCell<Option<(crate::hold_drag::HoldDrag, Queue)>> = const { RefCell::new(None) };
    /// 上一帧卡片在窗口中的左上角，供按下时 latch 抓取偏移。
    static CLIP_ORIGIN: RefCell<Option<Point<gpui::Pixels>>> = const { RefCell::new(None) };
    /// 触发那一条动画行的顶边 y（窗口坐标）。`inspector` 直接写这里，
    /// 不只依赖 queue 里的 Act，保证「卡片上边与那一行齐高」一定生效。
    pub static ANCHOR_Y: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) };
}

/// 记下打开弹窗的那一条动画行的顶边（行高 24，按下点通常在行中央 → −12）。
pub fn set_anchor_y(y: f32) {
    ANCHOR_Y.with(|a| a.set(Some(y)));
}

/// 头部栏是否正在拖动（根视图据此接管 move / up）。
pub fn drag_active() -> bool {
    CLIP_DRAG.with(|d| d.borrow().is_some())
}

/// 按下头部栏：latch 起点与抓取偏移，并把提交用的 queue 一起存住（根视图 move 要 push Act）。
pub fn begin_drag(at: Point<gpui::Pixels>, origin: Point<gpui::Pixels>, queue: &Queue) {
    let mut hold = crate::hold_drag::HoldDrag::new(3.0);
    hold.press(at, origin);
    CLIP_DRAG.with(|d| *d.borrow_mut() = Some((hold, queue.clone())));
}

/// 窗口级 move：按相对位移推出新落点并 push `MoveClipPopover`。根 div 覆盖整窗，
/// 指针移出卡片仍连续；返回 true 表示这次 move 被消费。
pub fn route_move(pos: Point<gpui::Pixels>) -> bool {
    CLIP_DRAG.with(|d| {
        let mut slot = d.borrow_mut();
        let Some((hold, queue)) = slot.as_mut() else { return false };
        if let Some(step) = hold.advance(pos) {
            queue.push(Act::MoveClipPopover(step.place));
        }
        true
    })
}

/// 松手 / 窗口外松手 / Escape：清状态，供 up 收尾。
pub fn end_drag() {
    CLIP_DRAG.with(|d| *d.borrow_mut() = None);
}

/// egui `clip_popover::field()` 的固定两列网格（PR#1624 `clip_popover.rs:45-58`）：
///   - 行高 `ROW_H = 32`；
///   - label 在 `x = 4`（固定 104 pt 列，**不** flex 撑满）；
///   - control 在 `x = 108`，宽 `CONTROL_W = 124`、高 `CONTROL_H = 24`，垂直居中；
///   - 右缘 `108 + 124 = 232`，在 240 pt 内容宽里留 8 pt。
/// 所有行共用这两条列轨道 → 严格对齐，就是 egui 的那张网格。绝不 `flex_1` label、
/// 也不让 control 贴死卡片右缘（那会「左右都填满」、没有列边界）。
const ROW_H: f32 = 32.0;
const LABEL_X: f32 = 4.0;
const CONTROL_X: f32 = 108.0;
const CONTROL_W: f32 = 124.0;
const CONTROL_H: f32 = 24.0;

fn field_line(label: impl Into<SharedString>, control: impl IntoElement) -> gpui::Div {
    let t = tokens();
    div()
        .h(pt(ROW_H))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        // 第 1 列：固定 108 pt（label 文字从 x=4 起），吃掉 control 左边的网格区。
        .child(
            div()
                .w(pt(CONTROL_X))
                .flex_none()
                .overflow_hidden()
                .whitespace_nowrap()
                .pl(pt(LABEL_X))
                .text_size(pt(11.0))
                .text_color(rgba(t.text_secondary))
                .child(label.into()),
        )
        // 第 2 列：固定 124 pt、24 高的 control 槽（下拉 / 数字框 / 眼睛组填这里）。
        .child(
            div()
                .w(pt(CONTROL_W))
                .h(pt(CONTROL_H))
                .flex_none()
                .child(control),
        )
    // 240 − 108 − 124 = 8 pt 自然留在右侧（egui 的右边距），不再顶到卡片边缘。
}

/// Render the settings window. Returns the card element (already positioned) for the root view.
pub fn panel(
    cx: &mut App,
    doc: &Document,
    ui: &MotionUi,
    queue: &Queue,
    fields: &mut ClipFields,
    vp: Size<gpui::Pixels>,
    root: gpui::Entity<crate::CanvasView>,
    // 右栏宽度，由 `host::motion_clip_popover_layer` 从 `self` 直接取并传入。
    // 不能在这里 `root.read(cx)`：渲染期 CanvasView 正被 update，读自己会 panic。
    right_w: f32,
) -> Vec<AnyElement> {
    // 两种浮窗互斥，同一时刻只画一个：帧间线的**独立缓动窗**（只曲线）与 clip 预设窗。
    if ui.clip_popover.is_none() && ui.segment_ease.is_some() {
        return segment_ease_panel(cx, doc, ui, queue, vp, root, right_w);
    }
    let Some((node, id)) = ui.clip_popover else { return Vec::new() };
    fields.sync(cx, doc, ui);
    let Some(c) = doc.node(node).and_then(|n| n.motion.clips.iter().find(|c| c.id == id)).cloned() else {
        return Vec::new()
    };
    let q = queue.clone();

    // ── Enum rows: each opens a MENU of its options on click (egui `menu_popup` + `menu_check`),
    //    not a click-to-cycle. The chosen value is applied via `Act::SetClip`, same as egui. ──────
    let kind_row = dropdown(SharedString::from(c.kind.name()), 0.0, None).fill().on_click({
        let root = root.clone();
        let q = q.clone();
        let d = fields.draft.clone();
        move |anchor, _window, cx: &mut App| {
            let d = d.clone();
            let q = q.clone();
            let menu: Vec<MenuRow> = ClipKind::ALL.iter().map(|k| MenuRow::new(k.name())).collect();
            let on_pick = std::rc::Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some(c) = d.borrow().as_ref() {
                    if let Some(k) = ClipKind::ALL.iter().find(|k| k.name() == label) {
                        let mut next = c.clone();
                        next.kind = *k;
                        next.direction = next.kind.default_direction();
                        q.push(Act::SetClip(node, id, Box::new(next)));
                    }
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(168.0), menu, on_pick, cx));
        }
    });
    let mode_row = dropdown(SharedString::from(c.mode.label(c.kind)), 0.0, None).fill().on_click({
        let root = root.clone();
        let q = q.clone();
        let d = fields.draft.clone();
        let kind = c.kind;
        move |anchor, _window, cx: &mut App| {
            let d = d.clone();
            let q = q.clone();
            let modes = [ClipMode::In, ClipMode::Out, ClipMode::Custom];
            let menu: Vec<MenuRow> = modes.iter().map(|m| MenuRow::new(m.label(kind))).collect();
            let on_pick = std::rc::Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some(c) = d.borrow().as_ref() {
                    if let Some(m) = modes.iter().find(|m| m.label(kind).as_str() == label) {
                        let mut next = c.clone();
                        next.mode = *m;
                        q.push(Act::SetClip(node, id, Box::new(next)));
                    }
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(160.0), menu, on_pick, cx));
        }
    });
    let dir_row = dropdown(SharedString::from(c.direction.label(c.mode)), 0.0, None).fill().on_click({
        let root = root.clone();
        let q = q.clone();
        let d = fields.draft.clone();
        let mode = c.mode;
        let dirs = c.kind.directions().to_vec();
        move |anchor, _window, cx: &mut App| {
            let d = d.clone();
            let q = q.clone();
            let dirs = dirs.clone();
            let menu: Vec<MenuRow> = dirs.iter().map(|dd| MenuRow::new(dd.label(mode))).collect();
            let on_pick = std::rc::Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some(c) = d.borrow().as_ref() {
                    if let Some(dd) = dirs.iter().find(|dd| dd.label(mode).as_str() == label) {
                        let mut next = c.clone();
                        next.direction = *dd;
                        q.push(Act::SetClip(node, id, Box::new(next)));
                    }
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(160.0), menu, on_pick, cx));
        }
    });
    let ease_row = dropdown(SharedString::from(c.easing.name()), 0.0, None).fill().on_click({
        let root = root.clone();
        let q = q.clone();
        let d = fields.draft.clone();
        move |anchor, _window, cx: &mut App| {
            let d = d.clone();
            let q = q.clone();
            let menu: Vec<MenuRow> = Ease::PRESETS.iter().map(|(name, _)| MenuRow::new(*name)).collect();
            let on_pick = std::rc::Rc::new(move |label: &str, _w: &mut Window, _cx: &mut App| {
                if let Some(c) = d.borrow().as_ref() {
                    if let Some((_, e)) = Ease::PRESETS.iter().find(|(name, _)| *name == label) {
                        let mut next = c.clone();
                        next.easing = *e;
                        q.push(Act::SetClip(node, id, Box::new(next)));
                    }
                }
            });
            root.update(cx, |cv, cx| cv.open_menu(anchor, Some(168.0), menu, on_pick, cx));
        }
    });

    let (labels, visible, _factor) = labels_for(&c);
    // A plain column: the card's own title already names the clip, and the unified grid's
    // two-column rows are what squeezed these fields in the first place.
    // 通用 body 插槽容器：里面每一项之间 8pt，间距只在 `popover_body` 里定义一次。
    let mut sec = popover_body()
        .child(field_line(tr("Animation"), kind_row))
        .child(field_line(tr("Type"), mode_row));
    // Only the fields this kind/mode shows (Custom shows From/To, vec2 kinds show Y).
    let shown: Vec<usize> = (0..7).filter(|i| visible[*i]).collect();
    for &i in &shown {
        sec = sec.child(field_line(SharedString::from(labels[i].clone()), fields.field(i)));
    }
    // Easing 行的眼睛 = 折叠/展开**下方曲线编辑器**（用户 2026-11-12 明确定正：
    // 它控的是曲线那块的显示隐藏，不是 clip 本身的显隐）。展开=Eye、折叠=EyeOff。
    // 状态按 clip 记在 `ui.easing_collapsed`，纯 UI，不写文档、不改 clip.visible。
    let q_eye = queue.clone();
    let collapsed = ui.easing_collapsed.contains(&(node, id));
    let eye = crate::widgets::icon_button::IconButton::new(if collapsed {
        crate::widgets::icon::Icon::EyeOff
    } else {
        crate::widgets::icon::Icon::Eye
    })
    .active(!collapsed)
    .on_click(move |_, _, _| q_eye.push(Act::ToggleClipEasingFold(node, id)));
    let easing_control = div()
        .flex().flex_row().items_center().gap(pt(4.0)).w_full()
        .child(div().flex_1().min_w(px(0.0)).child(ease_row))
        .child(div().flex_none().child(eye));
    sec = sec
        .child(field_line(tr("Direction"), dir_row))
        .child(field_line(tr("Easing"), easing_control));
    // egui PR#1624 `clip_popover::easing_rows`: Easing 下拉之后，编辑器直接占满卡片内容宽
    // (264 − 12×2 = 240)，高度由 `clip_editor_height` 给出。**不要**再套 `field_row_full`：
    // 那是 32pt 高的网格行，144pt 的图表会上下各溢出 56pt 压住标题和数值框，而且它自带的
    // 16/8/24 padding 会把 208pt 的画布挤到 128pt，横向画出卡片。
    // Easing 行的眼睛折叠时**不画曲线编辑器**（用户 2026-11-12：眼睛控曲线显示隐藏）。
    if !collapsed {
        sec = sec.child(crate::motion::easing::clip_editor(
            cx, c.easing, crate::motion::EaseTarget::Clip(node, id), queue, WIDTH - 24.0,
        ));
    }

    let title = format!("{} · {}", c.kind.name(), c.mode.label(c.kind));
    let dismiss: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new({
        let q = queue.clone();
        move |_, _| q.push(Act::CloseClipPopover)
    });
    let drag: Rc<dyn Fn(&gpui::MouseDownEvent, &mut Window, &mut App)> = Rc::new({
        let q = queue.clone();
        move |e, _, _| {
            // 按下只 latch，不移动卡片、也不 notify（保住头部栏按钮的 click）。
            // move 由根视图 `clip_popover::route_move` 推进、up 由 `end_drag` 收尾。
            let origin = CLIP_ORIGIN.with(|o| o.borrow().unwrap_or(e.position));
            begin_drag(e.position, origin, &q);
        }
    });

    // egui `clip_popover::show`：卡片右缘贴「右栏左缘 − 8」，顶边与触发行顶边对齐；
    // `clip_at`（拖动落点 / 时间轴点击）优先。溢出由 `open_pos` 上抬 + `clamp` 夹回屏内。
    let anchor_y = ANCHOR_Y.with(|a| a.get())
        .or_else(|| ui.clip_anchor.map(|b| f32::from(b.origin.y)))
        .unwrap_or(100.0);
    // 「右边靠」：卡片右缘贴右栏左缘 − 8（`Open::Anchor` 的契约）。`right_w` 由调用方
    // 从 `self` 取好传入（渲染期读 root 会重入 panic），用户拖过右栏宽度后依然准确。
    let anchor_x = f32::from(vp.width) - right_w - 8.0;
    let open = if let Some(p) = ui.clip_at {
        PopoverOpen::At(p)
    } else {
        PopoverOpen::Anchor(Point { x: px(anchor_x), y: px(anchor_y) })
    };
    // `Popover::height` 只用于抬升 / 夹住卡片的位置，按实际内容估：
    // 40 头部栏 + 24 pad + ROW_H/字段行 + 曲线编辑器（折叠时不含） + body 各项间的 8pt 间距。
    let n_fields = 2 + shown.len() + 2;
    let field_rows = n_fields as f32 * ROW_H;
    // 展开时 n_fields 行 + 1 编辑器 = n_fields+1 项 → n_fields 个间隔；折叠时少一项、少一个间隔。
    let n_items = n_fields + if collapsed { 0 } else { 1 };
    let item_gaps = (n_items.saturating_sub(1)) as f32 * ITEM_GAP;
    let editor_h = if collapsed { 0.0 } else { crate::motion::easing::clip_editor_height(c.easing) };
    let est_h = HEADER_H + 24.0 + field_rows + item_gaps + editor_h;
    // 卡片自身的 move：弹窗是独立 `occlude()` 浮层，根 div 的 on_mouse_move 收不到它上方的
    // 指针，所以必须把推进绑在卡片这一层（指针在弹窗内一定命中）。拖出弹窗后由根 div move
    // 兜底，两路都调同一个幂等的 `route_move`。
    let pop = popover_window(open)
        .width(WIDTH)
        .pad(12.0)
        .height(est_h)
        .title(title_text(title))
        .body(sec)
        .on_drag_start(move |e, w, cx| drag(e, w, cx))
        .on_drag_move(move |e: &gpui::MouseMoveEvent, _, _| {
            route_move(e.position);
        })
        // 松手收尾：occlude 弹窗吞掉根 div 的 up，指针停在卡片上方时靠这条清掉 CLIP_DRAG，
        // 否则「松手放不下、鼠标一动又跟着走」。组件自带能力，调用方只管接 end_drag。
        .on_drag_up(move |_, _, _| {
            // 只清 thread_local 拖拽状态：move 阶段已 push Act + notify，松手帧无位移、无需再刷新。
            end_drag();
        });
    let (origin, card) = pop.show(vp, dismiss);
    CLIP_ORIGIN.with(|o| *o.borrow_mut() = Some(origin));
    vec![card]
}

/// 帧间线（segment）的**独立缓动浮窗**：只有 Curve/Spring 曲线编辑器，不含 clip 的
/// Animation / Type / Delay / Duration / Distance / Direction 等 preset 字段（用户 2026-11-12：
/// 帧间线设置是「单独的」，但必须能弹窗设曲线）。编辑目标是 `EaseTarget::Key(k)`——
/// 段起点帧的 `easing` 决定这一段的插值曲线（`write_ease` 的 Key 分支）。
/// 外壳 / 拖动 / 锚点 / 关闭全部复用与 clip 窗同一套（同一时刻两者互斥）。
fn segment_ease_panel(
    cx: &mut App,
    doc: &Document,
    ui: &MotionUi,
    queue: &Queue,
    vp: Size<gpui::Pixels>,
    _root: gpui::Entity<crate::CanvasView>,
    right_w: f32,
) -> Vec<AnyElement> {
    let Some(k) = ui.segment_ease else { return Vec::new() };
    // 段起点帧的 easing（与 easing::panel 读法一致）。帧不存在（被删）就关窗。
    let Some(ease) = doc.node(k.node)
        .and_then(|n| n.motion.track(k.property))
        .and_then(|tr| tr.key_at(k.time_ms))
        .map(|key| key.easing) else {
        return Vec::new()
    };

    let body = popover_body().child(crate::motion::easing::clip_editor(
        cx, ease, crate::motion::EaseTarget::Key(k), queue, WIDTH - 24.0,
    ));
    let dismiss: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new({
        let q = queue.clone();
        move |_, _| q.push(Act::CloseSegmentEase)
    });
    let drag: Rc<dyn Fn(&gpui::MouseDownEvent, &mut Window, &mut App)> = Rc::new({
        let q = queue.clone();
        move |e, _, _| {
            let origin = CLIP_ORIGIN.with(|o| o.borrow().unwrap_or(e.position));
            begin_drag(e.position, origin, &q);
        }
    });

    // 只读 `clip_anchor`（segment 点击前 push 的 `SetClipAnchorY`），不读 `ANCHOR_Y`——
    // 那是 clip **行**点击的 thread_local，读了会残留上次的位置、与段窗串味。
    let anchor_y = ui.clip_anchor.map(|b| f32::from(b.origin.y)).unwrap_or(100.0);
    let anchor_x = f32::from(vp.width) - right_w - 8.0;
    let open = if let Some(p) = ui.clip_at {
        PopoverOpen::At(p)
    } else {
        PopoverOpen::Anchor(Point { x: px(anchor_x), y: px(anchor_y) })
    };
    // 高度：40 头 + 24 pad + 曲线编辑器（Curve/Spring + 图表 + 数值 + 可能弹簧参数行）。
    let est_h = HEADER_H + 24.0 + crate::motion::easing::clip_editor_height(ease);
    let pop = popover_window(open)
        .width(WIDTH)
        .pad(12.0)
        .height(est_h)
        .title(title_text(tr("Easing")))
        .body(body)
        .on_drag_start(move |e, w, cx| drag(e, w, cx))
        .on_drag_move(move |e: &gpui::MouseMoveEvent, _, _| { route_move(e.position); })
        .on_drag_up(move |_, _, _| { end_drag(); });
    let (origin, card) = pop.show(vp, dismiss);
    CLIP_ORIGIN.with(|o| *o.borrow_mut() = Some(origin));
    vec![card]
}



#[cfg(test)]
mod tests {
    use super::*;

    /// egui `clip_popover::field()` 的固定两列网格：label@4 / control@108 宽124 / 行高32 /
    /// 右缘留 8。回归测试锁死这些数值，防止再被改回 `flex_1` 撑满 + control 贴死右缘。
    #[test]
    fn field_grid_matches_egui_fixed_columns() {
        assert_eq!(ROW_H, 32.0, "行高 = egui ROW_H");
        assert_eq!(LABEL_X, 4.0, "label 从 x=4 起");
        assert_eq!(CONTROL_X, 108.0, "control 列起点 = egui CONTROL_X");
        assert_eq!(CONTROL_W, 124.0, "control 列宽 = egui CONTROL_W");
        assert_eq!(CONTROL_H, 24.0, "control 高 = egui CONTROL_H，垂直居中于 32 行");
        let right = CONTROL_X + CONTROL_W;
        assert_eq!(right, 232.0, "control 右缘");
        let body_w = WIDTH - 24.0; // 264 − 12×2 pad = 240 内容宽
        assert_eq!(body_w - right, 8.0, "右边留 8 pt，不贴死卡片边缘");
    }

    #[test]
    fn labels_follow_kind_and_custom_mode() {
        let mut c = Clip::new(ClipKind::Opacity, ClipMode::In);
        let (labels, visible, factor) = labels_for(&c);
        assert_eq!(factor, 100.0, "透明度用百分数");
        assert_eq!(labels[2], "Opacity %");
        assert_eq!(visible, [true, true, true, false, false, false, false], "In 模式不显示 From/To");

        c.kind = ClipKind::Position;
        c.mode = ClipMode::Custom;
        let (labels, visible, factor) = labels_for(&c);
        assert_eq!(factor, 1.0, "位移用 px");
        assert_eq!(labels[2], "Distance");
        assert_eq!(visible, [true, true, true, true, true, true, true], "Custom + vec2 显示 X/Y");

        c.kind = ClipKind::Rotation;
        let (labels, visible, _f) = labels_for(&c);
        assert_eq!(labels[2], "Angle °");
        assert_eq!(visible[4], false, "旋转不是 vec2，没有 From Y");
        assert_eq!(visible[6], false);
    }
}
