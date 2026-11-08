// ============================================================================
// 冻结此文件 (FROZEN) — 通用组件，固化后不允许再改动。
// 2026-11-05 用户硬性要求：ui-gpui 的全部通用组件（src/widgets/ 下所有文件）一经固化即禁止再修改；
// 后续任何行为调整都必须先与用户确认，不得擅自改动本文件。
// 已锁定的关键不变量（INVARIANT，禁止破坏）：
//   - 调试边框（FC_DEBUG_LAYOUT）一律按「字形墨盒」绘制，不得退回行盒（ascent+descent）。
//   - 小标题（sublabels / sublabels2）字形上方保留 pt(4.0) 留白，不得删除。
//   - 统一网格（segmented、sublabels2 空列、field 行）一律 w_full + flex_1，禁止手算列宽。
// ============================================================================
//! `EditBuffer` — the shared text-editing kernel for **every** GPUI text input.
//!
//! This is the shared text-editing kernel used by every GPUI text input:
//! state, selection, movement, IME composition and UTF-16 conversion, with no widget, no document
//! and no UI backend in it. Both the on-canvas text edit (through `Session::text_edit`) and the
//! panel's `TextField` speak this vocabulary, so a fix to caret movement or composition applies to
//! both without being written twice.
//!
//! ## Coordinates
//! Indices are **character indices**, not UTF-8 byte offsets. That matches
//! `figcraft_engine::textedit`, `figcraft_text::{grapheme_step, backspace_step}` and this crate's
//! IME bridge. GPUI's `ShapedLine` reports UTF-8 byte offsets instead; [`EditBuffer::from_byte`]
//! and [`EditBuffer::to_byte`] are the only place that conversion happens. The platform input
//! protocol uses UTF-16 code units, converted at [`EditBuffer::selected_utf16`].
//!
//! ## Composition
//! A preedit is UI-only text inserted at the caret into a *view* of the buffer — the committed
//! text is untouched until the platform commits. [`EditBuffer::composed`] produces that view,
//! which is what `EntityInputHandler` must describe (the same comparison,
//! `last_layout.text == content`). This is the piece a hand-rolled editor almost always gets
//! wrong, and why CJK input used to drop characters.

use figcraft_text::{backspace_step, grapheme_step};
use gpui::UTF16Selection;
use std::ops::Range;

/// What a keystroke asked the surrounding field to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditOutcome {
    /// Nothing changed (an ignored key or a no-op move).
    None,
    /// Text or selection changed; the view must repaint.
    Changed,
    /// Enter: commit the value.
    Submit,
    /// Escape: restore the saved value and leave editing.
    Cancel,
}

/// The IME's view of a buffer: committed text with the live composition spliced in at the caret.
pub struct Composed {
    pub text: String,
    pub caret: usize,
    pub anchor: usize,
    pub mark_start: usize,
    pub mark_end: usize,
    pub composing: bool,
}

impl Composed {
    /// UTF-16 length of the composed text (for clamping platform ranges).
    pub fn utf16_len(&self) -> usize {
        self.text.chars().map(char::len_utf16).sum()
    }
}

/// A single-line text buffer with caret, selection anchor and an IME composition.
#[derive(Clone, Debug, Default)]
pub struct EditBuffer {
    /// The committed text.
    text: String,
    /// Caret, in characters.
    caret: usize,
    /// Selection anchor, in characters.
    anchor: usize,
    /// Live IME composition: its text and the caret inside it, in characters.
    preedit: Option<(String, usize)>,
    /// True while a pointer drag is extending the selection.
    selecting: bool,
}

impl EditBuffer {
    pub fn new(text: impl Into<String>) -> Self {
        let mut b = Self { text: text.into(), ..Self::default() };
        b.caret = b.len();
        b.anchor = b.caret;
        b
    }

    // ── basics ──────────────────────────────────────────────────────────────────────────────
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn caret(&self) -> usize {
        self.caret.min(self.len())
    }
    pub fn anchor(&self) -> usize {
        self.anchor.min(self.len())
    }
    pub fn range(&self) -> Range<usize> {
        let c = self.caret();
        let a = self.anchor();
        a.min(c)..a.max(c)
    }
    pub fn is_selecting(&self) -> bool {
        self.selecting
    }
    pub fn set_selecting(&mut self, v: bool) {
        self.selecting = v;
    }
    /// Replace the committed text (a document value pushing into the field, or a commit).
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.caret = self.len();
        self.anchor = self.caret;
        self.preedit = None;
    }
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.len();
        self.preedit = None;
    }
    pub fn is_all_selected(&self) -> bool {
        self.range() == (0..self.len())
    }

    // ── coordinate conversion ───────────────────────────────────────────────────────────────
    /// UTF-8 byte offset → character index (GPUI's `ShapedLine` reports bytes).
    pub fn from_byte(&self, byte: usize) -> usize {
        let byte = byte.min(self.text.len());
        self.text.get(..byte).map(|s| s.chars().count()).unwrap_or(0)
    }
    /// Character index → UTF-8 byte offset (for `ShapedLine::x_for_index`).
    pub fn to_byte(&self, ci: usize) -> usize {
        self.text.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(self.text.len())
    }
    /// Character index → UTF-16 offset (platform input protocol).
    pub fn char_to_utf16(&self, ci: usize) -> usize {
        let n = ci.min(self.len());
        self.text.chars().take(n).map(char::len_utf16).sum()
    }
    /// UTF-16 offset → character index (clamped to a character boundary).
    pub fn utf16_to_char(&self, u: usize) -> usize {
        let mut rem = u;
        let mut n = 0;
        for c in self.text.chars() {
            let w = c.len_utf16();
            if rem < w {
                return n;
            }
            rem -= w;
            n += 1;
        }
        n
    }
    /// What the platform should consider selected. While composing this is **the whole marked
    /// range**, so the platform's next replacement hits exactly the composition — the same rule
    /// `CanvasView`'s IME bridge uses. Reporting a collapsed caret here drops CJK candidates.
    pub fn selected_utf16(&self) -> UTF16Selection {
        let c = self.composed();
        if c.composing {
            return UTF16Selection { range: char_to_utf16_str(&c.text, c.mark_start)..char_to_utf16_str(&c.text, c.mark_end), reversed: false };
        }
        UTF16Selection {
            range: char_to_utf16_str(&c.text, c.caret.min(c.anchor))..char_to_utf16_str(&c.text, c.caret.max(c.anchor)),
            reversed: c.caret < c.anchor,
        }
    }

    // ── caret & selection ───────────────────────────────────────────────────────────────────
    /// Collapse the selection to `ci` (a click without Shift).
    pub fn move_to(&mut self, ci: usize) {
        let ci = ci.min(self.len());
        self.caret = ci;
        self.anchor = ci;
        self.preedit = None;
    }
    /// Extend the selection to `ci` (a drag, or Shift+arrow).
    pub fn select_to(&mut self, ci: usize) {
        let ci = ci.min(self.len());
        let anchor = self.anchor;
        self.caret = ci;
        self.anchor = anchor;
        self.preedit = None;
    }
    /// The character index one grapheme cluster before/after `ci`.
    pub fn grapheme_step(&self, ci: usize, forward: bool) -> usize {
        grapheme_step(&self.text, ci.min(self.len()), forward).min(self.len())
    }
    pub fn prev_boundary(&self, ci: usize) -> usize {
        self.grapheme_step(ci, false)
    }
    pub fn next_boundary(&self, ci: usize) -> usize {
        self.grapheme_step(ci, true)
    }
    /// Backspace start at `ci`: a trailing combining mark is deleted on its own, while an emoji
    /// sequence stays atomic (`figcraft_text::backspace_step`).
    pub fn backspace_start(&self, ci: usize) -> usize {
        backspace_step(&self.text, ci.min(self.len()))
    }
    /// `word_class` from `figcraft_engine::textedit`: 0 = separator, 1 = letters/digits/_,
    /// 2 = Han, 3 = hiragana, 4 = katakana, 5 = Hangul. CJK scripts are separate words because
    /// text without spaces still needs word-wise movement.
    fn word_class(c: char) -> u8 {
        let u = c as u32;
        // Script first: `char::is_alphanumeric()` is also true for Han, so testing it first would
        // fold CJK into the Latin class and break word-wise motion in text without spaces.
        if matches!(u, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3FFFF) || matches!(c, '\u{3005}' | '\u{3006}' | '\u{3007}') {
            2
        } else if matches!(u, 0x3041..=0x309F) {
            3
        } else if matches!(u, 0x30A0..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F) {
            4
        } else if matches!(u, 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF) {
            5
        } else if c.is_alphanumeric() || c == '_' {
            1
        } else {
            0
        }
    }
    fn is_word(c: char) -> bool {
        Self::word_class(c) != 0
    }
    /// Word boundary left of `ci`. When `ci` sits inside a word, that word's start; otherwise
    /// the start of the preceding word (skipping the separator run between them).
    pub fn word_left(&self, ci: usize) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = ci.min(chars.len());
        while i > 0 && !Self::is_word(chars[i - 1]) {
            i -= 1;
        }
        let class = if i > 0 { Self::word_class(chars[i - 1]) } else { 0 };
        while i > 0 && class != 0 && Self::word_class(chars[i - 1]) == class {
            i -= 1;
        }
        i
    }
    /// Word boundary right of `ci`. Symmetric with [`EditBuffer::word_left`].
    pub fn word_right(&self, ci: usize) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let n = chars.len();
        let mut i = ci.min(n);
        while i < n && !Self::is_word(chars[i]) {
            i += 1;
        }
        let class = chars.get(i).copied().map(Self::word_class).unwrap_or(0);
        while i < n && class != 0 && Self::word_class(chars[i]) == class {
            i += 1;
        }
        i
    }
    /// Select the word at `ci` (a double-click).
    pub fn select_word(&mut self, ci: usize) {
        self.anchor = self.word_left(ci);
        self.caret = self.word_right(ci);
        self.preedit = None;
    }
    /// Select the whole line (a triple-click; a single-line buffer is all of it).
    pub fn select_line(&mut self) {
        self.select_all();
    }

    // ── editing ─────────────────────────────────────────────────────────────────────────────
    fn replace_chars(&mut self, a: usize, b: usize, with: &str) {
        let (a, b) = (a.min(b).min(self.len()), a.max(b).min(self.len()));
        let ba = self.to_byte(a);
        let bb = self.to_byte(b);
        if self.text.get(ba..bb).is_none() {
            return;
        }
        self.text.replace_range(ba..bb, with);
        let ci = a + with.chars().count();
        self.caret = ci;
        self.anchor = ci;
        self.preedit = None;
    }
    /// Insert text at the selection, replacing it (keyboard, paste, IME commit).
    pub fn insert(&mut self, text: &str) {
        let r = self.range();
        let clean: String = text.chars().filter(|c| *c >= ' ' || *c == '\t').collect();
        self.replace_chars(r.start, r.end, &clean);
    }
    /// Replace a platform (UTF-16) range, or the selection when `None`.
    pub fn replace_utf16(&mut self, range: Option<Range<usize>>, text: &str) {
        let (a, b) = match range {
            Some(r) => (self.utf16_to_char(r.start), self.utf16_to_char(r.end)),
            None => {
                if self.preedit.is_some() {
                    let c = self.composed();
                    (c.mark_start, c.mark_end)
                } else {
                    let r = self.range();
                    (r.start, r.end)
                }
            }
        };
        self.replace_chars(a, b, text);
        self.preedit = None;
    }
    pub fn delete_backward(&mut self) {
        let r = self.range();
        let a = if r.start == r.end { self.backspace_start(r.start) } else { r.start };
        self.replace_chars(a, r.end, "");
    }
    pub fn delete_forward(&mut self) {
        let r = self.range();
        let b = if r.start == r.end { self.next_boundary(r.end) } else { r.end };
        self.replace_chars(r.start, b, "");
    }

    /// A platform key event. `word` is Ctrl/Alt+arrow, `line` is Home/End on this one-line buffer.
    /// Shortcut keys (Ctrl+A/C/X/V) are handled by the caller because they need the clipboard.
    pub fn key(&mut self, key: &str, shift: bool, word: bool) -> EditOutcome {
        let r = self.range();
        let cur = self.caret();
        match key {
            "enter" => return EditOutcome::Submit,
            "escape" => return EditOutcome::Cancel,
            "tab" => return EditOutcome::None,
            "backspace" => self.delete_backward(),
            "delete" => self.delete_forward(),
            "arrowleft" | "home" => {
                let target = if word {
                    self.word_left(cur)
                } else if key == "home" {
                    0
                } else if !shift && r.start != r.end {
                    r.start
                } else {
                    self.prev_boundary(cur)
                };
                if shift {
                    self.select_to(target);
                } else {
                    self.move_to(target);
                }
            }
            "arrowright" | "end" => {
                let target = if word {
                    self.word_right(cur)
                } else if key == "end" {
                    self.len()
                } else if !shift && r.start != r.end {
                    r.end
                } else {
                    self.next_boundary(cur)
                };
                if shift {
                    self.select_to(target);
                } else {
                    self.move_to(target);
                }
            }
            "arrowup" | "pageup" => {
                if shift {
                    self.select_to(0);
                } else {
                    self.move_to(0);
                }
            }
            "arrowdown" | "pagedown" => {
                let n = self.len();
                if shift {
                    self.select_to(n);
                } else {
                    self.move_to(n);
                }
            }
            _ => {
                let ch = key.chars().next().filter(|c| *c >= ' ');
                match ch {
                    Some(c) => self.insert(&c.to_string()),
                    None => return EditOutcome::None,
                }
            }
        }
        EditOutcome::Changed
    }

    // ── IME composition ─────────────────────────────────────────────────────────────────────
    /// Update the live composition. `cursor` is a character index inside `new_text`.
    pub fn set_preedit(&mut self, new_text: &str, cursor: usize) {
        if new_text.is_empty() {
            self.preedit = None;
        } else {
            self.preedit = Some((new_text.to_string(), cursor.min(new_text.chars().count())));
        }
    }
    /// Commit the live composition into the buffer (called by `unmark_text`).
    pub fn commit_preedit(&mut self) -> bool {
        let Some((text, _)) = self.preedit.take() else { return false };
        self.insert(&text);
        true
    }
    pub fn composing(&self) -> bool {
        self.preedit.is_some()
    }
    /// The IME's view of the text: committed text + the composition spliced in at the caret.
    /// `text_for_range`, `selected_text_range` and `marked_text_range` all describe this view,
    /// so the platform always replaces exactly the composition on its next update.
    pub fn composed(&self) -> Composed {
        let at = self.caret();
        match &self.preedit {
            Some((pe, inner)) => {
                let n = pe.chars().count();
                let cursor = at + (*inner).min(n);
                let mut text = String::with_capacity(self.text.len() + pe.len());
                text.extend(self.text.chars().take(at));
                text.push_str(pe);
                text.extend(self.text.chars().skip(at));
                Composed { text, caret: cursor, anchor: cursor, mark_start: at, mark_end: at + n, composing: true }
            }
            None => Composed {
                text: self.text.clone(),
                caret: self.caret(),
                anchor: self.anchor(),
                mark_start: self.range().start,
                mark_end: self.range().end,
                composing: false,
            },
        }
    }
    /// Text for a platform UTF-16 range, with the adjusted range written back.
    pub fn text_for_utf16(&self, range: Range<usize>, adjusted: &mut Option<Range<usize>>) -> Option<String> {
        let c = self.composed();
        let a = utf16_to_char_str(&c.text, range.start);
        let b = utf16_to_char_str(&c.text, range.end);
        let out: String = c.text.chars().skip(a).take(b.saturating_sub(a)).collect();
        *adjusted = Some(char_to_utf16_str(&c.text, a)..char_to_utf16_str(&c.text, b));
        Some(out)
    }
    pub fn marked_utf16(&self) -> Option<Range<usize>> {
        let c = self.composed();
        c.composing.then(|| char_to_utf16_str(&c.text, c.mark_start)..char_to_utf16_str(&c.text, c.mark_end))
    }
    /// Replace + mark, as GPUI's `replace_and_mark_text_in_range` requests. The new selected
    /// range is UTF-16 and relative to `new_text`.
    pub fn replace_and_mark_utf16(&mut self, range: Option<Range<usize>>, new_text: &str, new_selected: Option<Range<usize>>) {
        // A composition is a *view*: the marked range lives in the composed text, not in the
        // committed buffer. While composing, the platform replaces that view wholesale — the
        // committed text must stay put. Only when there is no composition does an explicit range
        // address committed text (and then it is removed before the new composition starts).
        if self.preedit.take().is_none() {
            if let Some(r) = range {
                let a = self.utf16_to_char(r.start);
                let b = self.utf16_to_char(r.end);
                self.replace_chars(a, b, "");
            }
        }
        let cursor = new_selected.map(|r| utf16_to_char_str(new_text, r.end)).unwrap_or_else(|| new_text.chars().count());
        self.set_preedit(new_text, cursor);
    }
}

/// Free-standing UTF-16 helpers for composed text (which is not stored in the buffer).
pub fn char_to_utf16_str(s: &str, ci: usize) -> usize {
    s.chars().take(ci.min(s.chars().count())).map(char::len_utf16).sum()
}
pub fn utf16_to_char_str(s: &str, u: usize) -> usize {
    let mut rem = u;
    let mut n = 0;
    for c in s.chars() {
        let w = c.len_utf16();
        if rem < w {
            return n;
        }
        rem -= w;
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_round_trip_bytes_chars_and_utf16() {
        let mut b = EditBuffer::new("a中😀z");
        assert_eq!(b.len(), 4);
        for ci in 0..=4 {
            assert_eq!(b.from_byte(b.to_byte(ci)), ci);
            assert_eq!(b.utf16_to_char(b.char_to_utf16(ci)), ci);
        }
        assert_eq!(b.char_to_utf16(4), 5); // 😀 is a surrogate pair
        b.set_text("");
        assert_eq!(b.from_byte(0), 0);
    }

    #[test]
    fn grapheme_boundaries_keep_emoji_and_marks_atomic() {
        let mut b = EditBuffer::new("a😀b");
        b.move_to(3);
        assert_eq!(b.prev_boundary(b.caret()), 2, "back over the whole emoji");
        b.move_to(1);
        assert_eq!(b.next_boundary(b.caret()), 2);
        let c = EditBuffer::new("e\u{301}x"); // e + combining acute
        assert_eq!(c.backspace_start(2), 1, "a trailing combining mark deletes on its own");
    }

    #[test]
    fn word_selection_and_motion() {
        let mut b = EditBuffer::new("alpha beta");
        b.select_word(2);
        assert_eq!(b.range(), 0..5);
        // Matches `figcraft_engine::textedit::word_left`: at 6 it skips the space and walks back
        // over "alpha" to 0; `word_right` walks "beta" to the end.
        b.move_to(6);
        assert_eq!(b.word_right(6), 10);
        assert_eq!(b.word_left(6), 0);
        assert_eq!(b.word_left(5), 0);
        assert_eq!(b.word_left(10), 6);
        // CJK scripts are words of their own (the engine's `word_class`).
        let c = EditBuffer::new("abc日本語 xyz");
        assert_eq!(c.word_right(3), 6);
        assert_eq!(c.word_left(6), 3);
    }

    #[test]
    fn composition_is_a_view_not_a_document_edit() {
        let mut b = EditBuffer::new("ab");
        b.move_to(1);
        b.set_preedit("中文", 1);
        let c = b.composed();
        assert_eq!(c.text, "a中文b");
        assert_eq!(c.caret, 2);
        assert_eq!((c.mark_start, c.mark_end), (1, 3));
        assert_eq!(b.text(), "ab", "committed text untouched");
        assert_eq!(b.marked_utf16(), Some(1..3));
        b.commit_preedit();
        assert_eq!(b.text(), "a中文b");
        assert!(!b.composing());
    }

    #[test]
    fn selected_utf16_reports_the_whole_composition() {
        let mut b = EditBuffer::new("ab");
        b.move_to(1);
        b.set_preedit("中文", 1);
        let sel = b.selected_utf16();
        // The composed view is "a中文b": the marked range is characters 1..3.
        assert_eq!(sel.range, 1..3, "platform must replace exactly the composition");
        assert!(!sel.reversed);
        assert_eq!(b.marked_utf16(), Some(1..3));
        b.commit_preedit();
        let sel = b.selected_utf16();
        assert_eq!(sel.range, 3..3, "after commit the caret is collapsed");
    }

    #[test]
    fn replace_and_mark_swaps_the_composition() {
        let mut b = EditBuffer::new("ab");
        b.move_to(1);
        b.replace_and_mark_utf16(None, "日本", Some(0..2));
        assert_eq!(b.composed().text, "a日本b");
        b.replace_and_mark_utf16(Some(1..3), "語", Some(0..1));
        assert_eq!(b.composed().text, "a語b", "platform replaces exactly the marked range");
    }

    #[test]
    fn key_outcomes_and_edits() {
        let mut b = EditBuffer::new("abcd");
        assert_eq!(b.key("enter", false, false), EditOutcome::Submit);
        assert_eq!(b.key("escape", false, false), EditOutcome::Cancel);
        b.select_all();
        b.key("x", false, false);
        assert_eq!(b.text(), "x");
        b.insert("yz");
        assert_eq!(b.text(), "xyz");
        b.delete_backward();
        assert_eq!(b.text(), "xy");
    }
}
