//! 标签条的**纯几何与动画状态**：从 `storytold/figcraft@3635c230` 的
//! `crates/ui-egui/src/tabs.rs`（831 行）逐式搬过来，不依赖 GPUI / egui，因而可以直接单测。
//!
//! 视图层在 `tabs_view.rs`。上游这里还有 `tab_bar` / `paint_tab` / `TabGesture` 等与 egui
//! `Response` 绑定的绘制代码，那些按 GPUI Kit 重写在视图层；本文件只保留可测的纯函数。
//!
//! 2026-11-08（P0-1 第 1 步）：以 `git show upstream/main:crates/ui-egui/src/tabs.rs` 为准，
//! **不照工作区那份 714 行的本地 egui 参考版**（它被 111 个本地提交改写过）。

/// 标签条高度（上游 `HEIGHT`）。
pub const HEIGHT: f32 = 40.0;
/// 一个标签能被压缩到的最窄宽度（上游 `MIN_TAB_WIDTH`）。
pub const MIN_TAB_WIDTH: f32 = 96.0;
/// 溢出按钮宽度（上游 `OVERFLOW_WIDTH`）。
pub const OVERFLOW_WIDTH: f32 = 32.0;
/// Home / 新标签 / 主题 / 文件类型 / live / chevron 图标的边长（上游 `#1729` 图标规范）。
pub const BAR_ICON: f32 = 20.0;
/// 新标签与主题按钮的边长。
pub const BAR_BUTTON: f32 = 32.0;
/// 标签右上角关闭叉的边长。
pub const CLOSE_ICON: f32 = 12.0;
/// 一个标签为 live 标记预留的宽度。
pub const LIVE_MARK_WIDTH: f32 = 26.0;
/// 新标签滑入 + 渐显的时长（秒）。
pub const OPEN_TIME: f64 = 0.18;

/// 上一帧有哪些标签、以及新出现的标签何时开始入场。上游把它放在 egui 的 `data` 里，
/// GPUI 由 `CanvasView` 跨帧持有（`tabs_view::TabState`）。只在标签集合变化时分配。
#[derive(Clone, Default, Debug)]
pub struct TabMotion {
    initialized: bool,
    known: Vec<u64>,
    opening: Vec<(u64, f64)>,
}

impl TabMotion {
    /// 每帧调用：登记新增标签、让过期的动画出列。`reduce_motion` 时立刻完成所有入场。
    pub fn begin_frame(&mut self, current: impl Iterator<Item = u64> + Clone, now: f64, reduce_motion: bool) {
        if !self.known.iter().copied().eq(current.clone()) {
            if self.initialized && !reduce_motion {
                for uid in current.clone().filter(|uid| !self.known.contains(uid)) {
                    self.opening.push((uid, now));
                }
            }
            self.known = current.collect();
        }
        self.initialized = true;
        if reduce_motion {
            self.opening.clear();
        } else {
            let known = &self.known;
            self.opening.retain(|(uid, started)| known.contains(uid) && now - *started < OPEN_TIME);
        }
    }

    /// 本帧还在入场的标签（渲染层据此请求下一帧）。
    pub fn opening(&self) -> &[(u64, f64)] {
        &self.opening
    }
}

/// `uid` 这个标签的入场进度（0..=1）；不在入场中就是 1。
pub fn open_progress(opening: &[(u64, f64)], uid: u64, now: f64) -> f32 {
    opening.iter().find(|(u, _)| *u == uid).map_or(1.0, |(_, started)| ((now - started) / OPEN_TIME).clamp(0.0, 1.0) as f32)
}

/// 一次布局的结果：`first` 是第一个可见标签在 `preferred` 里的下标，`widths` 与之一一对应。
#[derive(Clone, Debug, PartialEq)]
pub struct TabLayout {
    pub first: usize,
    pub widths: Vec<f32>,
    pub overflow: bool,
}

/// **先收缩、再溢出**：把所有标签按 `MIN_TAB_WIDTH` 挤进 `available`，仍放不下才把多余的
/// 挪进溢出菜单，并给 32pt 的 chevron 留位。`active` 标签必须保持可见。
pub fn tab_layout(preferred: &[f32], available: f32, active: Option<usize>) -> TabLayout {
    let count = preferred.len();
    let overflow = count as f32 * MIN_TAB_WIDTH > available;
    let visible = if overflow { ((available - OVERFLOW_WIDTH).max(0.0) / MIN_TAB_WIDTH).floor() as usize } else { count }.min(count);
    let first = active.unwrap_or(0).saturating_sub(visible / 2).min(count.saturating_sub(visible));
    let mut widths = preferred[first..first + visible].to_vec();
    let budget = (available - if overflow { OVERFLOW_WIDTH } else { 0.0 }).max(0.0);
    let mut excess = (widths.iter().sum::<f32>() - budget).max(0.0);
    while excess > 0.01 {
        let flexible = widths.iter().filter(|w| **w > MIN_TAB_WIDTH + 0.01).count();
        if flexible == 0 {
            break;
        }
        let share = excess / flexible as f32;
        let mut removed = 0.0;
        for width in &mut widths {
            let shrink = (*width - MIN_TAB_WIDTH).max(0.0).min(share);
            *width -= shrink;
            removed += shrink;
        }
        excess -= removed;
    }
    TabLayout { first, widths, overflow }
}

/// 标签条上可见标签的水平区间：`(文档 index, (左边界, 右边界))`，窗口坐标。
pub type VisibleTab = (usize, (f32, f32));

/// `from` 这个标签被拖到 `pointer_x` 时的最终文档下标与插入标记线的 x。
/// 索引换算照搬上游（`index - usize::from(from < index)` 等），否则重排会差一位。
pub fn tab_drop_position(visible: &[VisibleTab], from: usize, pointer_x: f32) -> Option<(usize, f32)> {
    let others: Vec<_> = visible.iter().copied().filter(|(index, _)| *index != from).collect();
    let slot = others.partition_point(|(_, (min, max))| (min + max) / 2.0 < pointer_x);
    if let Some(&(index, (min, _))) = others.get(slot) {
        Some((index - usize::from(from < index), min))
    } else {
        let &(index, (_, max)) = others.last()?;
        Some((index + usize::from(from > index), max))
    }
}

/// 演示标签组能占用的宽度：文件标签同时存在时最多一半（上游 `present_room`）。
pub fn present_room(room: f32, preferred: &[f32], files: bool) -> f32 {
    preferred.iter().sum::<f32>().min(if files { room / 2.0 } else { room })
}

/// 演示标签的名字：`file - page`，页面无名时只有 `file`。
pub fn present_tab_name(file: &str, page_name: &str) -> String {
    if page_name.is_empty() { file.to_string() } else { format!("{file} - {page_name}") }
}

/// 一个标签的 live 徽标状态：上游 `tabs.rs::live_mark` / `live_lost` 的纯逻辑。
///
/// 视图层把 `Session::live_of(uid)` 的 `shared()` / `disconnected()` / `doc_uid` 原样传进来，
/// 判定因此不依赖引擎类型、可直接单测。`doc_uid` 必须等于这个文档的 uid —— 一个 live 会话
/// 只属于它同步的那一个文档，别的标签不能跟着亮。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveBadge {
    /// `Some(true)` 别人能进；`Some(false)` 在线文件只有自己能进；`None` 不画徽标。
    pub shared: Option<bool>,
    /// 连接已断：上游 `paint_tab` 先判 lost，再判 shared，所以断连优先。
    pub lost: bool,
}

impl LiveBadge {
    /// 是否需要预留 `LIVE_MARK_WIDTH`（`preferred_width` 的 `live` 参数）。
    pub fn visible(&self) -> bool {
        self.shared.is_some() || self.lost
    }
}

pub fn live_badge(shared: Option<bool>, disconnected: bool, doc_uid: Option<u64>, uid: u64) -> LiveBadge {
    let mine = doc_uid == Some(uid);
    LiveBadge { shared: shared.filter(|_| mine), lost: disconnected && mine }
}

/// 文件标签的理想宽度：名字 + 类型图标/关闭叉的 64pt + live 标记 + 未保存点，夹到 120..=220。
pub fn preferred_width(name: &str, live: bool, dirty: bool) -> f32 {
    let mark_w = if live { LIVE_MARK_WIDTH } else { 0.0 };
    let dot_w = if dirty && mark_w > 0.0 { 12.0 } else { 0.0 };
    (12.0 * 0.6 * name.chars().count() as f32 + 64.0 + mark_w + dot_w).clamp(120.0, 220.0)
}

/// 演示标签的理想宽度（上游 `present_tab_width`，`clamp(140,260)`）。
pub fn present_tab_width(name: &str) -> f32 {
    (12.0 * 0.6 * name.chars().count() as f32 + 64.0).clamp(140.0, 260.0)
}

#[cfg(test)]
mod layout_tests {
    use super::{
        LiveBadge, MIN_TAB_WIDTH, OPEN_TIME, OVERFLOW_WIDTH, TabMotion, live_badge, open_progress, preferred_width, present_room, tab_drop_position,
        tab_layout,
    };

    #[test]
    fn many_presentation_tabs_stay_inside_their_share_of_the_strip() {
        let preferred = vec![260.0; 8];
        let room = 900.0;
        let layout = tab_layout(&preferred, present_room(room, &preferred, true), Some(7));
        assert!(layout.overflow);
        assert!(layout.first <= 7 && 7 < layout.first + layout.widths.len());
        assert!(layout.widths.iter().all(|&width| width >= MIN_TAB_WIDTH));
        assert!(layout.widths.iter().sum::<f32>() + OVERFLOW_WIDTH <= room / 2.0 + 0.01);
        assert_eq!(present_room(room, &preferred, false), room);
        assert_eq!(present_room(room, &[200.0], true), 200.0);
    }

    #[test]
    fn tabs_shrink_to_fit_before_overflowing() {
        let preferred = vec![120.0; 10];
        let layout = tab_layout(&preferred, 1100.0, Some(5));
        assert!(!layout.overflow);
        assert_eq!(layout.first, 0);
        assert_eq!(layout.widths.len(), preferred.len());
        assert!(layout.widths.iter().all(|&width| (width - 110.0).abs() < 0.01));
    }

    #[test]
    fn excess_tabs_leave_room_for_overflow_and_keep_the_active_tab_visible() {
        let preferred = vec![120.0; 24];
        let available = 1100.0;
        let layout = tab_layout(&preferred, available, Some(23));
        assert!(layout.overflow);
        assert!(layout.first <= 23 && 23 < layout.first + layout.widths.len());
        assert!(layout.widths.iter().all(|&width| width >= MIN_TAB_WIDTH));
        assert!(layout.widths.iter().sum::<f32>() + OVERFLOW_WIDTH <= available + 0.01);
    }

    #[test]
    fn drop_position_places_a_tab_between_its_neighbors() {
        let visible: Vec<_> = (0..4).map(|index| (index, (index as f32 * 120.0, (index + 1) as f32 * 120.0))).collect();
        assert_eq!(tab_drop_position(&visible, 0, 330.0).map(|(to, _)| to), Some(2));
        assert_eq!(tab_drop_position(&visible, 3, 30.0).map(|(to, _)| to), Some(0));
        // 插入标记线落在目标标签的左 / 右边界（上游 `rect.min.x` / `rect.max.x`）。
        assert_eq!(tab_drop_position(&visible, 0, 330.0).map(|(_, x)| x), Some(360.0));
    }

    #[test]
    fn new_tabs_animate_unless_motion_is_reduced() {
        let mut motion = TabMotion::default();
        motion.begin_frame([1].into_iter(), 0.0, false);
        assert!(motion.opening().is_empty(), "the tabs open at start-up don't animate");
        motion.begin_frame([1, 2].into_iter(), 1.0, false);
        assert_eq!(open_progress(motion.opening(), 2, 1.0), 0.0);
        assert_eq!(open_progress(motion.opening(), 1, 1.0), 1.0);
        motion.begin_frame([1, 2].into_iter(), 1.0 + 2.0 * OPEN_TIME, false);
        assert!(motion.opening().is_empty(), "the animation ends");
        motion.begin_frame([1, 2, 3].into_iter(), 2.0, true);
        assert!(motion.opening().is_empty(), "Reduce motion opens tabs at once");
    }

    #[test]
    fn a_live_badge_belongs_only_to_the_session_s_document() {
        assert_eq!(live_badge(Some(true), false, Some(7), 7), LiveBadge { shared: Some(true), lost: false });
        assert_eq!(live_badge(Some(false), false, Some(9), 7), LiveBadge::default(), "another doc's session");
        assert_eq!(live_badge(Some(true), true, None, 7), LiveBadge::default(), "no synced doc");
        assert_eq!(live_badge(None, false, Some(7), 7), LiveBadge::default());
        assert!(LiveBadge { shared: None, lost: true }.visible());
        assert!(!LiveBadge::default().visible());
    }

    #[test]
    fn a_lost_connection_wins_over_the_shared_mark() {
        // 上游 `paint_tab`：先画 LiveDisconnected（danger），否则才画 online::mark(shared)。
        let badge = live_badge(Some(true), true, Some(7), 7);
        assert!(badge.lost);
        assert_eq!(badge.shared, Some(true), "the shared state is kept but not drawn");
    }

    #[test]
    fn live_and_unsaved_marks_widen_a_tab_but_stay_clamped() {
        // 名字要够长，让基础宽度越过 120pt 下限，否则 clamp 会吃掉 26pt 的增量。
        let name = "Design file";
        let plain = preferred_width(name, false, false);
        let live = preferred_width(name, true, false);
        let both = preferred_width(name, true, true);
        assert_eq!(live - plain, 26.0, "LIVE_MARK_WIDTH");
        assert_eq!(both - live, 12.0, "unsaved dot only costs width next to a live mark");
        assert_eq!(preferred_width(name, false, true), plain, "a lone dot needs no extra room");
        assert_eq!(preferred_width("A very long document name that will not fit", true, true), 220.0);
        assert_eq!(preferred_width("", false, false), 120.0);
    }

    #[test]
    fn empty_strip_does_not_panic() {
        let layout = tab_layout(&[], 800.0, Some(0));
        assert!(!layout.overflow);
        assert_eq!(layout.first, 0);
        assert!(layout.widths.is_empty());
        assert_eq!(tab_drop_position(&[], 0, 100.0), None);
    }
}
