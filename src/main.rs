// Release builds are tray apps with no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("Windows only for now; Linux gets a Wayland layer-shell window.rs later.");

mod bench;
mod hid;
mod keycodes;
mod layout;
mod qmk;
mod render;
mod settings;
mod tray;
mod visibility;
mod window;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use layout::Layout;
use render::Renderer;
use settings::Settings;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};
use visibility::Visibility;
use window::Overlay;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

/// How often the layout and settings files are checked for edits.
const RELOAD_POLL: Duration = Duration::from_secs(1);

const HELP: &str = "Simulated keyboard: <n> = switch to layer n, s = show, h = hide, \
c = create window, d = destroy window, m = memory, q = quit";

/// Events from the keyboard, tray, hotkey and (debug builds) the console simulator.
pub enum Cmd {
    /// Simulator: only this layer (plus base) active.
    Layer(usize),
    /// Keyboard: QMK layer_state bitmask.
    LayerState(u32),
    Show,
    Hide,
    /// Tray icon click or hotkey.
    Toggle,
    /// Tray menu item, by id.
    Menu(String),
    Create,
    Destroy,
    Mem,
    Quit,
}

pub struct App {
    /// [path] [keymap] for the layout, for reloads and the bench.
    layout_args: Vec<String>,
    layout: Layout,
    settings: Settings,
    stamps: Vec<Option<SystemTime>>,
    next_poll: Instant,
    renderer: Renderer,
    /// Monitor DPI scale; the render scale also includes settings.size.
    dpi: f32,
    /// QMK layer_state bitmask; the highest set bit is the layer shown.
    state: u32,
    overlay: Option<Overlay>,
    /// Decides when the overlay is on screen and how faded it is.
    vis: Visibility,
    /// What the real window is doing, mirrored from `vis` by `sync_window`.
    window_shown: bool,
    window_opacity: f32,
    tray: Option<tray::Tray>,
    /// Handed to the tray once the event loop is running.
    tray_proxy: Option<EventLoopProxy<Cmd>>,
    hotkeys: Option<GlobalHotKeyManager>,
    hotkey: Option<HotKey>,
    hid_target: hid::Target,
    errors: Errors,
    started: bool,
    probe: bool,
    bench: Option<bench::Bench>,
}

impl App {
    fn scale(&self) -> f32 {
        self.dpi * self.settings.size
    }

    /// Creates the (hidden) window if needed, with the current layer painted.
    fn ensure_window(&mut self, el: &ActiveEventLoop) {
        if self.overlay.is_some() {
            return;
        }
        let (w, h) = Renderer::size(&self.layout, self.scale());
        let margin = (self.settings.margin as f32 * self.dpi) as i32;
        self.overlay = Some(Overlay::create(el, w, h, self.settings.position == "top", margin));
        self.paint(self.window_opacity);
    }

    /// Renders the current layer into the window at `opacity`.
    fn paint(&mut self, opacity: f32) {
        let scale = self.scale();
        let Some(overlay) = &mut self.overlay else { return };
        let pm = self.renderer.render(&self.layout, self.state, scale);
        overlay.present(&pm, opacity);
    }

    /// Mirrors `vis` onto the real window: shows/hides it and steps fades.
    fn sync_window(&mut self, now: Instant) {
        self.vis.tick(now);
        let opacity = self.settings.opacity * self.vis.alpha(now);
        match (self.vis.on_screen(), self.window_shown) {
            (true, false) => self.show_at(opacity),
            (true, true) if opacity != self.window_opacity => {
                if let Some(o) = &self.overlay {
                    o.set_opacity(opacity);
                }
            }
            (false, true) => self.hide(),
            _ => {}
        }
        self.window_opacity = opacity;
    }

    /// Bench: put the window up instantly at full opacity.
    pub fn create_show(&mut self, el: &ActiveEventLoop) {
        self.ensure_window(el);
        self.show();
    }

    pub fn show(&mut self) {
        self.show_at(self.settings.opacity);
    }

    /// Paints first (the layer may have changed while hidden), then shows.
    fn show_at(&mut self, opacity: f32) {
        self.paint(opacity);
        if let Some(overlay) = &self.overlay {
            overlay.show();
            self.window_shown = true;
        }
    }

    pub fn hide(&mut self) {
        if let Some(overlay) = &self.overlay {
            overlay.hide();
        }
        self.window_shown = false;
    }

    pub fn destroy(&mut self) {
        self.overlay = None;
        self.window_shown = false;
    }

    /// Recreates the window (new size or position), keeping it on screen if it was.
    fn rebuild_window(&mut self, el: &ActiveEventLoop) {
        if self.overlay.is_none() {
            return;
        }
        let shown = self.window_shown;
        self.destroy();
        self.ensure_window(el);
        if shown {
            self.show_at(self.window_opacity);
        }
    }

    /// New layer state, repainting if on screen. No auto show/hide (the bench uses this).
    pub fn apply_state(&mut self, state: u32) {
        self.state = state;
        if self.window_shown {
            self.paint(self.window_opacity);
        }
    }

    /// New layer state from the keyboard; `vis` decides about showing.
    fn set_state(&mut self, state: u32) {
        self.apply_state(state);
        self.vis.layer_state(state, Instant::now());
    }

    pub fn top_layer(&self) -> usize {
        self.layout.top_layer(self.state)
    }

    fn toggle(&mut self, el: &ActiveEventLoop) {
        // Unpinning saves settings: pick up any hand edit first so it isn't lost.
        self.reload_if_changed(el);
        self.vis.toggle(Instant::now());
        if self.settings.pinned && !self.vis.pinned {
            self.settings.pinned = false; // hiding a pinned overlay unpins it
            self.sync_settings();
        }
    }

    /// Applies a new settings value: window, timing, hotkey, tray check marks, file.
    fn update_settings(&mut self, new: Settings, el: &ActiveEventLoop) {
        let old = std::mem::replace(&mut self.settings, new);
        let s = self.settings.clone();
        if (s.size, s.margin, &s.position) != (old.size, old.margin, &old.position) {
            self.rebuild_window(el);
        } else if s.opacity != old.opacity && self.window_shown {
            self.window_opacity = s.opacity * self.vis.alpha(Instant::now());
            self.paint(self.window_opacity);
        }
        self.vis.set_timing(s.timing());
        self.vis.auto_show = s.auto_show;
        if s.pinned != self.vis.pinned {
            self.vis.set_pinned(s.pinned, Instant::now());
        }
        if s.hotkey != old.hotkey {
            self.register_hotkey();
        }
        self.sync_settings();
    }

    /// Mirrors settings into the tray and saves them, but never over a file the
    /// user is mid-way through fixing (it doesn't parse) — that file wins.
    fn sync_settings(&mut self) {
        if let Some(tray) = &self.tray {
            tray.sync(&self.settings);
        }
        self.errors.settings = match Settings::load() {
            Ok(disk) if disk != self.settings => self.settings.save().err(),
            Ok(_) => None,
            Err(e) => Some(format!("{e} (fix it to save tray changes)")),
        };
        self.show_errors();
    }

    fn register_hotkey(&mut self) {
        self.errors.hotkey = None;
        if let Some(manager) = &self.hotkeys {
            if let Some(old) = self.hotkey.take() {
                let _ = manager.unregister(old);
            }
            if let Some(text) = &self.settings.hotkey {
                let registered = text.parse::<HotKey>().map_err(|e| e.to_string()).and_then(|hk| {
                    manager.register(hk).map(|_| hk).map_err(|e| e.to_string())
                });
                match registered {
                    Ok(hk) => self.hotkey = Some(hk),
                    Err(e) => self.errors.hotkey = Some(format!("hotkey {text}: {e}")),
                }
            }
        }
        self.show_errors();
    }

    fn menu(&mut self, id: &str, el: &ActiveEventLoop) {
        // Pick up a hand edit made since the last poll, so this click builds on it.
        self.reload_if_changed(el);
        let mut s = self.settings.clone();
        match id {
            "toggle" => return self.toggle(el),
            "quit" => return el.exit(),
            "autostart" => {
                tray::set_autostart(!tray::autostart_enabled());
                return self.sync_settings();
            }
            "settings" => {
                if !Settings::path().exists() {
                    self.errors.settings = self.settings.save().err();
                }
                open_file(&Settings::path());
                return;
            }
            "pinned" => s.pinned = !s.pinned,
            "auto_show" => s.auto_show = !s.auto_show,
            _ => match id.split_once(':') {
                Some(("opacity", v)) => s.opacity = v.parse::<f32>().unwrap_or(100.0) / 100.0,
                Some(("size", v)) => s.size = v.parse::<f32>().unwrap_or(100.0) / 100.0,
                Some(("position", v)) => s.position = v.into(),
                _ => return,
            },
        }
        self.update_settings(s, el);
    }

    /// Reloads the layout and settings if their files changed. A broken edit
    /// keeps what's on screen and shows the error in the tray tooltip.
    fn reload_if_changed(&mut self, el: &ActiveEventLoop) {
        let current = stamps(&self.watched());
        if current == self.stamps {
            return;
        }
        self.stamps = current;
        match Settings::load() {
            Ok(s) if s != self.settings => {
                self.errors.settings = None;
                self.update_settings(s, el);
            }
            Ok(_) => self.errors.settings = None,
            Err(e) => self.errors.settings = Some(e),
        }
        // The layout follows settings: a fixed settings.json, or a second launch
        // handing over a new board, switches the running overlay.
        if !self.settings.layout.is_empty() && self.settings.layout != self.layout_args {
            self.layout_args = self.settings.layout.clone();
        }
        let layout = load_layout(&self.layout_args);
        self.errors.layout = layout.as_ref().err().map(|e| format!("keeping previous layout: {e}"));
        self.show_errors();
        let Ok(layout) = layout else { return };
        let resized = Renderer::size(&layout, self.scale()) != Renderer::size(&self.layout, self.scale());
        self.layout = layout;
        // A different board (VID/PID) is picked up on the HID thread's next scan.
        *self.hid_target.lock().unwrap_or_else(|e| e.into_inner()) = self.layout.usb;
        // Reloading also refreshes stamps for files that just appeared/vanished.
        self.stamps = stamps(&self.watched());
        if resized {
            self.rebuild_window(el);
        } else if self.window_shown {
            self.paint(self.window_opacity);
        }
    }

    fn watched(&self) -> Vec<PathBuf> {
        let mut paths = self.layout.sources.clone();
        paths.push(Settings::path());
        paths
    }

    /// Puts the most important current problem in the tray tooltip (or clears it).
    fn show_errors(&self) {
        let e = &self.errors;
        let error = e.layout.as_ref().or(e.settings.as_ref()).or(e.hotkey.as_ref());
        if let Some(error) = error {
            eprintln!("{error}");
        }
        if let Some(tray) = &self.tray {
            tray.set_error(error.map(String::as_str));
        }
    }
}

/// Problems shown in the tray tooltip. Each source sets or clears its own, so
/// one succeeding can't hide another's error.
#[derive(Default)]
struct Errors {
    layout: Option<String>,
    settings: Option<String>,
    hotkey: Option<String>,
}

/// Opens a file with its default app (ShellExecute, not a command line), or in
/// Notepad if nothing is associated with it.
fn open_file(path: &std::path::Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain([0]).collect::<Vec<u16>>();
    let shell = |verb: &str, file: &std::ffi::OsStr, params: Option<&std::ffi::OsStr>| {
        let (verb, file, params) = (wide(verb.as_ref()), wide(file), params.map(wide));
        let params = params.as_ref().map_or(std::ptr::null(), |p| p.as_ptr());
        let result = unsafe {
            ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), params, std::ptr::null(), SW_SHOWNORMAL)
        };
        result as isize > 32 // documented ShellExecute convention for success
    };
    if !shell("open", path.as_os_str(), None) {
        let windir = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
        let notepad = std::path::Path::new(&windir).join("System32").join("notepad.exe");
        let mut quoted = std::ffi::OsString::from("\""); // parameters are a command line
        quoted.push(path.as_os_str());
        quoted.push("\"");
        shell("open", notepad.as_os_str(), Some(&quoted));
    }
}

/// Biggest input file read. Real keymaps/layouts are a few KB; this stops a
/// huge or hostile file from being slurped into memory and parsed.
const MAX_INPUT_BYTES: u64 = 4 * 1024 * 1024;

/// `fs::read_to_string` with a size cap (checked while reading, not before, so
/// a file growing underneath can't sneak past).
pub fn read_limited(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)?.take(MAX_INPUT_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_INPUT_BYTES {
        let msg = format!("larger than {} MB", MAX_INPUT_BYTES / 1024 / 1024);
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, msg));
    }
    Ok(text)
}

fn load_layout(args: &[String]) -> Result<Layout, String> {
    Layout::load(args.first().map(String::as_str), args.get(1).map(String::as_str))
}

fn stamps(paths: &[PathBuf]) -> Vec<Option<SystemTime>> {
    paths.iter().map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()).collect()
}

impl ApplicationHandler<Cmd> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.started {
            return;
        }
        self.started = true;
        self.dpi = el.primary_monitor().map_or(1.0, |m| m.scale_factor() as f32);
        if self.bench.is_some() {
            return; // the bench drives everything from about_to_wait
        }
        if self.probe {
            self.create_show(el);
            window::wait_for_compositor();
            let (ws, private) = bench::mem();
            println!("ready {ws} {private}");
            return el.exit();
        }
        // Tray and hotkey need the event loop running on this thread.
        let proxy = self.tray_proxy.take().expect("proxy for tray");
        self.tray = Some(tray::Tray::new(proxy.clone()));
        self.hotkeys = GlobalHotKeyManager::new().map_err(|e| eprintln!("hotkeys: {e}")).ok();
        let proxy = Mutex::new(proxy);
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed {
                let _ = proxy.lock().unwrap().send_event(Cmd::Toggle);
            }
        }));
        self.register_hotkey();
        self.sync_settings();
        self.ensure_window(el);
        self.sync_window(Instant::now()); // shows it if pinned
        println!("{HELP}");
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}

    fn user_event(&mut self, el: &ActiveEventLoop, cmd: Cmd) {
        let layer_change = matches!(cmd, Cmd::Layer(_) | Cmd::LayerState(_));
        match cmd {
            Cmd::Layer(n) => self.set_state(1 << (n % self.layout.layers.len().min(32))),
            Cmd::LayerState(state) => self.set_state(state),
            Cmd::Show => self.vis.show(Instant::now()),
            Cmd::Hide => self.vis.hide(Instant::now(), false),
            Cmd::Toggle => self.toggle(el),
            Cmd::Menu(id) => self.menu(&id, el),
            Cmd::Create if self.overlay.is_none() => self.create_show(el),
            Cmd::Create => println!("window already exists"),
            Cmd::Destroy => self.destroy(),
            Cmd::Mem => {
                let (ws, private) = bench::mem();
                println!("working set {}, private {}", bench::mb(ws), bench::mb(private));
            }
            Cmd::Quit => el.exit(),
        }
        if layer_change {
            let top = self.top_layer();
            println!("layer state {:#b} -> showing {top} ({})", self.state, self.layout.layers[top].name);
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(mut b) = self.bench.take() {
            if !b.tick(self, el) {
                self.bench = Some(b);
            }
            return;
        }
        if self.probe {
            return;
        }
        // Runs after every batch of events, so this is where `vis` reaches the window.
        let now = Instant::now();
        if now >= self.next_poll {
            self.reload_if_changed(el);
            self.next_poll = now + RELOAD_POLL;
        }
        self.sync_window(now);
        let wake = self.vis.next_wake(now).map_or(self.next_poll, |t| t.min(self.next_poll));
        el.set_control_flow(ControlFlow::WaitUntil(wake));
    }
}

fn spawn_stdin(proxy: EventLoopProxy<Cmd>) {
    std::thread::spawn(move || {
        for line in std::io::stdin().lines() {
            let Ok(line) = line else { break };
            let cmd = match line.trim() {
                "s" => Cmd::Show,
                "h" => Cmd::Hide,
                "c" => Cmd::Create,
                "d" => Cmd::Destroy,
                "m" => Cmd::Mem,
                "q" => Cmd::Quit,
                t => match t.parse() {
                    Ok(n) => Cmd::Layer(n),
                    Err(_) => {
                        println!("{HELP}");
                        continue;
                    }
                },
            };
            if proxy.send_event(cmd).is_err() {
                break;
            }
        }
    });
}

fn message_box(msg: &str) {
    show_box(msg, windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR);
}

fn warning_box(msg: &str) {
    show_box(msg, windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONWARNING);
}

/// First launch: no keyboard remembered yet. Asks for a QMK keyboard folder
/// until one loads; cancelling shows the built-in demo layout instead.
fn pick_keyboard() -> Option<Vec<String>> {
    warning_box(
        "No keyboard is set up yet.\n\nPick your keyboard's folder in your QMK firmware: the one with \
         keyboard.json (or info.json) and a keymaps folder, e.g. qmk_firmware\\keyboards\\crkbd.",
    );
    loop {
        let dir = browse_folder("Pick your keyboard's QMK folder (the one with keyboard.json and keymaps)")?;
        let keymaps = qmk::keymaps(std::path::Path::new(&dir));
        let Some(keymap) = keymaps.first().cloned() else {
            warning_box(&format!("{dir} has no keymaps folder.\n\nPick the keyboard's own folder."));
            continue;
        };
        let args = vec![dir, keymap];
        if let Err(e) = load_layout(&args) {
            warning_box(&format!("{e}\n\nPick another folder."));
            continue;
        }
        if keymaps.len() > 1 {
            let others = keymaps[1..].join(", ");
            show_box(
                &format!(
                    "Showing the \"{}\" keymap. This keyboard also has: {others}.\n\nTo use another one, open the \
                     settings file from the tray icon and change the keymap name (the second \"layout\" entry).",
                    args[1]
                ),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONINFORMATION,
            );
        }
        return Some(args);
    }
}

/// Folder picker; None if cancelled.
fn browse_folder(title: &str) -> Option<String> {
    use windows_sys::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
    use windows_sys::Win32::UI::Shell::{
        SHBrowseForFolderW, SHGetPathFromIDListW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW,
    };
    let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
    let mut path = [0u16; 260]; // MAX_PATH, what SHGetPathFromIDListW writes at most
    unsafe {
        // The resizable new-style dialog needs COM on this thread.
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as _);
        let info = BROWSEINFOW {
            lpszTitle: title.as_ptr(),
            ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
            ..std::mem::zeroed()
        };
        let pidl = SHBrowseForFolderW(&info);
        if pidl.is_null() {
            return None;
        }
        let ok = SHGetPathFromIDListW(pidl, path.as_mut_ptr());
        CoTaskMemFree(pidl.cast());
        let len = path.iter().position(|&c| c == 0)?;
        (ok != 0).then(|| String::from_utf16_lossy(&path[..len]))
    }
}

fn show_box(msg: &str, icon: u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW;
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    unsafe { MessageBoxW(std::ptr::null_mut(), wide(msg).as_ptr(), wide("which_layer").as_ptr(), icon) };
}

/// Fatal startup error: a message box, since release builds have no console.
fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    message_box(msg);
    std::process::exit(1);
}

/// Release builds have no console, so a panic would vanish silently (and with
/// autostart, repeat at every login). Log it next to settings.json and say so.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let log = Settings::path().with_file_name("crash.log");
        let entry = format!("{:?}: {info}\n", SystemTime::now());
        let _ = std::fs::create_dir_all(log.parent().unwrap());
        let _ = std::fs::OpenOptions::new().create(true).append(true).open(&log).and_then(|mut f| {
            use std::io::Write;
            f.write_all(entry.as_bytes())
        });
        if !cfg!(debug_assertions) {
            // Shown from its own thread so the modal loop can't re-enter winit
            // on a thread that is mid-panic.
            let msg = format!("which_layer crashed: {info}\n\nDetails were saved to {}", log.display());
            let _ = std::thread::spawn(move || message_box(&msg)).join();
        }
        // Whichever thread panicked, the app is now incomplete (e.g. no keyboard
        // link): exit rather than run on half-alive.
        std::process::exit(1);
    }));
}

/// The current user's SID ("S-1-5-21-…"). Unlike %USERNAME% it can't be
/// changed by whoever launches us, and differs between same-named accounts.
fn user_sid() -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        // TOKEN_USER plus the SID it points to; u64s keep the buffer aligned.
        let mut buf = [0u64; 64];
        let mut len = 0;
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), size_of_val(&buf) as u32, &mut len);
        CloseHandle(token);
        if ok == 0 {
            return None;
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut wide: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut wide) == 0 {
            return None;
        }
        let n = (0..).take_while(|&i| *wide.add(i) != 0).count();
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(wide, n));
        LocalFree(wide.cast());
        Some(sid)
    }
}

/// True if another copy is already running (autostart + a manual launch).
fn already_running() -> bool {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let user = user_sid().unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_default());
    let name: Vec<u16> = format!("Local\\which_layer_single_instance_{user}").encode_utf16().chain([0]).collect();
    // The handle is deliberately never closed: it lives as long as the process.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    !handle.is_null() && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS
}

/// Usage: which_layer [bench|probe] [layout.json | <qmk keyboard folder> [keymap]]
/// With no layout given, reopens the one remembered in settings.
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match args.first().map(String::as_str) {
        Some("bench" | "probe") => Some(args.remove(0)),
        _ => None,
    };
    if mode.as_deref() == Some("bench") {
        // A GUI-subsystem exe has no console; borrow the terminal it was run from.
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }

    install_panic_hook();
    if mode.is_none() && already_running() {
        // Hand a new layout to the running copy: it follows settings.layout.
        let warning = match Settings::load() {
            Ok(mut s) if !args.is_empty() && s.layout != args => {
                s.layout = args;
                // Saved: the running copy switches layouts, which is cue enough.
                s.save().err().map(|e| format!("which_layer is already running, and the new layout couldn't be passed to it: {e}"))
            }
            Err(e) if !args.is_empty() => Some(format!("which_layer is already running; fix {e} to switch layouts.")),
            _ => Some("which_layer is already running (see its tray icon).".into()),
        };
        if let Some(w) = warning {
            warning_box(&w);
        }
        return;
    }

    // A broken settings.json shouldn't stop the app (e.g. at login): run on
    // defaults, leave the file alone, and show the problem in the tray.
    let mut errors = Errors::default();
    let mut settings = Settings::load().unwrap_or_else(|e| {
        if mode.is_none() {
            // The tooltip alone is easy to miss. Own thread: the app keeps starting.
            let msg = format!("{e}\n\nRunning on default settings until it's fixed; settings won't be saved.");
            std::thread::spawn(move || warning_box(&msg));
        }
        errors.settings = Some(format!("{e} (using defaults)"));
        Settings::default()
    });
    if mode.is_none() {
        if args.is_empty() {
            args = settings.layout.clone();
            // First launch (a broken settings file already got its own popup).
            if args.is_empty() && errors.settings.is_none() {
                if let Some(picked) = pick_keyboard() {
                    settings.layout = picked.clone();
                    errors.settings = settings.save().err();
                    args = picked;
                }
            }
        } else if args != settings.layout && errors.settings.is_none() {
            settings.layout = args.clone();
            errors.settings = settings.save().err();
        }
    }
    let layout = load_layout(&args).unwrap_or_else(|e| fail(&e));

    let event_loop = EventLoop::<Cmd>::with_user_event().build().expect("event loop");
    let hid_target: hid::Target = std::sync::Arc::new(Mutex::new(layout.usb));
    if mode.is_none() {
        if cfg!(debug_assertions) {
            spawn_stdin(event_loop.create_proxy());
        }
        hid::spawn(event_loop.create_proxy(), hid_target.clone());
    }
    let vis = Visibility::new(settings.timing(), settings.pinned, settings.auto_show);
    let mut app = App {
        layout,
        settings,
        vis,
        stamps: Vec::new(),
        next_poll: Instant::now(),
        renderer: Renderer::new().unwrap_or_else(|e| fail(&e)),
        dpi: 1.0,
        state: 1,
        overlay: None,
        window_shown: false,
        window_opacity: 0.0,
        tray: None,
        tray_proxy: Some(event_loop.create_proxy()),
        hotkeys: None,
        hotkey: None,
        hid_target,
        errors,
        started: false,
        probe: mode.as_deref() == Some("probe"),
        bench: (mode.as_deref() == Some("bench")).then(|| bench::Bench::new(args.clone())),
        layout_args: args,
    };
    app.stamps = stamps(&app.watched());
    event_loop.run_app(&mut app).expect("run event loop");
}

#[cfg(test)]
mod tests {
    #[test]
    fn single_instance_name_uses_the_user_sid() {
        let sid = super::user_sid().expect("SID from the process token");
        assert!(sid.starts_with("S-1-") && sid.len() > 8, "{sid}");
    }
}
