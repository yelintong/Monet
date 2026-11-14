//! Keyframe tracks and the per-node motion container.

use super::clip::{Clip, ClipRepeat};
use super::easing::Ease;
use super::value::{AnimProperty, AnimValue};
use serde::{Deserialize, Serialize};

/// One keyframe: a value at a time, plus the easing of the segment that *starts* here.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Position on the timeline in milliseconds.
    pub time_ms: u32,
    pub value: AnimValue,
    /// Easing of the segment from this keyframe to the next.
    #[serde(default, skip_serializing_if = "Ease::is_linear")]
    pub easing: Ease,
    /// 空间**出手柄**（相对本帧位置，仅 Position 用）：这一段离开本帧时的切线方向。
    /// `None` = 直线。旧文件没有这个字段 → `None` → 行为与以前完全一致（用户 2026-11-12）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_handle: Option<[f64; 2]>,
    /// 空间**入手柄**（相对本帧位置）：这一段进入本帧时的切线方向。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_handle: Option<[f64; 2]>,
}

/// All keyframes of one property of one node, sorted by time with unique times.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub property: AnimProperty,
    pub keys: Vec<Keyframe>,
    /// The eye of the timeline's property row: a hidden track keeps its keyframes but is ignored
    /// by evaluation, playback and export.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub visible: bool,
}

fn default_true() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

impl Track {
    pub fn new(property: AnimProperty) -> Self {
        Self { property, keys: Vec::new(), visible: true }
    }

    /// Sorts keys by time, dropping all but the last of keys sharing a time, and drops keys whose
    /// value has the wrong shape or is not finite.
    pub fn normalize(&mut self) {
        let prop = self.property;
        self.keys.retain(|k| prop.accepts(&k.value) && k.value.is_finite());
        self.keys.sort_by_key(|k| k.time_ms);
        let mut out: Vec<Keyframe> = Vec::with_capacity(self.keys.len());
        for k in self.keys.drain(..) {
            match out.last_mut() {
                Some(l) if l.time_ms == k.time_ms => *l = k,
                _ => out.push(k),
            }
        }
        self.keys = out;
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Index of the keyframe exactly at `time_ms`.
    pub fn key_index(&self, time_ms: u32) -> Option<usize> {
        self.keys.binary_search_by_key(&time_ms, |k| k.time_ms).ok()
    }

    pub fn key_at(&self, time_ms: u32) -> Option<&Keyframe> {
        self.key_index(time_ms).map(|i| &self.keys[i])
    }

    /// The keyframe closest to `time_ms` within `tolerance_ms`.
    pub fn key_near(&self, time_ms: u32, tolerance_ms: u32) -> Option<&Keyframe> {
        self.keys.iter().filter(|k| k.time_ms.abs_diff(time_ms) <= tolerance_ms).min_by_key(|k| k.time_ms.abs_diff(time_ms))
    }

    /// Inserts a keyframe, or replaces the one at the same time.
    pub fn set_key(&mut self, time_ms: u32, value: AnimValue, easing: Option<Ease>) {
        match self.keys.binary_search_by_key(&time_ms, |k| k.time_ms) {
            Ok(i) => {
                self.keys[i].value = value;
                if let Some(e) = easing {
                    self.keys[i].easing = e;
                }
            }
            Err(i) => {
                let inherited = if i > 0 { self.keys[i - 1].easing } else { Ease::Linear };
                self.keys.insert(i, Keyframe { time_ms, value, easing: easing.unwrap_or(inherited), out_handle: None, in_handle: None });
            }
        }
    }

    pub fn remove_key(&mut self, time_ms: u32) -> Option<Keyframe> {
        self.key_index(time_ms).map(|i| self.keys.remove(i))
    }

    pub fn start_ms(&self) -> Option<u32> {
        self.keys.first().map(|k| k.time_ms)
    }

    pub fn end_ms(&self) -> Option<u32> {
        self.keys.last().map(|k| k.time_ms)
    }

    /// Time of the closest keyframe strictly before `time_ms`.
    pub fn prev_key_time(&self, time_ms: u32) -> Option<u32> {
        self.keys.iter().rev().find(|k| k.time_ms < time_ms).map(|k| k.time_ms)
    }

    /// Time of the closest keyframe strictly after `time_ms`.
    pub fn next_key_time(&self, time_ms: u32) -> Option<u32> {
        self.keys.iter().find(|k| k.time_ms > time_ms).map(|k| k.time_ms)
    }

    /// Index of the keyframe starting the segment that contains `t_ms`.
    pub fn segment_start(&self, t_ms: f64) -> Option<usize> {
        let n = self.keys.partition_point(|k| (k.time_ms as f64) <= t_ms);
        n.checked_sub(1)
    }

    /// Keyframed value at `t_ms`: the first value before the first key, the last value after the
    /// last key, eased interpolation in between. `None` for an empty track.
    pub fn value_at(&self, t_ms: f64) -> Option<AnimValue> {
        self.value_at_impl(t_ms, true)
    }

    /// Same as [`Self::value_at`] but **without the easing curve**: the segment progress is the
    /// raw time fraction. The motion path is drawn from this, because easing only decides *how
    /// fast* the object travels along the path — never the path's shape (user 2026-11-14).
    pub fn value_at_linear(&self, t_ms: f64) -> Option<AnimValue> {
        self.value_at_impl(t_ms, false)
    }

    fn value_at_impl(&self, t_ms: f64, eased: bool) -> Option<AnimValue> {
        let first = self.keys.first()?;
        let Some(i) = self.segment_start(t_ms) else { return Some(first.value) };
        let a = &self.keys[i];
        let Some(b) = self.keys.get(i + 1) else { return Some(a.value) };
        // Keys are sorted by construction, but a malformed change can still arrive unsorted or with
        // two keys on one time. Saturate instead of letting the subtraction overflow, and hold the
        // first key's value when the two sit on the same time instead of dividing by zero.
        let span = b.time_ms.saturating_sub(a.time_ms) as f32;
        if span <= 0.0 {
            return Some(a.value);
        }
        let frac = (t_ms - a.time_ms as f64) as f32 / span;
        let p = if eased { f64::from(a.easing.ease(frac)) } else { f64::from(frac) };
        // Position 且**至少一端有非零**空间手柄 → 三次贝塞尔；否则保持线性（直线）。
        //
        // ⚠️ 零手柄不能当贝塞尔：`p1=p0、p2=p3` 的三次贝塞尔**不等于线性插值**
        // （t=0.25 时进度只有 0.156），会画出一条莫名弯曲的线（用户 2026-11-12）。
        // 照抄矢量路径的 `Segment::is_straight`：手柄精确为 (0,0) 就是直线。
        let nz = |h: Option<[f64; 2]>| h.filter(|h| h[0] != 0.0 || h[1] != 0.0);
        let (ho, hi) = (nz(a.out_handle), nz(b.in_handle));
        if let (AnimValue::Vec2(v0), AnimValue::Vec2(v3)) = (&a.value, &b.value)
            && (ho.is_some() || hi.is_some())
        {
            let h1 = ho.unwrap_or([0.0, 0.0]);
            let h2 = hi.unwrap_or([0.0, 0.0]);
            let (x0, y0, x3, y3) = (v0.x, v0.y, v3.x, v3.y);
            let (x1, y1) = (x0 + h1[0], y0 + h1[1]);
            let (x2, y2) = (x3 + h2[0], y3 + h2[1]);
            let (t, u) = (p, 1.0 - p);
            let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            let x = x0 * w0 + x1 * w1 + x2 * w2 + x3 * w3;
            let y = y0 * w0 + y1 * w1 + y2 * w2 + y3 * w3;
            return Some(AnimValue::Vec2(figcraft_geom::Vec2::new(x, y)));
        }
        Some(AnimValue::lerp(&a.value, &b.value, p))
    }
}

/// Animation data of one node: keyframe tracks plus clips. Empty for most nodes.
// 注意：`Deserialize` 是手写的（见下），不是 derive——反序列化后必须立刻 `normalize()`，
// 让「文件 load / collab 变更 / 整节点读」三条路径都拿到不变量成立的 motion
// （PR#1624 的架构 gate）。本地 format 层不碰本字段、Node 靠 serde 派生读写，
// collab 的 `Json::read::<T>` 也走 `T::deserialize`，所以这里覆盖所有入口。
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct NodeMotion {
    /// One track per property at most, in creation order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<Track>,
    /// Clips (animation presets), in creation order; composition is order independent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<Clip>,
}

/// 每条读取 motion 的路径都在这里落地：反序列化后 `normalize()`，于是手改过的文件、
/// 协作端写入的脏数据、整节点读取，都会恢复成「同 property 只一条轨道 / key 排序去重 /
/// clip id 唯一」的不变量，而不必依赖每个写命令记得调 normalize（本地原先只在
/// `engine/cmd/animation.rs` 写命令时调，load / collab 路径有缺口）。
impl<'de> Deserialize<'de> for NodeMotion {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Repr {
            #[serde(default)]
            tracks: Vec<Track>,
            #[serde(default)]
            clips: Vec<Clip>,
        }
        let r = Repr::deserialize(d)?;
        let mut m = NodeMotion { tracks: r.tracks, clips: r.clips };
        m.normalize();
        Ok(m)
    }
}

impl NodeMotion {
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.clips.is_empty()
    }

    pub fn track(&self, property: AnimProperty) -> Option<&Track> {
        self.tracks.iter().find(|t| t.property == property)
    }

    pub fn track_mut(&mut self, property: AnimProperty) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.property == property)
    }

    /// The track for `property`, created empty when missing.
    pub fn ensure_track(&mut self, property: AnimProperty) -> &mut Track {
        let i = match self.tracks.iter().position(|t| t.property == property) {
            Some(i) => i,
            None => {
                self.tracks.push(Track::new(property));
                self.tracks.len() - 1
            }
        };
        &mut self.tracks[i]
    }

    pub fn clip(&self, id: u32) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == id)
    }

    /// A clip id not used by any clip of this node.
    /// PR#1624 加固：`max + 1` 用 `saturating_add` 防 `u32::MAX` 溢出，并 while 跳过任何
    /// 已占用的 id（本地原来 `map(|c| c.id + 1).max()` 在 id=MAX 时溢出、且可能撞上空洞里
    /// 已用的 id）。
    pub fn next_clip_id(&self) -> u32 {
        let mut id = self.clips.iter().map(|c| c.id).max().map_or(0, |m| m.saturating_add(1));
        while id != u32::MAX && self.clips.iter().any(|c| c.id == id) {
            id += 1;
        }
        id
    }

    /// Restores invariants after loading or bulk edits.
    pub fn normalize(&mut self) {
        let mut merged: Vec<Track> = Vec::new();
        for t in self.tracks.drain(..) {
            match merged.iter_mut().find(|m| m.property == t.property) {
                Some(m) => {
                    m.keys.extend(t.keys);
                    m.visible &= t.visible;
                }
                None => merged.push(t),
            }
        }
        for t in &mut merged {
            t.normalize();
        }
        merged.retain(|t| !t.is_empty());
        self.tracks = merged;
        let mut seen = std::collections::BTreeSet::new();
        // 同 `next_clip_id` 的加固：saturating + while 跳过已用 id，避免重复 id 重分配时
        // 又撞进另一个已用 id，或 `next += 1` 在 u32::MAX 处溢出。
        let mut next = self.clips.iter().map(|c| c.id).max().map_or(0, |m| m.saturating_add(1));
        for c in &mut self.clips {
            if !seen.insert(c.id) {
                while next != u32::MAX && seen.contains(&next) {
                    next += 1;
                }
                c.id = next;
                next = next.saturating_add(1);
                seen.insert(c.id);
            }
            c.sanitize();
        }
    }

    /// Sorted, deduplicated times of every keyframe across all tracks.
    pub fn key_times(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.tracks.iter().flat_map(|t| t.keys.iter().map(|k| k.time_ms)).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Closest keyframe time across all tracks strictly before `time_ms`.
    pub fn prev_key_time(&self, time_ms: u32) -> Option<u32> {
        self.tracks.iter().filter_map(|t| t.prev_key_time(time_ms)).max()
    }

    /// Closest keyframe time across all tracks strictly after `time_ms`.
    pub fn next_key_time(&self, time_ms: u32) -> Option<u32> {
        self.tracks.iter().filter_map(|t| t.next_key_time(time_ms)).min()
    }

    /// Time range covered by this node's animation.
    pub fn span_ms(&self, timeline_duration_ms: u32) -> Option<(u32, u32)> {
        let mut lo = u32::MAX;
        let mut hi = 0u32;
        let mut any = false;
        for t in &self.tracks {
            if let (Some(a), Some(b)) = (t.start_ms(), t.end_ms()) {
                lo = lo.min(a);
                hi = hi.max(b);
                any = true;
            }
        }
        for c in self.clips.iter().filter(|c| c.visible) {
            lo = lo.min(c.delay_ms);
            hi = hi.max(c.end_ms(timeline_duration_ms));
            any = true;
        }
        any.then_some((lo, hi))
    }

    /// Does this node animate `property` at all, via a track or a visible clip?
    pub fn animates(&self, property: AnimProperty) -> bool {
        self.track(property).is_some_and(|t| t.visible) || self.clips.iter().any(|c| c.visible && c.kind.properties().contains(&property))
    }

    /// Whether any clip repeats forever.
    pub fn has_infinite_clip(&self) -> bool {
        self.clips.iter().any(|c| c.visible && c.repeat == ClipRepeat::Infinite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::motion::{ClipKind, ClipMode};

    fn s(v: f64) -> AnimValue {
        AnimValue::Scalar(v)
    }

    fn track(keys: &[(u32, f64)]) -> Track {
        let mut t = Track::new(AnimProperty::Opacity);
        for (ms, v) in keys {
            t.set_key(*ms, s(*v), None);
        }
        t
    }

    #[test]
    fn keys_stay_sorted_and_unique() {
        let t = track(&[(500, 0.5), (0, 0.0), (1000, 1.0), (500, 0.7)]);
        let times: Vec<u32> = t.keys.iter().map(|k| k.time_ms).collect();
        assert_eq!(times, vec![0, 500, 1000]);
        assert_eq!(t.key_at(500).unwrap().value, s(0.7));
    }

    #[test]
    fn value_at_holds_outside_and_interpolates_inside() {
        let t = track(&[(100, 0.0), (300, 1.0)]);
        assert_eq!(t.value_at(0.0), Some(s(0.0)));
        assert_eq!(t.value_at(200.0), Some(s(0.5)));
        assert_eq!(t.value_at(900.0), Some(s(1.0)));
        assert_eq!(Track::new(AnimProperty::Opacity).value_at(5.0), None);
    }

    #[test]
    fn new_key_inherits_easing_of_split_segment() {
        let mut t = track(&[(0, 0.0), (1000, 1.0)]);
        t.keys[0].easing = Ease::EaseOut;
        t.set_key(400, s(0.9), None);
        assert_eq!(t.key_at(400).unwrap().easing, Ease::EaseOut);
        t.set_key(400, s(0.8), Some(Ease::Hold));
        assert_eq!(t.key_at(400).unwrap().easing, Ease::Hold);
    }

    #[test]
    fn normalize_repairs_hand_edited_data() {
        let mut m = NodeMotion::default();
        m.tracks.push(Track {
            property: AnimProperty::Opacity,
            keys: vec![
                Keyframe { time_ms: 500, value: s(1.0), easing: Ease::Linear, out_handle: None, in_handle: None },
                Keyframe { time_ms: 0, value: s(0.0), easing: Ease::Linear, out_handle: None, in_handle: None },
                Keyframe { time_ms: 500, value: s(0.5), easing: Ease::Linear, out_handle: None, in_handle: None },
                Keyframe { time_ms: 700, value: AnimValue::Vec2(figcraft_geom::Vec2::ZERO), easing: Ease::Linear, out_handle: None, in_handle: None },
            ],
            visible: true,
        });
        m.tracks.push(Track::new(AnimProperty::Width));
        m.normalize();
        assert_eq!(m.tracks.len(), 1);
        let times: Vec<u32> = m.tracks[0].keys.iter().map(|k| k.time_ms).collect();
        assert_eq!(times, vec![0, 500]);
        assert_eq!(m.tracks[0].key_at(500).unwrap().value, s(0.5));
    }

    /// PR#1624 gate：反序列化即 normalize。文件 load / collab 读取一条脏 JSON（同 property
    /// 两条轨道、key 未排序且重复、clip id 重复），`from_str` 回来的 `NodeMotion` 必须已经
    /// 是不变量成立的状态——不需要调用方记得调 `normalize()`。
    #[test]
    fn deserialize_normalizes_dirty_motion() {
        // 先手工构造脏 motion，序列化成字符串（Serialize 不会 normalize）。
        let mut dirty = NodeMotion::default();
        dirty.tracks.push(Track {
            property: AnimProperty::Opacity,
            keys: vec![
                Keyframe { time_ms: 500, value: s(0.5), easing: Ease::Linear, out_handle: None, in_handle: None },
                Keyframe { time_ms: 0, value: s(0.0), easing: Ease::Linear, out_handle: None, in_handle: None },
            ],
            visible: true,
        });
        // 同 property 再来一条（load 后应合并）
        dirty.tracks.push(Track {
            property: AnimProperty::Opacity,
            keys: vec![Keyframe { time_ms: 500, value: s(0.9), easing: Ease::Linear, out_handle: None, in_handle: None }],
            visible: true,
        });
        let json = serde_json::to_string(&dirty).unwrap();
        let loaded: NodeMotion = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.tracks.len(), 1, "同 property 的两条轨道合并成一条");
        let times: Vec<u32> = loaded.tracks[0].keys.iter().map(|k| k.time_ms).collect();
        assert_eq!(times, vec![0, 500], "key 排序去重");
        assert_eq!(loaded.tracks[0].key_at(500).unwrap().value, s(0.9), "后写的同 time key 胜出");
    }

    /// `next_clip_id` 在 id 触顶（u32::MAX）时不能溢出/回绕，且要跳过已占用的 id。
    #[test]
    fn next_clip_id_never_overflows_or_collides() {
        let mut m = NodeMotion::default();
        let mut c = Clip::new(ClipKind::Position, ClipMode::In);
        c.id = u32::MAX;
        m.clips.push(c);
        // id=MAX 触顶：saturating 后停在 MAX（PR 的 while 条件 `id != u32::MAX` 此时不再找空位）。
        // 关键是**不 panic**（本地旧式 `c.id + 1` 在 debug 下会溢出 panic）、**不回绕成 0**。
        assert_eq!(m.next_clip_id(), u32::MAX, "触顶停在 MAX，不溢出回绕");

        let mut m2 = NodeMotion::default();
        for id in [0u32, 2, 3] {
            let mut c = Clip::new(ClipKind::Position, ClipMode::In);
            c.id = id;
            m2.clips.push(c);
        }
        assert_eq!(m2.next_clip_id(), 4, "取 max+1");
        let mut m3 = NodeMotion::default();
        for id in [0u32, 1, 2] {
            let mut c = Clip::new(ClipKind::Position, ClipMode::In);
            c.id = id;
            m3.clips.push(c);
        }
        assert!(!m3.clips.iter().any(|c| c.id == m3.next_clip_id()), "连续 id 时取 max+1 不撞");
    }
}
