//! CoreGraphics display plumbing. Replaces displayplacer: the "contextual
//! screen id" it printed is CGDirectDisplayID, the "persistent screen id"
//! is the CGDisplayCreateUUIDFromDisplayID UUID, and the "serial screen
//! id" (DeskPad reports s1) is CGDisplaySerialNumber.

use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_foundation::uuid::{CFUUIDRef, CFUUID};
use core_graphics::display::{CGDirectDisplayID, CGDisplay, CGDisplayMode};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGDisplayCreateUUIDFromDisplayID(display: CGDirectDisplayID) -> CFUUIDRef;
}

pub struct DisplayInfo {
    pub uuid: String,
    pub id: CGDirectDisplayID,
    pub width: u64,
    pub height: u64,
    pub serial: u32,
}

/// Every online non-builtin display, mirroring the old
/// `displayplacer list` filter.
pub fn displays() -> Vec<DisplayInfo> {
    let ids = CGDisplay::active_displays().unwrap_or_default();
    let mut out = Vec::new();
    for id in ids {
        let d = CGDisplay::new(id);
        if d.is_builtin() {
            continue;
        }
        let Some(uuid) = display_uuid(id) else {
            continue;
        };
        let (width, height) = match d.display_mode() {
            Some(m) => (m.width(), m.height()),
            None => (0, 0),
        };
        out.push(DisplayInfo {
            uuid,
            id,
            width,
            height,
            serial: d.serial_number(),
        });
    }
    out
}

fn display_uuid(id: CGDirectDisplayID) -> Option<String> {
    unsafe {
        let uuid_ref = CGDisplayCreateUUIDFromDisplayID(id);
        if uuid_ref.is_null() {
            return None;
        }
        let uuid = CFUUID::wrap_under_create_rule(uuid_ref);
        let s =
            core_foundation::uuid::CFUUIDCreateString(std::ptr::null(), uuid.as_concrete_TypeRef());
        if s.is_null() {
            return None;
        }
        Some(CFString::wrap_under_create_rule(s).to_string())
    }
}

/// Set the mode (scaling off: point size == pixel size) and arrangement
/// origin in one permanent configuration transaction, exactly what
/// `displayplacer "id:.. res:.. hz:.. scaling:off origin:.."` did.
pub fn set_mode(
    id: CGDirectDisplayID,
    width: u64,
    height: u64,
    hz: u32,
    origin: [i32; 2],
) -> Result<(), String> {
    let modes = CGDisplayMode::all_display_modes(id, std::ptr::null()).ok_or("no display modes")?;
    let rate_ok = |m: &CGDisplayMode| (m.refresh_rate() - hz as f64).abs() < 0.5;
    let size_ok = |m: &CGDisplayMode| m.width() == width && m.height() == height;
    let mode = modes
        .iter()
        .find(|m| size_ok(m) && m.pixel_width() == width && rate_ok(m))
        .or_else(|| modes.iter().find(|m| size_ok(m) && rate_ok(m)))
        .or_else(|| modes.iter().find(|m| size_ok(m)))
        .ok_or_else(|| format!("no {width}x{height} mode"))?;

    let display = CGDisplay::new(id);
    let config = display
        .begin_configuration()
        .map_err(|e| format!("begin_configuration: {e}"))?;
    let apply = display
        .configure_display_with_display_mode(&config, mode)
        .map_err(|e| format!("configure mode: {e}"))
        .and_then(|_| {
            display
                .configure_display_origin(&config, origin[0], origin[1])
                .map_err(|e| format!("configure origin: {e}"))
        });
    if let Err(e) = apply {
        let _ = display.cancel_configuration(&config);
        return Err(e);
    }
    display
        .complete_configuration(
            &config,
            core_graphics::display::CGConfigureOption::ConfigurePermanently,
        )
        .map_err(|e| format!("complete_configuration: {e}"))
}
