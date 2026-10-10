# DSLogic firmware

FX2 firmware images (`*.fw`) and FPGA bitstreams (`*.bin`) for DreamSourceLab DSLogic logic
analyzers, copied unchanged from DSView's `DSView/res` directory. DreamSourceLab publishes them
under the MIT license in `LICENSE`; they are not covered by HSync's GPL.

The file names are the ones `crates/hsync-dslogic/src/profiles.rs` refers to. Only the
original DSLogic and the DSLogic Pro receive FX2 firmware from the host; every other model
boots its own and only needs its bitstream.
