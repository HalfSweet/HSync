# SLogic U3 protocol reference

`tests/fixtures/protocol.txt` records the control transfers made by the actual C implementation
in [Sipeed's libsigrok repository](https://github.com/sipeed/libsigrok/tree/slogic-dev), commit
`0d728b3e1226a61738c99c3333ad16d5c2814728`. The reference code is GPL-3.0-or-later;
the Rust protocol and sample unpacking files retain that license and attribution.

The generator compiles `slogic.c` and its model tables unchanged, replacing USB access with a
small register model. It does not link libusb, access hardware, or become part of HSync. The
Rust tests replay each command and response through `hsync-driver`'s `MockTransport`.

The SLogic16U3 cases cover its three channel widths, the fastest rates and slowest rate,
thresholds at both limits and between them, AUX readiness polling, extended payload
preservation, clock slot selection, and fallback after wrapping an empty clock slot. The
SLogic32U3 cases (`u32_*`) run the reference's 32U3 model against its two clock slots,
1400 MHz then 800 MHz: each of the four channel widths at its 10 Gbps limit, the 32-channel
limit of a 5 Gbps link, and the slowest rate. Each sequence includes
RESET, configuration, RUN, and STOP. Separate Rust tests cover short reads, polling limits,
open-time draining, and fragmented sample decoding.

Regenerate with a C compiler and the pinned reference checkout:

```sh
SLOGIC=/path/to/sipeed-libsigrok sh crates/hsync-slogic/tools/golden/build.sh
```

SLogic16U3 captures physical D0–D3, D0–D7, or D0–D15, and SLogic32U3 also D0–D31. The
4-channel stream puts the earlier sample in the low nibble; 8-channel samples are bytes;
16- and 32-channel samples are little-endian words of two and four bytes. On the
SLogic16U3 the first four bytes after RUN are discarded because its firmware opens the gate
mid-word; SLogic32U3 firmware gates on whole words, so nothing is dropped. Disabled channels
inside the selected wire mode are filtered on the host.

HSync preserves the reference's control commands and order. It rejects short register reads,
failures to stop an old capture, and exhausted drain budgets instead of silently continuing,
bounds all waits, and
caps individual bulk transfers at 1 MiB for nusb's WinUSB RAW_IO path. STOP is sent while
bulk reads are still queued, before cancellation, as required by the reference firmware.
