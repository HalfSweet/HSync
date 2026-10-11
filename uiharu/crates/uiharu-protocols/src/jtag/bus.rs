// SPDX-License-Identifier: Apache-2.0

//! Reading a JTAG event stream scan by scan, for decoders built on JTAG.
//!
//! [`JtagBus::next`] waits for the next scan, state, reset, gap, progress or end and hands out
//! a [`Scan`], a cursor over the bits of one scan: each read consumes one bit, with both lines'
//! values, and can wait across batches and blocks. A scan's normal end is only ever returned by
//! [`Scan::bit_or_end`]; everywhere else, running out of bits is an error that tells the
//! decoder how the scan ended, so that it can close what it had opened.
//!
//! The cursor offers the contract's primitives and nothing more: the next bit, the normal end,
//! draining. What a protocol makes of the bits — register lengths, the bypass bits of other
//! devices in the chain, field widths and their order — is that protocol's own code.
//!
//! Provenance comes at the contract's grain: a bit read from a batch hands over the ticks of
//! the whole batch, the `shift` event it came in.

use uiharu_core::{Event, Inbox, Item, Provenance, Tick, TickRange};

use super::events::{EndReason, JtagEvent, Register, ResetBy, SHIFT_BATCH_BITS, ScanId, TapState};

/// A data line of the port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Line {
    /// Test data in.
    Tdi,
    /// Test data out.
    Tdo,
}

/// Why a read did not return a bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The scan reached its update where a bit was expected: it is whole, the field read from
    /// it is not.
    #[error("the scan ended before the bit")]
    Incomplete,
    /// The scan was aborted: TRST reset the controller, or the samples ended or were lost.
    #[error("the scan was aborted")]
    Aborted,
    /// Samples are missing from here on; the gap itself follows from [`JtagBus::next`].
    #[error("samples are missing from tick {}", .0.start)]
    Gap(TickRange),
    /// The stream ended; the end itself follows from [`JtagBus::next`].
    #[error("the stream ended at tick {0}")]
    EndOfInput(Tick),
    /// The bit has no value on this line: it is not connected.
    #[error("{0:?} is not connected")]
    MissingLine(Line),
}

/// One bit as both lines carried it; a line that is not connected reads `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Bit {
    /// The bit shifted in on TDI.
    pub tdi: Option<bool>,
    /// The bit shifted out on TDO.
    pub tdo: Option<bool>,
}

impl Bit {
    /// The bit on TDI; an error when the line is not connected.
    pub fn tdi(&self) -> Result<bool, ReadError> {
        self.tdi.ok_or(ReadError::MissingLine(Line::Tdi))
    }

    /// The bit on TDO; an error when the line is not connected.
    pub fn tdo(&self) -> Result<bool, ReadError> {
        self.tdo.ok_or(ReadError::MissingLine(Line::Tdo))
    }
}

/// What [`Scan::bit_or_end`] finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// The next bit.
    Bit(Bit),
    /// The controller entered Update-IR or Update-DR: the scan ended normally.
    End,
}

/// What [`JtagBus::next`] finds.
#[derive(Debug)]
pub enum Next<'a> {
    /// A scan begins.
    Scan(Scan<'a>),
    /// The controller moved to another state outside a scan.
    State(TapState),
    /// The controller was reset.
    Reset(ResetBy),
    /// Samples, and whatever was on the port, are missing.
    Gap(TickRange),
    /// Nothing more comes before this tick.
    Progress(Tick),
    /// The stream ended.
    End(Tick),
}

#[derive(Debug)]
struct Current {
    id: ScanId,
    /// Whether the scan's end was reached (or the stream failed under it).
    ended: bool,
}

/// The bits of the `shift` event being read: at most [`SHIFT_BATCH_BITS`] a line.
#[derive(Debug, Default)]
struct Batch {
    tdi: Option<Vec<u8>>,
    tdo: Option<Vec<u8>>,
    bits: u16,
    next: u16,
}

impl Batch {
    fn take(&mut self) -> Option<Bit> {
        if self.next >= self.bits {
            return None;
        }
        let (byte, shift) = (usize::from(self.next / 8), self.next % 8);
        let level = |bytes: &Option<Vec<u8>>| {
            bytes
                .as_ref()
                .map(|bytes| bytes.get(byte).is_some_and(|byte| byte >> shift & 1 != 0))
        };
        self.next += 1;
        Some(Bit {
            tdi: level(&self.tdi),
            tdo: level(&self.tdo),
        })
    }
}

/// A decoder's reader on a JTAG event stream.
#[derive(Debug)]
pub struct JtagBus {
    inbox: Inbox<JtagEvent>,
    current: Option<Current>,
    batch: Batch,
    /// The ticks of the items consumed since [`Scan::consumed`] last took them.
    consumed: Option<Provenance>,
    /// Where the last consumed item was settled.
    last_settled: Tick,
}

impl JtagBus {
    /// A reader on `inbox`.
    pub fn new(inbox: Inbox<JtagEvent>) -> Self {
        Self {
            inbox,
            current: None,
            batch: Batch::default(),
            consumed: None,
            last_settled: Tick::ZERO,
        }
    }

    fn record(&mut self, event: &Event<JtagEvent>) {
        let at = Provenance::over(event.span, event.settled);
        self.consumed = Some(self.consumed.map_or(at, |have| have.join(at)));
        self.last_settled = event.settled;
    }

    /// The ticks of what was consumed since this was last called; an empty span where the
    /// last item was settled when nothing was.
    pub fn consumed(&mut self) -> Provenance {
        self.consumed
            .take()
            .unwrap_or(Provenance::at(self.last_settled))
    }

    /// Waits for the next scan, state, reset, gap, progress or end. Bits and ends outside a
    /// scan are skipped: their beginning was consumed by an earlier scan that was not drained,
    /// which [`JtagBus::finish`] prevents.
    pub async fn next(&mut self) -> Next<'_> {
        loop {
            match self.inbox.recv().await {
                Item::Event(event) => {
                    self.record(&event);
                    match event.payload {
                        JtagEvent::ScanBegin { scan, register } => {
                            self.current = Some(Current {
                                id: scan,
                                ended: false,
                            });
                            self.batch = Batch::default();
                            return Next::Scan(Scan {
                                bus: self,
                                id: scan,
                                register,
                            });
                        }
                        JtagEvent::State { state } => return Next::State(state),
                        JtagEvent::Reset { by } => return Next::Reset(by),
                        JtagEvent::Shift { .. } | JtagEvent::ScanEnd { .. } => {}
                    }
                }
                Item::Gap(range) => return Next::Gap(range),
                Item::Progress(tick) => return Next::Progress(tick),
                Item::End(tick) => return Next::End(tick),
            }
        }
    }

    /// Consumes what is left of the current scan, if the decoder left it open. Returns how
    /// the scan ended when it did not end normally.
    pub async fn finish(&mut self) -> Result<(), ReadError> {
        let result = match &self.current {
            Some(current) if !current.ended => self.drain().await,
            _ => Ok(()),
        };
        self.current = None;
        result
    }

    async fn drain(&mut self) -> Result<(), ReadError> {
        loop {
            if self.next_bit().await?.is_none() {
                return Ok(());
            }
        }
    }

    /// Takes the next bit of the current scan, or reports how it ended instead.
    async fn next_bit(&mut self) -> Result<Option<Bit>, ReadError> {
        let Some(current) = self.current.as_ref() else {
            return Err(ReadError::Incomplete);
        };
        if current.ended {
            return Err(ReadError::Incomplete);
        }
        let id = current.id;
        loop {
            if let Some(bit) = self.batch.take() {
                return Ok(Some(bit));
            }
            let peeked = self
                .inbox
                .peek_with(|item| match item {
                    Item::Event(event) => match &event.payload {
                        JtagEvent::Shift { scan, .. } if *scan == id => {
                            Peeked::Shift(event.clone())
                        }
                        // The pauses of a scan say nothing about its bits.
                        JtagEvent::State { .. } => Peeked::State(event.clone()),
                        JtagEvent::ScanEnd { scan, reason, .. } if *scan == id => {
                            Peeked::End(event.clone(), *reason)
                        }
                        _ => Peeked::Foreign,
                    },
                    Item::Gap(range) => Peeked::Gap(*range),
                    Item::Progress(_) => Peeked::Progress,
                    Item::End(tick) => Peeked::EndOfInput(*tick),
                })
                .await;
            let result = match peeked {
                Peeked::Shift(event) => {
                    self.consume(&event);
                    if let JtagEvent::Shift { tdi, tdo, bits, .. } = event.payload {
                        self.batch = Batch {
                            tdi,
                            tdo,
                            bits: bits.min(SHIFT_BATCH_BITS),
                            next: 0,
                        };
                    }
                    continue;
                }
                Peeked::State(event) => {
                    self.consume(&event);
                    continue;
                }
                Peeked::Progress => {
                    self.inbox.take();
                    continue;
                }
                Peeked::End(event, EndReason::Complete) => {
                    self.consume(&event);
                    Ok(None)
                }
                Peeked::End(event, EndReason::Aborted) => {
                    self.consume(&event);
                    Err(ReadError::Aborted)
                }
                // Another scan's event, or a reset: this one never ended. It stays for `next`.
                Peeked::Foreign => Err(ReadError::Aborted),
                // The gap and the end stay in the stream too, for `next` to report.
                Peeked::Gap(range) => Err(ReadError::Gap(range)),
                Peeked::EndOfInput(tick) => Err(ReadError::EndOfInput(tick)),
            };
            if let Some(current) = self.current.as_mut() {
                current.ended = true;
            }
            return result;
        }
    }

    /// Moves past `event`, charging the budget and recording its ticks.
    fn consume(&mut self, event: &Event<JtagEvent>) {
        self.inbox.take();
        self.record(event);
    }
}

/// What the next item of the stream means to the scan being read.
enum Peeked {
    Shift(Event<JtagEvent>),
    State(Event<JtagEvent>),
    End(Event<JtagEvent>, EndReason),
    Foreign,
    Gap(TickRange),
    Progress,
    EndOfInput(Tick),
}

/// A cursor over the bits of one scan.
#[derive(Debug)]
pub struct Scan<'a> {
    bus: &'a mut JtagBus,
    id: ScanId,
    register: Register,
}

impl Scan<'_> {
    /// The scan's id.
    pub fn id(&self) -> ScanId {
        self.id
    }

    /// The register the scan shifts through.
    pub fn register(&self) -> Register {
        self.register
    }

    /// Whether the scan has ended.
    pub fn is_ended(&self) -> bool {
        self.bus
            .current
            .as_ref()
            .is_none_or(|current| current.ended)
    }

    /// The ticks of what was consumed since this was last called; see [`JtagBus::consumed`].
    pub fn consumed(&mut self) -> Provenance {
        self.bus.consumed()
    }

    /// Reads the next bit; the scan's end, normal or not, is an error here. A sequence of
    /// reads that pauses between them keeps what it has read; nothing is read twice.
    pub async fn bit(&mut self) -> Result<Bit, ReadError> {
        match self.bit_or_end().await? {
            Read::Bit(bit) => Ok(bit),
            Read::End => Err(ReadError::Incomplete),
        }
    }

    /// Reads the next bit, or finds the scan's normal end.
    pub async fn bit_or_end(&mut self) -> Result<Read, ReadError> {
        Ok(self.bus.next_bit().await?.map_or(Read::End, Read::Bit))
    }

    /// Consumes the rest of the scan up to its normal end.
    pub async fn drain(&mut self) -> Result<(), ReadError> {
        self.bus.drain().await
    }
}
