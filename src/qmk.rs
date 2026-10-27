//! Builds a Layout straight from a QMK keyboard folder:
//!   geometry  keyboard.json / info.json `layouts` (the one the keymap uses)
//!   keycodes  keymaps/<name>/keymap.json if it has layers, else keymap.c
//!   tweaks    keymaps/<name>/which_layer.json (optional):
//!
//! ```json
//! {
//!   "labels": { "KC_BSLS": "Bslash" },
//!   "layers": [
//!     { "name": "Base" },
//!     { "name": "Fn", "color": "#2563eb",
//!       "keys": { "4,5": { "label": "Sym", "color": "#dc2626" } } }
//!   ]
//! }
//! ```
//! `labels` overrides a keycode everywhere; `keys` overrides one key (by matrix
//! "row,col") on one layer. A layer's `color` fills the keys that layer defines.

use crate::keycodes;
use crate::layout::{Key, Layer, Layout};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Info {
    keyboard_name: Option<String>,
    usb: Option<Usb>,
    layouts: BTreeMap<String, InfoLayout>,
}

#[derive(Deserialize)]
struct Usb {
    vid: String,
    pid: String,
}

fn hex_u16(s: &str) -> Option<u16> {
    u16::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
}

#[derive(Deserialize)]
struct InfoLayout {
    layout: Vec<Key>,
}

#[derive(Deserialize)]
struct KeymapJson {
    layout: Option<String>,
    #[serde(default)]
    layers: Vec<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct Overrides {
    #[serde(default)]
    labels: HashMap<String, String>,
    #[serde(default)]
    layers: Vec<LayerOverride>,
}

#[derive(Deserialize)]
struct LayerOverride {
    name: Option<String>,
    color: Option<String>,
    #[serde(default)]
    keys: HashMap<String, KeyOverride>,
}

#[derive(Deserialize)]
struct KeyOverride {
    label: Option<String>,
    color: Option<String>,
}

/// How many folders up from the keyboard to look for shared info.json files
/// when there's no `keyboards` folder to stop at (e.g. a standalone repo).
const MAX_INFO_DEPTH: usize = 4;

/// Reads an input file (size-capped). Ok(None) only when it doesn't exist; any
/// other problem (too big, not UTF-8, locked) is an error rather than "absent".
fn read(path: &Path) -> Result<Option<String>, String> {
    match crate::read_limited(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Keymap names are folder names; refuse anything that could step outside `keymaps`.
fn valid_keymap_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// QMK keyboards inherit settings from info.json / keyboard.json files in
/// parent folders (e.g. keyboards/foo/info.json + keyboards/foo/rev2/keyboard.json).
/// Merges them top-down like QMK does. Returns the merged JSON and every file
/// path worth watching, existing or not.
fn read_info(dir: &Path) -> Result<(serde_json::Value, Vec<PathBuf>), String> {
    let mut dirs: Vec<&Path> = dir
        .ancestors()
        .take_while(|d| d.file_name().is_some_and(|n| n != "keyboards"))
        .take(MAX_INFO_DEPTH)
        .collect();
    dirs.reverse();
    let mut merged = serde_json::Value::Object(Default::default());
    let (mut found, mut watch) = (false, Vec::new());
    for d in dirs {
        for name in ["info.json", "keyboard.json"] {
            let path = d.join(name);
            if d == dir {
                watch.push(path.clone());
            }
            let text = match read(&path)? {
                Some(t) => t,
                None => continue,
            };
            let value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            merge(&mut merged, value);
            if d != dir {
                watch.push(path);
            }
            found = true;
        }
    }
    if !found {
        return Err(format!("{}: no keyboard.json or info.json", dir.display()));
    }
    Ok((merged, watch))
}

/// Deep-merges `b` into `a`: objects merge key by key, anything else replaces.
fn merge(a: &mut serde_json::Value, b: serde_json::Value) {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                merge(a.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (a, b) => *a = b,
    }
}

/// Keymap folder names under `dir/keymaps`, sorted, "default" first.
pub fn keymaps(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir.join("keymaps"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid_keymap_name(n))
        .collect();
    names.sort_by_key(|n| (n != "default", n.clone()));
    names
}

pub fn load(dir: &Path, keymap: &str) -> Result<Layout, String> {
    if !valid_keymap_name(keymap) {
        return Err(format!("invalid keymap name {keymap:?}"));
    }
    let (info, mut sources) = read_info(dir)?;
    let info: Info = serde_json::from_value(info).map_err(|e| format!("{}: {e}", dir.display()))?;

    let km_dir = dir.join("keymaps").join(keymap);
    sources.extend(["keymap.json", "keymap.c", "which_layer.json"].map(|f| km_dir.join(f)));
    let (layout_name, codes) = read_keymap(&km_dir)?;
    let mut layouts = info.layouts;
    let keys = layouts
        .remove(&layout_name)
        .or_else(|| layouts.into_values().next())
        .ok_or("keyboard has no layouts")?
        .layout;

    let ov: Overrides = match read(&km_dir.join("which_layer.json"))? {
        Some(t) => serde_json::from_str(&t).map_err(|e| format!("which_layer.json: {e}"))?,
        None => Overrides::default(),
    };

    let n = keys.len();
    let layers = codes
        .iter()
        .enumerate()
        .map(|(li, codes)| {
            if codes.len() != n {
                eprintln!("layer {li}: {} keycodes for {n} keys", codes.len());
            }
            let mut labels: Vec<Option<String>> = codes
                .iter()
                .map(|kc| ov.labels.get(kc).map(|l| Some(l.clone())).unwrap_or_else(|| keycodes::label(kc)))
                .collect();
            labels.resize(n, None);
            let mut colors = vec![None; n];
            let o = ov.layers.get(li);
            for (pos, k) in o.map(|o| &o.keys).into_iter().flatten() {
                let Some(i) = matrix_index(&keys, pos) else {
                    eprintln!("which_layer.json: no key at matrix {pos}");
                    continue;
                };
                if let Some(l) = &k.label {
                    labels[i] = Some(l.clone());
                }
                colors[i] = k.color.clone();
            }
            let name = o.and_then(|o| o.name.clone());
            Layer {
                name: name.unwrap_or_else(|| if li == 0 { "Base".into() } else { format!("Layer {li}") }),
                labels,
                color: o.and_then(|o| o.color.clone()),
                colors,
            }
        })
        .collect();

    Ok(Layout {
        usb: info.usb.and_then(|u| Some((hex_u16(&u.vid)?, hex_u16(&u.pid)?))),
        name: info.keyboard_name.unwrap_or_default(),
        unit: 54.0,
        keys,
        layers,
        sources,
    })
}

fn matrix_index(keys: &[Key], pos: &str) -> Option<usize> {
    let (r, c) = pos.split_once(',')?;
    let m = [r.trim().parse().ok()?, c.trim().parse().ok()?];
    keys.iter().position(|k| k.matrix == Some(m))
}

/// (LAYOUT macro name, keycodes per layer)
fn read_keymap(km_dir: &Path) -> Result<(String, Vec<Vec<String>>), String> {
    if let Some(t) = read(&km_dir.join("keymap.json"))? {
        let j: KeymapJson = serde_json::from_str(&t).map_err(|e| format!("keymap.json: {e}"))?;
        if !j.layers.is_empty() {
            return Ok((j.layout.unwrap_or_default(), j.layers));
        }
    }
    let path = km_dir.join("keymap.c");
    let src = read(&path)?.ok_or_else(|| format!("{}: not found", path.display()))?;
    let layers = parse_keymap_c(&src);
    if layers.is_empty() {
        return Err(format!("{}: no LAYOUT(...) blocks found", path.display()));
    }
    let name = layers[0].0.clone();
    Ok((name, layers.into_iter().map(|l| l.1).collect()))
}

/// Upper bound on layers taken from `[N] =` designators, so a stray huge index
/// can't allocate a giant keymap.
const MAX_LAYERS: usize = 32;

/// Pulls every `[layer] = LAYOUT...(kc, kc, ...)` out of a keymap.c, ordered
/// by layer index. Layer names from `enum`s and simple `#define` aliases are
/// resolved; a layer index that can't be resolved falls back to file order.
/// Gaps in the indexes become fully transparent layers.
fn parse_keymap_c(src: &str) -> Vec<(String, Vec<String>)> {
    let s = strip_comments(src);
    let symbols = symbols(&s);
    let mut budget = EXPANSION_BUDGET;
    let b = s.as_bytes();
    let mut found: Vec<(Option<usize>, String, Vec<String>)> = Vec::new();
    let mut pos = s.find("keymaps[").unwrap_or(0);
    while let Some(off) = s[pos..].find("LAYOUT") {
        let start = pos + off;
        let designator = s[pos..start].rfind('[').and_then(|i| {
            let inner = &s[pos + i + 1..start];
            let (index, rest) = inner.split_once(']')?;
            if rest.trim() != "=" {
                return None;
            }
            substitute(index.trim(), &symbols, &mut budget).parse::<usize>().ok()
        });
        let mut j = start;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
            j += 1;
        }
        let name = &s[start..j];
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= b.len() || b[j] != b'(' {
            pos = j;
            continue;
        }
        let (mut depth, mut arg_start, mut args) = (0, j + 1, Vec::new());
        let mut k = j;
        while k < b.len() {
            match b[k] {
                b'(' => depth += 1,
                b')' | b',' if depth == 1 => {
                    let arg: String = s[arg_start..k].split_whitespace().collect();
                    if !arg.is_empty() {
                        args.push(substitute(&arg, &symbols, &mut budget));
                    }
                    arg_start = k + 1;
                    if b[k] == b')' {
                        break;
                    }
                }
                b')' => depth -= 1,
                _ => {}
            }
            k += 1;
        }
        found.push((designator, name.to_string(), args));
        pos = k;
    }

    let indexes: Vec<usize> = found.iter().filter_map(|f| f.0).collect();
    let usable = indexes.len() == found.len()
        && indexes.iter().all(|&i| i < MAX_LAYERS)
        && indexes.iter().collect::<std::collections::HashSet<_>>().len() == indexes.len();
    if !usable {
        return found.into_iter().map(|(_, name, args)| (name, args)).collect();
    }
    let name = found.first().map(|f| f.1.clone()).unwrap_or_default();
    let mut out = vec![(name, Vec::new()); indexes.iter().max().map_or(0, |m| m + 1)];
    for (i, name, args) in found {
        out[i.unwrap()] = (name, args);
    }
    out
}

/// Names a keymap.c defines: simple object-like `#define NAME value` and
/// numbered `enum` members (layer names). Function-like macros are skipped.
fn symbols(s: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in s.lines() {
        let Some(rest) = line.trim().strip_prefix("#define") else { continue };
        let rest = rest.trim_start();
        let name_len = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
        let (name, value) = rest.split_at(name_len);
        if !name.is_empty() && name.len() <= MAX_SYMBOL_LEN && !value.starts_with('(') && !value.trim().is_empty() {
            map.insert(name.to_string(), value.split_whitespace().collect());
        }
    }
    let is_ident = |n: &str| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let mut pos = 0;
    while let Some(off) = s[pos..].find("enum") {
        let start = pos + off + 4;
        pos = start;
        let word_before = s[..start - 4].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        let word_after = s[start..].chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        if word_before || word_after {
            continue;
        }
        // `enum name {` only: a ';' or ')' first means this isn't an enum body.
        // Either way scanning resumes past that point, keeping this linear.
        let Some(open) = s[start..].find(['{', ';', ')']).map(|i| start + i) else { break };
        if s.as_bytes()[open] != b'{' {
            pos = open;
            continue;
        }
        let Some(close) = s[open..].find('}').map(|i| open + i) else { break };
        let mut next: Option<i64> = Some(0);
        for entry in s[open + 1..close].split(',') {
            let (name, value) = match entry.split_once('=') {
                Some((n, v)) => (n.trim(), Some(v.trim())),
                None => (entry.trim(), None),
            };
            if !is_ident(name) || name.len() > MAX_SYMBOL_LEN {
                continue;
            }
            // `= SAFE_RANGE` style values aren't layer numbers; stop numbering.
            let n = match value {
                Some(v) => v.parse::<i64>().ok(),
                None => next,
            };
            if let Some(n) = n {
                map.insert(name.to_string(), n.to_string());
            }
            next = n.and_then(|n| n.checked_add(1));
        }
        pos = close;
    }
    map
}

/// Longest expansion of one keycode; anything bigger is left unexpanded.
const MAX_EXPANSION: usize = 256;
/// Nesting depth for macros that use other macros.
const MAX_EXPANSION_DEPTH: usize = 8;
/// Longest #define / enum name considered (real ones are short).
const MAX_SYMBOL_LEN: usize = 64;
/// Characters the expander may scan for a whole keymap.c; past this, keycodes
/// are left as written. Normal keymaps use a few thousand.
const EXPANSION_BUDGET: usize = 1_000_000;

/// Replaces known identifiers in a keycode expression, recursively (so a
/// #define can use an enum name): `LT(_NAV,KC_A)` -> `LT(2,KC_A)`. Like the C
/// preprocessor, a macro is never expanded inside its own expansion; output is
/// capped per keycode and work is capped per file (`budget`).
fn substitute(expr: &str, symbols: &HashMap<String, String>, budget: &mut usize) -> String {
    let mut out = String::new();
    if expand(expr, symbols, &mut Vec::new(), &mut out, budget) {
        out
    } else {
        expr.to_string()
    }
}

/// Appends the expansion of `expr` to `out`. False if it outgrew MAX_EXPANSION
/// or the work budget ran out.
fn expand<'a>(
    expr: &str,
    symbols: &'a HashMap<String, String>,
    active: &mut Vec<&'a str>,
    out: &mut String,
    budget: &mut usize,
) -> bool {
    let mut ident = String::new();
    for c in expr.chars().chain(std::iter::once('\0')) {
        let Some(left) = budget.checked_sub(1) else { return false };
        *budget = left;
        if c.is_ascii_alphanumeric() || c == '_' {
            ident.push(c);
            continue;
        }
        match symbols.get_key_value(&ident) {
            Some((name, value)) if active.len() < MAX_EXPANSION_DEPTH && !active.contains(&name.as_str()) => {
                active.push(name);
                let ok = expand(value, symbols, active, out, budget);
                active.pop();
                if !ok {
                    return false;
                }
            }
            _ => out.push_str(&ident),
        }
        ident.clear();
        if c != '\0' {
            out.push(c);
        }
        if out.len() > MAX_EXPANSION {
            return false;
        }
    }
    true
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match (c, it.peek()) {
            ('/', Some('/')) => {
                while it.peek().is_some_and(|&n| n != '\n') {
                    it.next();
                }
            }
            ('/', Some('*')) => {
                it.next();
                let mut prev = ' ';
                for n in it.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qmk/keyboards/dosboards/dos60s")
    }

    #[test]
    fn loads_the_dos60s_fixture() {
        let l = load(&fixture(), "socd").unwrap();
        assert_eq!(l.name, "Dos60S");
        assert_eq!(l.usb, Some((0xBD01, 0x0001)));
        assert_eq!(l.keys.len(), 60);
        assert_eq!(l.layers.len(), 3);
        // Thumb keys: rotated, 1.5u, matrix positions intact.
        let thumb = l.keys.iter().find(|k| k.matrix == Some([4, 4])).unwrap();
        assert_eq!((thumb.w, thumb.r, thumb.rx, thumb.ry), (1.5, 30.0, Some(4.0), Some(5.0)));
        let i = l.keys.iter().position(|k| k.matrix == Some([4, 5])).unwrap();
        assert_eq!(l.label(0b1, i), ("\\\nL2", true));
        // Layer 2: own key, transparent key falling through to base, which_layer.json name/colour.
        assert_eq!(l.layers[2].name, "Symbols / Nav");
        assert_eq!(l.layers[2].colors[i].as_deref(), Some("#dc2626"));
        assert_eq!(l.label(0b100, 0), ("`", true));
        assert_eq!(l.label(0b100, 1), ("1", false));
    }

    #[test]
    fn rejects_keymap_names_that_leave_the_keymaps_folder() {
        for bad in ["../socd", "..", "a/b", "a\\b", ""] {
            assert!(load(&fixture(), bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn lists_keymaps_default_first() {
        let kb = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qmk/keyboards/dosboards/dos60s");
        assert_eq!(keymaps(&kb), ["default", "socd"]);
        assert!(keymaps(&kb.join("nope")).is_empty());
    }

    #[test]
    fn merges_parent_info_json() {
        let root = std::env::temp_dir().join(format!("which_layer_test_{}", std::process::id()));
        let kb = root.join("keyboards").join("acme").join("rev2");
        fs::create_dir_all(kb.join("keymaps").join("default")).unwrap();
        fs::write(root.join("keyboards/acme/info.json"), r#"{"keyboard_name":"Acme","usb":{"vid":"0x1234","pid":"0x0001"},
            "layouts":{"LAYOUT":{"layout":[{"x":0,"y":0},{"x":1,"y":0}]}}}"#)
        .unwrap();
        fs::write(kb.join("keyboard.json"), r#"{"usb":{"pid":"0x0002"}}"#).unwrap();
        fs::write(kb.join("keymaps/default/keymap.json"), r#"{"layout":"LAYOUT","layers":[["KC_A","KC_B"]]}"#).unwrap();
        let l = load(&kb, "default");
        let _ = fs::remove_dir_all(&root);
        let l = l.unwrap();
        assert_eq!(l.name, "Acme");
        assert_eq!(l.usb, Some((0x1234, 0x0002)), "child overrides only pid");
        assert_eq!(l.keys.len(), 2);
    }

    #[test]
    fn keymap_c_resolves_enum_layers_defines_and_order() {
        let src = r#"
            enum layers { _BASE, _NAV, _SYM };
            enum custom_keycodes { MY_KEY = SAFE_RANGE, OTHER };
            void enumerate(void) { int KC_B = 7; }
            #define NAV_SPC LT(_NAV, KC_SPC)
            #define CTL_ESC(x) x
            const uint16_t PROGMEM keymaps[][MATRIX_ROWS][MATRIX_COLS] = {
                [_SYM]  = LAYOUT(KC_EXLM, MY_KEY),
                [_BASE] = LAYOUT(NAV_SPC, KC_B),
            };"#;
        let layers = parse_keymap_c(src);
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0].1, ["LT(1,KC_SPC)", "KC_B"]);
        assert!(layers[1].1.is_empty(), "_NAV missing -> empty (all transparent)");
        assert_eq!(layers[2].1, ["KC_EXLM", "MY_KEY"]);
    }

    #[test]
    fn self_referential_defines_do_not_blow_up() {
        let src = "#define X X|X|X|X|X|X|X|X|X|X\n#define A B\n#define B A\n#define Y LCTL(Z)\n#define Z KC_Q\n\
                   enum l { BIG = 9223372036854775807, NEXT };\n\
                   keymaps[] = { [0] = LAYOUT(X, A, Y) };";
        let layers = parse_keymap_c(src);
        let keys = &layers[0].1;
        assert!(keys[0].len() <= MAX_EXPANSION, "capped: {}", keys[0].len());
        assert_eq!(keys[1], "A", "A -> B -> A stops at the cycle");
        assert_eq!(keys[2], "LCTL(KC_Q)", "nested defines still expand");
    }

    #[test]
    fn pathological_keymap_c_parses_quickly() {
        let started = std::time::Instant::now();
        // Many `enum` keywords with no body (was quadratic).
        let enums = format!("{};\nkeymaps[] = {{ [0] = LAYOUT(KC_A) }};", "enum ".repeat(200_000));
        assert_eq!(parse_keymap_c(&enums)[0].1, ["KC_A"]);
        // A short define referenced by thousands of keys through a long chain.
        let long_name = "N".repeat(MAX_SYMBOL_LEN);
        let refs = vec![long_name.as_str(); 100].join("|");
        let keys = vec!["B"; 20_000].join(",");
        let src = format!("#define {long_name} x\n#define B {refs}\nkeymaps[] = {{ [0] = LAYOUT({keys}) }};");
        let layers = parse_keymap_c(&src);
        assert_eq!(layers[0].1.len(), 20_000);
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
    }

    #[test]
    fn oversized_input_files_are_refused() {
        let dir = std::env::temp_dir().join(format!("which_layer_big_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let big = dir.join("big.json");
        fs::write(&big, vec![b' '; (crate::MAX_INPUT_BYTES + 1) as usize]).unwrap();
        let r = read(&big);
        assert_eq!(read(&dir.join("missing.json")), Ok(None));
        let _ = fs::remove_dir_all(&dir);
        assert!(r.is_err());
    }

    #[test]
    fn keymap_c_huge_or_duplicate_indexes_fall_back_to_file_order() {
        let layers = parse_keymap_c("keymaps[] = { [99999] = LAYOUT(KC_A), [0] = LAYOUT(KC_B) };");
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].1, ["KC_A"]);
        let layers = parse_keymap_c("keymaps[] = { [0] = LAYOUT(KC_A), [0] = LAYOUT(KC_B) };");
        assert_eq!(layers.len(), 2);
    }

    #[test]
    fn keymap_c() {
        let src = r#"
            // LAYOUT(fake) in a comment
            const uint16_t PROGMEM keymaps[][MATRIX_ROWS][MATRIX_COLS] = {
                [0] = LAYOUT(KC_A, LT(1, KC_B), /* KC_X, */ KC_C),
                [1] = LAYOUT_split(KC_TRNS, MT(MOD_LCTL|MOD_LSFT, KC_D), KC_E,)
            };"#;
        let layers = parse_keymap_c(src);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].0, "LAYOUT");
        assert_eq!(layers[0].1, ["KC_A", "LT(1,KC_B)", "KC_C"]);
        assert_eq!(layers[1].0, "LAYOUT_split");
        assert_eq!(layers[1].1, ["KC_TRNS", "MT(MOD_LCTL|MOD_LSFT,KC_D)", "KC_E"]);
    }
}
