# SLogic16U3 driver

This crate connects Sipeed's SLogic16U3 to HSync using Rust and `nusb`. It is an experimental port: mocked USB transactions and synthetic sample streams exercise the implementation, and a real SLogic16U3 has passed repeated USB 3.0 smoke captures across all three channel modes, including short captures configured at the advertised maximum rates. Channel mapping, signal timing, sustained high-rate throughput and USB 2.0 remain unverified.

## Official sources

- [Sipeed PulseView](https://github.com/sipeed/pulseview): the official desktop application.
- [Sipeed libsigrok, `slogic-dev`](https://github.com/sipeed/libsigrok/tree/slogic-dev): its SLogic hardware driver.
- The reference revision for this port is [`0d728b3e1226a61738c99c3333ad16d5c2814728`](https://github.com/sipeed/libsigrok/tree/0d728b3e1226a61738c99c3333ad16d5c2814728/src/hardware/sipeed-slogic-analyzer). The `slogic/` directory contains the model table (`slogic16u3.c`), register and AUX protocol (`slogic.c`), sample unpacking (`slogic_data.c`) and transfer lifecycle (`slogic_usb.c`).

The source reference is GPL-3.0-or-later, and this port carries the same license. No C driver, libusb or libsigrok library is linked. The Apache-2.0 uiharu engine is independent of this crate.

## Supported behavior

Only the normal SLogic16U3 identity `359f:3031` is recognized. The driver claims USB interface 0 and reads bulk endpoint `0x82`. The device's installed firmware is used without uploading firmware or FPGA bitstreams. Bootloader mode, Combo8 and SLogic32U3 are not handled.

Captures stream into HSync; Single, Repeat, Loop and Instant use the existing application controls. There is no Buffer mode, RLE, external clock or external trigger. The shared threshold ranges from 0 to 6 V, initially 1.7 V. Software triggers support the existing edge and level conditions, combined across enabled channels. Their pre-trigger history is bounded to 64 MiB (33,554,432 `u16` samples); the emitted trigger offset reflects the history actually retained.

The highest enabled physical channel selects the wire width and advertised device rate limit; sustained host throughput has not yet been measured:

| Highest enabled channel | Wire channels | Maximum USB 3.0 rate | Maximum USB 2.0 rate |
| ----------------------- | ------------- | -------------------- | -------------------- |
| D0–D3                   | 4             | 800 MHz              | 80 MHz               |
| D4–D7                   | 8             | 400 MHz              | 40 MHz               |
| D8–D15                  | 16            | 200 MHz              | 20 MHz               |

Rates are selected from 5, 8, 10, 16, 20, 25, 32, 40, 50, 80, 100, 160, 200, 400 and 800 MHz after applying the link and channel limits. A sparse selection that includes D15 still uses the 16-channel wire format. Only the enabled physical channels appear in HSync's sample packets. The wire stream has no sequence numbers or overflow counter: USB failures and stalls can be reported, but missing samples cannot be identified from the payload alone. Hardware timing checks against a known signal are needed when validating sustained rates.

## Hardware smoke test

Close other software using the analyzer. Windows needs WinUSB, and Linux needs the SLogic16U3 udev rule in the [project README](../../README.md#first-launch). Use a USB 3.0 data cable and port for the full sampling rates.

List devices without opening or capturing:

```sh
cargo run -p hsync-slogic --example slogic_capture
```

Connect ground and a channel to a known low-frequency digital signal, then request a short capture explicitly:

```sh
cargo run -p hsync-slogic --example slogic_capture -- --capture
```

By default, the example uses all 16 channels at 5 MHz, a 1.7 V threshold and 65,536 samples; it stops if an acquisition takes five seconds. `--channels 4|8|16` enables D0 through the selected count minus one, `--rate HZ` selects a supported sample rate, and `--repeat 1..10` repeats captures without closing the device. Captures always require `--capture`. For example, take three consecutive captures on D0–D3:

```sh
cargo run -p hsync-slogic --example slogic_capture -- --capture --channels 4 --repeat 3
```

It prints the USB link, completion reason, sample count and per-channel edge counts, and writes no files. A successful completion checks sample delivery; checking edge counts against the source signal is still necessary to establish channel mapping and sample timing. Further real-device validation should cover Stop, unplugging during capture, sparse channel selections, simple triggers, Loop and USB 2.0 before the port loses its experimental label.

## Validation status

A real SLogic16U3 was discovered through nusb and opened on a SuperSpeed link. With a 1.7 V threshold, the following captures each delivered exactly 65,536 samples and ended with `Completed`:

| Enabled channels | Configured sample rate | Captures                        |
| ---------------- | ---------------------- | ------------------------------- |
| D0–D3            | 5 MHz                  | 3 consecutive, device kept open |
| D0–D7            | 5 MHz                  | 3 consecutive, device kept open |
| D0–D15           | 5 MHz                  | 3 consecutive, device kept open |
| D0–D3            | 800 MHz                | 1 short capture                 |
| D0–D7            | 400 MHz                | 1 short capture                 |
| D0–D15           | 200 MHz                | 1 short capture                 |

The 5 MHz runs completed in 117–133 ms including acquisition setup and teardown. This wall-clock duration is not a sustained-throughput measurement. All captured channels had zero edges, so the matrix verifies device communication, sample delivery and repeated capture setup, but does not establish channel mapping or sample timing. Known-signal timing checks, sustained high-rate captures and USB 2.0 remain unverified.
