//! User-visible feature names, kept in one place so a rename cannot drift between the right
//! panel, the toolbar, the a11y tree and the naming regression tests.
//!
//! Policy: `docs/feature-names.md` (#2135 / #2165). The old label is a registered trademark of
//! another product, so **no display label or translation may use it**. Command ids, preference
//! keys and palette search aliases keep their old spelling — they are identifiers, not labels,
//! and the Actions palette still has to find the command when someone types the old name.
//!
//! This module is deliberately UI-free (no GPUI imports): every front-end surface reads the same
//! constants, and the tests below run without opening a window.

/// The Design tab.
pub const DESIGN: &str = "Design";
/// The Prototype tab.
pub const PROTOTYPE: &str = "Prototype";
/// The dev-mode label shown in the toolbar (mode switcher) and as the right-panel tab-row text
/// while the Dev-Mode inspector is open. Was "Inspect" in GPUI, "Dev Mode" in the egui front-end;
/// both are retired (trademark + the user's 2026-11-10 request to label it 开发).
pub const DEVELOPER_MODE: &str = "开发";

/// The command id — an identifier, deliberately unchanged by the rename.
#[cfg(test)]
pub const DEV_MODE_COMMAND: &str = "view.devMode";

/// Right-panel tab labels, in tab order. `a11y` reads this instead of a hard-coded string.
pub const RIGHT_TAB_LABELS: [&str; 3] = [DESIGN, PROTOTYPE, DEVELOPER_MODE];

/// The label of a right-panel tab, by index. Unknown indices fall back to Design rather than
/// panicking (this is shipped UI code — no `unwrap`/`expect`/`panic!`).
pub(crate) fn right_tab_label(tab: u8) -> &'static str {
    RIGHT_TAB_LABELS.get(usize::from(tab)).copied().unwrap_or(DESIGN)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Retired display names; a single one reappearing is a legal/compliance regression.
    const RETIRED: &[&str] = &["Dev Mode", "DEV MODE", "dev mode", "Inspect"];

    #[test]
    fn third_right_tab_is_developer_mode() {
        assert_eq!(right_tab_label(0), "Design");
        assert_eq!(right_tab_label(1), "Prototype");
        assert_eq!(right_tab_label(2), "开发");
        assert_eq!(right_tab_label(255), "Design", "an unknown tab must not panic");
    }

    #[test]
    fn labels_never_use_the_retired_name() {
        for label in RIGHT_TAB_LABELS {
            for retired in RETIRED {
                assert!(!label.contains(retired), "{label:?} still contains {retired:?}");
            }
        }
        assert_eq!(RIGHT_TAB_LABELS[2], DEVELOPER_MODE);
    }

    #[test]
    fn labels_do_not_use_comparative_product_wording() {
        for label in RIGHT_TAB_LABELS {
            let lower = label.to_ascii_lowercase();
            assert!(!lower.contains("figma"), "{label:?} compares with another product");
            assert!(!lower.contains("like "), "{label:?} compares with another product");
        }
    }

    #[test]
    fn identifiers_and_search_aliases_survive_the_rename() {
        assert_eq!(DEV_MODE_COMMAND, "view.devMode");
        // The palette must still find the command when the user types the retired name; the
        // engine owns that alias table, so the test only asserts the id is still searchable.
        assert!(
            figcraft_engine::catalog::SEARCH_ALIASES.iter().any(|(id, _)| *id == DEV_MODE_COMMAND),
            "the command id must stay in the palette search aliases"
        );
    }
}
