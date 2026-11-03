# SLogic driver

This crate connects Sipeed's SLogic Combo 8, SLogic16U3 and SLogic32U3 to HSync using Rust and `nusb`. The two U3 models share Sipeed's U3 register/AUX protocol; mocked USB transactions, golden control traces and synthetic sample streams exercise the implementation for each. A real SLogic16U3 has passed repeated USB 3.0 smoke captures across all three channel modes, including short captures configured at the advertised maximum rates. The SLogic32U3 has also passed repeated hardware smoke captures. Channel mapping, signal timing and USB 2.0 remain unverified on both.

## Official sources

- [Sipeed SLogic](https://github.com/sipeed/SLogic): the official desktop applications (SLogicView, PulseView, AllLogic, ngscopeclient, sigrok-cli), with the protocol specification in `build/docs/slogic-protocol.md`.
- [Sipeed libsigrok, `slogic-dev`](https://github.com/sipeed/libsigrok/tree/slogic-dev): its SLogic hardware driver.
- The reference revision for this port is [`0d728b3e1226a61738c99c3333ad16d5c2814728`](https://github.com/sipeed/libsigrok/tree/0d728b3e1226a61738c99c3333ad16d5c2814728/src/hardware/sipeed-slogic-analyzer). The `slogic/` directory contains the model tables (`slogic16u3.c`, `slogic32u3.c`), register and AUX protocol (`slogic.c`), link-dependent rate caps and sample unpacking (`slogic_data.c`) and transfer lifecycle (`slogic_usb.c`).
- The transfer ring follows the later revision [`ba92c5fe8728992574950b40810b2df43528ff5d`](https://github.com/sipeed/libsigrok/commit/ba92c5fe8728992574950b40810b2df43528ff5d), which sizes requests to about 5 ms of data and keeps at least four in flight.

The source reference is GPL-3.0-or-later, and this port carries the same license. No C driver, libusb or libsigrok library is linked. The Apache-2.0 uiharu engine is independent of this crate.

## Supported behavior

The capture firmware's identities `359f:3031` (SLogic16U3) and `359f:3032` (SLogic32U3) are recognized. The driver claims USB interface 0 and reads bulk endpoint `0x82`. The device's installed firmware is used without uploading firmware or FPGA bitstreams. Bootloader mode (`359f:30f1`) is not handled. Combo 8 uses the separate command protocol described below. Sipeed requires SLogic32U3 firmware S/N 202610150528 or newer, which gates its stream on whole sample words; the driver does not check the version.

Captures stream into HSync; Single, Repeat, Loop and Instant use the existing application controls. There is no Buffer mode, RLE, external clock or external trigger, and the SLogic32U3 alone supports its optional ADC module (four 8-bit analog channels read through the digital inputs). On U3 models the shared threshold ranges from 0 to 6 V, initially 1.7 V. Software triggers support the existing edge and level conditions, combined across enabled channels. Their pre-trigger history retains at most 16,777,216 samples, one bit per kept channel each and so at most 64 MiB; the emitted trigger offset reflects the history actually retained.

Each USB request holds about 5 ms of samples, from 32 KiB up to 1 MiB, and up to 16 are in flight. The reference allows 2 MiB outside Windows; nusb always reads with WinUSB RAW_IO, which accepts at most 1 MiB, so every system keeps to that. On Linux the transfers of every program share usbcore's `usbfs_memory_mb` pool (16 MiB by default), and the ring keeps to three quarters of it: twelve 1 MiB requests by default, with smaller requests when the pool is too small for four. If the system still refuses a request, the capture goes on with those in flight while at least four remain, as the reference does, and otherwise fails with a hint to raise `/sys/module/usbcore/parameters/usbfs_memory_mb`. A finite untriggered capture queues no request once those in flight carry every byte it needs.

As in the capture engine of Sipeed's SLogic 2.3.3 ([`slogic/slogic_usb.c`](https://github.com/sipeed/SLogic/tree/af106af699ce41602fdda24e16a69f9d62e1387c/slogic)), one thread only takes completed requests, queues them again and hands their bytes to a second thread, which assembles them into blocks, applies the trigger and sends them to HSync. Between them a queue holds up to 256 MiB; a converter that falls further behind ends the capture with a USB bandwidth error rather than losing samples unnoticed. Loss detection compares the recent stream rate with 90% of the rate the analyzer sends. It samples cumulative byte counts at most every 25 ms and measures at least the latest 250 ms (normally 250–275 ms), retaining at most twelve checkpoints. A healthy prefix therefore cannot hide a later slowdown. The first transfer anchors the window without contributing bytes from its unknown sampling interval; captures shorter than a full window are not checked by this heuristic. The converter turns each block of 64 samples straight from the wire bytes into HSync's cross format, one word per kept channel: 8, 16 and 32 channels with two 8×8 transposes per byte plane, 2 and 4 channels with masks. It applies the trigger, its history and the limit to those blocks: a condition is a word operation over 64 samples, the history a ring of whole blocks, and a trigger whose history starts inside a block shifts the blocks sent directly into the output packets, reusing the held block instead of allocating temporary vectors per block. On one core of an i5-12400, on random data, it converts 2 to 32 wire channels at 1.55 to 1.65 GB/s, at 1.25 to 1.45 GB/s while waiting for a trigger and at about 1.05 GB/s with two analog groups; the fastest wire modes send 800 MB/s. `cargo run --release -p hsync-slogic --example stream_bench` measures immediate captures, captures waiting for a trigger, and captures after a trigger at sample 1 for every wire width and mixed analog inputs.

Once all bytes of a finite untriggered capture are queued, USB stops while the converter drains the queue. The device stays busy until conversion finishes. Converter failures, receiver disconnection and thread panics also terminate the acquisition; they do not depend on another USB completion to be noticed.

Each capture ends with one `slogic-report` line in the log, at info level, in the spirit of Sipeed's: bytes and transfers received, requests refused, `cold_ms` to the first data, the stream's `span_ms` and `mbps` beside `exp_mbps`, the converter's `backlog_peak` in requests, `submit_ms` spent queueing requests, `busy_ms` spent converting, and `convert_cpu_ms`, the converter's CPU time on Linux; below `busy_ms`, it waited for a CPU.

The highest enabled physical channel selects the wire width and the device's native rate limit, which the negotiated link may lower (`slogic_link_max_rate()`): a 5 Gbps link carries about 400 MB/s and USB 2.0 about 40 MB/s of samples. On an SLogic32U3 at 10 Gbps, every wire mode at its fastest rate (4 channels at 1400 MHz, 8 at 800, 16 at 400 and 32 at 200) streamed for a second without loss, and kept up for six seconds while waiting for a trigger that never fired.

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

By default, the example uses every channel of the model at 5 MHz, a 1.7 V threshold where adjustable and 65,536 samples, captured at once; it stops if an acquisition takes five seconds plus ten times the time the analyzer takes to sample them. `--channels 2|4|8|16|32` enables D0 through the selected count minus one, `--rate HZ` selects a supported sample rate, and `--repeat 1..10` repeats captures without closing the device. `--model NAME` selects an exact model when several analyzers are connected. `--samples N` sets the sample count, a positive multiple of 64, and the summary compares how long the capture took with how long the analyzer takes to sample it. `--trigger CHANNEL:high|low|rise|fall|edge`, which may be repeated, waits for all its conditions with a tenth of the samples before them; conditions that never hold, such as `--trigger 0:high --trigger 0:low`, keep a capture waiting at full rate until it times out. `--pattern emulation` has the analyzer send Sipeed's Emulation pattern instead of its inputs, a counter of the samples whose bits the channels carry, and checks every sample against it on the SLogic16U3 and SLogic32U3 with 4, 8, 16 or 32 channels and on the SLogic Combo 8 with 2, 4 or 8: from 0 for a capture started at once, from the counter of its first sample after a trigger. Captures always require `--capture`. For example, take three consecutive captures on D0–D3:

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

The SLogic32U3 path is covered by the golden control traces of the reference's 32U3 model
(all four wire modes, the 1400 MHz clock, the 800 MHz clock walk and the 32-channel mask)
and by mocked captures of every wire mode.

SLogic32U3 hardware smoke tests have also completed repeated captures on D0–D3 at
1400 MHz. The inputs were static; the Emulation pattern checks continuity from end to end,
but it carries no timing reference of its own.

Sustained captures on an SLogic32U3 at 10 Gbps, a second of samples each, with the smoke
test (`--samples` set to the sample rate; `--trigger 0:high --trigger 0:low` waits for
six seconds for a trigger that never fires; `--trigger 0:low` fires once the history of
16,777,216 samples has filled), before and after the two-thread capture and the block
conversion, on an i5-12400 with static inputs:

| Capture                      | Stream   | HSync 0.2.8           | Now             | Converter busy | Most waiting transfers |
| ---------------------------- | -------- | --------------------- | --------------- | -------------- | ---------------------- |
| D0–D3 at 1400 MHz            | 700 MB/s | 1058 ms               | 1058 ms         | 49%            | 9                      |
| D0–D7 at 400 MHz             | 400 MB/s | 1063 ms               | 1059 ms         | 37%            | 2                      |
| D0–D15 at 400 MHz            | 800 MB/s | 1344 ms, samples lost | 1057 ms         | 56%            | 14                     |
| D0–D31 at 200 MHz            | 800 MB/s | 1568 ms, samples lost | 1058 ms         | 56%            | 15                     |
| D0–D7 at 800 MHz             | 800 MB/s | 1859 ms, samples lost | 1058 ms         | 56%            | 4                      |
| D0–D3 at 1400 MHz, waiting   | 700 MB/s | not reported          | 6 s at 700 MB/s | 59%            | 9                      |
| D0–D7 at 800 MHz, waiting    | 800 MB/s | not reported          | 6 s at 800 MB/s | 63%            | 7                      |
| D0–D3 at 1400 MHz, triggered | 700 MB/s | not measured          | 1060 ms         | 53%            | 11                     |
| D0–D7 at 800 MHz, triggered  | 800 MB/s | not measured          | 1058 ms         | 59%            | 12                     |

The times include capture setup and teardown, about 60 ms. 0.2.8 completed every capture
with every sample it asked for; the analyzer had dropped what its FIFO could not hold.
Busy is the converter's share of the stream's span, from the `slogic-report` line.

With `--pattern emulation` on an SLogic32U3 and an SLogic16U3, each linked at 5 Gbps,
every sample matched the pattern, checked from counter 0. On both analyzers: 800,000,000
samples on D0–D3 at 800 MHz, 400,000,000 on D0–D7 at 400 MHz and 200,000,000 on D0–D15 at
200 MHz; on the SLogic32U3 also 100,000,000 on D0–D31 at 100 MHz, whose upper half follows
the pattern's second rule. Three repeated captures of D0–D7 matched too, and so did a
capture triggered on a rise of D3, checked from the counter 8 of its first sample. The same
captures with the Normal pattern carried no counter, and 2 channels, which the pattern does
not list, were reported as not checked.

An SLogic Combo 8 (product id 359f:3000, its newer firmware) on USB 2.0 streamed a second of
each wire mode at its fastest rate, D0–D1 at 160 MHz, D0–D3 at 80 MHz and D0–D7 at 40 MHz,
at 40 MB/s and without overflow in about 1058 ms; three repeated captures completed, a level
trigger fired, and a trigger that never held timed out. Its Emulation pattern, which Sipeed
does not list, is a plain counter as well: each channel toggles on the previous one's falling
edge, D0–D1 carry counter bits 1 and 2 and D0–D7 bits 0 to 7. Every sample of a second of
each wire mode matched, and so did captures triggered on a rise of D1 (checked from counter 4) and of D3 (from counter 8).

### Conversion after an unaligned trigger

A CPU-only comparison on an Apple M4 Pro, with the same `stream_bench` cases and
512 MiB of random wire data per case in release mode. The baseline is the merged
#78 implementation (`2593a6c`); each shifted case fires at sample 1 before timing
starts. These measurements exclude USB and capture storage and are not a hardware
continuity test.

| Shifted capture |       #78 | Reused buffers |
| --------------- | --------: | -------------: |
| 2 channels      |  541 MB/s |      1424 MB/s |
| 4 channels      |  824 MB/s |      1655 MB/s |
| 8 channels      | 1310 MB/s |      1902 MB/s |
| 16 channels     | 1725 MB/s |      2022 MB/s |
| 32 channels     | 1972 MB/s |      2102 MB/s |
| A0 A1 + D16–D31 | 1121 MB/s |      1476 MB/s |

The untriggered 4-channel case remained about 2.49 GB/s. Timings vary by host;
the benchmark includes both paths so shifted captures stay visible in future
performance work.
