# ESP32-S3 Boot Fix Summary

## Problem

Flashing appeared to succeed, but the board failed before the app started:

```text
Invalid image block, can't boot.
ets_main.c 329
```

The ROM bootloader was failing while loading the second-stage bootloader, before any Rust code could run.

## Diagnosis

The generated images were valid and flash verification passed, but the ESP32-S3 ROM was misreading flash when the image used `DIO/40 MHz`.

Evidence:

- Flash contents matched the generated bootloader after readback.
- Secure boot and flash encryption were disabled.
- The failure happened before the app partition was loaded.
- Reflashing with conservative flash settings made the same bootloader load correctly.

## Fix

The board's embedded flash boots reliably with:

```text
Flash mode: DOUT
Flash freq: 20 MHz
Flash size: 16 MB
```

These settings were made permanent in `sdkconfig.defaults`:

```text
CONFIG_ESPTOOLPY_FLASHMODE_DOUT=y
CONFIG_ESPTOOLPY_FLASHMODE="dout"
CONFIG_ESPTOOLPY_FLASHFREQ_20M=y
CONFIG_ESPTOOLPY_FLASHFREQ="20m"
```

The Cargo runner was also updated in `.cargo/config.toml` so normal flashing uses the same safe settings:

```toml
runner = "espflash flash --flash-mode dout --flash-freq 20mhz --flash-size 16mb --monitor"
```

## Important Flashing Detail

`espflash flash target/.../esp32-lcd-test --monitor` only flashes the Rust app image. If the bootloader is missing, erased, or was built with bad flash settings, the board may still fail before the app starts.

For recovery or first flash, flash the DOUT/20 MHz bootloader, partition table, and Rust ELF together using the command documented in `README.md`.

## Verification

After rebuilding and flashing with DOUT/20 MHz, the board booted successfully:

```text
Boot SPI Speed : 20MHz
SPI Mode       : DOUT
esp32_lcd_test: ESP32-S3 LCD test starting...
esp32_lcd_test: ST7789 init done
esp32_lcd_test: Entering display loop
```

Confirmed commands:

```bash
cargo build --release
cargo fmt --check
```

Both passed.
