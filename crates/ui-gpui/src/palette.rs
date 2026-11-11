//! B2: the Actions palette's **data** — three tabs, Recents, grouped commands, prefix-first
//! ranking and a 60-row cap.
//!
//! Port of `ui-egui/src/menus/palette.rs`'s `palette_sections` / `command_sections` /
//! `plugin_rows` / `widget_rows` / `asset_sections` / `recent_row`. This module owns no GPUI
//! widget: it turns the engine's catalog and the session's preferences into plain data, and
//! [`items`] maps that data onto the shared [`crate::menu_card::Item`] row model. Nothing under
//! `widgets/**` (FROZEN) is touched.
//!
//! A row's `key` is the EGUI-style action key (`command:<id>`, `component:<node>`,
//! `style:<id>`, `plugin:<plugin>/<command>`, `widget:<plugin>`); `CanvasView::pick_palette`
//! turns it back into an engine command.

use figcraft_i18n::tr;
use figcraft_engine::Session;
use figcraft_engine::catalog::{MENU, SEP};
use std::collections::HashSet;

/// The Actions palette's tab (egui `PaletteTab`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    All,
    Assets,
    Plugins,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::All, Tab::Assets, Tab::Plugins];

    /// The labels keep the EGUI wording; the third tab is deliberately wider than the others.
    pub fn label(self) -> &'static str {
        match self {
            Tab::All => tr("All"),
            Tab::Assets => tr("Assets"),
            Tab::Plugins => tr("Plugins & widgets"),
        }
    }

    pub fn from_index(i: usize) -> Tab {
        Tab::ALL.get(i).copied().unwrap_or_default()
    }
}

/// Most rows a search shows (the best matches). Without a query the menu's own order is kept and
/// the cap does not apply — same as EGUI.
pub const MAX_HITS: usize = 60;

/// One pickable palette row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: String,
    pub label: String,
    pub hint: Option<String>,
    pub disabled: bool,
}

/// A named group of rows (`Recents`, a top-level menu, `Components`, …).
#[derive(Clone, Debug)]
pub struct Section {
    pub name: String,
    pub rows: Vec<Row>,
}

impl Section {
    fn new(name: impl Into<String>, rows: Vec<Row>) -> Self {
        Self { name: name.into(), rows }
    }
}

/// Whether `label` (or `extra`) matches the query, in any case.
fn matches(q: &str, label: &str, extra: &str) -> bool {
    q.is_empty() || label.to_lowercase().contains(q) || extra.to_lowercase().contains(q)
}

/// `label` as two rows that read the same compare: any case, without a trailing ellipsis.
fn same_label(label: &str) -> String {
    label.trim().trim_end_matches(['…', '.']).trim_end().to_lowercase()
}

/// Retired names the engine still wants searchable (`view.devMode` → "Dev Mode").
fn aliases(id: &str) -> String {
    figcraft_engine::catalog::SEARCH_ALIASES.iter().filter(|(cmd, _)| *cmd == id).map(|(_, alias)| *alias).collect::<Vec<_>>().join(" ")
}

/// A command's row: its label, and its shortcut as the hint. A command the engine doesn't
/// register stays visible but disabled (GPUI's existing behaviour; EGUI hides it instead).
fn command_row(id: &str, label: String) -> Row {
    Row { key: format!("command:{id}"), label, hint: crate::main_menu::shortcut_text(id), disabled: figcraft_engine::find_command(id).is_none() }
}

fn menu_items() -> impl Iterator<Item = &'static figcraft_engine::catalog::MenuItem> {
    MENU.iter().filter(|m| m.label != SEP && !m.id.is_empty())
}

/// Commands grouped by top-level menu, filtered by the query. Without one, the menus' items in
/// catalog order; a search covers every engine command too (except those needing parameters),
/// "Other" for those no menu lists, best matches first (labels starting with the query, then
/// the shortest). The Plugins menu's items come with [`plugin_rows`].
pub fn command_sections(q: &str) -> Vec<Section> {
    let ql = q.trim().to_lowercase();
    let menu_of: std::collections::HashMap<&str, &'static figcraft_engine::catalog::MenuItem> =
        menu_items().fold(std::collections::HashMap::new(), |mut map, m| {
            map.entry(m.id).or_insert(m);
            map
        });

    // (id, english label, the menu entry that lists it — if any)
    let mut candidates: Vec<(&str, &str)> = Vec::new();
    for m in menu_items() {
        candidates.push((m.id, m.label));
    }
    if !ql.is_empty() {
        for c in figcraft_engine::all_commands().iter().filter(|c| !figcraft_engine::cmd::needs_params(c.id)) {
            candidates.push((c.id, c.label));
        }
    }

    let mut seen = HashSet::new();
    // The labels menu items show: a command no menu lists that reads the same is left out.
    let mut menu_labels: HashSet<String> = HashSet::new();
    // By top-level menu; `None`: no menu lists it.
    let mut hits: Vec<(Option<String>, Row)> = Vec::new();
    for (id, english) in candidates {
        if !seen.insert(id) {
            continue;
        }
        let menu = menu_of.get(id).copied();
        let group = menu.map(|m| m.menu.split('/').next().unwrap_or(m.menu));
        if group == Some(tr("Plugins")) {
            continue;
        }
        let label = match menu {
            Some(m) => figcraft_i18n::menu_item(id, m.label).into_owned(),
            None => figcraft_i18n::command(id, english).into_owned(),
        };
        let same = same_label(&label);
        if menu.is_some() {
            menu_labels.insert(same);
        } else if menu_labels.contains(&same) {
            continue;
        }
        let extra = format!("{english} {id} {}", aliases(id));
        if matches(&ql, &label, &extra) {
            hits.push((group.map(str::to_string), command_row(id, label)));
        }
    }
    if !ql.is_empty() {
        hits.sort_by_cached_key(|(_, row)| (!row.label.to_lowercase().starts_with(&ql), row.label.chars().count()));
        hits.truncate(MAX_HITS);
    }
    let mut groups: Vec<Section> = Vec::new();
    for (group, row) in hits {
        let name = group.unwrap_or_else(|| tr("Other").to_string());
        match groups.iter_mut().find(|s| s.name == name) {
            Some(s) => s.rows.push(row),
            None => groups.push(Section::new(name, vec![row])),
        }
    }
    groups
}

/// A recent action back as a row (gone commands, plugins or assets drop out).
fn recent_rows(session: &Session, q: &str) -> Vec<Row> {
    let ql = q.to_lowercase();
    session
        .prefs
        .recent_actions
        .iter()
        .filter_map(|key| {
            if let Some(id) = key.strip_prefix("command:") {
                let menu_label = menu_items().find(|m| m.id == id);
                let english = menu_label.map(|m| m.label).or_else(|| figcraft_engine::find_command(id).map(|s| s.label)).unwrap_or(id);
                let label = figcraft_i18n::command(id, english).into_owned();
                let extra = format!("{english} {id} {}", aliases(id));
                if !matches(&ql, &label, &extra) {
                    return None;
                }
                return Some(command_row(id, label));
            }
            if let Some(rest) = key.strip_prefix("plugin:") {
                let (pid, cid) = rest.split_once('/')?;
                let p = session.prefs.plugins.iter().find(|p| p.manifest.id == pid)?;
                let c = p.manifest.commands.iter().find(|c| c.id == cid)?;
                let label = if p.manifest.commands.len() == 1 { p.manifest.name.clone() } else { format!("{} — {}", p.manifest.name, c.label) };
                if !matches(&ql, &label, pid) {
                    return None;
                }
                return Some(Row { key: key.clone(), label, hint: Some(tr("Plugin").into()), disabled: false });
            }
            if let Some(pid) = key.strip_prefix("widget:") {
                let p = session.prefs.plugins.iter().find(|p| p.manifest.id == pid && p.manifest.widget)?;
                if !matches(&ql, &p.manifest.name, pid) {
                    return None;
                }
                return Some(Row { key: key.clone(), label: p.manifest.name.clone(), hint: Some(tr("Widget").into()), disabled: false });
            }
            None
        })
        .collect()
}

/// Installed plugins' commands, filtered by the query, plus the Plugins menu's own items.
fn plugin_rows(session: &Session, q: &str) -> Vec<Row> {
    let ql = q.to_lowercase();
    let mut rows = Vec::new();
    for p in &session.prefs.plugins {
        let m = &p.manifest;
        for c in &m.commands {
            let label = if m.commands.len() == 1 { m.name.clone() } else { format!("{} — {}", m.name, c.label) };
            if !matches(&ql, &label, &m.id) {
                continue;
            }
            rows.push(Row { key: format!("plugin:{}/{}", m.id, c.id), label, hint: Some("Plugin".into()), disabled: false });
        }
    }
    for m in menu_items().filter(|m| m.menu.split('/').next() == Some(tr("Plugins"))) {
        let label = figcraft_i18n::menu_item(m.id, m.label).into_owned();
        let extra = format!("{} {}", m.label, m.id);
        if matches(&ql, &label, &extra) {
            rows.push(command_row(m.id, label));
        }
    }
    rows
}

/// Installed widgets, filtered by the query.
fn widget_rows(session: &Session, q: &str) -> Vec<Row> {
    let ql = q.to_lowercase();
    session
        .prefs
        .plugins
        .iter()
        .filter(|p| p.manifest.widget)
        .filter(|p| matches(&ql, &p.manifest.name, &p.manifest.id))
        .map(|p| Row { key: format!("widget:{}", p.manifest.id), label: p.manifest.name.clone(), hint: Some("Widget".into()), disabled: false })
        .collect()
}

/// This file's components and styles, filtered by the query.
fn asset_sections(session: &Session, q: &str) -> Vec<Section> {
    let ql = q.to_lowercase();
    let Ok(ds) = session.ds() else { return Vec::new() };
    let doc = &ds.doc;
    let components: Vec<Row> = figcraft_engine::library::local_components(doc)
        .iter()
        .filter_map(|id| doc.node(*id))
        .filter(|n| matches(&ql, &n.name, ""))
        .map(|n| Row { key: format!("component:{}", n.id.0), label: n.name.clone(), hint: Some("Component".into()), disabled: false })
        .collect();
    let styles: Vec<Row> = figcraft_engine::library::local_styles(doc)
        .filter(|s| matches(&ql, &s.name, ""))
        .map(|s| Row { key: format!("style:{}", s.id), label: s.name.clone(), hint: Some("Style".into()), disabled: false })
        .collect();
    let mut out = Vec::new();
    if !components.is_empty() {
        out.push(Section::new(tr("Components"), components));
    }
    if !styles.is_empty() {
        out.push(Section::new(tr("Styles"), styles));
    }
    out
}

/// The rows a palette tab shows for the query, by section. All has Recents, the commands by
/// menu, then plugins and widgets; Assets this file's components and styles; Plugins & widgets
/// just those two.
pub fn sections(q: &str, tab: Tab, session: &Session) -> Vec<Section> {
    let mut out = match tab {
        Tab::All => {
            let mut v = vec![Section::new(tr("Recents"), recent_rows(session, q))];
            v.extend(command_sections(q));
            v
        }
        Tab::Assets => asset_sections(session, q),
        Tab::Plugins => Vec::new(),
    };
    if tab != Tab::Assets {
        out.push(Section::new(tr("Plugins"), plugin_rows(session, q)));
        out.push(Section::new(tr("Widgets"), widget_rows(session, q)));
    }
    out.retain(|s| !s.rows.is_empty());
    out
}

/// Flat rows for the shared menu-card control: one disabled header per section, then its rows.
/// The action key lives in `Item::key`, so a dynamic `component:` / `style:` / `plugin:` row can
/// travel through the same `OnPick` as a static command id.
pub fn items(sections: &[Section]) -> Vec<crate::menu_card::Item> {
    let mut out = Vec::new();
    for (i, s) in sections.iter().enumerate() {
        let mut head = crate::menu_card::Item::new("", s.name.clone()).disabled(true);
        if i > 0 {
            head = head.separator_before();
        }
        out.push(head);
        for r in &s.rows {
            let mut item = crate::menu_card::Item::new("", r.label.clone()).disabled(r.disabled).key(r.key.clone());
            if let Some(hint) = &r.hint {
                item = item.trail(hint.clone());
            }
            out.push(item);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_cover_all_three_labels_and_default_to_all() {
        assert_eq!(Tab::default(), Tab::All);
        assert_eq!(Tab::ALL.len(), 3);
        assert_eq!(Tab::ALL.map(Tab::label), ["All", "Assets", "Plugins & widgets"]);
        assert_eq!(Tab::from_index(2), Tab::Plugins);
        assert_eq!(Tab::from_index(99), Tab::All);
    }

    #[test]
    fn empty_query_keeps_menu_order_and_groups_by_top_level_menu() {
        let groups = command_sections("");
        assert!(!groups.is_empty(), "the menu catalog must yield rows");
        assert!(groups.iter().any(|s| s.name == "File"), "File is a top-level menu");
        assert!(!groups.iter().any(|s| s.name == "Plugins"), "plugin menu items get their own section");
        assert!(groups.iter().all(|s| !s.rows.is_empty()));
    }

    #[test]
    fn a_search_caps_the_hits_and_puts_prefix_matches_first() {
        let groups = command_sections("save");
        let rows: Vec<&Row> = groups.iter().flat_map(|s| s.rows.iter()).collect();
        assert!(!rows.is_empty());
        assert!(rows.len() <= MAX_HITS, "{} rows exceeds the cap", rows.len());
        // A label starting with the query outranks one that merely contains it.
        let first_prefix = rows.iter().position(|r| r.label.to_lowercase().starts_with("save"));
        let first_infix = rows.iter().position(|r| !r.label.to_lowercase().starts_with("save"));
        if let (Some(p), Some(i)) = (first_prefix, first_infix) {
            assert!(p < i, "prefix matches must come before infix matches");
        }
    }

    #[test]
    fn a_broad_search_never_exceeds_sixty_rows() {
        let rows = command_sections("e").iter().map(|s| s.rows.len()).sum::<usize>();
        assert!(rows > 0);
        assert!(rows <= MAX_HITS, "broad search showed {rows} rows");
    }

    #[test]
    fn retired_names_stay_searchable_through_engine_aliases() {
        assert!(aliases("view.devMode").contains("Dev Mode"));
        let groups = command_sections("dev mode");
        let rows: Vec<&Row> = groups.iter().flat_map(|s| s.rows.iter()).collect();
        assert!(rows.iter().any(|r| r.key == "command:view.devMode"), "the retired name must still find the command");
    }

    /// A menu item runs the UI flow (Place image opens the file dialog), so it may need
    /// parameters; only the engine-only candidates must run with none. Same rule as EGUI's
    /// `a_search_offers_only_what_runs_and_one_row_per_label`.
    #[test]
    fn engine_only_commands_need_no_parameters_and_do_not_duplicate_menu_labels() {
        for q in ["place", "image", "comment", "tool", "export", "page", "a"] {
            let groups = command_sections(q);
            let rows: Vec<&Row> = groups.iter().flat_map(|s| s.rows.iter()).collect();
            for r in &rows {
                let Some(id) = r.key.strip_prefix("command:") else { continue };
                if MENU.iter().any(|m| m.id == id) {
                    continue;
                }
                assert!(!figcraft_engine::cmd::needs_params(id), "{q:?} offers {id}, which needs parameters");
                let twins = rows.iter().filter(|o| same_label(&o.label) == same_label(&r.label)).count();
                assert_eq!(twins, 1, "{q:?} offers {} twice", r.label);
            }
        }
    }

    #[test]
    fn items_turn_sections_into_headers_and_keyed_rows() {
        let sections =
            vec![Section::new(tr("Recents"), vec![Row { key: "command:file.save".into(), label: tr("Save").into(), hint: None, disabled: false }])];
        let items = items(&sections);
        assert_eq!(items.len(), 2);
        assert!(items[0].disabled, "the section header is not pickable");
        assert_eq!(items[0].label.as_ref(), "Recents");
        assert_eq!(items[1].key.as_ref(), "command:file.save");
    }

    #[test]
    fn same_label_ignores_case_and_a_trailing_ellipsis() {
        assert_eq!(same_label("Export…"), same_label(" export "));
        assert_ne!(same_label("Save"), same_label("Save as"));
    }
}
