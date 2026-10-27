use crate::qmk;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tiny_skia::{Point, Transform};

/// A keyboard drawn as free-placed keys, so splits, staggers and odd shapes all
/// use the same format. Key geometry follows QMK info.json: x/y/w/h in key units,
/// absolute positions, optional rotation `r` (degrees, clockwise) around rx/ry.
#[derive(Deserialize)]
pub struct Layout {
    #[allow(dead_code)]
    pub name: String,
    /// Pixels per key unit at 100% display scaling.
    #[serde(default = "default_unit")]
    pub unit: f32,
    pub keys: Vec<Key>,
    pub layers: Vec<Layer>,
    /// Files this layout was built from, watched for live reload.
    #[serde(skip)]
    pub sources: Vec<PathBuf>,
    /// (VID, PID) from keyboard.json, used to find the board over raw HID.
    #[serde(skip)]
    pub usb: Option<(u16, u16)>,
}

#[derive(Deserialize)]
pub struct Key {
    /// QMK matrix [row, col]; how which_layer.json and the firmware name a key.
    #[serde(default)]
    pub matrix: Option<[u8; 2]>,
    pub x: f32,
    pub y: f32,
    #[serde(default = "one")]
    pub w: f32,
    #[serde(default = "one")]
    pub h: f32,
    #[serde(default)]
    pub r: f32,
    /// Rotation origin; defaults to the key's own centre.
    pub rx: Option<f32>,
    pub ry: Option<f32>,
}

/// `labels[i]` is the label for `keys[i]`. "" draws a blank key; null (or a
/// missing entry) is transparent like KC_TRNS and shows the layer below, dimmed.
/// "tap\nhold" draws the second line small at the bottom of the key.
/// `color` ("#rrggbb" or "#rrggbbaa") fills the keys this layer defines;
/// `colors[i]` overrides it per key.
#[derive(Deserialize)]
pub struct Layer {
    pub name: String,
    pub labels: Vec<Option<String>>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub colors: Vec<Option<String>>,
}

fn default_unit() -> f32 {
    54.0
}

/// Generous for any real board (two 100-key halves fit easily) while keeping a
/// worst-case repaint fast.
const MAX_KEYS: usize = 512;
/// QMK's layer_state is 32 bits.
const MAX_LAYERS: usize = 32;
const MIN_UNIT: f32 = 8.0;
const MAX_UNIT: f32 = 256.0;
/// Key positions in key units; real boards span ~25 units.
const MAX_COORD: f32 = 100.0;
const MIN_KEY_SIZE: f32 = 0.1;
const MAX_KEY_SIZE: f32 = 20.0;
/// Characters per label (a tap and hold line together); longer is cut.
const MAX_LABEL_CHARS: usize = 32;

fn truncate(s: &mut String) {
    if let Some((i, _)) = s.char_indices().nth(MAX_LABEL_CHARS) {
        s.truncate(i);
    }
}

fn one() -> f32 {
    1.0
}

impl Key {
    /// Rotation in key units.
    pub fn rotation(&self) -> Transform {
        let ox = self.rx.unwrap_or(self.x + self.w / 2.0);
        let oy = self.ry.unwrap_or(self.y + self.h / 2.0);
        Transform::from_rotate_at(self.r, ox, oy)
    }
}

impl Layout {
    /// `path` is a layout .json or a QMK keyboard folder (with `keymap`, default
    /// "default"); none loads the built-in demo.
    pub fn load(path: Option<&str>, keymap: Option<&str>) -> Result<Layout, String> {
        let layout = match path {
            Some(p) if Path::new(p).is_dir() => qmk::load(Path::new(p), keymap.unwrap_or("default"))?,
            Some(p) => {
                let text = crate::read_limited(Path::new(p)).map_err(|e| format!("{p}: {e}"))?;
                let mut l: Layout = serde_json::from_str(&text).map_err(|e| format!("{p}: {e}"))?;
                l.sources = vec![PathBuf::from(p)];
                l
            }
            None => serde_json::from_str(include_str!("../layouts/corne.json")).expect("built-in layout"),
        };
        let mut layout = layout;
        layout.validate()?;
        Ok(layout)
    }

    /// Rejects geometry the renderer can't draw sanely (non-finite, absurd
    /// sizes) and trims labels, so any layout file, however broken or hostile,
    /// gives an error instead of a crash or a multi-GB allocation.
    fn validate(&mut self) -> Result<(), String> {
        if self.keys.is_empty() || self.layers.is_empty() {
            return Err("layout needs at least one key and one layer".into());
        }
        if self.keys.len() > MAX_KEYS {
            return Err(format!("layout has {} keys (max {MAX_KEYS})", self.keys.len()));
        }
        if self.layers.len() > MAX_LAYERS {
            return Err(format!("layout has {} layers (QMK max {MAX_LAYERS})", self.layers.len()));
        }
        if !(self.unit.is_finite() && (MIN_UNIT..=MAX_UNIT).contains(&self.unit)) {
            return Err(format!("unit {} must be between {MIN_UNIT} and {MAX_UNIT}", self.unit));
        }
        for (i, k) in self.keys.iter().enumerate() {
            let coords = [Some(k.x), Some(k.y), k.rx, k.ry].into_iter().flatten();
            let ok = coords.into_iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)
                && [k.w, k.h].iter().all(|v| v.is_finite() && (MIN_KEY_SIZE..=MAX_KEY_SIZE).contains(v))
                && k.r.is_finite()
                && k.r.abs() <= 360.0;
            if !ok {
                return Err(format!("key {i} has out-of-range geometry (x/y/rx/ry within ±{MAX_COORD}, w/h {MIN_KEY_SIZE}..{MAX_KEY_SIZE}, |r| <= 360)"));
            }
        }
        for layer in &mut self.layers {
            truncate(&mut layer.name);
            for label in layer.labels.iter_mut().flatten() {
                truncate(label);
            }
        }
        Ok(())
    }

    /// Label for key `i` given QMK's `layer_state` bitmask (layer 0 always counts
    /// as active). Transparent entries fall through to the next *active* lower
    /// layer, like KC_TRNS. The bool is false when the label came from below the
    /// top active layer.
    pub fn label(&self, active: u32, i: usize) -> (&str, bool) {
        let active = active | 1;
        let top = self.top_layer(active);
        for l in (0..=top).rev() {
            if active & (1 << l) == 0 {
                continue;
            }
            if let Some(Some(s)) = self.layers[l].labels.get(i) {
                return (s, l == top);
            }
        }
        ("", false)
    }

    /// Highest active layer this layout has (layers it doesn't define are ignored).
    pub fn top_layer(&self, active: u32) -> usize {
        let defined = if self.layers.len() >= 32 { u32::MAX } else { (1u32 << self.layers.len()) - 1 };
        31 - ((active | 1) & defined).leading_zeros() as usize
    }

    /// Bounding box of all (rotated) keys in key units: (min_x, min_y, max_x, max_y).
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for k in &self.keys {
            let mut corners = [
                Point::from_xy(k.x, k.y),
                Point::from_xy(k.x + k.w, k.y),
                Point::from_xy(k.x, k.y + k.h),
                Point::from_xy(k.x + k.w, k.y + k.h),
            ];
            k.rotation().map_points(&mut corners);
            for p in corners {
                b = (b.0.min(p.x), b.1.min(p.y), b.2.max(p.x), b.3.max(p.y));
            }
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(json: &str) -> Layout {
        serde_json::from_str(json).unwrap()
    }

    fn three_layers() -> Layout {
        layout(
            r#"{ "name": "t", "keys": [{"x":0,"y":0},{"x":1,"y":0}], "layers": [
                { "name": "Base", "labels": ["A", "B"] },
                { "name": "One",  "labels": ["1", null] },
                { "name": "Two",  "labels": [null, null] } ] }"#,
        )
    }

    #[test]
    fn transparent_keys_fall_through_active_layers_only() {
        let l = three_layers();
        assert_eq!(l.label(0b010, 0), ("1", true));
        assert_eq!(l.label(0b010, 1), ("B", false));
        // Layer 2 on top of layer 1: falls through to 1, not straight to base.
        assert_eq!(l.label(0b110, 0), ("1", false));
        // Layer 2 alone: layer 1 isn't active, so base shows.
        assert_eq!(l.label(0b100, 0), ("A", false));
        assert_eq!(l.label(0, 0), ("A", true), "empty state means base");
    }

    #[test]
    fn top_layer_ignores_layers_the_layout_lacks() {
        let l = three_layers();
        assert_eq!(l.top_layer(0), 0);
        assert_eq!(l.top_layer(0b100), 2);
        assert_eq!(l.top_layer(0b1000_0010), 1, "layer 7 undefined, layer 1 active");
        assert_eq!(l.top_layer(u32::MAX), 2);
    }

    #[test]
    fn bounds_include_rotated_keys() {
        let l = layout(r#"{ "name": "t", "keys": [{"x":0,"y":0,"r":90}], "layers": [{ "name": "B", "labels": [] }] }"#);
        let (x0, y0, x1, y1) = l.bounds();
        assert!((x0 - 0.0).abs() < 1e-4 && (y0 - 0.0).abs() < 1e-4 && (x1 - 1.0).abs() < 1e-4 && (y1 - 1.0).abs() < 1e-4);
        let l = layout(r#"{ "name": "t", "keys": [{"x":0,"y":0,"w":2,"r":90}], "layers": [{ "name": "B", "labels": [] }] }"#);
        let (x0, y0, x1, y1) = l.bounds();
        assert!((x1 - x0 - 1.0).abs() < 1e-4 && (y1 - y0 - 2.0).abs() < 1e-4, "2u key stood on end");
    }

    fn validate(json: &str) -> Result<Layout, String> {
        let mut l: Layout = serde_json::from_str(json).map_err(|e| e.to_string())?;
        l.validate().map(|_| l)
    }

    #[test]
    fn hostile_geometry_is_rejected_not_rendered() {
        let key = |k: &str| format!(r#"{{ "name": "t", "keys": [{k}], "layers": [{{ "name": "B", "labels": ["A"] }}] }}"#);
        assert!(validate(&key(r#"{"x":0,"y":0}"#)).is_ok());
        for bad in [
            r#"{"x":1000,"y":0}"#,
            r#"{"x":1e39,"y":0}"#,
            r#"{"x":0,"y":0,"r":1e39}"#,
            r#"{"x":0,"y":0,"w":0}"#,
            r#"{"x":0,"y":0,"h":-1}"#,
            r#"{"x":0,"y":0,"rx":-500}"#,
        ] {
            assert!(validate(&key(bad)).is_err(), "{bad}");
        }
        let unit = |u: &str| format!(r#"{{ "name": "t", "unit": {u}, "keys": [{{"x":0,"y":0}}], "layers": [{{ "name": "B", "labels": [] }}] }}"#);
        for bad in ["0", "-54", "1e39", "100000"] {
            assert!(validate(&unit(bad)).is_err(), "unit {bad}");
        }
        assert!(validate(r#"{ "name": "t", "keys": [], "layers": [{ "name": "B", "labels": [] }] }"#).is_err());
    }

    #[test]
    fn long_labels_are_truncated() {
        let long = "é".repeat(10_000);
        let l = validate(&format!(r#"{{ "name": "t", "keys": [{{"x":0,"y":0}}], "layers": [{{ "name": "B", "labels": ["{long}"] }}] }}"#)).unwrap();
        assert_eq!(l.layers[0].labels[0].as_ref().unwrap().chars().count(), MAX_LABEL_CHARS);
    }

    #[test]
    fn built_in_layout_loads() {
        let l = Layout::load(None, None).unwrap();
        assert_eq!(l.keys.len(), 42);
        assert!(l.sources.is_empty());
    }
}
