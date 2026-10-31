# SLogic driver

This crate connects Sipeed's SLogic Combo 8, SLogic16U3 and SLogic32U3 to HSync using Rust and `nusb`. The two U3 models share Sipeed's U3 register/AUX protocol; mocked USB transactions, golden control traces and synthetic sample streams exercise the implementation for each. A real SLogic16U3 has passed repeated USB 3.0 smoke captures across all three channel modes, including short captures configured at the advertised maximum rates; the SLogic32U3 support has not yet run on hardware. Channel mapping, signal timing, sustained high-rate throughput and USB 2.0 remain unverified on both.

## Official sources

- [Sipeed SLogic](https://github.com/sipeed/SLogic): the official desktop applications (SLogicView, PulseView, AllLogic, ngscopeclient, sigrok-cli), with the protocol specification in `build/docs/slogic-protocol.md`.
- [Sipeed libsigrok, `slogic-dev`](https://github.com/sipeed/libsigrok/tree/slogic-dev): its SLogic hardware driver.
- The reference revision for this port is [`0d728b3e1226a61738c99c3333ad16d5c2814728`](https://github.com/sipeed/libsigrok/tree/0d728b3e1226a61738c99c3333ad16d5c2814728/src/hardware/sipeed-slogic-analyzer). The `slogic/` directory contains the model tables (`slogic16u3.c`, `slogic32u3.c`), register and AUX protocol (`slogic.c`), link-dependent rate caps and sample unpacking (`slogic_data.c`) and transfer lifecycle (`slogic_usb.c`).
- The transfer ring follows the later revision [`ba92c5fe8728992574950b40810b2df43528ff5d`](https://github.com/sipeed/libsigrok/commit/ba92c5fe8728992574950b40810b2df43528ff5d), which sizes requests to about 5 ms of data and keeps at least four in flight.

The source reference is GPL-3.0-or-later, and this port carries the same license. No C driver, libusb or libsigrok library is linked. The Apache-2.0 uiharu engine is independent of this crate.

## Supported behavior

The capture firmware's identities `359f:3031` (SLogic16U3) and `359f:3032` (SLogic32U3) are recognized. The driver claims USB interface 0 and reads bulk endpoint `0x82`. The device's installed firmware is used without uploading firmware or FPGA bitstreams. Bootloader mode (`359f:30f1`) is not handled. Combo 8 uses the separate command protocol described below. Sipeed requires SLogic32U3 firmware S/N 202610150528 or newer, which gates its stream on whole sample words; the driver does not check the version.

Captures stream into HSync; Single, Repeat, Loop and Instant use the existing application controls. There is no Buffer mode, RLE, external clock or external trigger, and the SLogic32U3 alone supports its optional ADC module (four 8-bit analog channels read through the digital inputs). On U3 models the shared threshold ranges from 0 to 6 V, initially 1.7 V. Software triggers support the existing edge and level conditions, combined across enabled channels. Their pre-trigger history is bounded to 64 MiB (16,777,216 32-bit samples); the emitted trigger offset reflects the history actually retained.

Each USB request holds about 5 ms of samples, from 32 KiB up to 1 MiB, and up to 16 are in flight. The reference allows 2 MiB outside Windows; nusb always reads with WinUSB RAW_IO, which accepts at most 1 MiB, so every system keeps to that. On Linux the transfers of every program share usbcore's `usbfs_memory_mb` pool (16 MiB by default), and the ring keeps to three quarters of it: twelve 1 MiB requests by default, with smaller requests when the pool is too small for four. If the system still refuses a request, the capture goes on with those in flight while at least four remain, as the reference does, and otherwise fails with a hint to raise `/sys/module/usbcore/parameters/usbfs_memory_mb`.

The highest enabled physical channel selects the wire width and the device's native rate limit, which the negotiated link may lower (`slogic_link_max_rate()`): a 5 Gbps link carries about 400 MB/s and USB 2.0 about 40 MB/s of samples. Sustained host throughput has not yet been measured.

| Model      | Highest enabled channel | Wire channels | 10 Gbps  | 5 Gbps  | USB 2.0 |
| ---------- | ----------------------- | ------------- | -------- | ------- | ------- |
| SLogic16U3 | D0–D3                   | 4             | —        | 800 MHz | 80 MHz  |
| SLogic16U3 | D4–D7                   | 8             | —        | 400 MHz | 40 MHz  |
| SLogic16U3 | D8–D15                  | 16            | —        | 200 MHz | 20 MHz  |
| SLogic32U3 | D0–D3                   | 4             | 1400 MHz | 800 MHz | 80 MHz  |
| SLogic32U3 | D4–D7                   | 8             | 800 MHz  | 400 MHz | 40 MHz  |
| SLogic32U3 | D8–D15                  | 16            | 400 MHz  | 200 MHz | 20 MHz  |
| SLogic32U3 | D16–D31                 | 32            | 200 MHz  | 100 MHz | 10 MHz  |

U3 rates are selected from 5, 8, 10, 16, 20, 25, 32, 40, 50, 80, 100, 160, 200, 400 and 800 MHz, plus 1400 MHz on the SLogic32U3, after applying the link and channel limits. The SLogic32U3 reports two sample clocks, 1400 MHz and 800 MHz; like the reference, every rate the 800 MHz clock can divide down to uses it, and only 1400 MHz takes the other. A sparse selection that includes the highest channel of a mode still uses that mode's wire format. Only the enabled physical channels appear in HSync's sample packets. The wire stream has no sequence numbers or overflow counter: USB failures and stalls can be reported, but missing samples cannot be identified from the payload alone. Hardware timing checks against a known signal are needed when validating sustained rates.

## SLogic Combo 8

The Combo 8 model, START command and sample order are ported from
[Sipeed libsigrok revision bf6b346e597d112f48102fe6a7b2318c6d7b49e2](https://github.com/sipeed/libsigrok/tree/bf6b346e597d112f48102fe6a7b2318c6d7b49e2/src/hardware/sipeed-slogic-analyzer/slogic):
`slogic8u2.c`, `slogic.c`, `slogic_data.c` and `slogic_usb.c`. This is GPL-3.0-or-later
protocol porting, with no C code linked into HSync.

Select logic-analyzer mode (blue indicator) on the device. Both the original USB ID
`359f:0300` and newer `359f:3000` (named SLogic8U2 upstream) appear as **SLogic Combo 8**.
It uses interface 0 and endpoint `0x81`, not the U3 register map or endpoint `0x82`.
A vendor request `0xb1`, value/index zero, carries four bytes: sample rate in MHz
(little-endian u16), wire channel count, and pattern mode zero. No firmware is uploaded.
There is no adjustable threshold or analog input, and USB 2.0 is its native link.

| Highest enabled channel | Wire channels | Maximum rate |
| ----------------------- | ------------- | ------------ |
| D0–D1                   | 2             | 160 MHz      |
| D2–D3                   | 4             | 80 MHz       |
| D4–D7                   | 8             | 40 MHz       |

Rates are 1, 2, 4, 5, 8, 10, 16, 20, 32, 40, 80 and 160 MHz, limited by wire width.
This follows the current upstream table, including its 160 MHz mode beyond the original
80 MHz product specification. Two-channel samples occupy consecutive pairs of bits,
earliest in bits 0–1; four-channel samples use low then high nibbles, and eight-channel
samples use whole bytes. No partial head is discarded.

Transfers are queued before START. Combo 8 has no reliable STOP or RESET command:
teardown cancels and releases the queue, then drains at most 32 reads of 64 KiB with a
50 ms timeout each. Like the reference, exhausting this drain budget is accepted for
Combo 8; U3 models must become silent after STOP. A silent initial START is retried once
with fresh transfers and the same configuration. Simple software triggers, bounded
pre-trigger history and Single/Repeat/Loop/Instant reuse the shared SLogic stream pipeline.

All 66 valid rate/width combinations across both USB IDs have control traces generated
by running the upstream C code. Mocked acquisitions check sample order, sparse channels,
repeated captures, triggers, cancellation, unplugging and startup failures. **No Combo 8
hardware capture has been performed yet**: timing, channel mapping, firmware variants,
Windows throughput and sustained rates remain to be verified. Its debugger and UART
modes are outside this driver. The smoke example below supports `--channels 2` and skips
threshold configuration on this model.

## Hardware smoke test

Close other software using the analyzer. Windows needs WinUSB, and Linux needs the SLogic udev rule in the [project README](../../README.md#first-launch). Use a USB 3.0 data cable and port for the SLogic16U3, and a 10 Gbps cable and port for the SLogic32U3's full sampling rates.

List devices without opening or capturing:

```sh
cargo run -p hsync-slogic --example slogic_capture
```

Connect ground and a channel to a known low-frequency digital signal, then request a short capture explicitly:

```sh
cargo run -p hsync-slogic --example slogic_capture -- --capture
```

By default, the example uses every channel of the model at 5 MHz, a 1.7 V threshold where adjustable and 65,536 samples; it stops if an acquisition takes five seconds. `--channels 2|4|8|16|32` enables D0 through the selected count minus one, `--rate HZ` selects a supported sample rate, and `--repeat 1..10` repeats captures without closing the device. Captures always require `--capture`. For example, take three consecutive captures on D0–D3:

```sh
cargo run -p hsync-slogic --example slogic_capture -- --capture --channels 4 --repeat 3
```

It prints the USB link, completion reason, sample count and per-channel edge counts, and writes no files. A successful completion checks sample delivery; checking edge counts against the source signal is still necessary to establish channel mapping and sample timing. Further real-device validation should cover Stop, unplugging during capture, sparse channel selections, simple triggers, Loop and USB 2.0 on both models, and on the SLogic32U3 every wire mode at its 10 Gbps and 5 Gbps limits, including 1400 MHz.

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

The SLogic32U3 path is covered by the golden control traces of the reference's 32U3 model (all four wire modes, the 1400 MHz clock, the 800 MHz clock walk and the 32-channel mask) and by mocked captures of every wire mode, but no SLogic32U3 has been opened by HSync yet.
