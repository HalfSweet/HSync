# ATK-Logic golden data

`tests/fixtures/protocol.txt` holds the bytes the ATK-Logic application builds for each case in
`cases.txt`: command frames (`USBControl::Write()`), the arguments of the capture settings and of
the simple trigger (`SessionController::start()`), and blocks put into and out of the device's
lane order (`logic_analyzer_convert_to_device()`, `logic_analyzer_convert_to_pc()`). The
`protocol` tests build the same cases with the Rust port and compare every byte.

To regenerate after changing `cases.txt`, with a C++ compiler installed:

```sh
ATK=/path/to/atk-logic ./build.sh
```

`build.sh` extracts `gCRC32()` and its table from `pv/static/util.cpp` and the lane conversions
from `pv/usb/usb_base.cpp` unchanged; `harness.cpp` transcribes the byte building of
`USBControl::Write()` and `SessionController::start()` without Qt.
