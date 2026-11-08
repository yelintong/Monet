//! Frame statistics for spotting wasted redraws（照搬 `ui-egui/src/canvas/frame_stats.rs`）。
//!
//! 原文：
//!
//! > Frame statistics for spotting wasted redraws: how many frames the UI drew and what asked for
//! > them (the `file:line` of each repaint request), read through `ui.inspect` (`framesDrawn`,
//! > `repaintCauses`) and reset with `ui.set {resetFrameStats: true}`. An idle window should draw
//! > no frames at all.
//!
//! 与原文的差异：`repaintCauses` 的数据来自 egui 的 `Context::repaint_causes()`——GPUI 没有把
//! 「谁请求了重绘」记在调用点上的机制，所以这里**只统计帧数**，`repaintCauses` 报 `null`
//! 而不是编一份假的。等 GPUI 提供等价 API（或我们自己插桩到 `cx.notify` 的调用点）再补。

use serde_json::{Value, json};

#[derive(Default)]
pub struct FrameStats {
    /// Frames drawn since the app started (or the last reset).
    pub frames: u64,
}

impl FrameStats {
    /// Count this frame (call once a frame).
    pub fn record(&mut self) {
        self.frames += 1;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `ui.inspect` 的 `framesDrawn` / `repaintCauses` 两个字段。
    pub fn to_json(&self) -> Value {
        json!({ "framesDrawn": self.frames, "repaintCauses": Value::Null })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_counted_and_reset() {
        let mut s = FrameStats::default();
        s.record();
        s.record();
        assert_eq!(s.to_json()["framesDrawn"], json!(2));
        s.reset();
        assert_eq!(s.to_json()["framesDrawn"], json!(0));
    }
}
