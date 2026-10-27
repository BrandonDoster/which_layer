# Dos60S

![Dos60S](https://i.imgur.com/PLACEHOLDER.jpg)

A 60-key column-staggered split keyboard: 5x6 per half with two rotated thumb keys, driven by a Pro Micro on each half.

* Keyboard Maintainer: [Brandon Doster](https://github.com/BrandonDoster)
* Hardware Supported: Dos60S PCB, Pro Micro (ATmega32U4)
* Hardware Availability: [DosBoards/Dos60S](https://github.com/DosBoards/Dos60S)

Make example for this keyboard (after setting up your build environment):

    make dosboards/dos60s:default

Flashing example for this keyboard:

    make dosboards/dos60s:default:flash

See the [build environment setup](https://docs.qmk.fm/#/getting_started_build_tools) and the [make instructions](https://docs.qmk.fm/#/getting_started_make_guide) for more information. Brand new to QMK? Start with our [Complete Newbs Guide](https://docs.qmk.fm/#/newbs).

## Bootloader

Enter the bootloader in 2 ways:

* **Bootmagic reset**: Hold down the top left key (Esc) on the left half and plug in the keyboard
* **Physical reset**: Short the RST and GND pins on the Pro Micro twice quickly

Flash each half separately with the USB cable plugged into that half.
