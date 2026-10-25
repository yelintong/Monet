//! The backends offered to wgpu: no Vulkan on Windows unless it is asked for (a faulting Vulkan
//! driver crashed the app as its window opened), and nothing changed anywhere else.

use eframe::egui_wgpu::WgpuConfiguration;
use eframe::wgpu::Backends;

use crate::graphics::{backends, configure, named, preferred};

/// What eframe offers when `WGPU_BACKEND` isn't set.
const DEFAULTS: Backends = Backends::PRIMARY.union(Backends::GL);

#[test]
fn windows_is_offered_direct3d_and_not_vulkan() {
    let offered = backends(None, true, DEFAULTS);
    assert!(offered.contains(Backends::DX12), "Direct3D 12 draws the window");
    assert!(offered.contains(Backends::GL), "OpenGL is there for hardware without a Direct3D 12 driver");
    assert!(!offered.contains(Backends::VULKAN), "a Vulkan driver that faults can't be picked");
}

#[test]
fn wgpu_backend_names_the_backend_on_every_system() {
    for windows in [true, false] {
        for asked in [Backends::VULKAN, Backends::DX12, Backends::GL, Backends::VULKAN | Backends::GL] {
            assert_eq!(backends(Some(asked), windows, DEFAULTS), asked, "windows: {windows}");
        }
    }
}

#[test]
fn other_systems_keep_eframes_defaults() {
    assert_eq!(backends(None, false, DEFAULTS), DEFAULTS);
    assert_eq!(backends(None, false, Backends::METAL), Backends::METAL);
}

#[test]
fn the_preference_names_one_backend_and_system_names_none() {
    for (asked, want) in [
        ("dx12", Backends::DX12),
        ("vulkan", Backends::VULKAN),
        ("gl", Backends::GL),
        ("metal", Backends::METAL),
        ("DX12", Backends::DX12),
        (" vulkan ", Backends::VULKAN),
        // The names `WGPU_BACKEND` takes, so a settings file written by hand reads the same.
        ("d3d12", Backends::DX12),
        ("opengl", Backends::GL),
    ] {
        assert_eq!(preferred(asked), Some(want), "{asked}");
    }
    for blank in ["", "system", "webgpu", "dx12, gl", "direct3d"] {
        assert_eq!(preferred(blank), None, "{blank}");
    }
}

#[test]
fn the_preference_is_what_the_window_opens_with() {
    // Whatever the system, a named backend is the one offered: the platform default stands back.
    let mut options = WgpuConfiguration::default();
    configure(&mut options, "vulkan");
    assert_eq!(offered(&options), Backends::VULKAN);
    // "system" (and anything unknown) leaves the platform's own choice in place.
    let mut options = WgpuConfiguration::default();
    configure(&mut options, "system");
    assert_eq!(offered(&options), if cfg!(windows) { Backends::DX12 | Backends::GL } else { DEFAULTS });
}

#[test]
fn the_environment_variable_is_what_named_the_api() {
    // The variable is only read here, never set: a test that set it would race the other tests.
    match std::env::var_os("WGPU_BACKEND") {
        Some(_) => assert_eq!(named("dx12"), Some("WGPU_BACKEND"), "the variable wins over the preference"),
        None => {
            assert_eq!(named("dx12"), Some("preference"), "the preference named the API");
            assert_eq!(named("system"), None, "FigCraft's own choice names nothing");
        }
    }
}

/// The backends a configuration offers wgpu.
fn offered(options: &WgpuConfiguration) -> Backends {
    match &options.wgpu_setup {
        eframe::egui_wgpu::WgpuSetup::CreateNew(create) => create.instance_descriptor.backends,
        eframe::egui_wgpu::WgpuSetup::Existing { .. } => panic!("the default configuration creates a new instance"),
    }
}
