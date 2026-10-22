// SPDX-License-Identifier: Apache-2.0

use super::{Ack, JtagDpEvent, ScanStatus};
use crate::jtag::{JtagBus, JtagEvent, Next, Read, Register, Scan};
use serde::{Deserialize, Serialize};
use uiharu_core::{
    Inbox, InputRef, InputSpec, Io, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef,
    OutputSpec, Props, Requires,
};

/// Single-TAP decoding has no selectable options; chain layouts are unsupported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JtagDpConfig {}
/// Arm ADIv5 single-TAP debug transport decoder.
#[derive(Clone, Copy, Debug, Default)]
pub struct JtagDp;
impl JtagDp {
    /// Construct a single-TAP decoder.
    pub fn new(_config: JtagDpConfig) -> Self {
        Self
    }
    /// Typed JTAG input.
    pub fn input(node: NodeId) -> InputRef<JtagEvent> {
        InputRef::of_node(node, 0)
    }
    /// Typed debug transport output.
    pub fn events(node: NodeId) -> OutputRef<JtagDpEvent> {
        OutputRef::of_node(node, 0)
    }
    /// Both data directions are required to distinguish requests and responses.
    pub fn requires() -> Requires {
        Requires::new().equals("tdi", true).equals("tdo", true)
    }
}
impl Node for JtagDp {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "jtag-dp".into(),
            inputs: vec![InputSpec::new::<JtagEvent>("bus", Self::requires())],
            outputs: vec![OutputSpec::new::<JtagDpEvent>(
                "events",
                Props::new().with("singleTap", true).with("irBits", 4),
            )],
        }
    }
    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.input(0), io.output(0)))
    }
}
#[derive(Default)]
struct Captured {
    tdi: u64,
    tdo: u64,
    bits: u32,
    missing: bool,
}
async fn capture(scan: &mut Scan<'_>) -> (Captured, bool) {
    let mut captured = Captured::default();
    loop {
        match scan.bit_or_end().await {
            Ok(Read::End) => return (captured, true),
            Err(_) => return (captured, false),
            Ok(Read::Bit(bit)) => {
                captured.missing |= bit.tdi.is_none() || bit.tdo.is_none();
                if captured.bits < 35 {
                    captured.tdi |= u64::from(bit.tdi.unwrap_or(false)) << captured.bits;
                    captured.tdo |= u64::from(bit.tdo.unwrap_or(false)) << captured.bits;
                }
                captured.bits = captured.bits.saturating_add(1);
            }
        }
    }
}
async fn run(input: Inbox<JtagEvent>, mut out: Outbox<JtagDpEvent>) {
    let mut bus = JtagBus::new(input, out.progress_sink());
    let mut instruction = None;
    let mut previous_scan = None;
    loop {
        match bus.next().await {
            Next::Scan(mut scan) => {
                let id = scan.id();
                let ir = scan.register() == Register::Ir;
                let (captured, complete) = capture(&mut scan).await;
                let at = scan.consumed();
                let payload = match captured {
                    c if !complete => {
                        if ir {
                            instruction = None;
                        }
                        previous_scan = None;
                        JtagDpEvent::Scan {
                            scan: id,
                            ir,
                            bits: c.bits,
                            status: ScanStatus::Aborted,
                        }
                    }
                    c if c.missing => {
                        if ir {
                            instruction = None;
                        }
                        previous_scan = None;
                        JtagDpEvent::Scan {
                            scan: id,
                            ir,
                            bits: c.bits,
                            status: ScanStatus::MissingLine,
                        }
                    }
                    c if ir => {
                        if c.bits == 4 {
                            let value = c.tdi as u8;
                            instruction = Some(value);
                            // Only DPACC/APACC/BYPASS preserve an access result's association.
                            if !matches!(value, 0xa | 0xb | 0xf) {
                                previous_scan = None;
                            }
                            JtagDpEvent::Instruction { scan: id, value }
                        } else {
                            instruction = None;
                            previous_scan = None;
                            JtagDpEvent::Scan {
                                scan: id,
                                ir,
                                bits: c.bits,
                                status: ScanStatus::InvalidLength,
                            }
                        }
                    }
                    c => {
                        let expected = match instruction {
                            Some(0xe) => Some(32),
                            Some(0xf) => Some(1),
                            Some(0x8 | 0xa | 0xb) => Some(35),
                            _ => None,
                        };
                        match expected {
                            None => {
                                previous_scan = None;
                                JtagDpEvent::Scan {
                                    scan: id,
                                    ir,
                                    bits: c.bits,
                                    status: ScanStatus::UnknownInstruction,
                                }
                            }
                            Some(35)
                                if c.bits >= 3
                                    && c.bits != 35
                                    && matches!(instruction, Some(0xa | 0xb))
                                    && c.tdo & 7 == 1 =>
                            {
                                // WAIT can be detected after ACK alone: Update-DR discards
                                // this scan's shifted-in bits, keeping the prior access.
                                JtagDpEvent::Scan {
                                    scan: id,
                                    ir,
                                    bits: c.bits,
                                    status: ScanStatus::Wait,
                                }
                            }
                            Some(bits) if bits != c.bits => {
                                previous_scan = None;
                                JtagDpEvent::Scan {
                                    scan: id,
                                    ir,
                                    bits: c.bits,
                                    status: ScanStatus::InvalidLength,
                                }
                            }
                            Some(_) => match instruction.expect("known instruction") {
                                0xe => JtagDpEvent::Idcode {
                                    scan: id,
                                    value: c.tdo as u32,
                                },
                                0xf => JtagDpEvent::Bypass {
                                    scan: id,
                                    tdi: c.tdi != 0,
                                    tdo: c.tdo != 0,
                                },
                                0x8 => {
                                    previous_scan = None;
                                    JtagDpEvent::Abort {
                                        scan: id,
                                        value: (c.tdi >> 3) as u32,
                                        low_bits: (c.tdi & 7) as u8,
                                    }
                                }
                                value => {
                                    let ack_bits = (c.tdo & 7) as u8;
                                    let ack = super::events::ack_from_bits(ack_bits);
                                    let previous = previous_scan;
                                    let accepted = match ack {
                                        Ack::OkFault => {
                                            previous_scan = Some(id);
                                            Some(true)
                                        }
                                        Ack::Wait => Some(false),
                                        Ack::Invalid => {
                                            previous_scan = None;
                                            None
                                        }
                                    };
                                    JtagDpEvent::Access {
                                        scan: id,
                                        instruction: value,
                                        read: c.tdi & 1 != 0,
                                        addr: ((c.tdi >> 1) & 3) as u8 * 4,
                                        data_in: (c.tdi >> 3) as u32,
                                        ack_bits,
                                        ack,
                                        response_data: (c.tdo >> 3) as u32,
                                        previous_scan: previous,
                                        accepted,
                                    }
                                }
                            },
                        }
                    }
                };
                out.emit(at, payload).await;
            }
            Next::Reset(_) => {
                instruction = Some(0xe);
                previous_scan = None;
                out.emit(bus.consumed(), JtagDpEvent::Reset).await;
            }
            Next::State(_) => {
                let _ = bus.consumed();
            }
            Next::Gap(range) => {
                instruction = None;
                previous_scan = None;
                out.gap(range).await;
            }
            Next::Progress(tick) => out.progress(tick).await,
            Next::End(tick) => {
                out.end(tick).await;
                return;
            }
        }
    }
}
