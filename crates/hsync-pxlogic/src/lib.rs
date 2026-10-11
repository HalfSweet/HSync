//! PXLogic driver (PX-Logic 32, 16 Pro, 16 Plus, 16 Base): the `pxlogic` driver of PXView's
//! libsigrok (`src/hardware/pxlogic`) ported to Rust and nusb.

#![forbid(unsafe_code)]

pub mod profiles;
pub mod protocol;
