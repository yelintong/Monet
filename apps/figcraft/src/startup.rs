//! When the window can't open.
//!
//! Release builds on Windows have no console, so an error from `eframe::run_native` (no usable
//! graphics adapter, a surface or device the driver refuses) went to a standard error nobody sees
//! and the app quit without a word (#261). Now it is logged and shown in a message box.

use std::path::Path;

/// Log the failure and, unless `quiet` (started for an agent with `--control`: a modal box would
/// keep the caller waiting, and it reads standard error), show it in a message box. `named` is what
/// chose the graphics API (`WGPU_BACKEND`, or the preference), and `settings` where the
/// preferences live, so one chosen there can be undone without the window.
pub fn failed(error: &eframe::Error, quiet: bool, named: Option<&str>, settings: Option<&Path>) {
    // In the interface language: the text and its catalog entries are the UI crate's.
    let text = figcraft_ui_egui::dialogs::window_failed(&error.to_string(), named, settings);
    // Standard error even when RUST_LOG turned errors off.
    if log::log_enabled!(log::Level::Error) {
        log::error!("{text}");
    } else {
        eprintln!("figcraft: {text}");
    }
    if quiet {
        return;
    }
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("FigCraft")
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
