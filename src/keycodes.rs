//! QMK keycode -> overlay label. A label may carry a second line ("tap\nhold"),
//! drawn small at the bottom of the key. None means transparent (KC_TRNS).

/// Real keycodes nest two or three deep; past this the raw text is shown, so a
/// pathological keymap can't recurse the stack away.
const MAX_DEPTH: usize = 8;

pub fn label(kc: &str) -> Option<String> {
    label_at(kc, 0)
}

fn label_at(kc: &str, depth: usize) -> Option<String> {
    let kc = kc.trim();
    if matches!(kc, "KC_TRNS" | "KC_TRANSPARENT" | "_______") {
        return None;
    }
    if depth >= MAX_DEPTH {
        return Some(kc.to_string());
    }
    if let Some((f, inner)) = kc.strip_suffix(')').and_then(|s| s.split_once('(')) {
        let args = split_args(inner);
        let tap = |k: &str| label_at(k, depth + 1).unwrap_or_default();
        return Some(match (f, args.as_slice()) {
            ("LT", [l, k]) => format!("{}\nL{l}", tap(k)),
            ("MO", [l]) => format!("L{l}"),
            ("TG", [l]) => format!("TG {l}"),
            ("TO", [l]) => format!("TO {l}"),
            ("TT", [l]) => format!("TT {l}"),
            ("DF", [l]) => format!("DF {l}"),
            ("OSL", [l]) => format!("OSL {l}"),
            ("OSM", [m]) => format!("OS {}", mods(m)),
            ("MT", [m, k]) => format!("{}\n{}", tap(k), mods(m)),
            ("LM", [l, m]) => format!("L{l}\n{}", mods(m)),
            (f, [k]) if f.ends_with("_T") => format!("{}\n{}", tap(k), modname(&f[..f.len() - 2])),
            (f, [k]) => format!("{}+{}", modname(f), tap(k)),
            _ => kc.to_string(),
        });
    }
    Some(basic(kc).map(str::to_string).unwrap_or_else(|| {
        kc.trim_start_matches("KC_").trim_start_matches("QK_").to_string()
    }))
}

/// Splits macro arguments on top-level commas: "1,LCTL(KC_A)" -> ["1", "LCTL(KC_A)"].
fn split_args(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0, 0);
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(s[start..].trim());
    out
}

/// "MOD_LCTL|MOD_LSFT" -> "Ctrl+Shift"
fn mods(m: &str) -> String {
    m.split('|').map(|p| modname(p.trim().trim_start_matches("MOD_"))).collect::<Vec<_>>().join("+")
}

fn modname(m: &str) -> &str {
    match m {
        "LCTL" | "CTL" | "C" => "Ctrl",
        "RCTL" => "RCtrl",
        "LSFT" | "SFT" | "S" => "Shift",
        "RSFT" => "RShift",
        "LALT" | "ALT" | "LOPT" | "A" => "Alt",
        "RALT" | "ALGR" | "ROPT" => "RAlt",
        "LGUI" | "GUI" | "LWIN" | "LCMD" | "G" => "Win",
        "RGUI" | "RWIN" | "RCMD" => "RWin",
        "MEH" => "Meh",
        "HYPR" => "Hyper",
        other => other,
    }
}

/// Keycodes whose label isn't just the name minus "KC_" (KC_A -> "A" and
/// KC_F1 -> "F1" come for free).
fn basic(kc: &str) -> Option<&'static str> {
    Some(match kc {
        "KC_NO" | "XXXXXXX" => "",
        "KC_ENT" | "KC_ENTER" => "Enter",
        "KC_ESC" | "KC_ESCAPE" | "QK_GESC" | "QK_GRAVE_ESCAPE" => "Esc",
        "KC_BSPC" | "KC_BACKSPACE" => "Bksp",
        "KC_TAB" => "Tab",
        "KC_SPC" | "KC_SPACE" => "Space",
        "KC_MINS" | "KC_MINUS" => "-",
        "KC_EQL" | "KC_EQUAL" => "=",
        "KC_LBRC" | "KC_LEFT_BRACKET" => "[",
        "KC_RBRC" | "KC_RIGHT_BRACKET" => "]",
        "KC_BSLS" | "KC_BACKSLASH" => "\\",
        "KC_SCLN" | "KC_SEMICOLON" => ";",
        "KC_QUOT" | "KC_QUOTE" => "'",
        "KC_GRV" | "KC_GRAVE" => "`",
        "KC_COMM" | "KC_COMMA" => ",",
        "KC_DOT" => ".",
        "KC_SLSH" | "KC_SLASH" => "/",
        "KC_CAPS" | "KC_CAPS_LOCK" => "Caps",
        "KC_LCTL" | "KC_LEFT_CTRL" => "Ctrl",
        "KC_RCTL" | "KC_RIGHT_CTRL" => "RCtrl",
        "KC_LSFT" | "KC_LEFT_SHIFT" | "KC_RSFT" | "KC_RIGHT_SHIFT" => "Shift",
        "KC_LALT" | "KC_LEFT_ALT" | "KC_LOPT" => "Alt",
        "KC_RALT" | "KC_RIGHT_ALT" | "KC_ALGR" => "RAlt",
        "KC_LGUI" | "KC_LEFT_GUI" | "KC_LWIN" | "KC_LCMD" => "Win",
        "KC_RGUI" | "KC_RIGHT_GUI" | "KC_RWIN" | "KC_RCMD" => "RWin",
        "KC_LEFT" => "←",
        "KC_RGHT" | "KC_RIGHT" => "→",
        "KC_UP" => "↑",
        "KC_DOWN" => "↓",
        "KC_HOME" => "Home",
        "KC_END" => "End",
        "KC_PGUP" | "KC_PAGE_UP" => "PgUp",
        "KC_PGDN" | "KC_PAGE_DOWN" => "PgDn",
        "KC_DEL" | "KC_DELETE" => "Del",
        "KC_INS" | "KC_INSERT" => "Ins",
        "KC_PSCR" | "KC_PRINT_SCREEN" => "PrtSc",
        "KC_SCRL" | "KC_SCROLL_LOCK" => "ScrLk",
        "KC_PAUS" | "KC_PAUSE" => "Pause",
        "KC_APP" | "KC_APPLICATION" => "Menu",
        "KC_MUTE" | "KC_AUDIO_MUTE" => "Mute",
        "KC_VOLU" | "KC_AUDIO_VOL_UP" => "Vol+",
        "KC_VOLD" | "KC_AUDIO_VOL_DOWN" => "Vol-",
        "KC_MPLY" | "KC_MEDIA_PLAY_PAUSE" => "Play",
        "KC_MNXT" | "KC_MEDIA_NEXT_TRACK" => "Next",
        "KC_MPRV" | "KC_MEDIA_PREV_TRACK" => "Prev",
        "QK_BOOT" => "Boot",
        "KC_EXLM" => "!",
        "KC_AT" => "@",
        "KC_HASH" => "#",
        "KC_DLR" => "$",
        "KC_PERC" => "%",
        "KC_CIRC" => "^",
        "KC_AMPR" => "&",
        "KC_ASTR" => "*",
        "KC_LPRN" => "(",
        "KC_RPRN" => ")",
        "KC_UNDS" => "_",
        "KC_PLUS" => "+",
        "KC_LCBR" => "{",
        "KC_RCBR" => "}",
        "KC_PIPE" => "|",
        "KC_TILD" => "~",
        "KC_COLN" => ":",
        "KC_DQUO" | "KC_DQT" => "\"",
        "KC_LABK" | "KC_LT" => "<",
        "KC_RABK" | "KC_GT" => ">",
        "KC_QUES" => "?",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn labels() {
        let l = |kc| label(kc);
        assert_eq!(l("KC_TRNS"), None);
        assert_eq!(l("KC_NO").as_deref(), Some(""));
        assert_eq!(l("KC_Q").as_deref(), Some("Q"));
        assert_eq!(l("KC_F11").as_deref(), Some("F11"));
        assert_eq!(l("KC_SCLN").as_deref(), Some(";"));
        assert_eq!(l("LT(2,KC_BSLS)").as_deref(), Some("\\\nL2"));
        assert_eq!(l("LT(1, KC_UP)").as_deref(), Some("↑\nL1"));
        assert_eq!(l("RALT_T(KC_SPC)").as_deref(), Some("Space\nRAlt"));
        assert_eq!(l("MT(MOD_LCTL|MOD_LSFT,KC_A)").as_deref(), Some("A\nCtrl+Shift"));
        assert_eq!(l("LCTL(KC_C)").as_deref(), Some("Ctrl+C"));
        assert_eq!(l("MO(3)").as_deref(), Some("L3"));
        assert_eq!(l("SOME_CUSTOM").as_deref(), Some("SOME_CUSTOM"));
    }

    #[test]
    fn absurd_nesting_does_not_overflow_the_stack() {
        let kc = format!("{}KC_A{}", "LCTL(".repeat(10_000), ")".repeat(10_000));
        // Run on a small stack to prove the depth cap, not the stack size, stops it.
        let out = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || label(&kc)).unwrap().join();
        assert!(out.unwrap().is_some());
    }
}
