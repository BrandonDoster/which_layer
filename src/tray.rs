//! Tray icon + menu, and the "start with Windows" Run-key entry. Menu clicks and
//! icon clicks are forwarded to the event loop as `Cmd`s.

use crate::settings::Settings;
use crate::Cmd;
use std::sync::Mutex;
use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::*;
use winit::event_loop::EventLoopProxy;

const OPACITIES: [u32; 4] = [100, 90, 75, 50];
const SIZES: [u32; 4] = [75, 100, 125, 150];
const POSITIONS: [(&str, &str); 2] = [("bottom", "Bottom"), ("top", "Top")];

pub struct Tray {
    icon: TrayIcon,
    /// Every check item, by menu id, so `sync` can mirror the settings.
    checks: Vec<(String, CheckMenuItem)>,
}

impl Tray {
    pub fn new(proxy: EventLoopProxy<Cmd>) -> Tray {
        let mut checks = Vec::new();
        let mut check = |id: String, text: &str| {
            let item = CheckMenuItem::with_id(id.as_str(), text, true, false, None);
            checks.push((id, item.clone()));
            item
        };
        let pinned = check("pinned".into(), "Keep on screen");
        let auto_show = check("auto_show".into(), "Show on layer change");
        let autostart = check("autostart".into(), "Start with Windows");
        let opacity: Vec<_> = OPACITIES.iter().map(|o| check(format!("opacity:{o}"), &format!("{o}%"))).collect();
        let size: Vec<_> = SIZES.iter().map(|s| check(format!("size:{s}"), &format!("{s}%"))).collect();
        let position: Vec<_> = POSITIONS.iter().map(|(id, text)| check(format!("position:{id}"), text)).collect();

        let submenu = |text: &str, items: &[CheckMenuItem]| {
            let items: Vec<&dyn IsMenuItem> = items.iter().map(|i| i as &dyn IsMenuItem).collect();
            Submenu::with_items(text, true, &items).expect("tray submenu")
        };
        let menu = Menu::with_items(&[
            &MenuItem::with_id("toggle", "Show / hide", true, None),
            &pinned,
            &auto_show,
            &PredefinedMenuItem::separator(),
            &submenu("Opacity", &opacity),
            &submenu("Size", &size),
            &submenu("Position", &position),
            &PredefinedMenuItem::separator(),
            &autostart,
            &MenuItem::with_id("settings", "Open settings file (hotkey, timing)", true, None),
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("quit", "Quit", true, None),
        ])
        .expect("tray menu");

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("which_layer")
            .with_icon(icon())
            .build()
            .expect("tray icon");

        // The handlers run on other threads; the proxy is only shared behind a lock.
        let proxy = std::sync::Arc::new(Mutex::new(proxy));
        let p = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = p.lock().unwrap().send_event(Cmd::Menu(e.id.0));
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                let _ = proxy.lock().unwrap().send_event(Cmd::Toggle);
            }
        }));

        Tray { icon, checks }
    }

    /// Mirrors the settings into the menu's check marks.
    pub fn sync(&self, s: &Settings) {
        let on = |id: &str| match id {
            "pinned" => s.pinned,
            "auto_show" => s.auto_show,
            "autostart" => autostart_enabled(),
            _ => match id.split_once(':') {
                Some(("opacity", v)) => v.parse() == Ok((s.opacity * 100.0).round() as u32),
                Some(("size", v)) => v.parse() == Ok((s.size * 100.0).round() as u32),
                Some(("position", v)) => v == s.position,
                _ => false,
            },
        };
        for (id, item) in &self.checks {
            item.set_checked(on(id));
        }
    }

    /// Shows an error (e.g. a broken keymap edit) in the tooltip; None clears it.
    pub fn set_error(&self, error: Option<&str>) {
        let text = match error {
            Some(e) => format!("which_layer: {e}"),
            None => "which_layer".into(),
        };
        // Windows caps tooltips at 127 characters.
        let _ = self.icon.set_tooltip(Some(text.chars().take(127).collect::<String>()));
    }
}

/// The exe's own icon (assets/icon.ico, embedded by build.rs as resource 1),
/// at the tray's size for the current DPI so Windows picks a sharp frame.
fn icon() -> Icon {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
    let px = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    Icon::from_resource(1, Some((px, px))).expect("icon resource (build.rs)")
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "which_layer";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// The Run-key command line: the quoted exe path, no arguments (a plain launch
/// reopens the layout remembered in settings).
fn run_command() -> Option<String> {
    std::env::current_exe().ok().map(|exe| format!("\"{}\"", exe.display()))
}

/// On only if the Run entry exists *and* launches this exe; an entry left by a
/// copy that has since moved counts as off, so toggling repairs it.
pub fn autostart_enabled() -> bool {
    let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
    let get = |buf: *mut u16, bytes: &mut u32| unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.cast(), bytes)
    };
    // Ask for the size first so long paths work; the value can change between
    // the two calls, so a second failure just reads as "off".
    let mut bytes = 0u32;
    if get(std::ptr::null_mut(), &mut bytes) != ERROR_SUCCESS || bytes > 64 * 1024 {
        return false;
    }
    let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
    if get(buf.as_mut_ptr(), &mut bytes) != ERROR_SUCCESS {
        return false;
    }
    let len = (bytes as usize / 2).min(buf.len());
    let stored = String::from_utf16_lossy(&buf[..len]);
    run_command().is_some_and(|cmd| stored.trim_end_matches('\0').eq_ignore_ascii_case(&cmd))
}

/// Adds (pointing at this exe) or removes the HKCU Run entry.
pub fn set_autostart(on: bool) {
    let (key, value) = (wide(RUN_KEY), wide(RUN_VALUE));
    let status = unsafe {
        if on {
            let Some(cmd) = run_command() else { return };
            let data = wide(&cmd);
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr())
        }
    };
    if status != ERROR_SUCCESS {
        eprintln!("start with Windows: registry error {status}");
    }
}
