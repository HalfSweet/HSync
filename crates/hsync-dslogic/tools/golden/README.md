# DSLogic golden data

`tests/fixtures/settings.txt` holds the bytes DSView's own `dsl_fpga_arm()` sends to arm a
capture (`DSL_setting`, plus `DSL_setting_ext32` on 32-channel models) for each case in
`cases.txt`. The `setting` tests build the same cases with the Rust port and compare every byte.

To regenerate after changing `cases.txt`, with glib, libusb (headers only) and a C compiler
installed:

```sh
DSVIEW=/path/to/DSView ./build.sh
```

`harness.c` links DSView's `hardware/DSL/dsl.c`, `hardware/DSL/command.c` and `trigger.c`
unchanged, intercepts `libusb_bulk_transfer` and `libusb_control_transfer`, and fills the device
context the way DSView's capture start does (`dev_acquisition_start`, `TriggerDock`,
`LogicSignal::commit_trig`).
