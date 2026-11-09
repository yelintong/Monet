//! 通用菜单行为：子菜单「安全三角」/ 悬停延迟。
//!
//! 这是一个**行为文件**，不含任何菜单的具体数据或绘制：主菜单（`main_menu.rs`）、
//! 右键菜单（`context_menu.rs`）以及今后任何多级菜单都共用同一份状态机，保证
//! 「斜着划向子菜单不会闪、经过的父行不抢高亮」这套手感在每个菜单里完全一致。
//!
//! 判定以「上一次指针位置 → 这一次位置」这一小步为单位，而不是维护一个持久的
//! 「正在瞄准」状态：
//!
//! - 行悬停时，拿这一小步做判定：只要这一步是朝「已展开子菜单的近边」收敛的，这一行就先等
//!   [`MENU_AIM_REST`]（300ms）再接管；否则立刻接管（子菜单行再等 [`MENU_HOVER_DELAY`]
//!   150ms 才真正展开）。
//! - 指针停在一行上不动 → 到点后这一行接管；指针继续走 → 每次悬停都会把等待重置，
//!   所以「划过去」不会一路闪开。
//! - **没有**每帧 tick、没有持久的 `aiming` 标志、没有「静止超时后交接」的额外状态。
//!   旧方案那套持久 aiming 会在用户只是斜着往下划时把整列行都屏蔽掉，表现为「选择被干扰」，
//!   用户 2026-11-07 明确指出旧方案的手感不对。
//!
//! 控件不读时钟、不碰 `App`：纯逻辑、可单测，任何多级菜单共用。
//!
//! ## ⛔ 2026-11-08 起：安全三角选区已停用（不修，只关）
//! 用户指令原文：「把下拉菜单的三角选区关一下，这个东西有 bug，但是现在不修，后面再说」。
//! 因此 [`MENU_AIM_ENABLED`] 设为 `false`：所有菜单（主菜单 / 右键菜单 / 下拉菜单共用本状态机）
//! 都不再做三角形屏蔽，悬停行为退化为「立刻跟随指针」。
//! **实现与单测全部保留**，测的是 `set_aim_enabled(true)` 的开启态；把常量改回 `true` 即可恢复，
//! 便于后续定位 bug 时直接复用，不需要重写。悬停展开延迟 [`MENU_HOVER_DELAY`] 不属于三角选区，
//! 本次**未改动**。

use gpui::{Bounds, Pixels, Point, point, px};

/// 子菜单「安全三角选区」总开关。**当前停用**：见文件头 2026-11-08 说明。
///
/// 关掉后 `hover()` 跳过第 2 步（三角形屏蔽 / 300ms 静止等待），
/// 上下移动立即切换高亮与子菜单，斜向划过也不再被吞。
pub const MENU_AIM_ENABLED: bool = false;

/// 悬停子菜单行多久才展开（秒）。
pub const MENU_HOVER_DELAY: f64 = 0.15;
/// 朝已展开子菜单移动时，被划过的行要等多久才接管（秒）。
pub const MENU_AIM_REST: f64 = 0.3;

fn xy(p: Point<Pixels>) -> (f32, f32) {
    (f32::from(p.x), f32::from(p.y))
}

fn rect(r: Bounds<Pixels>) -> (f32, f32, f32, f32) {
    (f32::from(r.origin.x), f32::from(r.origin.y), f32::from(r.size.width), f32::from(r.size.height))
}

fn contains(r: Bounds<Pixels>, p: Point<Pixels>) -> bool {
    let (x, y, w, h) = rect(r);
    let (px_, py_) = xy(p);
    px_ >= x && px_ <= x + w && py_ >= y && py_ <= y + h
}

/// `to` 是否落在「以 `from` 为顶点、以 `panel` 靠近 `from` 的那条竖边为底边」的三角里。
///
/// 底边取 panel 的整条近边（不额外放宽），直上直下、往回走都不算。
/// 往回走都不算。`panel` 是**当前已经展开的子菜单**的矩形。
pub fn toward(from: Point<Pixels>, to: Point<Pixels>, panel: Bounds<Pixels>) -> bool {
    let (fx, fy) = xy(from);
    let (px_, py_) = xy(to);
    let (x, y, w, h) = rect(panel);
    if ![fx, fy, px_, py_, x, y, w, h].iter().all(|v| v.is_finite()) {
        return false;
    }
    let near = if x >= fx {
        x
    } else if x + w <= fx {
        x + w
    } else {
        return false;
    };
    let top = point(px(near), px(y));
    let bottom = point(px(near), px(y + h));
    let side = |a: Point<Pixels>, b: Point<Pixels>| {
        let (ax, ay) = xy(a);
        let (bx, by) = xy(b);
        (bx - ax) * (py_ - ay) - (by - ay) * (px_ - ax)
    };
    let sides = [side(from, top), side(top, bottom), side(bottom, from)];
    sides.iter().all(|s| *s >= 0.0) || sides.iter().all(|s| *s <= 0.0)
}

/// 一次 hover / 定时结算后，调用方应对菜单树做的动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AimAction {
    /// 无状态变化：还在等延迟，或这一行就是已展开的那行。
    Hold,
    /// 立即收起该级子菜单（悬停到了没有子菜单的行）。
    Close(usize),
    /// 展开该级第 `row` 行的子菜单。
    Open(usize, usize),
}

#[derive(Clone, Debug, Default)]
struct AimLevel {
    /// 本级展开的子菜单行（`None` = 没展开）。
    open: Option<usize>,
    /// 本级卡片、展开行、子菜单卡片的矩形（每次输入由调用方刷新）。
    card: Option<Bounds<Pixels>>,
    trigger: Option<Bounds<Pixels>>,
    panel: Option<Bounds<Pixels>>,
    /// 上一次被判定时的指针位置。
    pointer: Option<Point<Pixels>>,
    /// 指针当前悬停的行。定时结算时用它确认「等待的那一行
    /// 仍然是指针下这一行」——指针已经移到别的行就不能再替旧行展开。
    over: Option<usize>,
    /// 等待接管的目标行（`None` = 没有等待）。`(目标行, 开始等待时刻, 需要等待的秒数)`。
    pending: Option<(Option<usize>, f64, f64)>,
}

/// 一棵菜单树的瞄准状态，按层级下标索引。菜单树关闭时调 `forget()`。
#[derive(Clone, Debug)]
pub struct MenuAim {
    levels: Vec<AimLevel>,
    /// 三角选区开关。实例级而非全局 static，好让单测能显式打开验证保留下来的实现。
    aim_enabled: bool,
}

impl Default for MenuAim {
    fn default() -> Self {
        Self { levels: Vec::new(), aim_enabled: MENU_AIM_ENABLED }
    }
}

impl MenuAim {
    pub fn new() -> Self {
        Self::default()
    }

    /// 显式开 / 关三角选区。生产路径只用 `MENU_AIM_ENABLED` 的默认值；
    /// 单测用它把保留的实现打开，后续修 bug 时无需重写测试。
    pub fn set_aim_enabled(&mut self, on: bool) {
        self.aim_enabled = on;
    }

    pub fn aim_enabled(&self) -> bool {
        self.aim_enabled
    }

    fn level_mut(&mut self, level: usize) -> &mut AimLevel {
        while self.levels.len() <= level {
            self.levels.push(AimLevel::default());
        }
        &mut self.levels[level]
    }

    /// 菜单树关闭 / 重开：忘掉全部状态。
    pub fn forget(&mut self) {
        self.levels.clear();
    }

    /// 同步某级的展开行与其更深层级。**不清等待**：切到另一行子菜单时，调用方先立刻收起
    /// 旧子菜单（`AimAction::Close`），再靠这里保留的等待在 150ms 后展开新行——这里同样
    /// 「先 truncate 再补新层级」，等待由独立的 pending 任务管。
    pub fn set_open(&mut self, level: usize, row: Option<usize>) {
        self.levels.truncate(level + 1);
        self.level_mut(level).open = row;
    }

    /// 刷新某级的几何。每次输入 / 重绘都要喂，窗口或菜单尺寸变化才跟得上。
    pub fn set_geometry(&mut self, level: usize, card: Bounds<Pixels>, trigger: Option<Bounds<Pixels>>, panel: Option<Bounds<Pixels>>) {
        let l = self.level_mut(level);
        l.card = Some(card);
        l.trigger = trigger;
        l.panel = panel;
    }

    /// 指针悬停某级第 `row` 行（`is_submenu` = 这行有子菜单）。
    ///
    /// 判定顺序：
    /// 1. 就是已展开的那一行 → 什么都不用做；
    /// 2. 从**上一次指针位置**到这一次是朝已展开子菜单收敛的 → 这一行先等 300ms；
    /// 3. 否则立刻接管：没有子菜单的行立即收起子菜单，有子菜单的行等 150ms 展开。
    pub fn hover(&mut self, level: usize, row: usize, is_submenu: bool, at: Point<Pixels>, now: f64) -> AimAction {
        let triangle_enabled = self.aim_enabled;
        let l = self.level_mut(level);
        let from = l.pointer.replace(at);
        l.over = Some(row);
        let target = is_submenu.then_some(row);

        // 1. 已经展开在这一行：保持，清掉任何等待。
        if l.open == Some(row) {
            l.pending = None;
            return AimAction::Hold;
        }

        // 2. 朝已展开的子菜单移动：这一行被安全三角屏蔽，等一会儿再接管。
        //
        // ⛔ 只有「明显横向」的移动才谈得上朝子菜单瞄准。纯上下（或近乎上下）绝不屏蔽：
        // 菜单的上下选择必须立刻跟着指针走，否则三角会把上下选择整个吃掉。
        // 这里用 `|dx| >= |dy|` 把它限制在真正的斜向移动上，避免轻微横向抖动被误判成「朝面板」。
        if triangle_enabled {
            if let (Some(from), Some(panel), Some(_)) = (from, l.panel, l.open) {
                let (fx, fy) = xy(from);
                let (ax, ay) = xy(at);
                if (ax - fx).abs() >= (ay - fy).abs() && toward(from, at, panel) {
                    l.pending = Some((target, now, MENU_AIM_REST));
                    return AimAction::Hold;
                }
            }
        }

        // 3. 不朝子菜单走：立刻接管。
        match target {
            Some(row) => {
                // 子菜单行：先立即 truncate 掉旧子菜单，再等
                // 150ms 展开新行。所以这里「立刻收起旧子菜单」+「保留一个 150ms 后的展开」。
                l.pending = Some((Some(row), now, MENU_HOVER_DELAY));
                if l.open.is_some() {
                    AimAction::Close(level)
                } else {
                    AimAction::Hold
                }
            }
            None => {
                l.pending = None;
                if l.open.is_some() {
                    AimAction::Close(level)
                } else {
                    AimAction::Hold
                }
            }
        }
    }

    /// 定时结算：等待到点的行接管。指针已经离开该级卡片时，等待作废（不产生任何动作）。
    pub fn settle(&mut self, pointer: Point<Pixels>, now: f64) -> Vec<AimAction> {
        let mut out = Vec::new();
        for level in 0..self.levels.len() {
            let l = &mut self.levels[level];
            let Some((target, since, delay)) = l.pending else { continue };
            if let Some(card) = l.card {
                if !contains(card, pointer) {
                    l.pending = None;
                    continue;
                }
            }
            if now - since >= delay {
                l.pending = None;
                match target {
                    Some(r) => {
                        // 指针已经移到别的行：作废，不能替旧行展开。
                        if l.over == Some(r) {
                            out.push(AimAction::Open(level, r));
                        }
                    }
                    None => out.push(AimAction::Close(level)),
                }
            }
        }
        out
    }

    /// 还有多少秒需要一次唤醒（`None` = 不用）；调用方据此安排一次延迟 `cx.notify()`。
    pub fn next_wakeup(&self, now: f64) -> Option<f64> {
        let mut best: Option<f64> = None;
        for l in &self.levels {
            if let Some((_, since, delay)) = l.pending {
                let left = delay - (now - since);
                if left.is_finite() && left > 0.0 {
                    best = Some(best.map_or(left, |b: f64| b.min(left)));
                }
            }
        }
        best
    }

    /// 展开行是否仍应保持高亮：只要它还是本级展开行就保持（等待中的行不抢走高亮）。
    pub fn keeps_highlight(&self, level: usize, row: usize) -> bool {
        self.levels.get(level).is_some_and(|l| l.open == Some(row))
    }
}

#[cfg(test)]
mod aim_tests {
    use super::*;
    use gpui::size;

    fn p(x: f32, y: f32) -> Point<Pixels> {
        point(px(x), px(y))
    }
    fn r(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds { origin: p(x, y), size: size(px(w), px(h)) }
    }

    /// 一个打开的父菜单 + 右侧子菜单：行在 (60,78)-(230,110)，panel 在 (390,70)-(560,246)。
    fn setup() -> (MenuAim, Bounds<Pixels>, Bounds<Pixels>, Bounds<Pixels>) {
        let card = r(60.0, 70.0, 170.0, 176.0);
        let row = r(60.0, 78.0, 170.0, 32.0);
        let panel = r(390.0, 70.0, 170.0, 176.0);
        let mut aim = MenuAim::new();
        // 生产默认已停用三角选区；这里显式打开，继续守住保留下来的实现，
        // 便于后面修 bug 时直接复用这些断言。
        aim.set_aim_enabled(true);
        aim.set_open(0, Some(0));
        aim.set_geometry(0, card, Some(row), Some(panel));
        (aim, card, row, panel)
    }

    fn triangle_geometry(aim: &mut MenuAim) {
        aim.set_open(0, Some(0));
        aim.set_geometry(
            0,
            r(60.0, 70.0, 170.0, 176.0),
            Some(r(60.0, 78.0, 170.0, 32.0)),
            Some(r(390.0, 70.0, 170.0, 176.0)),
        );
    }

    /// 2026-11-08：用户要求「关一下」。默认态下，斜着划过的行必须立刻接管，
    /// 不再被安全三角屏蔽，也不再产生 300ms 静止等待。
    #[test]
    fn triangle_selection_is_off_by_default() {
        assert!(!MENU_AIM_ENABLED, "改动前请读文件头 2026-11-08 的停用说明");
        let mut aim = MenuAim::new();
        assert!(!aim.aim_enabled());
        triangle_geometry(&mut aim);
        assert_eq!(aim.hover(0, 0, true, p(200.0, 94.0), 0.0), AimAction::Hold);
        assert_eq!(
            aim.hover(0, 1, false, p(250.0, 126.0), 0.05),
            AimAction::Close(0),
            "斜向划过不该再被三角吞掉"
        );
        assert!(aim.next_wakeup(0.05).is_none(), "不应再安排 MENU_AIM_REST 唤醒");
    }

    /// 开关是可逆的：后面要修 bug 时把 `MENU_AIM_ENABLED` 打开即可，行为与旧版一致。
    #[test]
    fn enabling_the_flag_restores_the_safe_triangle() {
        let mut aim = MenuAim::new();
        aim.set_aim_enabled(true);
        triangle_geometry(&mut aim);
        assert_eq!(aim.hover(0, 0, true, p(200.0, 94.0), 0.0), AimAction::Hold);
        assert_eq!(aim.hover(0, 1, false, p(250.0, 126.0), 0.05), AimAction::Hold);
        assert!(aim.next_wakeup(0.05).is_some());
    }

    #[test]
    fn toward_only_counts_moves_converging_on_the_panel() {
        let panel = r(300.0, 50.0, 200.0, 600.0);
        let from = p(250.0, 100.0);
        assert!(toward(from, p(260.0, 130.0), panel), "diagonally toward the panel");
        assert!(!toward(from, p(250.0, 130.0), panel), "straight down");
        assert!(!toward(from, p(250.0, 80.0), panel), "straight up");
        assert!(!toward(from, p(240.0, 105.0), panel), "backing away");
        let left = r(0.0, 50.0, 200.0, 600.0);
        assert!(toward(from, p(240.0, 130.0), left), "mirrored panel on the left");
        assert!(!toward(from, p(260.0, 130.0), left));
        assert!(!toward(p(f32::NAN, 0.0), from, panel), "junk positions");
    }

    #[test]
    fn crossing_rows_toward_the_submenu_does_not_take_over_while_moving() {
        let (mut aim, _card, _row, _panel) = setup();
        // 先把指针放在展开行上。
        assert_eq!(aim.hover(0, 0, true, p(200.0, 94.0), 0.0), AimAction::Hold);
        // 斜着划向子菜单：每一步都是朝 panel 收敛，划过的行必须 Hold。
        let mut last = p(200.0, 94.0);
        for (k, pt_) in [p(250.0, 126.0), p(300.0, 158.0), p(360.0, 182.0)].into_iter().enumerate() {
            let now = 0.05 * (k as f64 + 1.0);
            assert_eq!(aim.hover(0, k + 1, false, pt_, now), AimAction::Hold, "crossed row {k} must not take over");
            last = pt_;
        }
        // 指针还在动 → 等待被不断重置，不会提前展开。
        assert!(aim.next_wakeup(0.15).is_some());
        let _ = last;
    }

    #[test]
    fn straight_down_closes_at_once() {
        let (mut aim, _card, _row, _panel) = setup();
        assert_eq!(aim.hover(0, 0, true, p(200.0, 94.0), 0.0), AimAction::Hold);
        // 直上直下：不朝 panel 收敛 → 立即收起。
        assert_eq!(aim.hover(0, 1, false, p(200.0, 130.0), 0.05), AimAction::Close(0));
    }

    #[test]
    fn resting_on_a_crossed_row_lets_it_take_over() {
        let (mut aim, _card, _row, _panel) = setup();
        assert_eq!(aim.hover(0, 0, true, p(200.0, 94.0), 0.0), AimAction::Hold);
        // 斜着走到第 1 行（仍在卡片内），然后停住。
        let crossed = p(225.0, 105.0);
        assert_eq!(aim.hover(0, 1, true, crossed, 0.05), AimAction::Hold);
        // 静止：到 300ms 后由 settle 交给这一行。
        let actions = aim.settle(crossed, 0.05 + MENU_AIM_REST + 0.01);
        assert_eq!(actions, vec![AimAction::Open(0, 1)], "the crossed row takes over once still");
    }

    #[test]
    fn a_submenu_opens_only_after_the_hover_delay() {
        let mut aim = MenuAim::new();
        aim.set_geometry(0, r(60.0, 70.0, 170.0, 176.0), None, None);
        assert_eq!(aim.hover(0, 2, true, p(100.0, 130.0), 0.0), AimAction::Hold, "not opened at once");
        assert!(aim.next_wakeup(0.0).is_some(), "a wake-up is due");
        assert!(aim.settle(p(100.0, 130.0), MENU_HOVER_DELAY - 0.01).is_empty(), "not yet");
        assert_eq!(aim.settle(p(100.0, 130.0), MENU_HOVER_DELAY + 0.01), vec![AimAction::Open(0, 2)]);
    }

    #[test]
    fn sweeping_across_rows_only_opens_where_it_stops() {
        let mut aim = MenuAim::new();
        aim.set_geometry(0, r(60.0, 70.0, 170.0, 176.0), None, None);
        let mut last = p(100.0, 100.0);
        for (k, row) in [1, 2, 3, 4].into_iter().enumerate() {
            let now = k as f64 * 0.02;
            last = p(100.0, 100.0 + 28.0 * row as f32);
            assert_eq!(aim.hover(0, row, true, last, now), AimAction::Hold, "row {row} opened during a sweep");
        }
        assert!(aim.next_wakeup(0.08).is_some());
        assert_eq!(aim.settle(last, 0.08 + MENU_HOVER_DELAY + 0.01), vec![AimAction::Open(0, 4)]);
    }

    #[test]
    fn the_open_row_keeps_its_highlight_while_another_waits() {
        let (mut aim, _card, _row, _panel) = setup();
        assert_eq!(aim.hover(0, 0, true, p(200.0, 94.0), 0.0), AimAction::Hold);
        assert_eq!(aim.hover(0, 1, true, p(250.0, 126.0), 0.05), AimAction::Hold);
        assert!(aim.keeps_highlight(0, 0), "the open row stays lit while the crossed one waits");
        assert!(!aim.keeps_highlight(0, 1));
    }

    #[test]
    fn leaving_the_card_drops_the_pending_row() {
        let (mut aim, _card, _row, _panel) = setup();
        assert_eq!(aim.hover(0, 0, true, p(200.0, 94.0), 0.0), AimAction::Hold);
        assert_eq!(aim.hover(0, 1, true, p(250.0, 126.0), 0.05), AimAction::Hold);
        // 指针跑到菜单外：等待作废，不产生动作，原展开行保持不变。
        assert!(aim.settle(p(900.0, 700.0), 1.0).is_empty());
        assert!(aim.keeps_highlight(0, 0));
        assert!(aim.next_wakeup(1.0).is_none());
    }

    #[test]
    fn forget_clears_the_tree() {
        let (mut aim, _card, _row, _panel) = setup();
        assert_eq!(aim.hover(0, 0, true, p(200.0, 94.0), 0.0), AimAction::Hold);
        aim.forget();
        assert!(aim.next_wakeup(0.0).is_none());
        assert!(aim.levels.is_empty());
    }
}
