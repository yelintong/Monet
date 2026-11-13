//! 画布格子：按页面坐标对齐的稳定网格、脏格判定与合成策略。
//!
//! 规则抄自 `longbridge/gpui-fast` 的 `crates/gpui/src/fast/layers/tiles.rs`、
//! `policy.rs` 与 `docs/scroll-layers.md`，只用 `std` 数据结构，不依赖 GPUI
//! 内部类型，也不引入新依赖。
//!
//! 本模块**尚未接入渲染路径**：这一批只把算法与参数搬进来并锁进单测，
//! 画布输出逐像素不变。接线留待后续独立 PR（见 `docs/canvas-pipeline-plan.md`
//! §3.6）。抄来的三条要点：
//! 1. 格子按页面坐标对齐，平移只新增行列，旧格整块复用；
//! 2. 每格哈希「会画到这格的图元」（含阴影扩散后的可见边界），按绘制顺序、
//!    平移到格内坐标；**遮罩先裁到格再哈希**，否则一次平移会把所有格弄脏；
//! 3. 路径类图元不进格子（屏幕空间导数抗锯齿，奇数平移会差一级），
//!    留作覆盖层每帧按绘制顺序画。

#![allow(dead_code)]

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// 格子边长（设备像素）。与上游 `fast::layers::paint::TILE_SIZE` 同量级。
pub const TILE_SIDE: u32 = 512;
/// 一次视图最多允许多少个格子，防止异常视口撑爆内存。
pub const MAX_TILES: usize = 512;
/// 向外多画的屏数（合成缓冲），抄 `fast::layers::paint::OVERSCAN_VIEWPORTS`。
pub const OVERSCAN_VIEWPORTS: u32 = 2;
/// 向外多画的像素上限，抄上游 `render_tile` 的 bleed 上限。
pub const MAX_BLEED: u32 = 256;
/// 最近 16 帧里内容变化达到这个数就退回普通绘制。
const BYPASS_CHANGED_FRAMES: u8 = 8;
/// 整页像素达到这个数才启用格子。
const TILED_MIN_PIXELS: u64 = 4_000_000;
/// 高倍缩放时降低门槛。
const HIGH_ZOOM_TILED_MIN_PIXELS: u64 = 1_000_000;

/// 页面坐标矩形 `[x0, y0, x1, y1]`。
pub type PageRect = [f64; 4];

/// 稳定网格坐标：页面坐标除以格子边长，与屏幕位置无关。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileCoord {
    pub x: i32,
    pub y: i32,
}

impl TileCoord {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// 一个渲染单元的唯一身份键。格子、几何缓存、特效步骤、批处理共用它，
/// 避免各自发明「什么算变了」而互相打架。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnitKey {
    pub tile: TileCoord,
    pub zoom_bucket: i32,
    /// 文档**身份**（`Arc` 指针），不是内容版本：同一个手势里原地改文档时它不变。
    /// 当前渲染路径不消费单元，所以不会漏刷新；接线到真实渲染前必须换成
    /// `Document` 暴露的内容 revision，否则格子会一直判「没变」。
    pub doc_identity: u64,
    /// 当前页面：同一文档的不同页可以有完全相同的视口与缩放档位。
    pub page: u64,
    pub params: u64,
    pub size: (u32, u32),
    pub fonts: u64,
    /// 格子边长进身份：边长变了，同一个 (x, y) 代表的是不同的页面范围。
    pub side: u32,
}

/// 缩放档位按 2 的幂划分。跨档时几何要重新展平，必须渐进迁移，别在手势里卡住。
pub fn zoom_bucket(scale: f64) -> i32 {
    if !scale.is_finite() || scale <= 0.0 {
        return 0;
    }
    scale.log2().floor() as i32
}

fn floor_coord(v: f64) -> Option<i32> {
    if !v.is_finite() {
        return None;
    }
    let f = v.floor();
    if f < f64::from(i32::MIN) || f > f64::from(i32::MAX) {
        return None;
    }
    Some(f as i32)
}

/// 矩形覆盖到的格子，按行返回。坐标只由页面矩形决定，所以平移后旧格坐标不变。
pub fn tiles_over(rect: PageRect, side: u32) -> Vec<TileCoord> {
    if side == 0 {
        return Vec::new();
    }
    let s = f64::from(side);
    if !rect.iter().all(|v| v.is_finite()) || rect[2] <= rect[0] || rect[3] <= rect[1] {
        return Vec::new();
    }
    let (Some(x0), Some(y0), Some(x1), Some(y1)) =
        (floor_coord(rect[0] / s), floor_coord(rect[1] / s), floor_coord((rect[2] / s).ceil() - 1.0), floor_coord((rect[3] / s).ceil() - 1.0))
    else {
        return Vec::new();
    };
    let nx = i64::from(x1) - i64::from(x0) + 1;
    let ny = i64::from(y1) - i64::from(y0) + 1;
    if nx <= 0 || ny <= 0 || nx.saturating_mul(ny) > MAX_TILES as i64 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity((nx * ny) as usize);
    for y in y0..=y1 {
        for x in x0..=x1 {
            out.push(TileCoord::new(x, y));
        }
    }
    out
}

/// 一类绘制图元。只有「不读背景」且能安全栅格化进格子的种类会参与哈希。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    Rect,
    Path,
    Sprite,
    Shadow,
    ClipStart,
    ClipEnd,
    Overlay,
}

/// 一条绘制指令的抽象描述。`visible` 已含阴影/模糊扩散后的可见边界。
#[derive(Clone, Debug)]
pub struct DrawOp {
    pub kind: OpKind,
    pub visible: PageRect,
    /// 决定像素的字段指纹（颜色、渐变、圆角、纹理编号等）。
    pub fields: u64,
    pub clip: Option<PageRect>,
}

fn hash_f64(h: &mut DefaultHasher, v: f64) {
    v.to_bits().hash(h);
}

fn hash_rect(h: &mut DefaultHasher, r: PageRect) {
    for v in r {
        hash_f64(h, v);
    }
}

fn tile_origin(c: TileCoord, side: u32) -> (f64, f64) {
    let s = f64::from(side);
    (f64::from(c.x) * s, f64::from(c.y) * s)
}

/// 把矩形裁进格子的局部坐标；空交集用固定哨兵值，保证所有空遮罩哈希一致。
fn clipped_to_tile(rect: PageRect, origin: (f64, f64), side: u32) -> PageRect {
    let s = f64::from(side);
    let x0 = (rect[0] - origin.0).max(0.0);
    let y0 = (rect[1] - origin.1).max(0.0);
    let x1 = (rect[2] - origin.0).min(s);
    let y1 = (rect[3] - origin.1).min(s);
    if x1 <= x0 || y1 <= y0 {
        return [0.0, 0.0, -1.0, -1.0];
    }
    [x0, y0, x1, y1]
}

fn hash_op(h: &mut DefaultHasher, op: &DrawOp, c: TileCoord, side: u32) {
    (op.kind as u8).hash(h);
    op.fields.hash(h);
    let origin = tile_origin(c, side);
    hash_rect(h, [op.visible[0] - origin.0, op.visible[1] - origin.1, op.visible[2] - origin.0, op.visible[3] - origin.1]);
    // 有没有遮罩本身要先哈希：空交集与「没有遮罩」是两回事，
    // 否则撤掉一个不相交的遮罩会让图元变可见而格哈希不变。
    match op.clip {
        Some(r) => {
            1u8.hash(h);
            hash_rect(h, clipped_to_tile(r, origin, side));
        }
        None => 0u8.hash(h),
    }
}

/// 两个页面矩形的交集；没有重叠返回 None。
fn intersect_rect(a: PageRect, b: PageRect) -> Option<PageRect> {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}

/// 每个格子的内容哈希，空格子也会得到一个值。两块哈希相同即像素相同。
pub fn tile_hashes(ops: &[DrawOp], region: PageRect, side: u32) -> HashMap<TileCoord, u64> {
    let mut hashers: HashMap<TileCoord, DefaultHasher> = HashMap::new();
    for c in tiles_over(region, side) {
        hashers.insert(c, DefaultHasher::new());
    }
    for op in ops {
        // 路径与覆盖层不进格子：屏幕空间导数抗锯齿在奇数平移下会差一级。
        if matches!(op.kind, OpKind::Path | OpKind::Overlay) {
            continue;
        }
        // 大图元的可见边界可能覆盖超过 MAX_TILES 个格子，直接交给 tiles_over 会被上限
        // 判为空，整条指令就不进哈希。先裁到本轮哈希的 region 再枚举；hash_op 仍收原始
        // 几何，格内坐标不失真。
        let Some(visible) = intersect_rect(op.visible, region) else { continue };
        for c in tiles_over(visible, side) {
            if let Some(h) = hashers.get_mut(&c) {
                hash_op(h, op, c, side);
            }
        }
    }
    hashers.into_iter().map(|(c, h)| (c, h.finish())).collect()
}

/// 新旧哈希不同、或旧图里没有的格子，排序后返回。
pub fn dirty_tiles(old: &HashMap<TileCoord, u64>, new: &HashMap<TileCoord, u64>) -> Vec<TileCoord> {
    let mut dirty: Vec<TileCoord> = new.iter().filter(|(c, hash)| old.get(c) != Some(*hash)).map(|(c, _)| *c).collect();
    dirty.sort();
    dirty
}

/// 向外多画的像素：按最大背景模糊半径算，上限 256。
/// 这一圈只作采样上下文，**算完裁掉丢弃**，不是拖动时的像素存货。
pub fn bleed_for(max_backdrop_radius: f64, scale: f64) -> u32 {
    let v = (max_backdrop_radius.max(0.0) * scale.max(0.0) * 1.5 + 3.0).ceil();
    if !v.is_finite() {
        return 3;
    }
    v.clamp(3.0, f64::from(MAX_BLEED)) as u32
}

/// 渲染某格时实际画的矩形（向外扩 bleed）。
pub fn expanded_rect(c: TileCoord, side: u32, bleed: u32) -> PageRect {
    let (ox, oy) = tile_origin(c, side);
    let s = f64::from(side);
    let b = f64::from(bleed);
    [ox - b, oy - b, ox + s + b, oy + s + b]
}

/// 算完只取中间那一块贴上去。
pub fn crop_rect(c: TileCoord, side: u32) -> PageRect {
    let (ox, oy) = tile_origin(c, side);
    let s = f64::from(side);
    [ox, oy, ox + s, oy + s]
}

/// 是否值得启用格子：大画布才划算，特效扩散范围必须小于一格，背景模糊不能太宽。
pub fn uses_tiles(pixels: u64, scale: f64, effect_reach: f64, backdrop_radius: f64) -> bool {
    let big = pixels >= TILED_MIN_PIXELS || (scale >= 8.0 && pixels >= HIGH_ZOOM_TILED_MIN_PIXELS);
    let bleed = bleed_for(backdrop_radius, scale);
    big && effect_reach.is_finite() && effect_reach >= 0.0 && effect_reach <= f64::from(TILE_SIDE) && bleed < MAX_BLEED
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// 只合成：偏移变了，内容没变，直接把旧格按新位置贴。
    Reuse,
    /// 重绘脏格。
    Repaint,
    /// 退回普通绘制（无法保证同像素或当前坐标）。
    Bypass,
}

#[derive(Clone, Copy, Debug)]
pub struct PolicyInput {
    pub only_offset_changed: bool,
    pub opaque_background: bool,
    pub focused_input: bool,
    pub animation_requested: bool,
    pub moved_past_cache: bool,
    /// 最近 16 帧里内容变化的帧数。
    pub changed_in_last_16: u8,
}

/// 三态决策。绕过条件抄 `fast::layers::policy::decide`：背景不是单色不透明、
/// 有焦点输入或动画、滚动越过缓存范围、内容变化太频繁。
pub fn decide(p: PolicyInput, dirty: &[TileCoord]) -> Decision {
    if !p.opaque_background || p.focused_input || p.animation_requested || p.moved_past_cache || p.changed_in_last_16 >= BYPASS_CHANGED_FRAMES {
        return Decision::Bypass;
    }
    if dirty.is_empty() && p.only_offset_changed {
        return Decision::Reuse;
    }
    if dirty.is_empty() {
        return Decision::Reuse;
    }
    Decision::Repaint
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> PageRect {
        [x0, y0, x1, y1]
    }

    fn op(kind: OpKind, visible: PageRect, fields: u64) -> DrawOp {
        DrawOp { kind, visible, fields, clip: None }
    }

    #[test]
    fn pan_reuses_old_tiles_and_only_adds_a_column() {
        let first = tiles_over(rect(0.0, 0.0, 1024.0, 1024.0), TILE_SIDE);
        let second = tiles_over(rect(100.0, 0.0, 1124.0, 1024.0), TILE_SIDE);
        assert_eq!(first.len(), 4);
        assert_eq!(second.len(), 6);
        assert!(first.iter().all(|c| second.contains(c)));
    }

    #[test]
    fn dirty_tiles_reports_only_changed_cells() {
        let ops = vec![op(OpKind::Rect, rect(10.0, 10.0, 40.0, 40.0), 7)];
        let old = tile_hashes(&ops, rect(0.0, 0.0, 1024.0, 1024.0), TILE_SIDE);
        let mut changed = ops.clone();
        changed[0].fields = 8;
        let new = tile_hashes(&changed, rect(0.0, 0.0, 1024.0, 1024.0), TILE_SIDE);
        assert_eq!(dirty_tiles(&old, &new), vec![TileCoord::new(0, 0)]);
    }

    #[test]
    fn clip_is_hashed_clipped_so_a_pan_does_not_dirty_every_tile() {
        let mut a = op(OpKind::Sprite, rect(0.0, 0.0, 512.0, 512.0), 1);
        a.clip = Some(rect(0.0, 0.0, 512.0, 512.0));
        let mut b = a.clone();
        b.clip = Some(rect(-100.0, -100.0, 1000.0, 1000.0));
        let region = rect(0.0, 0.0, 512.0, 512.0);
        let ha = tile_hashes(&[a], region, TILE_SIDE);
        let hb = tile_hashes(&[b], region, TILE_SIDE);
        assert_eq!(ha.get(&TileCoord::new(0, 0)), hb.get(&TileCoord::new(0, 0)));
    }

    #[test]
    fn paths_are_kept_out_of_tiles() {
        let region = rect(0.0, 0.0, 512.0, 512.0);
        let empty = tile_hashes(&[], region, TILE_SIDE);
        let path = tile_hashes(&[op(OpKind::Path, rect(1.0, 1.0, 50.0, 50.0), 9)], region, TILE_SIDE);
        assert_eq!(empty.get(&TileCoord::new(0, 0)), path.get(&TileCoord::new(0, 0)));
    }

    #[test]
    fn bleed_is_bounded_and_crop_keeps_only_the_center() {
        assert_eq!(bleed_for(0.0, 1.0), 3);
        assert_eq!(bleed_for(1000.0, 8.0), MAX_BLEED);
        let c = TileCoord::new(1, 1);
        let expanded = expanded_rect(c, TILE_SIDE, 16);
        let crop = crop_rect(c, TILE_SIDE);
        assert_eq!(expanded, rect(496.0, 496.0, 1040.0, 1040.0));
        assert_eq!(crop, rect(512.0, 512.0, 1024.0, 1024.0));
    }

    #[test]
    fn zoom_bucket_changes_at_powers_of_two() {
        assert_eq!(zoom_bucket(0.5), -1);
        assert_eq!(zoom_bucket(1.0), 0);
        assert_eq!(zoom_bucket(2.0), 1);
        assert_eq!(zoom_bucket(0.0), 0);
    }

    #[test]
    fn policy_bypasses_when_pixels_cannot_be_guaranteed() {
        let safe = PolicyInput {
            only_offset_changed: true,
            opaque_background: true,
            focused_input: false,
            animation_requested: false,
            moved_past_cache: false,
            changed_in_last_16: 0,
        };
        assert_eq!(decide(safe, &[]), Decision::Reuse);
        assert_eq!(decide(safe, &[TileCoord::new(0, 0)]), Decision::Repaint);
        let mut animated = safe;
        animated.animation_requested = true;
        assert_eq!(decide(animated, &[]), Decision::Bypass);
        let mut busy = safe;
        busy.changed_in_last_16 = BYPASS_CHANGED_FRAMES;
        assert_eq!(decide(busy, &[]), Decision::Bypass);
    }

    #[test]
    fn tiles_over_rejects_bad_input_without_panicking() {
        assert!(tiles_over([f64::NAN, 0.0, 1.0, 1.0], TILE_SIDE).is_empty());
        assert!(tiles_over(rect(0.0, 0.0, 0.0, 0.0), TILE_SIDE).is_empty());
        assert!(tiles_over(rect(0.0, 0.0, 1.0, 1.0), 0).is_empty());
    }

    #[test]
    fn absent_clip_differs_from_a_disjoint_clip() {
        let region = rect(0.0, 0.0, 512.0, 512.0);
        let no_clip = op(OpKind::Rect, rect(10.0, 10.0, 40.0, 40.0), 1);
        let mut disjoint = no_clip.clone();
        disjoint.clip = Some(rect(2000.0, 2000.0, 2100.0, 2100.0));
        let mut outside = no_clip.clone();
        outside.clip = Some(rect(-100.0, -100.0, -10.0, -10.0));
        let h0 = tile_hashes(&[no_clip], region, TILE_SIDE);
        let h1 = tile_hashes(&[disjoint], region, TILE_SIDE);
        let h2 = tile_hashes(&[outside], region, TILE_SIDE);
        assert_ne!(h0.get(&TileCoord::new(0, 0)), h1.get(&TileCoord::new(0, 0)));
        assert_ne!(h0.get(&TileCoord::new(0, 0)), h2.get(&TileCoord::new(0, 0)));
        // 两个不相交的遮罩裁进同一格后都是空交集，哈希仍应一致（平移不弄脏所有格）。
        assert_eq!(h1.get(&TileCoord::new(0, 0)), h2.get(&TileCoord::new(0, 0)));
    }

    #[test]
    fn a_large_operation_is_hashed_within_a_small_region() {
        let region = rect(0.0, 0.0, 512.0, 512.0);
        // 16384x16384 覆盖的格子数远超 MAX_TILES，不能被 tiles_over 直接丢掉。
        let big = op(OpKind::Rect, rect(0.0, 0.0, 16384.0, 16384.0), 1);
        let mut changed = big.clone();
        changed.fields = 2;
        let old = tile_hashes(&[big], region, TILE_SIDE);
        let new = tile_hashes(&[changed], region, TILE_SIDE);
        assert_eq!(dirty_tiles(&old, &new), vec![TileCoord::new(0, 0)]);
    }
}

/// 视口 → 渲染单元清单（第 2 批：接口层，仍未接线）。
///
/// 这一步只把「当前视口由哪些渲染单元组成」算出来，供后续 PR 接到 `JobKey`
/// 与 `canvas::show`；本批不改变任何渲染行为。
pub mod view_units {
    use super::{PageRect, TILE_SIDE, TileCoord, UnitKey, tiles_over, zoom_bucket};

    /// 生成单元键所需的、与格子位置无关的输入。
    #[derive(Clone, Copy, Debug)]
    pub struct ViewInput {
        /// 文档**身份**（`Arc` 指针），不是内容版本：同一手势内原地编辑不会改变它。
        /// 接线到真实渲染前必须换成内容 revision（见 UnitKey::doc_identity）。
        pub doc_identity: u64,
        /// 当前页面（NodeId 的数值）：不同页的同一视口不能共用单元。
        pub page: u64,
        pub params: u64,
        pub size: (u32, u32),
        pub fonts: u64,
        pub side: u32,
    }

    impl Default for ViewInput {
        fn default() -> Self {
            Self { doc_identity: 0, page: 0, params: 0, size: (0, 0), fonts: 0, side: TILE_SIDE }
        }
    }

    /// 视口覆盖到的每个格子生成一个身份键，缩放档位对所有格子一致。
    pub fn for_view(view: PageRect, scale: f64, input: ViewInput) -> Vec<UnitKey> {
        let mut out = Vec::new();
        for_view_into(view, scale, input, &mut out);
        out
    }

    /// 与 `for_view` 相同，但写入调用方持有的 Vec：画布每帧复用 `CanvasState.units`，
    /// 不为每帧重新分配一整张表。
    pub fn for_view_into(view: PageRect, scale: f64, input: ViewInput, out: &mut Vec<UnitKey>) {
        out.clear();
        let bucket = zoom_bucket(scale);
        out.extend(tiles_over(view, input.side).into_iter().map(|tile| UnitKey {
            tile,
            zoom_bucket: bucket,
            doc_identity: input.doc_identity,
            page: input.page,
            params: input.params,
            size: input.size,
            fonts: input.fonts,
            side: input.side,
        }));
    }

    /// 下一帧需要重跑的格子：身份键变了或新出现的格。
    pub fn changed(prev: &[UnitKey], next: &[UnitKey]) -> Vec<TileCoord> {
        next.iter().filter(|u| !prev.iter().any(|p| p == *u)).map(|u| u.tile).collect()
    }
}

#[cfg(test)]
mod view_unit_tests {
    use super::view_units::{ViewInput, changed, for_view, for_view_into};
    use super::{TILE_SIDE, TileCoord};

    fn view(x0: f64, x1: f64) -> [f64; 4] {
        [x0, 0.0, x1, 1024.0]
    }

    #[test]
    fn a_pan_keeps_old_units_and_only_adds_new_ones() {
        let input = ViewInput::default();
        let before = for_view(view(0.0, 1024.0), 1.0, input);
        let after = for_view(view(100.0, 1124.0), 1.0, input);
        assert_eq!(before.len(), 4);
        assert_eq!(after.len(), 6);
        assert!(before.iter().all(|u| after.contains(u)));
        assert_eq!(changed(&before, &after).len(), 2);
    }

    #[test]
    fn crossing_a_zoom_bucket_invalidates_every_unit() {
        let input = ViewInput::default();
        let before = for_view(view(0.0, 1024.0), 1.0, input);
        let after = for_view(view(0.0, 1024.0), 2.0, input);
        assert_eq!(changed(&before, &after).len(), before.len());
    }

    #[test]
    fn a_document_change_invalidates_every_unit() {
        let first = for_view(view(0.0, 1024.0), 1.0, ViewInput { doc_identity: 1, ..ViewInput::default() });
        let second = for_view(view(0.0, 1024.0), 1.0, ViewInput { doc_identity: 2, ..ViewInput::default() });
        assert_eq!(changed(&first, &second).len(), first.len());
    }

    #[test]
    fn an_oversized_view_yields_no_units_instead_of_allocating() {
        let units = for_view([0.0, 0.0, 1.0e9, 1.0e9], 1.0, ViewInput::default());
        assert!(units.is_empty());
        assert_eq!(TILE_SIDE, 512);
        assert_eq!(TileCoord::new(0, 0).x, 0);
    }

    #[test]
    fn a_page_switch_invalidates_every_unit() {
        let first = for_view(view(0.0, 1024.0), 1.0, ViewInput { page: 1, ..ViewInput::default() });
        let second = for_view(view(0.0, 1024.0), 1.0, ViewInput { page: 2, ..ViewInput::default() });
        assert_eq!(changed(&first, &second).len(), first.len());
    }

    #[test]
    fn a_tile_side_change_invalidates_units_with_the_same_coordinate() {
        // side 从 512 改成 1024 后，(0, 0) 代表的页面范围变了；若 side 不进键，
        // changed 会把这一格误判成「没变」。
        let first = for_view(view(0.0, 1024.0), 1.0, ViewInput::default());
        let second = for_view(view(0.0, 1024.0), 1.0, ViewInput { side: 1024, ..ViewInput::default() });
        assert_eq!(changed(&first, &second), vec![TileCoord::new(0, 0)]);
    }

    #[test]
    fn for_view_into_reuses_the_buffer_and_clears_stale_units() {
        let mut out = for_view(view(0.0, 1024.0), 1.0, ViewInput::default());
        let capacity = out.capacity();
        for_view_into(view(0.0, 512.0), 1.0, ViewInput::default(), &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out.capacity(), capacity);
        for_view_into([0.0, 0.0, 1.0e9, 1.0e9], 1.0, ViewInput::default(), &mut out);
        assert!(out.is_empty());
        assert_eq!(out.capacity(), capacity);
    }
}
