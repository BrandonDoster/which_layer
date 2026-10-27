// Copyright 2026 Brandon Doster (@BrandonDoster)
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// which_layer QMK community module: reports the layer state and on/off toggles
// (Caps Lock, NKRO, SOCD, ...) over raw HID to the which_layer overlay
// (https://github.com/BrandonDoster/which_layer). Add it in keymap.json:
//
//     { "modules": ["brandondoster/which_layer"] }
//
// Nothing else is needed. Optional, in the keymap's config.h:
//   WHICH_LAYER_TOGGLES             built-in toggles to report (default: Caps Lock, NKRO, SOCD)
//   WHICH_LAYER_NO_RAW_HID_RECEIVE  the keymap has its own raw_hid_receive (see below)
//   WHICH_LAYER_RESEND_MS           resend interval when it can't answer requests (default 1000)

#pragma once

#include <stdbool.h>
#include <stdint.h>

// Toggle bits the overlay already has names for. Bits 8-31 are free for the
// keymap's own toggles, named in which_layer.json.
#define WHICH_LAYER_CAPS (1UL << 0)
#define WHICH_LAYER_NUM (1UL << 1)
#define WHICH_LAYER_SCROLL (1UL << 2)
#define WHICH_LAYER_NKRO (1UL << 3)
#define WHICH_LAYER_SOCD (1UL << 4)

// Optional: report the keymap's own toggles. Set bits in `on`, and mark every
// bit you report in `known`, e.g.
//     void which_layer_toggles_user(uint32_t *on, uint32_t *known) {
//         *known |= 1UL << 8;
//         if (game_mode) *on |= 1UL << 8;
//     }
void which_layer_toggles_user(uint32_t *on, uint32_t *known);

// For keymaps that already own raw_hid_receive (VIA, or define
// WHICH_LAYER_NO_RAW_HID_RECEIVE): call this first from your handler (or from
// via_command_kb with VIA) to answer the overlay's state request right away.
// Returns true if the report was for which_layer.
bool which_layer_raw_hid_receive(uint8_t *data, uint8_t length);
