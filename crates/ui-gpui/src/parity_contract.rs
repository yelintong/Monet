//! The mechanism that stops "the plan says done, the code isn't" from happening again.
//!
//! 2026-11-10 复盘发现 `docs/gpui-egui-parity-plan.md` 里 A6/A7 被当作「已接好」，实际只是
//! A19 通用壳的最小版本（缺预览、缺语言/滚轮缩放/重置），A1 更是一行没写。根因是**状态由
//! 人手写、证据不会自动失效**。这里把状态变成可验证的断言：
//!
//! * [`CONTRACTS`] 是唯一事实源：每个条目声明它要求的能力 token；
//! * [`capabilities`] **只从代码派生**这些 token —— 遍历 `dialogs::Kind::ALL` 的真实 spec、
//!   读 `ui_action` 的分派结果、扫描源码里的通用件与状态读取。没有任何手写的"已完成"清单；
//! * 文档 I 节的 `PARITY_CONTRACT_STATUS` 块必须与推导结果**双向一致**：代码没做到却标
//!   `done`、或代码做到了却还标 `partial`，`cargo test` 都会失败并指名缺哪个 token；
//! * `every_dialog_command_is_a_real_engine_command` 强制所有对话框按钮/选项只推引擎真有的
//!   命令 —— 落实「尽可能使用引擎已有能力，而不是自己弄」。
//!
//! 维护规则：新增/删减一项能力时，先改 `CONTRACTS`，再改代码，最后同步文档状态块；
//! 三者不一致时测试会挡住，不需要靠人记得更新表格。

use crate::{dialogs, panels};
use serde_json::json;
use std::collections::BTreeSet;

/// One plan item and the capabilities its "done" claim depends on.
#[derive(Clone, Copy, Debug)]
pub struct Contract {
    pub id: &'static str,
    pub requires: &'static [&'static str],
}

/// The single source of truth for "what does done mean".
///
/// `A18B4` deliberately includes `detach_window`: the engine has a `window.new` request but the
/// GPUI front-end has no OS multi-window view (sharing one `Session` would need an ownership
/// refactor). Keeping the missing token in the contract is what makes the plan's `partial` status
/// honest — implementing it later flips both code and document at once.
pub const CONTRACTS: &[Contract] = &[
    Contract { id: "A19", requires: &["popover_window", "field_row", "popup_backdrop", "dropdown(", "on_drag_start", "on_drag_move"] },
    Contract { id: "B1C1", requires: &["developer_mode_label"] },
    Contract { id: "B9", requires: &["ui_flags"] },
    Contract { id: "B2", requires: &["Tab::Assets", "MAX_HITS", "recent_actions", "SEARCH_ALIASES"] },
    Contract {
        id: "A6",
        // The egui Export dialog is caption + preset rows, not an invented format/scale form.
        requires: &["file.export", "preview", "The selection as one file", "The whole file", "PNG @2x", ".fig (experimental)"],
    },
    Contract {
        id: "A7",
        requires: &["view.themeDark", "pref.language", "pref.keepTool", "pref.userName", "pref.publishServer", "pref.nudge", "pref.uiScaleReset"],
    },
    Contract { id: "A18B4", requires: &["file.close", "file.save", "detach_window"] },
    Contract { id: "A1", requires: &["home", "action_card", "home_section", "home-screen"] },
    Contract {
        id: "A2B3A27",
        // A board swaps in its own nine groups, its own tail (templates + the Design panel's
        // button) and a row of **choices** for its one-tool groups. Reverting any of that to a
        // hand-written pill or to chips removes a token and fails the contract.
        requires: &["BOARD_GROUPS", "board_panel", "board_templates", "choices(", "command_for", "board.setColor", "board.setStyle"],
    },
];

/// Every engine command a dialog can push (buttons + per-option choice commands).
fn dialog_commands() -> Vec<&'static str> {
    let empty = json!({});
    let mut out = Vec::new();
    for kind in dialogs::Kind::ALL {
        let spec = dialogs::spec(kind, &empty);
        for f in &spec.fields {
            match f {
                dialogs::Field::Choice { commands: Some(c), .. } => out.extend(c.iter().copied()),
                dialogs::Field::Action { id, .. } => out.push(*id),
                _ => {}
            }
        }
        for b in &spec.buttons {
            match &b.action {
                dialogs::Action::Submit { id, .. } | dialogs::Action::Run { id } | dialogs::Action::SubmitFields { id, .. } => out.push(*id),
                dialogs::Action::Close => {}
            }
        }
    }
    out
}

/// Capability tokens derived **from the code**, never from a hand-maintained list.
fn capabilities() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let empty = json!({});
    for kind in dialogs::Kind::ALL {
        let spec = dialogs::spec(kind, &empty);
        for f in &spec.fields {
            match f {
                dialogs::Field::Text { key, .. } | dialogs::Field::Number { key, .. } | dialogs::Field::Choice { key, .. } => {
                    out.insert((*key).to_string());
                }
                dialogs::Field::Preview { .. } => {
                    out.insert("preview".into());
                }
                dialogs::Field::Section { title } => {
                    out.insert((*title).to_string());
                }
                dialogs::Field::Action { label, id, .. } => {
                    out.insert((*label).to_string());
                    out.insert((*id).to_string());
                }
                dialogs::Field::Note { .. } => {}
            }
            if let dialogs::Field::Choice { commands: Some(c), .. } = f {
                out.extend(c.iter().map(|id| (*id).to_string()));
            }
        }
        for b in &spec.buttons {
            out.insert(b.label.to_string());
            match &b.action {
                dialogs::Action::Submit { id, .. } | dialogs::Action::Run { id } | dialogs::Action::SubmitFields { id, .. } => {
                    out.insert((*id).to_string());
                }
                dialogs::Action::Close => {}
            }
        }
    }

    // Shared chrome / state reads / palette data: source text is the evidence, so deleting the
    // real code removes the token automatically.
    let dialogs_src = include_str!("dialogs.rs");
    // `dropdown(` / `on_drag_start` / `on_drag_move` are the fidelity invariants: reverting to
    // chips or a fixed window removes the token and the A19 contract fails.
    for token in ["popover_window", "field_row", "popup_backdrop", "dropdown(", "on_drag_start", "on_drag_move"] {
        if dialogs_src.contains(token) {
            out.insert(token.into());
        }
    }
    let lib_src = include_str!("lib.rs");
    if lib_src.contains("ui_hidden") && lib_src.contains("ui_minimized") {
        out.insert("ui_flags".into());
    }
    let palette_src = include_str!("palette.rs");
    for token in ["Tab::Assets", "MAX_HITS", "recent_actions", "SEARCH_ALIASES"] {
        if palette_src.contains(token) {
            out.insert(token.into());
        }
    }
    // A1: the Home screen must keep the egui sidebar/card layout, not fall back to note()+pill().
    let extra_src = include_str!("panels/extra.rs");
    for token in ["action_card", "home_section", "home-screen"] {
        if extra_src.contains(token) {
            out.insert(token.into());
        }
    }

    // A2/B3/A27: the board toolbar and its two pickers, derived from the sources that draw them.
    let toolbar_src = include_str!("toolbar.rs");
    for token in ["BOARD_GROUPS", "board_panel", "board_templates"] {
        if toolbar_src.contains(token) {
            out.insert(token.into());
        }
    }
    let board_bar_src = include_str!("board_bar.rs");
    if board_bar_src.contains("pub(crate) fn choices(") {
        out.insert("choices(".into());
    }
    for token in ["board.setColor", "board.setStyle"] {
        if board_bar_src.contains(token) {
            out.insert(token.into());
        }
    }
    let picker_src = include_str!("board_picker.rs");
    if picker_src.contains("fn command_for") && picker_src.contains("board.template") && picker_src.contains("board.table") {
        out.insert("command_for".into());
    }

    // C1: the GPUI label is compliant AND no egui front-end UI string still uses the retired name.
    let egui_ui =
        [include_str!("../../ui-egui/src/app.rs"), include_str!("../../ui-egui/src/canvas.rs"), include_str!("../../ui-egui/src/menus.rs")].concat();
    if include_str!("naming.rs").contains("DEVELOPER_MODE: &str = \"开发\"") && !egui_ui.contains("tr(\"Dev Mode\")") {
        out.insert("developer_mode_label".into());
    }

    // A1: the engine's `home` ui_request actually reaches a panel.
    if panels::extra::ui_action("home", &empty, 0).is_some() {
        out.insert("home".into());
    }
    // A16/B4: only becomes true once the front-end really handles the engine's `window.new`
    // request. It is *not* a token we can declare by hand.
    if panels::extra::ui_action("window.new", &empty, 0).is_some() {
        out.insert("detach_window".into());
    }
    out
}

/// The document's machine-readable status block (I section of the parity plan).
fn declared_status(id: &str) -> Option<String> {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/gpui-egui-parity-plan.md"))
        .expect("the parity plan must exist next to the crates");
    let block = doc.split("<!-- PARITY_CONTRACT_STATUS").nth(1)?.split("-->").next()?;
    for line in block.lines() {
        if let Some(value) = line.trim().strip_prefix(&format!("{id}=")) {
            return Some(value.trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Use the engine's existing abilities, don't invent command names."
    #[test]
    fn every_dialog_command_is_a_real_engine_command() {
        for id in dialog_commands() {
            assert!(
                figcraft_engine::find_command(id).is_some(),
                "dialogs.rs pushes `{id}`, which is not registered in the engine — wire it to an existing command instead of inventing one"
            );
        }
    }

    /// The toolbar's board groups push `tool.{their first tool}` when a choice is picked, and the
    /// templates picker pushes `board.template` / `board.table`: a typo in any of them would
    /// silently do nothing, so every one has to be a registered engine command.
    #[test]
    fn every_board_toolbar_command_is_a_real_engine_command() {
        for g in crate::toolbar::BOARD_GROUPS {
            let id = format!("tool.{}", g.tools.first().map(|t| t.0.id()).unwrap_or("move"));
            assert!(
                figcraft_engine::find_command(&id).is_some(),
                "toolbar group `{}` pushes `{id}`, which is not registered in the engine",
                g.key
            );
        }
        let last = figcraft_engine::board_templates::TEMPLATES.len();
        for i in [0, last] {
            let (id, _) = crate::board_picker::command_for(i);
            assert!(figcraft_engine::find_command(id).is_some(), "board_picker pushes `{id}`, which is not registered in the engine");
        }
    }

    /// The core regression guard: the plan's status and the code's capabilities must agree in
    /// BOTH directions. A stale ✅ and a stale 🔄 both fail here, with the missing token named.
    #[test]
    fn parity_plan_agrees_with_the_contract() {
        let caps = capabilities();
        for c in CONTRACTS {
            let missing: Vec<&str> = c.requires.iter().copied().filter(|r| !caps.contains(*r)).collect();
            let satisfied = missing.is_empty();
            let declared = declared_status(c.id).unwrap_or_else(|| panic!("{} is missing from PARITY_CONTRACT_STATUS", c.id));
            let declared_done = declared == "done";
            assert_eq!(
                satisfied,
                declared_done,
                "{id}: code satisfied={satisfied} (missing {missing:?}), document says `{declared}`. \
                 Update the code AND the status block together — do not just edit the document.",
                id = c.id
            );
        }
    }

    #[test]
    fn contracts_are_well_formed() {
        let mut ids = BTreeSet::new();
        for c in CONTRACTS {
            assert!(ids.insert(c.id), "duplicate contract id {}", c.id);
            assert!(!c.requires.is_empty(), "{} declares no capabilities", c.id);
        }
        assert_eq!(dialogs::Kind::ALL.len(), 15, "a new dialog needs an audited contract or spec");
    }
}
