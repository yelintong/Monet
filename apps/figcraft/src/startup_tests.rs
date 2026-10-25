//! The message shown when the window can't open: it carries the error, a next step and where to
//! report it, instead of the app quitting silently (#261).

use figcraft_ui_egui::dialogs::window_failed as message;

const ERROR: &str = "WGPU error: Failed to create surface for any enabled backend: {}";

#[test]
fn the_message_carries_the_error_and_where_to_report_it() {
    let text = message(ERROR, None, None);
    assert!(text.contains(ERROR), "the error itself: {text}");
    assert!(text.contains("https://github.com/storytold/figcraft/issues"), "where to report it: {text}");
}

#[test]
fn without_wgpu_backend_it_suggests_the_driver_and_another_graphics_api() {
    let text = message(ERROR, None, None);
    assert!(text.contains("graphics driver"), "{text}");
    assert!(text.contains("WGPU_BACKEND"), "another graphics API to try: {text}");
    assert!(!text.contains("unsetting"), "{text}");
}

#[cfg(windows)]
#[test]
fn on_windows_it_shows_how_to_start_with_opengl() {
    let text = message(ERROR, None, None);
    assert!(text.contains("set WGPU_BACKEND=gl"), "a release build has no console to look this up from: {text}");
}

#[test]
fn with_wgpu_backend_set_it_suggests_unsetting_it() {
    let text = message(ERROR, Some("WGPU_BACKEND"), None);
    assert!(text.contains("unsetting"), "{text}");
    assert!(!text.contains("graphics driver"), "the chosen backend is the likelier cause: {text}");
}

#[test]
fn a_renderer_chosen_in_preferences_is_undone_there_or_in_the_settings_file() {
    // The window won't open, so Preferences is out of reach: the settings file is the way back.
    let text = message(ERROR, Some("preference"), Some(std::path::Path::new("/home/me/.config/FigCraft/state.json")));
    assert!(text.contains("Graphics renderer"), "{text}");
    assert!(text.contains("back to System in Preferences"), "{text}");
    assert!(text.contains("graphics_backend"), "{text}");
    assert!(text.contains("/home/me/.config/FigCraft/state.json"), "{text}");
    assert!(!text.contains("unsetting"), "the preference is not unset, it is set back: {text}");
}
