// SPDX-License-Identifier: Apache-2.0

//! MDIO, the management interface of Ethernet PHYs: the decoder from logic samples to
//! `mdio.events`, one event per management frame, and a waveform synthesizer.
//!
//! Written from IEEE Std 802.3: clause 22, the MII management interface — the MDC and MDIO
//! signals and the management frame structure, its preamble, start, operation codes,
//! addresses, turnaround and data — and clause 45, the management interface of MDIO
//! manageable devices, whose frames start with `00` and address a register in two steps. The
//! rules:
//!
//! - every bit is MDIO's level at the tick MDC rises; MDC's falling edges and MDIO's own
//!   changes are not looked at, so MDC may run between frames or stop;
//! - a frame is 32 bits after its preamble: ST (2 bits), OP (2), PHYAD — PRTAD in clause 45 —
//!   (5), REGAD — DEVAD in clause 45 — (5), the turnaround TA (2) and the data (16), every
//!   field most significant bit first. ST `01` is clause 22, whose OP is `10` for a read and
//!   `01` for a write; ST `00` is clause 45, whose OP is `00` for an address frame, `01` for a
//!   write, `11` for a read and `10` for a post-read-increment-address read;
//! - in sync, every one is idle or preamble and the first zero is ST's first bit, however many
//!   ones came before it — none for a station that suppresses the preamble and stops MDC
//!   between frames. The ones in a row before it are counted up to [`PREAMBLE_BITS`], the
//!   length of a preamble: ones before those are idle, and the frame spans from the first one
//!   counted, or from ST's first bit without a preamble;
//! - the decoder is in sync after a whole frame and after [`PREAMBLE_BITS`] ones in a row, as
//!   a PHY must see them before it answers. At the first sample, after a gap and after a start
//!   error it is not: it skips bits until that many ones in a row, so a frame already under
//!   way there is not decoded, nor are frames without a preamble until then;
//! - a clause 22 start with an OP that clause 22 does not define, `00` or `11`, is no frame:
//!   `error { reason: start }` covering ST and OP, and the decoder skips bits until it is in
//!   sync again, since it cannot tell where the bits it read end;
//! - the turnaround follows who drives the data: on a write and an address frame the station
//!   keeps the line and sends `10`; on a read and a post-read-increment read it releases the
//!   line for the first bit, which is then nobody's, and the device drives the second `0`. A
//!   turnaround that breaks this is reported as `error { reason: turnaround }` when its second
//!   bit is read — a read with no device answering shows a second bit of `1` — and the frame
//!   goes on: it is reported when whole, with the turnaround bits as seen;
//! - a frame is reported once its last data bit is read, settled there;
//! - a gap or the end of the samples cuts the frame under way: `error { reason: aborted }`
//!   covering it from its start through the last bit read, settled at the gap's start or the
//!   end of the samples. Ones counted before a gap do not count after it;
//! - progress is passed on, held back to the start of the frame under way, or to the first of
//!   the ones that may turn out to be the next frame's preamble, so that no event that follows
//!   starts before it;
//! - a bit decides at most one event.
//!
//! Limits: one station's view of one bus, read on one clock edge; the frame's fields are bus
//! data, not interpreted — no register is named, and a clause 45 register access is not
//! paired with the address frame before it. Frames longer than 32 bits, such as some
//! vendors' extensions, are not decoded.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{FRAME_BITS, Mdio, MdioConfig, MdioConfigError, PREAMBLE_BITS};
pub use events::{Clause, ErrorReason, MDIO_EVENTS, MdioEvent, Op, UnknownClause};
