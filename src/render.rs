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
/// Largest overlay side in pixels (about a 4K monitor); bigger layouts are
/// drawn smaller instead, which also bounds memory per repaint (~64 MB).
const MAX_PX: u32 = 4096;

type GlyphKey = (char, u32, u8); // char, font px bits, alpha

pub struct Renderer {
    font: Font,
    glyphs: RefCell<HashMap<GlyphKey, (Metrics, Option<Pixmap>)>>,
}

impl Renderer {
    pub fn new() -> Result<Renderer, String> {
        Ok(Renderer { font: load_font()?, glyphs: RefCell::default() })
    }

    /// Pixels per key unit: `unit * scale`, shrunk if needed so the overlay
    /// never exceeds MAX_PX on a side.
    fn unit_px(layout: &Layout, scale: f32) -> f32 {
        let (x0, y0, x1, y1) = layout.bounds();
        let span = (x1 - x0 + 2.0 * PAD).max(y1 - y0 + 2.0 * PAD + TITLE);
        let u = layout.unit * scale;
        if u.is_finite() && span.is_finite() && span > 0.0 {
            u.min(MAX_PX as f32 / span)
        } else {
            1.0
        }
    }

    pub fn size(layout: &Layout, scale: f32) -> (u32, u32) {
        let (x0, y0, x1, y1) = layout.bounds();
        let u = Self::unit_px(layout, scale);
        let px = |units: f32| ((units * u).ceil() as u32).clamp(1, MAX_PX);
        (px(x1 - x0 + 2.0 * PAD), px(y1 - y0 + 2.0 * PAD + TITLE))
    }

    /// `state` is QMK's layer_state bitmask; the highest active layer is drawn.
    pub fn render(&self, layout: &Layout, state: u32, scale: f32) -> Pixmap {
        let layer = layout.top_layer(state);
        let (w, h) = Self::size(layout, scale);
        // size() keeps both sides within 1..=MAX_PX, which Pixmap always accepts.
        let mut pm = Pixmap::new(w, h).expect("overlay size within 1..=MAX_PX");
        let u = Self::unit_px(layout, scale);
        let (x0, y0, _, _) = layout.bounds();

        let panel = round_rect(0.0, 0.0, w as f32, h as f32, 0.2 * u);
        fill(&mut pm, &panel, Color::from_rgba8(18, 18, 22, 210), Transform::identity());
        let name = &layout.layers[layer].name;
        self.text(&mut pm, name, w as f32 / 2.0, (PAD + TITLE / 2.0) * u, 0.35 * u, w as f32, 255);

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
            fill(&mut pm, &key, color, t);

            // Labels stay upright on rotated keys: easier to read at a glance.
            let (tap, hold) = label.split_once('\n').unwrap_or((label, ""));
            let cx = k.x + k.w / 2.0;
            let tap_y = k.y + k.h / 2.0 - if hold.is_empty() { 0.0 } else { 0.1 };
            let mut p = [Point::from_xy(cx, tap_y), Point::from_xy(cx, k.y + k.h - 0.2)];
            t.map_points(&mut p);
            let alpha = if own { 255 } else { 100 };
            self.text(&mut pm, tap, p[0].x, p[0].y, 0.3 * u, (k.w - 0.2) * u, alpha);
            self.text(&mut pm, hold, p[1].x, p[1].y, 0.18 * u, (k.w - 0.15) * u, alpha / 4 * 3);
        }
        pm
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
        let (w, h) = Renderer::size(&l, 4.0);
        assert!(w <= MAX_PX && h <= MAX_PX && w > 1000, "{w}x{h}");
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
        r.render(&l, 0, 4.0); // warm the glyph cache, like the first show
        let started = std::time::Instant::now();
        r.render(&l, 0, 4.0);
        // The shipped (release) build must stay snappy; debug builds are ~15x slower.
        let limit = std::time::Duration::from_secs(if cfg!(debug_assertions) { 15 } else { 1 });
        assert!(started.elapsed() < limit, "repaint took {:?}", started.elapsed());
    }

    #[test]
    fn renders_every_layer_of_the_built_in_layout_at_its_reported_size() {
        let r = Renderer::new().unwrap();
        let l = Layout::load(None, None).unwrap();
        for scale in [1.0, 1.5] {
            let (w, h) = Renderer::size(&l, scale);
            for state in [0, 0b10, 0b100, u32::MAX] {
                let pm = r.render(&l, state, scale);
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
