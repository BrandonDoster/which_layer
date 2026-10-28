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
ON/OFF popup when one changes, and a status line under the keys.

Add it to any existing QMK keymap with the bundled community module (needs a
QMK with community modules API 1.1.0, i.e. 2025 or newer), without touching `keymap.c`:

1. Put [`qmk_module`](qmk_module) where QMK finds modules, as `brandondoster`:

    ```sh
    git clone https://github.com/BrandonDoster/which_layer
    ln -s "$(pwd)/which_layer/qmk_module" ~/qmk_firmware/modules/brandondoster
    ```

2. Add it to your keymap's `keymap.json` (create the file next to `keymap.c` if
   there isn't one; the two work together):

    ```json
    { "modules": ["brandondoster/which_layer"] }
    ```

3. Build and flash as usual. The module turns on raw HID itself.

It reports Caps Lock and NKRO (when the build has NKRO), and SOCD when getreuer's
`socd_cleaner` module is in the build too. Optional settings for your `config.h`:

| Define | |
|---|---|
| `WHICH_LAYER_TOGGLES` | Built-in toggles to report, e.g. `(WHICH_LAYER_CAPS \| WHICH_LAYER_NUM)` |
| `WHICH_LAYER_NO_RAW_HID_RECEIVE` | Your keymap has its own `raw_hid_receive` (see below) |
| `WHICH_LAYER_RESEND_MS` | Resend interval when the module can't answer requests (default 1000) |

Toggles of your own (a game mode, a custom flag) use bits 8-31 and get their
names from `which_layer.json`. Report them from `keymap.c`:

```c
#include "which_layer.h"

void which_layer_toggles_user(uint32_t *on, uint32_t *known) {
    *known |= 1UL << 8;
    if (game_mode) *on |= 1UL << 8;
}
```

**VIA, or your own `raw_hid_receive`:** only one piece of code can own
`raw_hid_receive`. With VIA the module steps aside automatically; with a handler
of your own, define `WHICH_LAYER_NO_RAW_HID_RECEIVE` (otherwise the build stops
with a "multiple definition of raw_hid_receive" error). Either way the module
then resends the state every second, so a newly started overlay catches up within
a second. For an instant answer, call `which_layer_raw_hid_receive(data, length)`
first in your handler (or in `via_command_kb` with VIA); it returns true for
which_layer's reports.

Without the firmware side the overlay still works from the tray or hotkey,
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
| `show_delay_ms` | 0 | Hold a layer this long before the overlay appears, so quick taps don't flash it (e.g. 300) |
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

The app is [AGPL-3.0-or-later](LICENSE). The QMK module in
[`qmk_module`](qmk_module) is [GPL-2.0-or-later](qmk_module/LICENSE), like QMK
itself, so it can go into any QMK firmware.
