//! User settings, stored as JSON in %APPDATA%\which_layer\settings.json. The
//! tray menu writes it; hand edits (e.g. the hotkey) are picked up live.

use crate::visibility::Timing;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Keep the overlay on screen all the time.
    pub pinned: bool,
    /// Show the overlay while any layer above Base is active.
    pub auto_show: bool,
    /// Hide an auto-shown overlay after this long even if the layer stays on
    /// (a toggled layer). 0 = only hide on return to Base.
    pub auto_hide_secs: f32,
    /// Fade-in length when the overlay appears. Keep it short so it isn't laggy.
    pub fade_in_ms: u32,
    /// Fade-out length when it goes away.
    pub fade_out_ms: u32,
    /// How long it stays fully visible after returning to Base before fading out.
    pub hide_delay_ms: u32,
    /// 0.2 ..= 1.0
    pub opacity: f32,
    /// "bottom" or "top" of the primary monitor.
    pub position: String,
    /// Distance from that screen edge, in pixels at 100% scaling.
    pub margin: i32,
    /// Multiplier on the layout's key size.
    pub size: f32,
    /// Global hotkey that toggles the overlay, e.g. "Ctrl+Alt+L". None by default.
    pub hotkey: Option<String>,
    /// Layout arguments (layout .json, or QMK folder + keymap) remembered from
    /// the last launch that passed them, so a plain launch reopens the same board.
    pub layout: Vec<String>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            pinned: false,
            auto_show: true,
            auto_hide_secs: 5.0,
            fade_in_ms: 60,
            fade_out_ms: 150,
            hide_delay_ms: 250,
            opacity: 0.9,
            position: "bottom".into(),
            margin: 80,
            size: 1.0,
            hotkey: None,
            layout: Vec::new(),
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        base.join("which_layer").join("settings.json")
    }

    /// Missing file = defaults. Anything else that goes wrong (not UTF-8, e.g.
    /// saved as UTF-16; locked; no permission; invalid JSON) is an error, so
    /// callers never mistake a real file for "no settings" and save over it.
    /// A hard-linked file is an error too: the atomic save would split the link.
    pub fn load() -> Result<Settings, String> {
        let path = Self::path();
        match crate::read_limited(&path) {
            Ok(_) if hard_links(&path) > 1 => Err(format!(
                "{} is hard-linked, which isn't supported (saving would break the link); use a symlink instead",
                path.display()
            )),
            Ok(t) => Self::parse(&t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Settings, String> {
        let mut s: Settings = serde_json::from_str(text).map_err(|e| format!("settings.json: {e}"))?;
        s.sanitize();
        Ok(s)
    }

    /// Pulls hand-edited values back into ranges the overlay can draw.
    pub fn sanitize(&mut self) {
        let d = Settings::default();
        let clamp = |v: f32, lo: f32, hi: f32, def: f32| if v.is_finite() { v.clamp(lo, hi) } else { def };
        self.opacity = clamp(self.opacity, 0.2, 1.0, d.opacity);
        self.size = clamp(self.size, 0.25, 4.0, d.size);
        self.auto_hide_secs = clamp(self.auto_hide_secs, 0.0, 3600.0, d.auto_hide_secs);
        self.margin = self.margin.clamp(0, 4000);
        if self.position != "top" && self.position != "bottom" {
            self.position = d.position;
        }
    }

    pub fn timing(&self) -> Timing {
        Timing::from_millis(self.fade_in_ms, self.fade_out_ms, self.hide_delay_ms, self.auto_hide_secs)
    }

    /// Writes to a temp file and renames it over the old one, so a crash or
    /// power loss mid-save can't leave a truncated settings.json.
    /// A symlinked settings.json (e.g. into a dotfiles repo) is followed, so the
    /// link survives and the real file is replaced.
    pub fn save(&self) -> Result<(), String> {
        let link = Self::path();
        let path = std::fs::canonicalize(&link).unwrap_or(link);
        let tmp = path.with_extension("json.tmp");
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap())?;
            std::fs::rename(&tmp, &path).inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp);
            })
        };
        write().map_err(|e| format!("saving {}: {e}", path.display()))
    }
}

/// Number of names the file has (follows symlinks); 1 if it can't be read.
#[cfg(windows)]
fn hard_links(path: &Path) -> u32 {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    let Ok(file) = std::fs::File::open(path) else { return 1 };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return 1;
    }
    info.nNumberOfLinks
}

#[cfg(unix)]
fn hard_links(path: &Path) -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).map_or(1, |m| m.nlink() as u32)
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn hard_links_are_counted() {
        let dir = std::env::temp_dir().join(format!("which_layer_links_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.json"), dir.join("b.json"));
        std::fs::write(&a, "{}").unwrap();
        assert_eq!(super::hard_links(&a), 1);
        std::fs::hard_link(&a, &b).unwrap();
        assert_eq!(super::hard_links(&a), 2);
        assert_eq!(super::hard_links(&dir.join("missing.json")), 1);
        let _ = std::fs::remove_dir_all(&dir); // our own temp files
    }

    #[test]
    fn missing_fields_take_defaults() {
        let s = Settings::parse(r#"{ "pinned": true }"#).unwrap();
        assert!(s.pinned);
        assert_eq!(s.fade_in_ms, Settings::default().fade_in_ms);
    }

    #[test]
    fn broken_values_are_pulled_into_range() {
        let s = Settings::parse(r#"{ "opacity": 7, "size": 0, "margin": -50, "position": "left",
                                     "auto_hide_secs": -3 }"#)
            .unwrap();
        assert_eq!((s.opacity, s.size, s.margin), (1.0, 0.25, 0));
        assert_eq!(s.position, "bottom");
        assert_eq!(s.auto_hide_secs, 0.0);
    }

    #[test]
    fn invalid_json_is_an_error_not_defaults() {
        assert!(Settings::parse("{ nope").is_err());
        assert!(Settings::parse(r#"{ "opacity": "high" }"#).is_err());
    }
}
