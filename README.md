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

The overlay listens for the layer state over QMK raw HID. Enable raw HID in your
keymap's `rules.mk` (`RAW_ENABLE = yes`) or `keymap.json`
(`"config": {"features": {"raw": true}}`), and add this to `keymap.c`:

```c
#include "raw_hid.h"

#define WL_REPORT_SIZE 32 // QMK raw HID report size
#define WL_MAGIC 0xAA
#define WL_LAYER_STATE 0x01

static void wl_send_layer_state(layer_state_t state) {
    uint8_t  report[WL_REPORT_SIZE] = {WL_MAGIC, WL_LAYER_STATE};
    uint32_t s                      = state;
    memcpy(&report[2], &s, sizeof(s));
    raw_hid_send(report, sizeof(report));
}

layer_state_t layer_state_set_user(layer_state_t state) {
    wl_send_layer_state(state);
    return state;
}

void raw_hid_receive(uint8_t *data, uint8_t length) {
    if (length >= 2 && data[0] == WL_MAGIC && data[1] == WL_LAYER_STATE) {
        wl_send_layer_state(layer_state);
    }
}
```

If your keymap already defines `layer_state_set_user` or `raw_hid_receive`
(VIA does the latter), merge these into them.

Without the firmware change the overlay still works from the tray or hotkey,
it just can't follow your layers.

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
  ]
}
```

`labels` renames keycodes everywhere. `layers` are in keymap order. Per-key
entries use the matrix position `"row,col"` from `keyboard.json`. Colours are
`#rrggbb` or `#rrggbbaa`. Two-line labels use `\n`: the second line is drawn small
(handy for tap/hold keys).

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
| `margin` | 80 | Pixels from the top/bottom edge |
| `layout` | | `["<keyboard folder>", "<keymap>"]` or `["<layout.json>"]` |

## Building

```sh
cargo build --release   # target/release/which_layer.exe
cargo test
```

Command line: `which_layer [<keyboard folder> [keymap] | <layout.json>]`. With
no arguments it reopens the remembered layout. Debug builds read layer numbers
from stdin to simulate a keyboard.

Releases: push a `v*` tag; `.github/workflows/release.yml` tests, builds, packages,
attests and publishes. `build_firmware.ps1` builds QMK firmware in Docker (via WSL).
`tools/icon.py` regenerates the icon from source art.
