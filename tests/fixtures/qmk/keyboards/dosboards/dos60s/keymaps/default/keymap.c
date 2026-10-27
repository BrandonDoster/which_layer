// Copyright 2026 Brandon Doster (@BrandonDoster)
// SPDX-License-Identifier: GPL-3.0-or-later

#include QMK_KEYBOARD_H

const uint16_t PROGMEM keymaps[][MATRIX_ROWS][MATRIX_COLS] = {
    [0] = LAYOUT(
        QK_GESC, KC_1, KC_2, KC_3, KC_4, KC_5,                                      KC_6, KC_7, KC_8, KC_9, KC_0, KC_BSPC, 
        KC_TAB, KC_Q, KC_W, KC_E, KC_R, KC_T,                                       KC_Y,KC_U, KC_I, KC_O, KC_P, KC_EQL, 
        LT(1,KC_CAPS), KC_A, KC_S, KC_D, KC_F, KC_G,                               KC_H, KC_J, KC_K, KC_L, KC_SCLN, KC_QUOT, 
        KC_LSFT, KC_Z, KC_X, KC_C, KC_V, KC_B,                                      KC_N, KC_M, KC_COMM, KC_DOT, KC_SLSH, KC_RSFT,
        KC_LCTL, KC_NO, KC_LGUI, KC_LALT, KC_SPC, LT(2,KC_BSLS),                  KC_ENT, RALT_T(KC_SPC), KC_LEFT, LT(1,KC_UP), LT(1,KC_DOWN), KC_RGHT 
    ),
    [1] = LAYOUT(
        KC_TRNS, KC_F1, KC_F2, KC_F3, KC_F4, KC_F5,                          KC_F6, KC_F7, KC_F8, KC_F9, KC_F10, KC_F11, 
        KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS,               KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_F12, 
        KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS,               KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, 
        KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS,               KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS,
        KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS,               KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS
    ),
    [2] = LAYOUT(
        KC_GRAVE, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS,              KC_TRNS, KC_TRNS, KC_TRNS, KC_LBRC, KC_RBRC, KC_DEL, 
        KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS,               KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_MINUS,
        KC_TRNS, KC_TRNS, SOCDTOG, KC_TRNS, KC_TRNS, KC_TRNS,               KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS,
        KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS,               KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS,
        KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS,               KC_TRNS, KC_TRNS, KC_HOME, KC_PAGE_UP, KC_PAGE_DOWN, KC_END
    )
};
socd_cleaner_t socd_opposing_pairs[] = {
    {{KC_A, KC_D}, SOCD_CLEANER_LAST},
};

// Layer state over raw HID for the which_layer overlay
// (https://github.com/BrandonDoster/which_layer).
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

// The overlay asks for the current state when it connects.
void raw_hid_receive(uint8_t *data, uint8_t length) {
    if (length >= 2 && data[0] == WL_MAGIC && data[1] == WL_LAYER_STATE) {
        wl_send_layer_state(layer_state);
    }
}
