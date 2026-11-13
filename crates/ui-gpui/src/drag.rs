//! 通用「列表 / 树」拖拽件 —— 三套 egui 行为的唯一实现，供任何面板复用。
//!
//! 提炼来源（全部来自 `crates/ui-egui`，行为按原样对齐，不是重新设计）：
//!
//! - `panels/list_drag.rs`：`drop_slot`（按「有多少行的中心在指针之上」定边界）、`drop_zone`
//!   （中间 0.25..0.75 为 Into，其余上下半）、`hold_to_expand`（悬停折叠行 0.6s 展开）、
//!   `finish_leveled`（每帧只解析落点来画线，**松手才应用**，一次 undo step）、
//!   `insertion_line` / `drag_ghost`（2pt 强调色插入线、跟随指针的小标签盒）、Escape 取消、
//!   落在自身或子树里不接。
//! - `panels/layers.rs`：Above / Into / Below 三区；Into 画整行圆角描边，其余画插入线；
//!   被拖的是选中集里的一行时整组一起移动；禁止落点变 NotAllowed 光标；一次 `node.reparent`。
//! - `dialogs/variables_panel.rs` 的 groups：`group_drop_slot` 用**指针 x 决定层级**
//!   （`floor((x-base)/INDENT)+1`，往左拖层级变浅、插入线变宽），浅层级落在整棵子树**之后**，
//!   `group_insert_index` / `drop_is_noop_or_cycle`（浅层级移出是合法操作，同级或更深才是
//!   no-op / 环），`apply_group_drop` 的 parent 由层级推出。
//!
//! 整合规则：指针落在某行的中间带且该行能容纳子节点 → `Zone::Into`（层级 = 该行 +1）；
//! 否则按指针 x 选层级，浅层级 = 移到该行祖先层、落在整棵子树之后，同层级 = 兄弟前后。
//! 因此「左右移动鼠标改变层级」和「上下移动改变位置 / 拖进容器」在同一套解析里共存。
//!
//! 本模块分两层，互不耦合：
//! - `Row` / `Config` / `resolve` / `insert_index` / `parent_path_at` / `is_noop_or_cycle` 是
//!   **纯几何与纯数据**（只借用 gpui 的 `Bounds` / `Point` 数值类型，不碰引擎、不碰节点类型）；
//! - `Drag` 是拖拽状态机，`overlay` 是 GPUI 视觉件，`calibrate` 解决 GPUI 拿不到全部行
//!   布局矩形的问题（一次命中行的真实矩形 + 固定行高即可重建整列）。
//!
//! 调用方只做两件事：把已布局的行喂进来，把 `Target` 翻译成自己的命令。

use gpui::prelude::*;
use gpui::{AnyElement, Bounds, Pixels, Point, Size, div, px};

use crate::widgets::{pt, rgba, tokens};

/// 一行的通用标识。树用 `NodeId(pub u64)` 的 `.0`，平面列表用模型索引。
pub type Id = u64;

/// 每层级横向缩进（egui groups 的 24pt；layers 传自己的缩进值）。
pub const INDENT: f32 = 24.0;
/// Into 的中间带（egui `drop_zone`）。
pub const INTO_BAND: (f32, f32) = (0.25, 0.75);
/// 悬停折叠行多久自动展开（egui layers 的 0.6s）。
pub const HOLD_SECS: f64 = 0.6;

/// 一行已布局的信息。`rect` 是**窗口坐标**。
#[derive(Clone, Debug)]
pub struct Row {
    pub id: Id,
    /// `/` 分隔的祖先 id 链（含自身），如 `"3/12/45"`。段数即层级（1 起）。
    /// 树直接拼祖先 id；平面列表用单段即可。用 id 而不是名字，重名也不会串。
    pub path: String,
    /// 能否容纳子节点（落进中间带时有效）。
    pub can_into: bool,
    /// 有子节点但折叠着（拖到中间带停留一会儿会请求展开）。
    pub collapsed: bool,
    /// 窗口坐标矩形。GPUI 没有「元素 id → 布局矩形」查询，用 [`calibrate`] 由任一命中行推算。
    pub rect: Bounds<Pixels>,
}

impl Row {
    /// 该行的层级（1 起）。
    pub fn level(&self) -> usize {
        self.path.split('/').count()
    }
    /// 视觉深度（0 起）。
    pub fn depth(&self) -> usize {
        self.level().saturating_sub(1)
    }
}

/// 路径的层级（1 起），与 [`Row::level`] 一致。
pub fn level_of(path: &str) -> usize {
    path.split('/').count()
}

/// 层级 1 的左边界（窗口 x）与每层缩进。
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub base: f32,
    pub indent: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self { base: 0.0, indent: INDENT }
    }
}

/// 指针落在目标行的哪个区。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    /// 列表里在它上面（z 序在它之后）。
    Above,
    /// 落进它内部，成为第一个子节点。
    Into,
    /// 列表里在它下面（z 序在它之前）。
    Below,
}

/// 照搬 `list_drag.rs::drop_zone`：能容纳子节点时中间带是 Into，否则按上下半分。
pub fn drop_zone(rel: f32, can_into: bool) -> Zone {
    if can_into && rel > INTO_BAND.0 && rel < INTO_BAND.1 {
        Zone::Into
    } else if rel < 0.5 {
        Zone::Above
    } else {
        Zone::Below
    }
}

/// 要画的落点视觉：插入线，或 Into 时整行描边。
#[derive(Clone, Debug)]
pub struct Line {
    /// 插入线左端：随层级收窄 / 变宽（egui groups 的 `x0 = base + (level-1)*INDENT`）。
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
    /// Into 时不画线，改为整行圆角描边（egui layers）。
    pub into_rect: Option<Bounds<Pixels>>,
}

/// 一次解析出的落点。`resolve` 每帧都算（只为画线），只有松手时调用方才应用。
#[derive(Clone, Debug)]
pub struct Target {
    /// 指针实际落在的那一行（egui layers 的 `r.contains(p)` 行）。
    /// `zone` / `can_into` / `level` / 拒绝判定全部以它为准，不能用「中心在指针之上的行数」
    /// 推出的下一行 —— 那会让 `rel` 恒为负，`Below` 永不触发。
    pub anchor: usize,
    pub anchor_id: Id,
    pub zone: Zone,
    /// 指针 x 选出的层级（1 起）；Into 时为 anchor 层级 + 1。
    pub level: usize,
    /// 落在「level 层祖先」整棵子树之后（列表更下）还是之前（列表更上）。
    pub after: bool,
    /// **实际插入缝**：新节点会占据列表中的这个位置（0 = 第一行之上，len = 最后一行之下）。
    /// 插入线的 y 与提交的 `node.reparent` 都由它推导，二者不可能再错位。
    pub gap: usize,
    pub line: Line,
    /// 落进自身 / 自身子树且层级不比原来浅 → no-op 或环，不接（画禁止光标，提交时忽略）。
    pub refused: bool,
    /// 落在原位，提交也是空操作。
    pub noop: bool,
}

/// 解析 `rows[from]` 这一行拖到 `pointer` 时的落点。
///
/// 返回 `None` 只在指针离开列表上下各半行的范围时；在列表内始终给出落点（哪怕就在被拖行
/// 旁边，此时 `noop` 为真），这样插入线不会闪。
pub fn resolve(rows: &[Row], from: usize, pointer: Point<Pixels>, cfg: Config) -> Option<Target> {
    if rows.is_empty() || from >= rows.len() {
        return None;
    }
    let first = rows.first()?.rect;
    let last = rows.last()?.rect;
    let h = f32::from(first.size.height).max(1.0);
    let slack = h / 2.0;
    let min_x = f32::from(first.origin.x);
    let max_x = f32::from(last.origin.x) + f32::from(last.size.width);
    let top = f32::from(first.origin.y);
    let bottom = f32::from(last.origin.y) + f32::from(last.size.height);
    let px = f32::from(pointer.x);
    let py = f32::from(pointer.y);
    // `drop_slot` 的 area：行区上下各放宽半行，左右不放宽。
    if px < min_x || px > max_x || py < top - slack || py > bottom + slack {
        return None;
    }

    // ── 指针实际所在的那一行的索引（egui layers `r.contains(p)`）──────────────
    // 先用真实矩形找，找不到（行间 / 上下 slack）再按 y 钳到最近行。绝不能用「中心在指针
    // 之上的行数」当 anchor：那是**下一行**，拿它算 rel 会恒为负，Below 永不触发、Into 偏半行。
    let hit = rows
        .iter()
        .position(|r| {
            let y0 = f32::from(r.rect.origin.y);
            py >= y0 && py < y0 + f32::from(r.rect.size.height)
        })
        .unwrap_or(if py < top { 0 } else { rows.len() - 1 });
    let at_bottom = py >= bottom;
    let ar = &rows[hit];
    let anchor_level = ar.level();
    let old = &rows[from];
    let old_level = old.level();

    // ── layers 的三区：rel 相对「指针所在行」，而不是下一行 ─────────────────────
    let row_top = f32::from(ar.rect.origin.y);
    let row_h = f32::from(ar.rect.size.height).max(1.0);
    let rel = if at_bottom { 1.0 } else { ((py - row_top) / row_h).clamp(0.0, 1.0) };
    let zone = if at_bottom { Zone::Below } else { drop_zone(rel, ar.can_into) };

    // ── groups 的「指针 x 决定层级」 ──────────────────────────────────────────
    // 往左拖出更深层行的缩进 → 层级变浅 → 插入线变宽（提到父级）。层级不深于 anchor。
    let mut level = ((((px - cfg.base) / cfg.indent.max(1.0)).floor() as i32) + 1).clamp(1, anchor_level as i32) as usize;
    if zone == Zone::Into {
        // 拖进容器 = 成为它的第一个子节点，层级比它深一层。
        level = anchor_level + 1;
    }

    // ── 唯一落点：level 层祖先的整棵子树，之前或之后 ──────────────────────────
    // 同层级时这棵「子树」就是 hit 自己；浅层级时是包含 hit 的那棵祖先子树。
    // `after` 与线缝严格一一对应：之后 = 子树末行之下，之前 = 子树首行之上。
    // 提交的 `node.reparent` 用同一组 (level, after) 反推 parent/boundary，所以线画在哪
    // 条缝，松手后节点就出现在哪条缝（用户 2026-11-06：实际插入位置要和插入线一致）。
    let ancestor_path = ar.path.split('/').take(level).collect::<Vec<_>>().join("/");
    let prefix = format!("{ancestor_path}/");
    let subtree_start = rows.iter().position(|r| r.path == ancestor_path).unwrap_or(hit);
    let subtree_end = rows.iter().rposition(|r| r.path == ancestor_path || r.path.starts_with(&prefix)).unwrap_or(hit);
    // 浅层级一律落在整棵子树之后（egui groups：绝不插在父与子之间）；同层级按三区走。
    let after = level < anchor_level || zone == Zone::Below;
    let gap = if after { subtree_end + 1 } else { subtree_start };

    // ── 插入线几何：y 直接由 gap 取，不再另算一套 ─────────────────────────────
    let x0 = cfg.base + (level.saturating_sub(1) as f32) * cfg.indent;
    let y = if gap >= rows.len() { bottom } else { f32::from(rows[gap].rect.origin.y) };
    let line = Line { x0, x1: max_x.max(x0 + 1.0), y, into_rect: if zone == Zone::Into { Some(ar.rect) } else { None } };

    // ── 防环 / no-op（`drop_is_noop_or_cycle`） ──────────────────────────────
    // 落在自己或自己子树里，且层级不比原来浅 → 空操作或成环；**浅层级是合法移出**。
    let inside = ar.path == old.path || ar.path.starts_with(&format!("{}/", old.path));
    let refused = inside && (level >= old_level || zone == Zone::Into);
    // `noop` 与 `refused` 独立：落在自己身上时两者都为真（既是空操作、也在自身子树里），
    // 调用方任一为真就跳过提交。分开记便于显示「禁止光标」和「不画线」的不同处理。
    let noop = zone != Zone::Into && level == old_level && (gap == from || gap == from + 1);

    Some(Target { anchor: hit, anchor_id: ar.id, zone, level, after, gap, line, refused, noop })
}

/// `group_drop_slot` 的 no-op / 环判定：同级或更深落在自己子树里才算，浅层级是移出。
pub fn is_noop_or_cycle(old_path: &str, anchor_path: &str, level: usize) -> bool {
    let old_level = level_of(old_path);
    let inside = anchor_path == old_path || anchor_path.starts_with(&format!("{old_path}/"));
    inside && level >= old_level
}

/// `group_insert_index`：`after` 时落在 `sib` 及其所有后代**之后**，否则就在 `sib` 之前。
pub fn insert_index(rows: &[Row], sib_path: &str, after: bool) -> usize {
    if after {
        let prefix = format!("{sib_path}/");
        rows.iter().enumerate().filter(|(_, r)| r.path == sib_path || r.path.starts_with(&prefix)).map(|(k, _)| k).max().map_or(rows.len(), |k| k + 1)
    } else {
        rows.iter().position(|r| r.path == sib_path).unwrap_or(rows.len())
    }
}

/// `apply_group_drop` 的父路径：取 anchor 路径的前 `level-1` 段；层级 1 → 顶层。
pub fn parent_path_at(anchor_path: &str, level: usize) -> String {
    if level <= 1 { String::new() } else { anchor_path.split('/').take(level - 1).collect::<Vec<_>>().join("/") }
}

/// GPUI 0.2.1 没有公开的「元素 id → 布局矩形」查询；`on_drag_move` 只给出**命中那一行**的
/// 矩形。由于列表行高固定，一次命中即可推算整列：`y_i = hit_y + (i - hit)*row_h`。
/// 于是根层的全局 `on_mouse_move` 在指针移到行间 / 列表外时也能继续解析落点（egui 的行为），
/// 不再像旧实现那样「只在命中行里才更新」。
pub fn calibrate(rows: &mut [Row], hit: usize, bounds: Bounds<Pixels>) {
    if hit >= rows.len() {
        return;
    }
    let h = f32::from(bounds.size.height).max(1.0);
    let w = f32::from(bounds.size.width);
    let x = f32::from(bounds.origin.x);
    let y0 = f32::from(bounds.origin.y);
    for (i, r) in rows.iter_mut().enumerate() {
        let y = y0 + (i as f32 - hit as f32) * h;
        r.rect = Bounds { origin: Point::new(px(x), px(y)), size: Size::new(px(w), px(h)) };
    }
}

/// 拖拽状态机。调用方每帧 `update`，松手时 `take`，Escape 时 `cancel`。
#[derive(Clone, Debug)]
pub struct Drag {
    pub from: usize,
    pub from_id: Id,
    /// 跟随光标的标签（egui：单行是名字，多选是「N layers」）。
    pub label: String,
    pub target: Option<Target>,
    /// 悬停展开计时（egui `hold_to_expand`）。
    hold: Option<(Id, f64)>,
}

impl Drag {
    pub fn start(from: usize, from_id: Id, label: impl Into<String>) -> Self {
        Self { from, from_id, label: label.into(), target: None, hold: None }
    }

    /// 每帧用全局指针重算落点；返回「停留够久、请求展开」的那一行（只在完成那一帧返回一次）。
    pub fn update(&mut self, rows: &[Row], pointer: Point<Pixels>, cfg: Config, now: f64) -> Option<Id> {
        self.target = resolve(rows, self.from, pointer, cfg);
        let want = self
            .target
            .as_ref()
            .filter(|t| t.zone == Zone::Into && !t.refused && !t.noop)
            .and_then(|t| rows.get(t.anchor))
            .filter(|r| r.collapsed)
            .map(|r| r.id);
        match (want, self.hold) {
            (Some(id), Some((k, since))) if k == id && now - since > HOLD_SECS => {
                self.hold = None;
                Some(id)
            }
            (Some(id), Some((k, since))) if k == id => {
                let _ = since;
                None
            }
            (Some(id), _) => {
                self.hold = Some((id, now));
                None
            }
            _ => {
                self.hold = None;
                None
            }
        }
    }

    /// 松手：取出落点并清掉悬停计时。调用方负责判断 `refused` / `noop`。
    pub fn take(&mut self) -> Option<Target> {
        self.hold = None;
        self.target.take()
    }

    pub fn cancel(&mut self) {
        self.target = None;
        self.hold = None;
    }

    /// 落点被拒绝（自身 / 子树）→ 调用方画禁止光标。
    pub fn refused(&self) -> bool {
        self.target.as_ref().is_some_and(|t| t.refused)
    }
}

/// GPUI 视觉件：插入线（2pt 强调色）、Into 整行圆角描边、跟随指针的拖拽标签。
/// 全部用窗口坐标绝对定位，挂在根 `relative` 容器的最后，覆盖所有面板。
pub fn overlay(drag: &Drag, pointer: Point<Pixels>) -> Vec<AnyElement> {
    let t = tokens();
    let mut out: Vec<AnyElement> = Vec::new();
    if let Some(d) = &drag.target
        && !d.refused
        && !d.noop
    {
        if let Some(b) = d.line.into_rect {
            out.push(
                div()
                    .absolute()
                    .left(pt(f32::from(b.origin.x)))
                    .top(pt(f32::from(b.origin.y)))
                    .w(pt(f32::from(b.size.width)))
                    .h(pt(f32::from(b.size.height)))
                    .rounded(pt(5.0))
                    .border_1()
                    .border_color(rgba(t.accent))
                    .into_any_element(),
            );
        } else {
            out.push(
                div()
                    .absolute()
                    .left(pt(d.line.x0))
                    .top(pt(d.line.y - 1.0))
                    .w(pt(d.line.x1 - d.line.x0))
                    .h(pt(2.0))
                    .bg(rgba(t.accent))
                    .into_any_element(),
            );
        }
    }
    // `drag_ghost`：at + (14,10) 的小圆角盒子，面板底 + 强调色描边 + 名字。
    out.push(
        div()
            .absolute()
            .left(pt(f32::from(pointer.x) + 14.0))
            .top(pt(f32::from(pointer.y) + 10.0))
            .px(pt(8.0))
            .py(pt(4.0))
            .bg(rgba(t.panel))
            .border_1()
            .border_color(rgba(t.accent))
            .rounded(pt(5.0))
            .child(div().child(drag.label.clone()).text_size(pt(11.0)).text_color(rgba(t.text)))
            .into_any_element(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds { origin: Point::new(px(x), px(y)), size: Size::new(px(w), px(h)) }
    }
    fn pt(x: f32, y: f32) -> Point<Pixels> {
        Point::new(px(x), px(y))
    }
    fn row(id: Id, path: &str, y: f32, can_into: bool) -> Row {
        Row { id, path: path.to_string(), can_into, collapsed: false, rect: bounds(0.0, y, 240.0, 32.0) }
    }
    /// 一棵三层树：a (L1) / a/b (L2) / a/b/c (L3)，后面再跟一个顶层 space。
    fn rows() -> Vec<Row> {
        vec![row(1, "1", 0.0, true), row(2, "1/2", 32.0, true), row(3, "1/2/3", 64.0, true), row(4, "4", 96.0, false)]
    }
    fn cfg() -> Config {
        Config { base: 0.0, indent: 24.0 }
    }

    #[test]
    fn middle_band_is_into_and_edges_are_above_below() {
        assert_eq!(drop_zone(0.1, true), Zone::Above);
        assert_eq!(drop_zone(0.3, true), Zone::Into);
        assert_eq!(drop_zone(0.5, true), Zone::Into);
        assert_eq!(drop_zone(0.7, true), Zone::Into);
        assert_eq!(drop_zone(0.9, true), Zone::Below);
        assert_eq!(drop_zone(0.3, false), Zone::Above);
        assert_eq!(drop_zone(0.7, false), Zone::Below);
        assert_eq!(drop_zone(0.25, true), Zone::Above);
        assert_eq!(drop_zone(0.75, true), Zone::Below);
    }

    #[test]
    fn pointer_x_selects_the_level_and_widens_the_line() {
        let r = rows();
        // 指针在第三行 (L3) 的上半，x=60 落在 level 3 的列。
        let deep = resolve(&r, 3, pt(60.0, 66.0), cfg()).unwrap();
        assert_eq!(deep.level, 3);
        let x_deep = deep.line.x0;
        // 往左拖一列 → level 2；再往左 → level 1，插入线一次比一次宽（起点更左）。
        let up1 = resolve(&r, 3, pt(30.0, 66.0), cfg()).unwrap();
        let up2 = resolve(&r, 3, pt(5.0, 66.0), cfg()).unwrap();
        assert_eq!(up1.level, 2);
        assert_eq!(up2.level, 1);
        assert!(up1.line.x0 < x_deep);
        assert!(up2.line.x0 < up1.line.x0);
        assert_eq!(up2.line.x0, 0.0);
        // 层级不深于 anchor；顶层行上无论 x 多远都是 level 1。
        let top = resolve(&r, 3, pt(200.0, 98.0), cfg()).unwrap();
        assert_eq!(top.level, 1);
    }

    #[test]
    fn shallow_level_lands_after_the_whole_subtree() {
        let r = rows();
        // 在 L3 行的边界上方往左拉到 level 1：必须落在整棵子树之后。
        let d = resolve(&r, 3, pt(5.0, 65.0), cfg()).unwrap();
        assert_eq!(d.level, 1);
        assert!(d.after);
        // `group_insert_index` 的同一语义：after("1") 跳过 1、1/2、1/2/3 三行。
        assert_eq!(insert_index(&r, "1", true), 3);
        assert_eq!(insert_index(&r, "1", false), 0);
        assert_eq!(insert_index(&r, "1/2", false), 1);
    }

    #[test]
    fn into_goes_one_level_deeper_and_draws_the_row_outline() {
        let r = rows();
        // 第二行 (L2) 的中间带 → Into，层级 = 3，画整行描边而不是插入线。
        let d = resolve(&r, 3, pt(24.0, 48.0), cfg()).unwrap();
        assert_eq!(d.zone, Zone::Into);
        assert_eq!(d.level, 3);
        assert!(d.line.into_rect.is_some());
        // 叶片行（can_into=false）中间带不会 Into。
        let d2 = resolve(&r, 3, pt(24.0, 112.0), cfg()).unwrap();
        assert_ne!(d2.zone, Zone::Into);
        assert!(d2.line.into_rect.is_none());
    }

    #[test]
    fn drop_into_own_subtree_is_refused_but_a_shallower_level_is_a_real_move_out() {
        // 拖 "1/2"（L2）到它自己的子树里的更深层级 → 拒绝。
        assert!(is_noop_or_cycle("1/2", "1/2", 2));
        assert!(is_noop_or_cycle("1/2", "1/2/3", 3));
        // 提到 level 1 是合法移出。
        assert!(!is_noop_or_cycle("1/2", "1/2", 1));
        assert!(!is_noop_or_cycle("1/2", "1/2/3", 1));
        // 解析层同样：往左拉到父级不算拒绝。
        let r = rows();
        let out = resolve(&r, 1, pt(5.0, 66.0), cfg()).unwrap();
        assert_eq!(out.level, 1);
        assert!(!out.refused, "shallow level is a move out, not a cycle");
    }

    #[test]
    fn parent_path_follows_the_level() {
        assert_eq!(parent_path_at("1/2/3", 1), "");
        assert_eq!(parent_path_at("1/2/3", 2), "1");
        assert_eq!(parent_path_at("1/2/3", 3), "1/2");
        assert_eq!(level_of("1/2/3"), 3);
    }

    #[test]
    fn noop_when_dropped_right_next_to_itself() {
        let r = rows();
        // 第 4 行原地附近：gap == from 或 from+1 → noop。
        assert!(resolve(&r, 3, pt(10.0, 100.0), cfg()).unwrap().noop);
        assert!(resolve(&r, 3, pt(10.0, 124.0), cfg()).unwrap().noop);
        assert!(!resolve(&r, 3, pt(10.0, 4.0), cfg()).unwrap().noop);
    }

    #[test]
    fn outside_the_list_resolves_to_none() {
        let r = rows();
        assert!(resolve(&r, 3, pt(10.0, 400.0), cfg()).is_none());
        assert!(resolve(&r, 3, pt(400.0, 50.0), cfg()).is_none());
        assert!(resolve(&[], 0, pt(10.0, 10.0), cfg()).is_none());
    }

    #[test]
    fn one_hit_rectangle_rebuilds_the_whole_column() {
        let mut r = rows();
        // 先破坏所有行的矩形，模拟 GPUI 只给出了命中行的真实 bounds。
        for x in r.iter_mut() {
            x.rect = bounds(0.0, 0.0, 10.0, 10.0);
        }
        calibrate(&mut r, 1, bounds(12.0, 500.0, 260.0, 30.0));
        assert_eq!(f32::from(r[0].rect.origin.y), 470.0);
        assert_eq!(f32::from(r[1].rect.origin.y), 500.0);
        assert_eq!(f32::from(r[2].rect.origin.y), 530.0);
        assert_eq!(f32::from(r[3].rect.origin.y), 560.0);
        assert_eq!(f32::from(r[2].rect.origin.x), 12.0);
        assert_eq!(f32::from(r[2].rect.size.width), 260.0);
    }

    /// 回归（用户 2026-11-06「实际插入位置和插入线位置不匹配」）：
    /// 线缝 `gap`、线的 y、提交的 `after` 必须是同一个落点。
    /// 修复前 anchor 取的是「指针下方那一行」，rel 恒为负 → Below 永不触发、Into 偏半行，
    /// 浅层级提交落到整棵子树之外而线仍画在指针边界。
    #[test]
    fn line_matches_the_committed_seam() {
        let r = rows();
        let seam_y = |gap: usize| {
            if gap >= r.len() {
                f32::from(r.last().unwrap().rect.origin.y) + f32::from(r.last().unwrap().rect.size.height)
            } else {
                f32::from(r[gap].rect.origin.y)
            }
        };
        for (x, y) in [(60.0, 66.0), (30.0, 66.0), (5.0, 66.0), (24.0, 48.0), (10.0, 112.0), (10.0, 100.0), (200.0, 98.0), (5.0, 24.0)] {
            let d = resolve(&r, 3, pt(x, y), cfg()).unwrap();
            assert_eq!(d.line.y, seam_y(d.gap), "线 y 必须等于 gap 那道缝 (x={x}, y={y})");
            if !d.after {
                // 之前：落在子树首行之上。
                assert!(d.gap <= d.anchor);
            } else {
                assert!(d.gap > d.anchor);
            }
        }

        // 指针在容器 "1" 的下半（0.75 带）：修复前 anchor 错取子行 "1/2"、line.y=32（A/B
        // 之间），提交却因浅层级落到整棵子树之外 —— 线在中间、节点跳到列表底部。
        let d = resolve(&r, 3, pt(5.0, 24.0), cfg()).unwrap();
        assert_eq!(d.anchor, 0, "anchor 必须是指针所在的容器行");
        assert_eq!(d.zone, Zone::Below);
        assert_eq!(d.gap, 3, "level 1 → 落在 \"1\" 整棵子树之后（第 4 行之上）");
        assert_eq!(d.line.y, 96.0, "线必须跟着落到子树之后，而不是留在 32");
        assert!(d.after);

        // 同层级下半：Below 现在能真正触发（修复前 rel 恒负 → 永远 Above）。
        let d = resolve(&r, 0, pt(10.0, 112.0), cfg()).unwrap();
        assert_eq!(d.zone, Zone::Below);
        assert_eq!(d.gap, 4);
        assert_eq!(d.line.y, 128.0);

        // Into 的中间带下半：修复前被推到下一行、判成 Above。
        let d = resolve(&r, 3, pt(24.0, 52.0), cfg()).unwrap();
        assert_eq!(d.zone, Zone::Into);
        assert_eq!(d.anchor, 1);
    }

    #[test]
    fn hold_over_a_collapsed_row_asks_to_expand_once() {
        let mut r = rows();
        r[1].collapsed = true;
        let mut d = Drag::start(3, 4, "space");
        // 指针落在第二行中间带（Into）。
        assert_eq!(d.update(&r, pt(24.0, 48.0), cfg(), 0.0), None);
        assert_eq!(d.update(&r, pt(24.0, 48.0), cfg(), 0.3), None);
        assert_eq!(d.update(&r, pt(24.0, 48.0), cfg(), 0.7), Some(2));
        // 展开后不再请求。
        r[1].collapsed = false;
        assert_eq!(d.update(&r, pt(24.0, 48.0), cfg(), 1.0), None);
    }
}
