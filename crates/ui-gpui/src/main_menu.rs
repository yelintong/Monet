//! 主菜单：**多级、悬停展开**——行为与几何照 `ui-egui/src/menus/tree.rs`。
//!
//! 为什么不复用 `widgets/menu.rs::menu_popup`：那是**单层**菜单，一行只能「点击选中」，
//! 既没有悬停回调，也没有「出现在第 i 行的右侧、且与该行顶部对齐」这种定位能力。
//! 主菜单需要的是：
//!
//! * 顶层卡片贴左竖栏菜单按钮的**底边往下 4pt**；
//! * **悬停**一行就展开它的子菜单（不用点）；
//! * 子菜单出现在父卡片的**右边 4pt**，并且**与触发它的那一行顶部齐平**；
//! * 三级同理，逐级往右。
//!
//! 后两条的来源是 egui `submenu_at`（`menus/tree.rs:280`）：
//!
//! ```text
//! let mut pos = pos2(r.max.x + gap, r.min.y - 8.0);
//! ```
//!
//! `r.min.y - 8.0` 里的 8 正是外框的内边距（`Margin::same(8)`）——子卡片顶边比那一行
//! 顶边高 8pt，于是子卡片的**第一行**刚好和父菜单的那一行齐平。这里直接按
//! 「子卡片 y = 父卡片 y + 该行之前所有行的高度之和」实现，等价于那条公式。
//!
//! 外框规格（用户 2026-11-05 规定，与 egui `menu_frame` 一致）：圆角 13、四边内边距 8；
//! 菜单项圆角 5、行高 28（egui `allocate_exact_size(vec2(w, 28.0))`）、文字从行左 28pt 起
//! （勾选列）。

use figcraft_i18n::tr_noop;
use figcraft_i18n::tr;
use crate::widgets::menu::MenuRow;
use crate::widgets::{AimAction, MenuAim, TEXT_PT, pt, rgba, tokens};
use gpui::{AnyElement, App, Bounds, MouseButton, Pixels, Point, Window, div, point, prelude::*, px, size};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

/// 顶层菜单名——**逐字照 `ui-egui/src/menus.rs::TOP_MENUS`**（顺序、个数都不许改）。
/// 它们同时是 `figcraft_engine::catalog::MENU` 里 `menu` 路径的第一段。
/// 注意 `MENU` 里还有一组 `Tools`，那是工具栏的，不属于主菜单。
pub(crate) const TOP_MENUS: [&'static str; 11] =
    [tr_noop("File"), tr_noop("Edit"), tr_noop("View"), tr_noop("Object"), tr_noop("Text"), tr_noop("Arrange"), tr_noop("Vector"), tr_noop("Plugins"), tr_noop("Preferences"), tr_noop("Libraries"), tr_noop("Help")];

/// 菜单行高（egui `menus/tree.rs:257` 的 28）。
const ROW_H: f32 = 28.0;
/// 分隔线占的高度：`my(4) + h(1) + my(4)`。
const SEP_H: f32 = 9.0;
/// 勾选列宽度：egui 的文字起点是 `r.min.x + 28.0`。
const CHECK_W: f32 = 28.0;
/// 外框内边距（egui `Margin::same(8)`）。
const PAD: f32 = 8.0;
/// 菜单项圆角（egui `rect_filled(r, CornerRadius::same(5), …)`）。
const ITEM_RADIUS: f32 = 5.0;
/// 子菜单与父卡片之间的间距（egui `submenu(…, gap = 4.0, …)`）。
const GAP: f32 = 4.0;
/// 顶层列表宽度：只有菜单名，200 够了（用户 2026-11-05 指定；egui 的 `MENU_W` 是 240，
/// 但那一版顶层还带一行 "Actions…" 搜索框，我们没有，240 会显得空）。
const W_TOP: f32 = 200.0;
/// 展开层宽度：要放下「标签 + 右对齐快捷键」（egui 的 `MENU_W = 240`）。
const W_SUB: f32 = 240.0;

/// 菜单项的动作。
#[derive(Clone)]
pub(crate) enum Act {
    /// 执行引擎命令。
    Run(&'static str),
    /// 展开子菜单（值是完整的菜单路径，如 `"View/Theme"`）。
    Sub(String),
}

/// 一级菜单的数据。
pub(crate) struct Level {
    pub rows: Vec<MenuRow>,
    /// 与 `rows` 一一对应的 `(显示名, 动作)`；`on_hover` / 点击只拿得到下标，靠它反查。
    pub items: Vec<(String, Act)>,
}

impl Level {
    fn empty() -> Self {
        Self { rows: Vec::new(), items: Vec::new() }
    }
}

/// 一个打开的主菜单：第 0 级是顶层菜单名，之后每级是一个展开的子菜单。
pub(crate) struct MainMenu {
    pub levels: Vec<Level>,
    /// 每级当前展开的子项下标（与 `levels` 等长；`None` = 这一级没展开子菜单）。
    pub open: Vec<Option<usize>>,
    /// 左竖栏菜单按钮的矩形（窗口坐标）。render 时每次重设，窗口 / 面板变化就跟着动
    /// (`用户 2026-11-06`)；`origin()` 由它现算，不在这里写死。
    pub(crate) anchor: Bounds<Pixels>,
    /// 通用控件 `widgets::MenuAim`：安全三角 / 悬停延迟 / 静止交接。
    pub(crate) aim: MenuAim,
    /// 最近一次指针位置（窗口坐标）；静止超时结算时没有新鼠标事件，用它。
    pointer: Point<Pixels>,
    born: Instant,
    /// 是否已有一次延迟唤醒在飞，避免每次 mousemove 都堆一个 timer。
    pub(crate) aim_busy: Rc<Cell<bool>>,
}

impl MainMenu {
    /// `anchor` 是左竖栏菜单按钮的矩形：顶层卡片贴它的**底边往下 4pt**。
    pub fn new(anchor: Bounds<Pixels>, first: Level) -> Self {
        Self {
            levels: vec![first],
            open: vec![None],
            anchor,
            aim: MenuAim::new(),
            pointer: point(px(0.0), px(0.0)),
            born: Instant::now(),
            aim_busy: Rc::new(Cell::new(false)),
        }
    }

    fn now(&self) -> f64 {
        self.born.elapsed().as_secs_f64()
    }

    /// 第 `level` 级卡片的矩形（窗口坐标）。`Menu::show` 用 `at_point(x, y).gap(0)`，
    /// 卡片左上角就是 `pos(level)`；宽 = `width(level)`，高 = 行高总和 + 上下各 8pt。
    fn card_bounds(&self, level: usize) -> Bounds<Pixels> {
        let (x, y) = self.pos(level);
        let rows_h = self.levels.get(level).map(|l| l.rows.iter().map(row_height).sum::<f32>()).unwrap_or(0.0);
        Bounds { origin: point(px(x), px(y)), size: size(px(self.width(level)), px(rows_h + 2.0 * PAD)) }
    }

    /// 该级展开子菜单的那一行的矩形（三角的顶点所在）。
    fn trigger_bounds(&self, level: usize) -> Option<Bounds<Pixels>> {
        let row = (*self.open.get(level)?)?;
        let (x, y) = self.pos(level);
        let l = self.levels.get(level)?;
        let y0 = y + PAD + l.rows.iter().take(row).map(row_height).sum::<f32>();
        Some(Bounds { origin: point(px(x), px(y0)), size: size(px(self.width(level)), px(ROW_H)) })
    }

    /// 子菜单卡片 = 下一级卡片；三角的底边就是它的近边。
    fn panel_bounds(&self, level: usize) -> Option<Bounds<Pixels>> {
        (self.levels.len() > level + 1).then(|| self.card_bounds(level + 1))
    }

    /// 每次输入把各级几何喂给状态机。判定本身由 `MenuAim::hover` 用「上一次指针位置 →
    /// 这一次位置」这一小步完成，这里只负责喂几何。
    fn refresh_geometry(&mut self, pointer: Point<Pixels>, _now: f64) {
        for l in 0..self.levels.len() {
            let card = self.card_bounds(l);
            let trigger = self.trigger_bounds(l);
            let panel = self.panel_bounds(l);
            self.aim.set_geometry(l, card, trigger, panel);
        }
        let _ = pointer;
    }

    /// 把状态机的裁决落到 `levels` / `open` 上；返回菜单树是否真的变了。
    /// 「已经在展开的同一个子菜单」不重建 —— 重建会把下一级连同它自己的展开状态一起丢掉。
    fn apply(&mut self, action: AimAction, build: &impl Fn(&str) -> Level) -> bool {
        match action {
            AimAction::Hold => false,
            AimAction::Close(level) => {
                if level >= self.open.len() || (self.open[level].is_none() && self.levels.len() <= level + 1) {
                    return false;
                }
                self.open[level] = None;
                self.open.truncate(level + 1);
                self.levels.truncate(level + 1);
                self.aim.set_open(level, None);
                true
            }
            AimAction::Open(level, row) => {
                if self.open.get(level) == Some(&Some(row)) && self.levels.len() > level + 1 {
                    return false;
                }
                let Some(path) = self.levels.get(level).and_then(|l| l.items.get(row)).and_then(|(_, a)| match a {
                    Act::Sub(p) => Some(p.clone()),
                    Act::Run(_) => None,
                }) else {
                    return false;
                };
                self.open[level] = Some(row);
                self.open.truncate(level + 1);
                self.levels.truncate(level + 1);
                self.levels.push(build(&path));
                self.open.push(None);
                self.aim.set_open(level, Some(row));
                true
            }
        }
    }

    /// 展开 / 收起后立刻重刷几何并再 tick 一次：此时指针通常还在触发行上，
    /// 这一步把三角顶点种下，用户紧接着斜移才不会漏判。`now` 错开一点以绕开 tick 去重。
    fn reseed(&mut self, pointer: Point<Pixels>, now: f64) -> Option<f64> {
        let next = now + 0.001;
        self.refresh_geometry(pointer, next);
        self.aim.next_wakeup(next)
    }

    /// 顶层卡片的左上角（窗口坐标）：贴着菜单按钮的底边往下 `GAP`。
    fn origin(&self) -> (f32, f32) {
        (f32::from(self.anchor.origin.x), f32::from(self.anchor.origin.y) + f32::from(self.anchor.size.height) + GAP)
    }

    fn width(&self, level: usize) -> f32 {
        if level == 0 { W_TOP } else { W_SUB }
    }

    /// 第 `level` 级卡片的左上角：x 逐级 = 上一级右边 + 4；
    /// y = 上一级卡片 y + 触发它的那一行之前所有行的高度之和。
    fn pos(&self, level: usize) -> (f32, f32) {
        let (ox, oy) = self.origin();
        let mut x = ox;
        let mut y = oy;
        for k in 0..level {
            x += self.width(k) + GAP;
            if let Some(row) = self.open[k] {
                y += self.levels[k].rows.iter().take(row).map(row_height).sum::<f32>();
            }
        }
        (x, y)
    }

    /// 悬停第 `level` 级的第 `row` 行。裁决交给通用控件 `MenuAim`：
    /// 指针正朝已展开的子菜单移动时，这一行被安全三角屏蔽，不接管高亮、不重建下一级。
    ///
    /// 返回还需要多少秒后唤醒一次（`None` = 不用），供调用方安排延迟重绘以结算悬停延迟 /
    /// 静止超时。`build` 在需要展开时构造下一级数据（要读 session 的勾选态，由调用方给）。
    pub fn hover(&mut self, level: usize, row: usize, pointer: Point<Pixels>, build: impl Fn(&str) -> Level) -> Option<f64> {
        self.hover_at(level, row, pointer, self.now(), build)
    }

    /// `hover` 的可测内核：时间由调用方给，测试里就能喂虚拟时间。
    fn hover_at(&mut self, level: usize, row: usize, pointer: Point<Pixels>, now: f64, build: impl Fn(&str) -> Level) -> Option<f64> {
        if level >= self.levels.len() || row >= self.levels[level].items.len() {
            return None;
        }
        self.pointer = pointer;
        self.refresh_geometry(pointer, now);
        let is_sub = matches!(self.levels[level].items[row].1, Act::Sub(_));
        let action = self.aim.hover(level, row, is_sub, pointer, now);
        if self.apply(action, &build) {
            return self.reseed(pointer, now);
        }
        self.aim.next_wakeup(now)
    }

    /// 定时唤醒：没有新的鼠标事件时也要结算悬停延迟 / 三角内静止超时。
    pub fn settle(&mut self, build: impl Fn(&str) -> Level) -> Option<f64> {
        self.settle_at(self.now(), build)
    }

    fn settle_at(&mut self, now: f64, build: impl Fn(&str) -> Level) -> Option<f64> {
        let pointer = self.pointer;
        let actions = self.aim.settle(pointer, now);
        let mut changed = false;
        for action in actions {
            changed |= self.apply(action, &build);
        }
        if changed {
            return self.reseed(pointer, now);
        }
        self.aim.next_wakeup(now)
    }

    /// 点击第 `level` 级的第 `row` 行：命令项返回它的 id（子菜单项已经被悬停展开，点了不做事）。
    pub fn click(&mut self, level: usize, row: usize) -> Option<&'static str> {
        match &self.levels.get(level)?.items.get(row)?.1 {
            Act::Run(id) => Some(id),
            Act::Sub(_) => None,
        }
    }
}

/// 一行占的高度（分隔线额外占 `SEP_H`）。
fn row_height(row: &MenuRow) -> f32 {
    ROW_H + if row.separator_before { SEP_H } else { 0.0 }
}

/// 把 `figcraft_engine::catalog::MENU` 里 `prefix` 这一层摊平成菜单行 ——
/// egui `menus.rs::menu_level` 的等价实现：
///
/// * `m.menu == prefix` 的项 → 一行普通菜单项（带勾选态 + 快捷键）；
/// * `m.menu` 以 `prefix/` 开头 → 取下一段作为**子菜单**，同一段只出现一次，位置取它第一次
///   出现的地方（与 egui 的遍历顺序一致）；
/// * `label == SEP` → 分隔线，但**不单独占一行**：这里的分隔线是在行**上方**画一条线，
///   所以一律挂到下一条真实菜单项的 `separator()` 上（否则会多出一条空白行）。
pub(crate) fn build_level(prefix: &str, checked: &dyn Fn(&str) -> Option<bool>) -> Level {
    let mut level = Level::empty();
    let mut subs: Vec<String> = Vec::new();
    let mut pending_sep = false;
    let mut last_sep = true; // 开头不画分隔线

    for m in figcraft_engine::catalog::MENU.iter() {
        if m.menu == prefix {
            if m.label == figcraft_engine::catalog::SEP {
                if !last_sep {
                    pending_sep = true;
                }
                continue;
            }
            last_sep = false;
            // `checked` 只在命令**真的**有勾选态时才是 `Some`：没有勾选态的行不留勾选列
            // （见 `render` 里的 `has_check`）。
            // 命令文案走 `i18n::menu_item(id, english)`：先查 `[menu]` 再查 `[commands]`，
            // 与 egui `menus.rs` 同一句。直接用 `m.label` 会永远是英文。
            let mut row = MenuRow::new(figcraft_i18n::menu_item(m.id, m.label)).checked(checked(m.id));
            if pending_sep {
                row = row.separator();
                pending_sep = false;
            }
            if let Some(sc) = shortcut_text(m.id) {
                row = row.trail(sc);
            }
            level.rows.push(row);
            level.items.push((m.label.to_string(), Act::Run(m.id)));
        } else if let Some(rest) = m.menu.strip_prefix(prefix).and_then(|r| r.strip_prefix('/')) {
            let sub = rest.split('/').next().unwrap_or(rest).to_string();
            if subs.contains(&sub) {
                continue;
            }
            subs.push(sub.clone());
            last_sep = false;
            // 子菜单行永远没有勾选态 —— 不占勾选列（这一整级若全是子菜单，就完全不留缩进）。
            let mut row = MenuRow::new(tr(&sub).to_string()).trail("▸");
            if pending_sep {
                row = row.separator();
                pending_sep = false;
            }
            level.rows.push(row);
            level.items.push((sub.clone(), Act::Sub(format!("{prefix}/{sub}"))));
        }
    }
    level
}

/// 顶层那 11 个菜单名（都是子菜单入口，没有命令，也没有勾选态）。
pub(crate) fn top_level() -> Level {
    Level {
        // `TOP_MENUS` 是 `const`，只能放 `tr_noop`（英文）；**画的时候必须过 `tr`**，
        // 否则主菜单永远是英文（用户 2026-11-10：「主菜单的一些文本都还没有来得及本地化」）。
        rows: TOP_MENUS.iter().map(|l| MenuRow::new(tr(l)).trail("▸")).collect(),
        items: TOP_MENUS.iter().map(|l| (l.to_string(), Act::Sub(l.to_string()))).collect(),
    }
}

/// 快捷键的显示文本（egui `pretty_shortcut(shortcuts::shortcut(id))` 的等价物）。
pub(crate) fn shortcut_text(id: &str) -> Option<String> {
    let spec = figcraft_engine::catalog::shortcut(id)?;
    let text = figcraft_engine::keymap::display(spec, figcraft_engine::keymap::Platform::current());
    (!text.is_empty()).then_some(text)
}

/// 画出整个菜单树（每级一张卡片，从顶层往右排）。
///
/// `on_hover(level, row)` 悬停一行；`on_click(level, row)` 点击一行。两者都由根视图转成
/// `cx.notify()` 的实体更新，这里只负责几何和外观。
pub(crate) fn render(
    m: &MainMenu,
    vw: f32,
    vh: f32,
    on_hover: Rc<dyn Fn(usize, usize, &mut Window, &mut App)>,
    on_click: Rc<dyn Fn(usize, usize, &mut Window, &mut App)>,
) -> Vec<AnyElement> {
    let t = tokens();
    let mut out = Vec::with_capacity(m.levels.len());
    for (li, level) in m.levels.iter().enumerate() {
        let w = m.width(li);
        let (x, y0) = m.pos(li);
        let rows_h = level.rows.iter().map(row_height).sum::<f32>();

        let mut col = div().flex().flex_col().w_full();
        // 这一级里**有**可勾选的行，才留 28pt 勾选列；整级都没有勾选态（比如顶层菜单名、
        // 全是子菜单或全是命令的一级）就不留 —— 否则左边白白空出一大条缩进。
        let has_check = level.rows.iter().any(|r| r.checked.is_some());
        for (ri, row) in level.rows.iter().enumerate() {
            if row.separator_before {
                col = col.child(div().my(pt(4.0)).h(pt(1.0)).w_full().bg(rgba(t.border)));
            }
            // 展开行保持高亮，除非指针已经停在别的行等待接管（`MenuAim::keeps_highlight`）。
            let is_open = m.open.get(li) == Some(&Some(ri)) && m.aim.keeps_highlight(li, ri);
            let hover = on_hover.clone();
            let click = on_click.clone();
            let mut r = div()
                .h(pt(ROW_H))
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .rounded(pt(ITEM_RADIUS))
                .text_color(rgba(t.menu_text))
                .text_size(pt(TEXT_PT))
                .on_mouse_move(move |_e, window, cx| hover(li, ri, window, cx))
                .on_mouse_down(MouseButton::Left, move |_e, window, cx| click(li, ri, window, cx));
            r = if is_open { r.bg(rgba(t.menu_hover)) } else { r.hover(|s| s.bg(rgba(t.menu_hover))) };
            // 勾选列：egui `menu_row_draw` 里勾号画在 `r.min.x + 12`（12×12），文字从
            // `r.min.x + 28.0` 起。这一级没有任何可勾选的行时整列不要，但**左边至少留 8pt**
            // 内边距——不能把文字顶到行首（用户 2026-11-05：「不是让你一次性给他打成零」）。
            if has_check {
                let check = if row.checked == Some(true) { "✓" } else { "" };
                r = r.child(div().w(pt(CHECK_W)).flex_none().flex().flex_row().items_center().pl(pt(6.0)).child(check));
            } else {
                r = r.pl(pt(8.0));
            }
            r = r.child(div().flex_1().min_w(px(0.0)).overflow_hidden().whitespace_nowrap().child(row.label.clone()));
            if let Some(trail) = row.trail.clone() {
                r = r.child(div().ml(pt(crate::widgets::MENU_TRAIL_GAP)).pr(pt(10.0)).whitespace_nowrap().text_color(rgba(t.menu_dim)).child(trail));
            }
            col = col.child(r);
        }

        // 与右键菜单 / 工具栏飞出 / 下拉共用同一个浮层控件，主菜单不自己画 chrome。
        out.push(
            crate::widgets::menu(crate::widgets::at_point(x, y0))
                .gap(0.0)
                .width(crate::widgets::Width::Min(w))
                .viewport(vw, vh)
                .content(0.0, rows_h, ROW_H)
                .tag(format!("main-{li}"))
                .show(|| col.into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
        );
    }
    out
}

#[cfg(test)]
mod aim_integration_tests {
    use super::*;
    use crate::widgets::MENU_HOVER_DELAY;

    fn level(prefix: &str) -> Level {
        build_level(prefix, &|_| None)
    }

    fn anchor() -> Bounds<Pixels> {
        Bounds { origin: point(px(40.0), px(40.0)), size: size(px(24.0), px(24.0)) }
    }

    /// 顶层卡片左上角 y = 40 + 24 + 4 = 68；第 i 行中心 y = 68 + 8 + 28i + 14。
    fn row_center(i: usize) -> Point<Pixels> {
        point(px(100.0), px(90.0 + ROW_H * i as f32))
    }

    /// 悬停 File 直到子菜单展开，并把三角顶点种在 File 行上。
    fn open_file(m: &mut MainMenu) {
        assert!(m.hover_at(0, 0, row_center(0), 0.0, level).is_some(), "a hover delay is due");
        assert_eq!(m.open[0], None, "not opened at once");
        m.settle_at(MENU_HOVER_DELAY + 0.01, level);
        assert_eq!(m.open[0], Some(0), "File opens after the delay");
        assert_eq!(m.levels.len(), 2);
    }

    #[test]
    fn slanting_toward_the_submenu_keeps_file_open() {
        let mut m = MainMenu::new(anchor(), top_level());
        open_file(&mut m);
        let panel_x = m.pos(1).0;
        // 从 File 行斜穿 Edit / View / Object 走向子菜单近边。
        let from = row_center(0);
        for (k, row) in [1, 2, 3].into_iter().enumerate() {
            let t = (k as f32 + 1.0) / 4.0;
            let x = 100.0 + (panel_x + 20.0 - 100.0) * t;
            let y = 90.0 + (ROW_H * 3.0) * t;
            let p = point(px(x), px(y));
            m.hover_at(0, row, p, 0.2 + k as f64 * 0.02, level);
            assert_eq!(m.open[0], Some(0), "row {row} took over at {p:?}");
            assert_eq!(m.levels.len(), 2, "the File submenu was rebuilt");
        }
        let _ = from;
    }

    #[test]
    fn straight_down_switches_the_open_submenu() {
        let mut m = MainMenu::new(anchor(), top_level());
        open_file(&mut m);
        // 同一列直下到 Edit：没有朝子菜单收敛，File 立即收起。
        m.hover_at(0, 1, row_center(1), 0.2, level);
        assert_eq!(m.open[0], None, "File closes at once");
        assert_eq!(m.levels.len(), 1);
        m.settle_at(0.2 + MENU_HOVER_DELAY + 0.01, level);
        assert_eq!(m.open[0], Some(1), "Edit opens after the hover delay");
    }

    #[test]
    fn hovering_a_row_twice_does_not_rebuild_the_submenu() {
        let mut m = MainMenu::new(anchor(), top_level());
        open_file(&mut m);
        let before = m.levels[1].rows.len();
        m.hover_at(0, 0, row_center(0), 0.3, level);
        assert_eq!(m.open[0], Some(0));
        assert_eq!(m.levels.len(), 2);
        assert_eq!(m.levels[1].rows.len(), before, "no rebuild / no flicker");
    }
}
