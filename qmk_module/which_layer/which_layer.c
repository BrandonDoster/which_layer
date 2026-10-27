// Copyright 2026 Brandon Doster (@BrandonDoster)
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Protocol (32-byte raw HID reports, byte 0 = 0xAA):
//   board -> host  [0xAA, 0x01, layer_state u32 LE]           on every layer change
//   board -> host  [0xAA, 0x02, on u32 LE, known u32 LE]      when a toggle changes
//   host -> board  [0xAA, 0x01]                               request both (on connect)

#include QMK_KEYBOARD_H
#include "raw_hid.h"
#include "which_layer.h"

ASSERT_COMMUNITY_MODULES_MIN_API_VERSION(1, 1, 0);

#define WL_REPORT_SIZE 32 // QMK raw HID report size
#define WL_MAGIC 0xAA
#define WL_LAYER_STATE 0x01
#define WL_TOGGLES 0x02

#ifndef WHICH_LAYER_TOGGLES
#    define WHICH_LAYER_TOGGLES (WHICH_LAYER_CAPS | WHICH_LAYER_NKRO | WHICH_LAYER_SOCD)
#endif

// The module answers the overlay's request itself unless something else owns
// raw_hid_receive; then it resends the state periodically instead, so a newly
// started overlay still catches up. (Not by default: on hosts that don't read
// the endpoint while no app is open, each unread send can stall a scan ~10 ms.)
#if defined(VIA_ENABLE) || defined(WHICH_LAYER_NO_RAW_HID_RECEIVE)
#    define WL_OWNS_RECEIVE 0
#    ifndef WHICH_LAYER_RESEND_MS
#        define WHICH_LAYER_RESEND_MS 1000
#    endif
#else
#    define WL_OWNS_RECEIVE 1
#    ifndef WHICH_LAYER_RESEND_MS
#        define WHICH_LAYER_RESEND_MS 0
#    endif
#endif

// Present only when getreuer's socd_cleaner module is in the build; a weak
// reference resolves to NULL otherwise, so no build flags are needed.
extern bool socd_cleaner_enabled __attribute__((weak));

__attribute__((weak)) void which_layer_toggles_user(uint32_t *on, uint32_t *known) {}

static void wl_send(uint8_t type, uint32_t a, uint32_t b) {
    uint8_t report[WL_REPORT_SIZE] = {WL_MAGIC, type};
    memcpy(&report[2], &a, sizeof(a));
    memcpy(&report[6], &b, sizeof(b));
    raw_hid_send(report, sizeof(report));
}

static void wl_toggles(uint32_t *on, uint32_t *known) {
    led_t    led   = host_keyboard_led_state();
    uint32_t avail = WHICH_LAYER_CAPS | WHICH_LAYER_NUM | WHICH_LAYER_SCROLL;
#ifdef NKRO_ENABLE
    avail |= WHICH_LAYER_NKRO;
#endif
    if (&socd_cleaner_enabled != NULL) {
        avail |= WHICH_LAYER_SOCD;
    }
    *known = WHICH_LAYER_TOGGLES & avail;
    *on    = (led.caps_lock ? WHICH_LAYER_CAPS : 0) | (led.num_lock ? WHICH_LAYER_NUM : 0) |
          (led.scroll_lock ? WHICH_LAYER_SCROLL : 0) | (keymap_config.nkro ? WHICH_LAYER_NKRO : 0);
    if (&socd_cleaner_enabled != NULL && socd_cleaner_enabled) {
        *on |= WHICH_LAYER_SOCD;
    }
    which_layer_toggles_user(on, known);
    *on &= *known;
}

static void wl_send_all(void) {
    uint32_t on, known;
    wl_toggles(&on, &known);
    wl_send(WL_LAYER_STATE, layer_state, 0);
    wl_send(WL_TOGGLES, on, known);
}

layer_state_t layer_state_set_which_layer(layer_state_t state) {
    state = layer_state_set_which_layer_kb(state);
    if (is_keyboard_master()) {
        wl_send(WL_LAYER_STATE, state, 0);
    }
    return state;
}

// Toggles change from many places (host LEDs, keycodes, other modules), so
// they're watched here rather than hooked one by one.
void housekeeping_task_which_layer(void) {
    housekeeping_task_which_layer_kb();
    if (!is_keyboard_master()) {
        return;
    }
    static uint32_t last_on = UINT32_MAX, last_known = UINT32_MAX;
    uint32_t        on, known;
    wl_toggles(&on, &known);
    if (on != last_on || known != last_known) {
        last_on    = on;
        last_known = known;
        wl_send(WL_TOGGLES, on, known);
    }
#if WHICH_LAYER_RESEND_MS > 0
    static uint16_t resent = 0;
    if (timer_elapsed(resent) >= WHICH_LAYER_RESEND_MS) {
        resent = timer_read();
        wl_send_all();
    }
#endif
}

bool which_layer_raw_hid_receive(uint8_t *data, uint8_t length) {
    if (length < 2 || data[0] != WL_MAGIC) {
        return false;
    }
    if (data[1] == WL_LAYER_STATE) {
        wl_send_all();
    }
    return true;
}

#if WL_OWNS_RECEIVE
void raw_hid_receive(uint8_t *data, uint8_t length) {
    which_layer_raw_hid_receive(data, length);
}
#endif
