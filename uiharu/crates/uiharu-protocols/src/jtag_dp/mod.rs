// SPDX-License-Identifier: Apache-2.0

//! Single-TAP Arm ADIv5 JTAG-DP, from ARM IHI0031C §§3.3–3.4:
//! <https://e2e.ti.com/cfs-file/__key/communityserver-discussions-components-files/538/IHI0031C_5F00_debug_5F00_interface_5F00_as.pdf>.
//!
//! - Only normally completed scans commit instructions or requests.
//! - Four-bit IR and 1/32/35-bit DR scans are decoded, least significant bit first.
//! - Captured response belongs to the preceding accepted access, not the current request.
//! - WAIT discards the current request and preserves the preceding request association.
//! - OK/FAULT never claims success; response data is always raw, possibly unknown.
//! - A gap, invalid scan or ABORT loses pipeline association; an observed TAP reset
//!   selects IDCODE. A gap also loses instruction knowledge until Update-IR or reset.
//! - At most 35 bits per line and one preceding scan id are kept.

mod decoder;
mod events;
pub mod synth;
pub use decoder::{JtagDp, JtagDpConfig};
pub use events::{Ack, JTAG_DP_EVENTS, JtagDpEvent, ScanStatus};
