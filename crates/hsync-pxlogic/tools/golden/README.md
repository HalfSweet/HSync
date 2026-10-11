# PXLogic golden data

`tests/fixtures/sequences.txt` holds what PXView's own `pxlogic` driver does on the wire for each
case in `cases.txt`: the register reads and writes, cleared halts, uploads and transfers of opening
a device (`hw_usb_open()`) and of starting a capture (`start_transfers()`). The `sequences` tests
run the same cases through the Rust port against a simulated device and compare every operation
but the transfers on the data endpoint: PXView reads whole blocks of up to 4 MiB, HSync reads them
in transfers of at most 1 MiB (`MAX_TRANSFER` in `src/acquisition.rs` says why).

To regenerate after changing `cases.txt`, with glib, libusb (headers only) and a C compiler
installed, against a checkout of [PXLogic's libsigrok](https://github.com/PXLogic/libsigrok)
(commit `c7dc641`, which PXView 1.6.5 pins):

```sh
PXLIB=/path/to/libsigrok ./build.sh
```

`harness.c` builds `src/hardware/pxlogic/pxlogic.c` and `usb_ctrl.c` unchanged, replaces libusb
with a fake device that records the transfers, and sets each case up through the driver's own
`config_set()`.
