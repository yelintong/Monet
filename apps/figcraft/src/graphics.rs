//! Which graphics API draws the window.
//!
//! eframe offers wgpu every backend it was built with, and between equal GPUs wgpu takes Vulkan
//! first, on Windows too. A Vulkan driver that faults there takes the app down as the window opens:
//! an access violation inside the vendor's driver, which no guard of ours can catch (seen on a laptop
//! with two AMD GPUs whose drivers were years apart). Direct3D 12 is part of Windows, so on Windows
//! wgpu is offered Direct3D 12, and OpenGL for hardware without a Direct3D 12 driver.
//!
//! `WGPU_BACKEND` (`dx12`, `vulkan`, `gl`, `metal`) still names the backend on every system, and so
//! does the Preferences' Graphics renderer (`pref.graphicsBackend`), which is read when the window
//! opens and so takes effect on the next start. The variable wins over the preference: it is the
//! support escape hatch. Systems other than Windows keep eframe's defaults.

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup};
use eframe::wgpu::Backends;

/// Direct3D 12, then OpenGL: wgpu ranks a hardware GPU on either ahead of Direct3D's software one.
const WINDOWS: Backends = Backends::DX12.union(Backends::GL);

/// The backends wgpu chooses from: the ones `WGPU_BACKEND` asks for (`requested`), else [`WINDOWS`]
/// on Windows, else eframe's `defaults`.
pub fn backends(requested: Option<Backends>, windows: bool, defaults: Backends) -> Backends {
    match requested {
        Some(requested) => requested,
        None if windows => WINDOWS,
        None => defaults,
    }
}

/// The backend the Graphics renderer preference names (`backend`: `"system"`, blank or anything
/// unknown: none), by the names `WGPU_BACKEND` takes, so a settings file written by hand reads the
/// same either way.
pub fn preferred(backend: &str) -> Option<Backends> {
    match backend.trim().to_ascii_lowercase().as_str() {
        "dx12" | "d3d12" => Some(Backends::DX12),
        "vulkan" | "vk" => Some(Backends::VULKAN),
        "gl" | "opengl" | "gles" => Some(Backends::GL),
        "metal" | "mtl" => Some(Backends::METAL),
        _ => None,
    }
}

/// What named the graphics API, for the message when the window can't open: the environment
/// variable, the preference, or nothing (FigCraft's own choice). The same order [`configure`]
/// resolves them in.
pub fn named(backend: &str) -> Option<&'static str> {
    if std::env::var_os("WGPU_BACKEND").is_some() {
        return Some("WGPU_BACKEND");
    }
    preferred(backend).is_some().then_some("preference")
}

/// Offer wgpu [`backends`] instead of eframe's defaults: what `WGPU_BACKEND` asks for, else what
/// the preference names, else what the system gets.
pub fn configure(options: &mut WgpuConfiguration, backend: &str) {
    if let WgpuSetup::CreateNew(create) = &mut options.wgpu_setup {
        let offered = &mut create.instance_descriptor.backends;
        let asked = Backends::from_env().or_else(|| preferred(backend));
        *offered = backends(asked, cfg!(windows), *offered);
    }
}

/// Start this executable again with the arguments it was started with (the Preferences' restart
/// button): the new process opens its window — with the renderer now chosen — as this one ends.
/// The old process keeps running until the new one exists, so a failed start is only a log line.
pub fn relaunch() {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            log::error!("restart: where this executable is: {e}");
            return;
        }
    };
    // Without the executable's own name (args[0]); the rest is kept, `--control` included.
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match std::process::Command::new(exe).args(&args).spawn() {
        Ok(_) => log::info!("figcraft: starting again"),
        Err(e) => log::error!("restart: {e}"),
    }
}
