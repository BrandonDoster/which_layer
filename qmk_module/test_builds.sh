#!/bin/bash
# Builds the which_layer module into stock keymaps (JSON keymaps in /wl, outside
# the QMK tree) and checks which raw HID pieces got linked: plain AVR, AVR with
# VIA (VIA keeps raw_hid_receive), ARM, and a split board (crkbd, LTO: symbols
# are inlined, so only the size shows). Run inside the QMK CLI image, with the
# module installed as modules/brandondoster/which_layer:
#   docker run --rm -v ~/qmk_firmware:/qmk_firmware -v /tmp/wl:/wl \
#     ghcr.io/qmk/qmk_cli bash /wl/test_builds.sh
cd /qmk_firmware
mkdir -p /wl/out
onekey() { # keyboard, name, extra-json
cat > /wl/$2.json <<EOF
{ "keyboard": "$1", "keymap": "$2", "layout": "LAYOUT_ortho_1x1",
  "layers": [["KC_A"], ["KC_B"]], "modules": ["brandondoster/which_layer"] $3 }
EOF
}
onekey handwired/onekey/promicro wl_avr ''
onekey handwired/onekey/promicro wl_avr_via ', "config": {"features": {"via": true}}'
onekey handwired/onekey/blackpill_f411 wl_arm ''
qmk c2json -kb crkbd/rev1 -km wl_split keyboards/crkbd/keymaps/default/keymap.c 2>/dev/null \
  | python3 -c 'import json,sys; d=json.load(sys.stdin); d["keymap"]="wl_split"; d["modules"]=["brandondoster/which_layer"]; print(json.dumps(d))' > /wl/wl_split.json

for k in wl_avr wl_avr_via wl_arm wl_split; do
  echo "=== $k"
  if qmk compile /wl/$k.json > /wl/out/$k.log 2>&1; then
    grep -E "firmware size|is fine|bytes free" /wl/out/$k.log | tail -1
    elf=$(ls -t .build/*$k*.elf | head -1)
    nm=avr-nm; [[ $k == wl_arm ]] && nm=arm-none-eabi-nm
    $nm -C $elf | grep -E " (raw_hid_receive|via_command_kb|housekeeping_task_which_layer|layer_state_set_which_layer|socd_cleaner_enabled)$" | awk '{print "   ", $2, $3}'
  else
    echo "   BUILD FAILED"; grep -iE "error|warning" /wl/out/$k.log | head -8
  fi
done
