//! Preferences ▸ Graphics renderer: which graphics API draws the window. The menu names the API in
//! use for a screen reader, is reachable and changeable without a pointer, and a choice is kept as
//! a preference (it takes effect on the next start, when wgpu opens with the API chosen).

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, Popup};
use figcraft_engine::Session;

use crate::a11y::focused_name;
use crate::dialogs::renderers;
use crate::i18n::tr;
use crate::layers_drag_tests::Ui;

/// A key press, then two quiet frames for the focus (and the name a screen reader reads) to settle.
fn press(u: &mut Ui, key: Key) {
    u.frame(vec![
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
    ]);
    u.frame(vec![]);
    u.frame(vec![]);
}

/// Preferences open, the Graphics renderer menu focused (Tab walks the main window first: the
/// dialog is not modal for the keyboard).
fn open(u: &mut Ui) {
    let mut tabs = 0;
    while focused_name(&u.ctx) != "Graphics renderer" {
        assert!(tabs < 140, "Tab never reaches the Graphics renderer menu (focus: {:?})", focused_name(&u.ctx));
        press(u, Key::Tab);
        tabs += 1;
    }
    press(u, Key::Enter);
    assert!(Popup::is_any_open(&u.ctx), "Enter opens the menu");
}

#[test]
fn the_graphics_renderer_menu_takes_a_choice_and_keeps_it() {
    let mut u = Ui::new(Session::new());
    u.app.ui.preferences = true;
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    assert_eq!(u.app.session.prefs.graphics_backend, "system");

    open(&mut u);
    // The arrows enter the menu on its first renderer and walk down; Enter picks the focused one.
    press(&mut u, Key::ArrowDown);
    assert_eq!(focused_name(&u.ctx), tr(renderers()[0].1));
    press(&mut u, Key::ArrowDown);
    let (value, name) = renderers()[1];
    assert_eq!(focused_name(&u.ctx), tr(name));
    press(&mut u, Key::Enter);
    assert!(!Popup::is_any_open(&u.ctx), "picking a renderer closes the menu");
    assert!(u.app.ui.preferences, "picking a renderer leaves Preferences open");
    assert_eq!(u.app.session.prefs.graphics_backend, value);

    // The menu shows the renderer chosen, and picking it again changes nothing.
    u.frame(vec![]);
    let combo = |u: &mut Ui| {
        let out = u.frame_output(vec![]);
        let nodes: Vec<_> = out.platform_output.accesskit_update.expect("an AccessKit update").nodes.into_iter().map(|(_, n)| n).collect();
        let n = nodes.iter().find(|n| n.role() == Role::ComboBox && n.label() == Some("Graphics renderer")).expect("a Graphics renderer menu");
        (n.value().map(str::to_owned), n.is_expanded())
    };
    assert_eq!(combo(&mut u), (Some(tr(name).to_owned()), Some(false)), "the renderer chosen");
    open(&mut u);
    press(&mut u, Key::Escape);
    assert!(!Popup::is_any_open(&u.ctx), "Escape closes the menu alone");
    assert!(u.app.ui.preferences, "Preferences stays open");
    assert_eq!(u.app.session.prefs.graphics_backend, value, "Escape changes nothing");
}
