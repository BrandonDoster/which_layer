use crate::layout::Layout;
use fontdue::{Font, FontSettings, Metrics};
use std::cell::RefCell;
use std::collections::HashMap;
use tiny_skia::{
    Color, FillRule, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Point, PremultipliedColorU8, Transform,
};

/// Padding around the keys and height of the layer-name strip, in key units.
const PAD: f32 = 0.25;
const TITLE: f32 = 0.6;
/// Gap between neighbouring keys, in key units.
const GAP: f32 = 0.05;
/// Height of the toggle status strip under the keys, when there is one.
const STATUS: f32 = 0.6;
/// Largest overlay side in pixels (about a 4K monitor); bigger layouts are
/// drawn smaller instead, which also bounds memory per repaint (~64 MB).
const MAX_PX: u32 = 4096;

type GlyphKey = (char, u32, u8); // char, font px bits, alpha

pub struct Renderer {
    font: Font,
    glyphs: RefCell<HashMap<GlyphKey, (Metrics, Option<Pixmap>)>>,
}

/// What to draw besides the layer.
#[derive(Clone, Copy)]
pub struct View<'a> {
    /// The keyboard itself; false = only the popup, on a clear background.
    pub keys: bool,
    /// Status strip: (toggle name, on). Empty = no strip (and a shorter overlay).
    pub status: &'a [(String, bool)],
    /// Big key in the middle: (toggle name, on).
    pub popup: Option<(&'a str, bool)>,
}

impl Renderer {
    pub fn new() -> Result<Renderer, String> {
        Ok(Renderer { font: load_font()?, glyphs: RefCell::default() })
    }

    /// Height of everything but the keys, in key units.
    fn chrome(status: bool) -> f32 {
        2.0 * PAD + TITLE + if status { STATUS } else { 0.0 }
    }

    /// Pixels per key unit: `unit * scale`, shrunk if needed so the overlay
    /// never exceeds MAX_PX on a side.
    fn unit_px(layout: &Layout, scale: f32, status: bool) -> f32 {
        let (x0, y0, x1, y1) = layout.bounds();
        let span = (x1 - x0 + 2.0 * PAD).max(y1 - y0 + Self::chrome(status));
        let u = layout.unit * scale;
        if u.is_finite() && span.is_finite() && span > 0.0 {
            u.min(MAX_PX as f32 / span)
        } else {
            1.0
        }
    }

    /// Overlay size in pixels; `status` adds the toggle strip.
    pub fn size(layout: &Layout, scale: f32, status: bool) -> (u32, u32) {
        let (x0, y0, x1, y1) = layout.bounds();
        let u = Self::unit_px(layout, scale, status);
        let px = |units: f32| ((units * u).ceil() as u32).clamp(1, MAX_PX);
        (px(x1 - x0 + 2.0 * PAD), px(y1 - y0 + Self::chrome(status)))
    }

    /// `state` is QMK's layer_state bitmask; the highest active layer is drawn.
    pub fn render(&self, layout: &Layout, state: u32, scale: f32, view: View) -> Pixmap {
        let status = !view.status.is_empty();
        let (w, h) = Self::size(layout, scale, status);
        // size() keeps both sides within 1..=MAX_PX, which Pixmap always accepts.
        let mut pm = Pixmap::new(w, h).expect("overlay size within 1..=MAX_PX");
        let u = Self::unit_px(layout, scale, status);
        if view.keys {
            self.draw_keys(&mut pm, layout, state, u);
            if status {
                let (x0, y0, x1, y1) = layout.bounds();
                let top = (PAD + TITLE + y1 - y0 + 0.1) * u;
                self.draw_status(&mut pm, view.status, PAD * u, top, (x1 - x0) * u, (STATUS - 0.1) * u);
            }
        }
        if let Some((name, on)) = view.popup {
            self.draw_popup(&mut pm, name, on, u);
        }
        pm
    }

    /// One pill per toggle across the strip: green when on, dim when off.
    fn draw_status(&self, pm: &mut Pixmap, items: &[(String, bool)], x: f32, y: f32, w: f32, h: f32) {
        let slot = w / items.len() as f32;
        let pill_w = (slot - 0.15 * h).min(6.0 * h);
        for (i, (name, on)) in items.iter().enumerate() {
            let cx = x + slot * (i as f32 + 0.5);
            let (color, alpha) = if *on { (Color::from_rgba8(22, 163, 74, 235), 255) } else { (Color::from_rgba8(40, 40, 48, 200), 130) };
            let pill = round_rect(cx - pill_w / 2.0, y, pill_w, h, h / 2.0);
            fill(pm, &pill, color, Transform::identity());
            self.text(pm, name, cx, y + h / 2.0, 0.55 * h, pill_w - 0.4 * h, alpha);
        }
    }

    /// Big ON/OFF key centred on the overlay.
    fn draw_popup(&self, pm: &mut Pixmap, name: &str, on: bool, u: f32) {
        let (w, h) = (pm.width() as f32, pm.height() as f32);
        let (pw, ph) = ((3.5 * u).min(w * 0.9), (1.8 * u).min(h * 0.9));
        let (x, y) = ((w - pw) / 2.0, (h - ph) / 2.0);
        let color = if on { Color::from_rgba8(22, 163, 74, 245) } else { Color::from_rgba8(71, 85, 105, 245) };
        fill(pm, &round_rect(x - 0.08 * u, y - 0.08 * u, pw + 0.16 * u, ph + 0.16 * u, 0.3 * u), Color::from_rgba8(18, 18, 22, 230), Transform::identity());
        fill(pm, &round_rect(x, y, pw, ph, 0.22 * u), color, Transform::identity());
        self.text(pm, name, w / 2.0, y + ph * 0.32, 0.42 * u, pw - 0.3 * u, 255);
        self.text(pm, if on { "ON" } else { "OFF" }, w / 2.0, y + ph * 0.68, 0.6 * u, pw - 0.3 * u, 255);
    }

    fn draw_keys(&self, pm: &mut Pixmap, layout: &Layout, state: u32, u: f32) {
        let layer = layout.top_layer(state);
        let (w, h) = (pm.width(), pm.height());
        let (x0, y0, _, _) = layout.bounds();

        let panel = round_rect(0.0, 0.0, w as f32, h as f32, 0.2 * u);
        fill(pm, &panel, Color::from_rgba8(18, 18, 22, 210), Transform::identity());
        let name = &layout.layers[layer].name;
        self.text(pm, name, w as f32 / 2.0, (PAD + TITLE / 2.0) * u, 0.35 * u, w as f32, 255);

        // Key units -> pixels, with the layout's top-left landing inside the padding.
        let base = Transform::from_scale(u, u).pre_translate(PAD - x0, PAD + TITLE - y0);
        for (i, k) in layout.keys.iter().enumerate() {
            let t = base.pre_concat(k.rotation());
            let (label, own) = layout.label(state, i);
            let lay = &layout.layers[layer];
            let key_color = lay.colors.get(i).and_then(|c| c.as_deref()).and_then(hex);
            let color = match key_color {
                Some(c) => c,
                None if own && !label.is_empty() => {
                    lay.color.as_deref().and_then(hex).unwrap_or(Color::from_rgba8(58, 60, 72, 235))
                }
                None => Color::from_rgba8(40, 40, 48, 200),
            };
            let key = round_rect(k.x + GAP, k.y + GAP, k.w - 2.0 * GAP, k.h - 2.0 * GAP, 0.12);
            fill(pm, &key, color, t);

            // Labels stay upright on rotated keys: easier to read at a glance.
            let (tap, hold) = label.split_once('\n').unwrap_or((label, ""));
            let cx = k.x + k.w / 2.0;
            let tap_y = k.y + k.h / 2.0 - if hold.is_empty() { 0.0 } else { 0.1 };
            let mut p = [Point::from_xy(cx, tap_y), Point::from_xy(cx, k.y + k.h - 0.2)];
            t.map_points(&mut p);
            let alpha = if own { 255 } else { 100 };
            self.text(pm, tap, p[0].x, p[0].y, 0.3 * u, (k.w - 0.2) * u, alpha);
            self.text(pm, hold, p[1].x, p[1].y, 0.18 * u, (k.w - 0.15) * u, alpha / 4 * 3);
        }
    }

    /// White text centred on (cx, cy), shrunk to fit `max_w`.
    #[allow(clippy::too_many_arguments)]
    fn text(&self, pm: &mut Pixmap, s: &str, cx: f32, cy: f32, px: f32, max_w: f32, alpha: u8) {
        if s.is_empty() {
            return;
        }
        let width = |px| s.chars().map(|c| self.font.metrics(c, px).advance_width).sum::<f32>();
        let px = px * (max_w / width(px)).min(1.0);
        let Some(lm) = self.font.horizontal_line_metrics(px) else { return };
        let baseline = cy + (lm.ascent + lm.descent) / 2.0;
        let mut pen = cx - width(px) / 2.0;
        let mut cache = self.glyphs.borrow_mut();
        if cache.len() > MAX_CACHED_GLYPHS {
            cache.clear();
        }
        for c in s.chars() {
            let (m, glyph) = cache.entry((c, px.to_bits(), alpha)).or_insert_with(|| {
                let (m, coverage) = self.font.rasterize(c, px);
                (m, glyph_pixmap(m.width, m.height, &coverage, alpha))
            });
            if let Some(glyph) = glyph {
                let gx = (pen + m.xmin as f32).round() as i32;
                let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
                pm.draw_pixmap(gx, gy, glyph.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            }
            pen += m.advance_width;
        }
    }
}

/// Rendered glyphs kept between frames (labels repeat on every repaint).
const MAX_CACHED_GLYPHS: usize = 4096;

fn glyph_pixmap(w: usize, h: usize, coverage: &[u8], alpha: u8) -> Option<Pixmap> {
    let mut g = Pixmap::new(w as u32, h as u32)?;
    for (p, &c) in g.pixels_mut().iter_mut().zip(coverage) {
        let a = (c as u16 * alpha as u16 / 255) as u8;
        *p = PremultipliedColorU8::from_rgba(a, a, a, a)?;
    }
    Some(g)
}

/// "#rrggbb" (drawn slightly see-through) or "#rrggbbaa".
fn hex(s: &str) -> Option<Color> {
    let s = s.strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
    match s.len() {
        6 => Some(Color::from_rgba8(byte(0)?, byte(2)?, byte(4)?, 235)),
        8 => Some(Color::from_rgba8(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors() {
        assert_eq!(hex("#ff0000"), Some(Color::from_rgba8(255, 0, 0, 235)));
        assert_eq!(hex("#00ff0080"), Some(Color::from_rgba8(0, 255, 0, 128)));
        for bad in ["ff0000", "#ff00", "#gg0000", "#ff0000ff00", "", "#", "#ééé"] {
            assert_eq!(hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn overlay_size_is_capped() {
        let l: Layout = serde_json::from_str(
            r#"{ "name": "t", "unit": 256, "keys": [{"x":0,"y":0},{"x":100,"y":100}], "layers": [{ "name": "B", "labels": [] }] }"#,
        )
        .unwrap();
        for status in [false, true] {
            let (w, h) = Renderer::size(&l, 4.0, status);
            assert!(w <= MAX_PX && h <= MAX_PX && w > 1000, "{w}x{h}");
        }
    }

    const PLAIN: View = View { keys: true, status: &[], popup: None };

    #[test]
    fn status_strip_and_popup() {
        let r = Renderer::new().unwrap();
        let l = Layout::load(None, None).unwrap();
        let status = [("Caps Lock".to_string(), true), ("SOCD".to_string(), false)];
        let (w, h) = Renderer::size(&l, 1.0, false);
        let (ws, hs) = Renderer::size(&l, 1.0, true);
        assert_eq!(ws, w);
        assert!((hs as f32 - h as f32 - STATUS * l.unit).abs() <= 1.0, "strip adds its height");
        let pm = r.render(&l, 0, 1.0, View { status: &status, ..PLAIN });
        assert_eq!((pm.width(), pm.height()), (ws, hs));

        // Popup alone: clear background, solid key in the middle.
        let pm = r.render(&l, 0, 1.0, View { keys: false, status: &status, popup: Some(("Caps Lock", true)) });
        assert_eq!(pm.pixel(1, 1).unwrap().alpha(), 0, "no panel behind a lone popup");
        let mid = pm.pixel(ws / 2, hs / 2 - 2).unwrap();
        assert!(mid.alpha() > 200, "popup drawn in the middle");
    }

    #[test]
    fn worst_case_valid_layout_renders_in_reasonable_time() {
        // Max keys, max-length labels, packed into a small span at max size.
        let keys: Vec<String> = (0..512).map(|i| format!(r#"{{"x":{},"y":{}}}"#, i % 23, i / 23)).collect();
        let labels: Vec<String> = (0..512).map(|_| format!("\"{}\"", "W".repeat(32))).collect();
        let json = format!(
            r#"{{ "name": "t", "unit": 256, "keys": [{}], "layers": [{{ "name": "B", "labels": [{}] }}] }}"#,
            keys.join(","),
            labels.join(",")
        );
        let l: Layout = serde_json::from_str(&json).unwrap();
        let r = Renderer::new().unwrap();
        r.render(&l, 0, 4.0, PLAIN); // warm the glyph cache, like the first show
        let started = std::time::Instant::now();
        r.render(&l, 0, 4.0, PLAIN);
        // The shipped (release) build must stay snappy; debug builds are ~15x slower.
        let limit = std::time::Duration::from_secs(if cfg!(debug_assertions) { 15 } else { 1 });
        assert!(started.elapsed() < limit, "repaint took {:?}", started.elapsed());
    }

    #[test]
    fn renders_every_layer_of_the_built_in_layout_at_its_reported_size() {
        let r = Renderer::new().unwrap();
        let l = Layout::load(None, None).unwrap();
        for scale in [1.0, 1.5] {
            let (w, h) = Renderer::size(&l, scale, false);
            for state in [0, 0b10, 0b100, u32::MAX] {
                let pm = r.render(&l, state, scale, PLAIN);
                assert_eq!((pm.width(), pm.height()), (w, h));
                assert!(pm.pixels().iter().any(|p| p.alpha() > 0), "drew something");
            }
        }
    }
}

fn fill(pm: &mut Pixmap, path: &Option<Path>, color: Color, t: Transform) {
    let Some(path) = path else { return };
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    pm.fill_path(path, &paint, FillRule::Winding, t, None);
}

fn round_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

/// Fonts tried in order, all shipped with every Windows install.
const FONTS: [&str; 4] = ["segoeui.ttf", "arial.ttf", "tahoma.ttf", "verdana.ttf"];

fn load_font() -> Result<Font, String> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    for name in FONTS {
        let Ok(bytes) = std::fs::read(format!(r"{windir}\Fonts\{name}")) else { continue };
        if let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
            return Ok(font);
        }
    }
    Err(format!("no usable font found in {windir}\\Fonts (tried {})", FONTS.join(", ")))
}
