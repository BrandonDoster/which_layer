# which_layer

<img src="assets/icon.png" width="96" align="right" alt="">

A tiny, always-on-top overlay that shows which layer your QMK keyboard is on,
and what every key does on it. It fades in when you leave the base layer and
fades out when you return. It is click-through and never takes focus.

- Native Rust, no Electron: about 1 MB of RAM while hidden, and it shows in under 10 ms.
- Reads the layout straight from your QMK keyboard folder (`keyboard.json` /
  `info.json` and `keymap.c` / `keymap.json`), including split, staggered
  and rotated keys. Edits show up live.
- Transparent keys (`KC_TRNS`) show what they fall through to on the active layers.
- Tray icon for show/hide, auto-show, opacity, size, position and start with Windows.
- Toggles like Caps Lock and SOCD pop up as a big ON/OFF key when they change, and
  are listed under the keys, so boards without LEDs still tell you what's on.
- Optional global hotkey, fade timing, and per-layer / per-key colours.

Windows 10/11 for now. Linux (KDE Wayland) is planned.

## Install

Download the installer or the zip (just the exe) from
[Releases](https://github.com/BrandonDoster/which_layer/releases). Both are
built by GitHub Actions with a build provenance attestation, which you can check with:

```sh
gh attestation verify which_layer-1.0.0-setup.exe --repo BrandonDoster/which_layer
```

The installer is per-user (no admin prompt). The exe isn't code-signed yet, so
SmartScreen may warn on first run.

On first launch, pick your keyboard's folder in your QMK firmware (the one with
`keyboard.json` and a `keymaps` folder). The `default` keymap is used; to pick
another, open the settings file from the tray and change the keymap name.

## Keyboard firmware

The overlay listens over QMK raw HID for the layer state and for on/off
toggles like Caps Lock, NKRO and SOCD. Boards without indicator LEDs get a big
ON/OFF popup when one changes, and a status line under the keys. Enable raw HID
in your keymap's `rules.mk` (`RAW_ENABLE = yes`) or `keymap.json`
(`"config": {"features": {"raw": true}}`), and add this to `keymap.c`:

```c
#include "raw_hid.h"

#define WL_REPORT_SIZE 32 // QMK raw HID report size
#define WL_MAGIC 0xAA
#define WL_LAYER_STATE 0x01
#define WL_TOGGLES 0x02

// Toggle bits which_layer names on its own (bits 8-31 are free for your own,
// named in which_layer.json). WL_KNOWN is what this board reports.
#define WL_CAPS (1UL << 0)
#define WL_NUM (1UL << 1)
#define WL_SCROLL (1UL << 2)
#define WL_NKRO (1UL << 3)
#define WL_SOCD (1UL << 4)
#define WL_KNOWN (WL_CAPS | WL_NKRO)

static void wl_send(uint8_t type, uint32_t a, uint32_t b) {
    uint8_t report[WL_REPORT_SIZE] = {WL_MAGIC, type};
    memcpy(&report[2], &a, sizeof(a));
    memcpy(&report[6], &b, sizeof(b));
    raw_hid_send(report, sizeof(report));
}

static uint32_t wl_toggles(void) {
    return (host_keyboard_led_state().caps_lock ? WL_CAPS : 0) | (keymap_config.nkro ? WL_NKRO : 0);
}

layer_state_t layer_state_set_user(layer_state_t state) {
    wl_send(WL_LAYER_STATE, state, 0);
    return state;
}

// Toggles change from many places (host LEDs, keycodes), so watch for changes.
void housekeeping_task_user(void) {
    static uint32_t last = UINT32_MAX;
    if (!is_keyboard_master()) {
        return;
    }
    uint32_t now = wl_toggles();
    if (now != last) {
        last = now;
        wl_send(WL_TOGGLES, now, WL_KNOWN);
    }
}

// The overlay asks for the current state when it connects.
void raw_hid_receive(uint8_t *data, uint8_t length) {
    if (length >= 2 && data[0] == WL_MAGIC && data[1] == WL_LAYER_STATE) {
        wl_send(WL_LAYER_STATE, layer_state, 0);
        wl_send(WL_TOGGLES, wl_toggles(), WL_KNOWN);
    }
}
```

Report only what your board has: add a bit to `WL_KNOWN` and to `wl_toggles()`.
For example, with getreuer's `socd_cleaner` module, add `WL_SOCD` to `WL_KNOWN` and
`| (socd_cleaner_enabled ? WL_SOCD : 0)` to `wl_toggles()`. Toggles of your own
(a game mode, a custom flag) use bits 8-31 and get their names from `which_layer.json`.

If your keymap already defines `layer_state_set_user`, `housekeeping_task_user`
or `raw_hid_receive` (VIA does the last), merge these into them.

Without the firmware change the overlay still works from the tray or hotkey,
it just can't follow your layers or toggles.
## Customising

### Labels and colours: `which_layer.json`

Put a `which_layer.json` next to your `keymap.c`. Every field is optional:

```json
{
  "labels": { "KC_GRV": "`", "KC_BSLS": "Bslash" },
  "layers": [
    { "name": "Base" },
    { "name": "Fn", "color": "#2563eb" },
    { "name": "Symbols", "color": "#b45309", "keys": { "4,5": { "label": "Nav", "color": "#dc2626" } } }
  ],
  "toggles": { "8": { "name": "Game mode" }, "3": { "popup": false } }
}
```

`labels` renames keycodes everywhere. `layers` are in keymap order. Per-key
entries use the matrix position `"row,col"` from `keyboard.json`. Colours are
`#rrggbb` or `#rrggbbaa`. Two-line labels use `\n`: the second line is drawn small
(handy for tap/hold keys).

`toggles` is keyed by bit number. Bits 0-4 are already named Caps Lock, Num Lock,
Scroll Lock, NKRO and SOCD. Each entry can set `name`, and turn off `status`
(listing it under the keys) or `popup` (the big ON/OFF key when it changes).
Layout files take the same `toggles` field.

### Any shape: a layout file

Boards without a QMK folder, or that you want drawn differently, can use a
standalone layout file. Pass it instead of a folder, or set it as the first
`layout` entry in settings:

```json
{
  "name": "My board",
  "unit": 54,
  "keys": [ {"x": 0, "y": 0}, {"x": 1, "y": 0, "w": 1.5}, {"x": 3, "y": 0.2, "r": 15, "rx": 3, "ry": 0} ],
  "layers": [
    { "name": "Base", "labels": ["Esc", "Tab", "Q"] },
    { "name": "Fn", "color": "#2563eb", "labels": ["`", null, "1"] }
  ]
}
```

Positions and sizes are in key units, KLE-style (`r` is degrees around
`rx`/`ry`). `null` labels fall through to lower active layers.
See [`layouts/corne.json`](layouts/corne.json) for a full split example.

### Settings

`%APPDATA%\which_layer\settings.json` (tray, then "Open settings file"). Edits
apply live. The tray covers most of these; the rest:

| Setting | Default | |
|---|---|---|
| `hotkey` | `null` | e.g. `"Ctrl+Alt+L"` toggles the overlay |
| `fade_in_ms` / `fade_out_ms` | 60 / 150 | Fade lengths |
| `hide_delay_ms` | 250 | How long it stays after returning to base |
| `auto_hide_secs` | 5 | Hide a toggled layer after this long (0 = never) |
| `popup_ms` | 1000 | How long a toggle's ON/OFF popup stays up (0 = no popups) |
| `margin` | 80 | Pixels from the top/bottom edge |
| `layout` | | `["<keyboard folder>", "<keymap>"]` or `["<layout.json>"]` |

## Building

```sh
cargo build --release   # target/release/which_layer.exe
cargo test
```

Command line: `which_layer [<keyboard folder> [keymap] | <layout.json>]`. With
no arguments it reopens the remembered layout. Debug builds read layer numbers
(and `t<bit>` to flip a toggle) from stdin to simulate a keyboard.

Releases: push a `v*` tag; `.github/workflows/release.yml` tests, builds, packages,
attests and publishes. `build_firmware.ps1` builds QMK firmware in Docker (via WSL).
`tools/icon.py` regenerates the icon from source art.

## License

[AGPL-3.0-or-later](LICENSE).
