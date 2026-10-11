# Device firmware

Firmware images and FPGA bitstreams that HSync loads into the devices it drives, copied
unchanged from each vendor's own application.

## DSLogic

FX2 firmware images (`*.fw`) and FPGA bitstreams (`DSLogic*.bin`) for DreamSourceLab DSLogic
logic analyzers, copied from DSView's `DSView/res` directory. DreamSourceLab publishes them
under the MIT license in `LICENSE`; they are not covered by HSync's GPL.

The file names are the ones `crates/hsync-dslogic/src/profiles.rs` refers to. Only the
original DSLogic and the DSLogic Pro receive FX2 firmware from the host; every other model
boots its own and only needs its bitstream.

## PXLogic

The MCU firmware (`SCI_LOGIC.bin`) and FPGA bitstreams (`hspi_ddr_RST.bin`, `hspi_ddr.bin`)
for PXLogic logic analyzers, copied from the `PXView/res` directory of PXView 1.6.5 (commit
`ce89195`). PXView ships them in its GPL-3.0-or-later repository without a separate license.

The file names are the ones `crates/hsync-pxlogic/src/profiles.rs` refers to. Every opening
loads both bitstreams into the FPGA; a device running another MCU firmware version first has
`SCI_LOGIC.bin` written into its flash and restarts.
