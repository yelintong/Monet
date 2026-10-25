//! The message shown when the window can't open: it carries the error, a next step and where to
//! report it, instead of the app quitting silently (#261).
//!
//! The app takes the interface language from the preferences, and without one from the system's;
//! the tests that start an app set it for the whole process, so these assertions go through `tr`
//! for the very keys the message uses: they hold in whichever language is current, and they still
//! say which hint the message picked.

use figcraft_ui_egui::dialogs::window_failed as message;
use figcraft_ui_egui::i18n::tr;

const ERROR: &str = "WGPU error: Failed to create surface for any enabled backend: {}";

/// The hint a failed start picks when nothing names a graphics API: the driver, and OpenGL on
/// Windows.
fn driver_hint() -> &'static str {
    if cfg!(windows) {
        tr("Updating the graphics driver often helps. To use OpenGL only, start FigCraft from a Command Prompt with:")
    } else {
        tr("Updating the graphics driver often helps. WGPU_BACKEND (vulkan, gl, metal) names another graphics API to try.")
    }
}

/// The hint a failed start picks when `WGPU_BACKEND` named the API.
fn unset_hint() -> &'static str {
    tr("WGPU_BACKEND is set and names the graphics API to use; unsetting it lets FigCraft choose.")
}

#[test]
fn the_message_carries_the_error_and_where_to_report_it() {
    let text = message(ERROR, None, None);
    assert!(text.contains(ERROR), "the error itself: {text}");
    assert!(text.contains("https://github.com/storytold/figcraft/issues"), "where to report it: {text}");
}

#[test]
fn without_wgpu_backend_it_suggests_the_driver_and_another_graphics_api() {
    let text = message(ERROR, None, None);
    assert!(text.contains(driver_hint()), "{text}");
    assert!(text.contains("WGPU_BACKEND"), "another graphics API to try: {text}");
    assert!(!text.contains(unset_hint()), "nothing named the API, so nothing is unset: {text}");
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
    assert!(text.contains(unset_hint()), "{text}");
    assert!(!text.contains(driver_hint()), "the chosen backend is the likelier cause: {text}");
}

#[test]
fn a_renderer_chosen_in_preferences_is_undone_there_or_in_the_settings_file() {
    // The window won't open, so Preferences is out of reach: the settings file is the way back.
    let text = message(ERROR, Some("preference"), Some(std::path::Path::new("/home/me/.config/FigCraft/state.json")));
    let chosen = tr("The Graphics renderer preference names the graphics API to use; setting it back to System in Preferences lets FigCraft choose.");
    assert!(text.contains(chosen), "{text}");
    assert!(text.contains("graphics_backend"), "{text}");
    assert!(text.contains("/home/me/.config/FigCraft/state.json"), "{text}");
    assert!(!text.contains(unset_hint()), "the preference is not unset, it is set back: {text}");
}
