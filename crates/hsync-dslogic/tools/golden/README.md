# DSLogic golden data

`tests/fixtures/settings.txt` holds the bytes DSView's own `dsl_fpga_arm()` sends to arm a
capture (`DSL_setting`, plus `DSL_setting_ext32` on 32-channel models) for each case in
`cases.txt`. The `setting` tests build the same cases with the Rust port and compare every byte.

To regenerate after changing `cases.txt`, with glib, libusb and a C compiler
installed:

```sh
DSVIEW=/path/to/DSView ./build.sh
```

`harness.c` links DSView's `hardware/DSL/dslogic.c`, `hardware/DSL/dsl.c`,
`hardware/DSL/command.c` and `trigger.c`
unchanged, intercepts `libusb_bulk_transfer` and `libusb_control_transfer`, and fills the device
context the way DSView's capture start does (`dev_acquisition_start`, `TriggerDock`,
`LogicSignal::commit_trig`).

The instant option goes through the real `DSLogic_driver_info.config_set()` in logic mode.
DSView only sets `devc.instant` in oscilloscope mode; an instant logic capture resets the
trigger without setting FPGA mode bit 9. Assigning the context field directly would create
settings that DSView never sends for logic captures.

The cases include U3Pro16 Buffer captures at 500 MHz and 1 GHz on USB 2 and USB 3, USB 3 Stream
captures at both rates, and instant captures with simple, advanced and serial triggers.
The fixture was regenerated against DSView commit
`2e9e2c8e726df4ef5687d39b83d4f797cc44b574`.
