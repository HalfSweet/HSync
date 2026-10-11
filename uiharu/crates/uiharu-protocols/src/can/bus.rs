// SPDX-License-Identifier: Apache-2.0

//! Reading a CAN event stream frame by frame, for decoders built on CAN.
//!
//! [`CanBus::next`] waits for a frame to begin and hands out a [`Frame`], a cursor over its
//! events. The cursor offers the contract's primitives and nothing more: the identifier, the
//! control field, the next data byte, and the frame's end with its CRC and acknowledgement.
//! Every read moves forward through the frame and can wait across blocks; what it passes over
//! is consumed, and a read that finds its field gone — the data field used up or absent, or
//! the frame ended before the field — returns [`ReadError::Missing`] and leaves what it found
//! for the next read. Only [`Frame::end`] returns the frame's end; it drains whatever was not
//! read.
//!
//! What a protocol makes of a frame — which identifiers it reads, byte order, how many bytes
//! make a value — is that protocol's own code, so that the contract stays as general as the
//! bus.

use uiharu_core::{Event, Inbox, Item, Provenance, Tick, TickRange};

use super::events::{CanEvent, EndReason, FrameId};

/// A frame's identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Identifier {
    /// 11 bits in the base format, 29 in the extended format.
    pub id: u32,
    /// Whether the frame is in the extended format.
    pub extended: bool,
}

/// A frame's control field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Control {
    /// A remote frame: it has no data field.
    pub remote: bool,
    /// An FD frame.
    pub fd: bool,
    /// The data phase ran at the data bit rate.
    pub brs: bool,
    /// The transmitter was error passive.
    pub esi: bool,
    /// The data length code.
    pub dlc: u8,
    /// The number of data bytes the code stands for.
    pub length: u8,
}

/// A frame's CRC field as it was received.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CrcField {
    /// The CRC sequence.
    pub value: u32,
    /// Whether it matched the frame.
    pub ok: bool,
    /// The stuff count of an FD frame.
    pub stuff_count: Option<u8>,
}

/// How a frame ended, with what its trailer said.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ending {
    /// Why the frame ended.
    pub reason: EndReason,
    /// Its CRC field, if the frame got that far.
    pub crc: Option<CrcField>,
    /// Whether a receiver acknowledged it, if the frame got that far.
    pub acked: Option<bool>,
}

impl Ending {
    /// Whether receivers accept the frame: it ended with its end of frame and its CRC matched.
    pub fn is_valid(&self) -> bool {
        self.reason == EndReason::Eof && self.crc.is_some_and(|crc| crc.ok)
    }
}

/// Why a read did not return what it asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The frame has no such field left: the data field is used up or absent, or the frame
    /// ended before the field. [`Frame::end`] tells how it ended.
    #[error("the frame has no such field left")]
    Missing,
    /// Samples are missing from here on; the gap itself follows from [`CanBus::next`].
    #[error("samples are missing from tick {}", .0.start)]
    Gap(TickRange),
    /// The stream ended; the end itself follows from [`CanBus::next`].
    #[error("the stream ended at tick {0}")]
    EndOfInput(Tick),
}

/// What [`CanBus::next`] finds.
#[derive(Debug)]
pub enum Next<'a> {
    /// A frame begins.
    Frame(Frame<'a>),
    /// Samples, and whatever was on the bus, are missing.
    Gap(TickRange),
    /// Nothing more comes before this tick.
    Progress(Tick),
    /// The stream ended.
    End(Tick),
}

#[derive(Debug)]
struct Current {
    frame: FrameId,
    partial: bool,
    /// The bytes of the data event at the head of the stream read so far.
    taken: usize,
    /// Whether the frame's end was consumed, or the stream failed under it.
    ended: bool,
    crc: Option<CrcField>,
    acked: Option<bool>,
}

/// The next item, as the frame being read sees it.
#[derive(Clone, Copy, Debug)]
enum Head {
    Id(Identifier),
    Control(Control),
    /// A data event and the byte the reader is at in it.
    Data {
        byte: Option<u8>,
        last: bool,
    },
    Crc(CrcField),
    Ack(bool),
    End(EndReason),
    /// An event of another frame: this one never ended.
    Foreign,
    Gap(TickRange),
    Progress,
    EndOfInput(Tick),
}

impl Head {
    /// The position of the event in a frame; fields come in this order.
    fn rank(&self) -> u8 {
        match self {
            Head::Id(_) => 1,
            Head::Control(_) => 2,
            Head::Data { .. } => 3,
            Head::Crc(_) => 4,
            Head::Ack(_) => 5,
            _ => 6,
        }
    }
}

const RANK_ID: u8 = 1;
const RANK_CONTROL: u8 = 2;
const RANK_DATA: u8 = 3;

/// A decoder's reader on a CAN event stream.
#[derive(Debug)]
pub struct CanBus {
    inbox: Inbox<CanEvent>,
    current: Option<Current>,
    /// The ticks of the items consumed since [`Frame::consumed`] last took them.
    consumed: Option<Provenance>,
    /// Where the last consumed item was settled.
    last_settled: Tick,
}

impl CanBus {
    /// A reader on `inbox`.
    pub fn new(inbox: Inbox<CanEvent>) -> Self {
        Self {
            inbox,
            current: None,
            consumed: None,
            last_settled: Tick::ZERO,
        }
    }

    fn record(&mut self, at: Provenance) {
        self.consumed = Some(self.consumed.map_or(at, |have| have.join(at)));
        self.last_settled = at.settled;
    }

    /// The ticks of what was consumed since this was last called; an empty span where the
    /// last item was settled when nothing was.
    pub fn consumed(&mut self) -> Provenance {
        self.consumed
            .take()
            .unwrap_or(Provenance::at(self.last_settled))
    }

    /// Waits for the next frame, gap, progress or end. Events outside a frame are skipped:
    /// their frame began before an earlier one was drained, which [`CanBus::finish`] prevents.
    pub async fn next(&mut self) -> Next<'_> {
        loop {
            match self.inbox.recv().await {
                Item::Event(event) => {
                    self.record(Provenance::over(event.span, event.settled));
                    if let CanEvent::Begin { frame, partial } = event.payload {
                        self.current = Some(Current {
                            frame,
                            partial,
                            taken: 0,
                            ended: false,
                            crc: None,
                            acked: None,
                        });
                        return Next::Frame(Frame { bus: self });
                    }
                }
                Item::Gap(range) => return Next::Gap(range),
                Item::Progress(tick) => return Next::Progress(tick),
                Item::End(tick) => return Next::End(tick),
            }
        }
    }

    /// Consumes what is left of the current frame, if the decoder left it open.
    pub async fn finish(&mut self) -> Result<(), ReadError> {
        let result = match &self.current {
            Some(current) if !current.ended => Frame { bus: self }.end().await.map(|_| ()),
            _ => Ok(()),
        };
        self.current = None;
        result
    }

    /// Looks at the next item without taking it.
    async fn head(&mut self) -> (Head, Option<Provenance>) {
        let (frame, taken) = self
            .current
            .as_ref()
            .map_or((FrameId(0), 0), |current| (current.frame, current.taken));
        self.inbox
            .peek_with(move |item| match item {
                Item::Event(event) => (head_of(event, frame, taken), Some(provenance(event))),
                Item::Gap(range) => (Head::Gap(*range), None),
                Item::Progress(_) => (Head::Progress, None),
                Item::End(tick) => (Head::EndOfInput(*tick), None),
            })
            .await
    }

    /// Takes the item last looked at, recording its ticks.
    fn take(&mut self, at: Option<Provenance>) {
        self.inbox.take();
        if let Some(at) = at {
            self.record(at);
        }
    }

    fn end_current(&mut self) {
        if let Some(current) = self.current.as_mut() {
            current.ended = true;
        }
    }

    /// Moves through the frame up to the first event ranked `rank` or later; returns it, not
    /// taken, when it is ranked `rank`. What ends the frame is left for [`Frame::end`].
    async fn seek(&mut self, rank: u8) -> Result<(Head, Option<Provenance>), ReadError> {
        if self.current.as_ref().is_none_or(|current| current.ended) {
            return Err(ReadError::Missing);
        }
        loop {
            let (head, at) = self.head().await;
            match head {
                // Progress inside a frame says nothing about its fields.
                Head::Progress => self.take(None),
                // The gap and the end stay in the stream, for `end` and `next` to report.
                Head::Gap(range) => return Err(ReadError::Gap(range)),
                Head::EndOfInput(tick) => return Err(ReadError::EndOfInput(tick)),
                // Another frame's event: this one never ended.
                Head::Foreign => return Err(ReadError::Missing),
                _ if head.rank() < rank => self.pass(head, at),
                _ if head.rank() == rank => return Ok((head, at)),
                _ => return Err(ReadError::Missing),
            }
        }
    }

    /// Consumes an event the reader moves past, keeping what the frame's end reports.
    fn pass(&mut self, head: Head, at: Option<Provenance>) {
        if let Some(current) = self.current.as_mut() {
            current.taken = 0;
            match head {
                Head::Crc(crc) => current.crc = Some(crc),
                Head::Ack(acked) => current.acked = Some(acked),
                _ => {}
            }
        }
        self.take(at);
    }
}

fn provenance(event: &Event<CanEvent>) -> Provenance {
    Provenance::over(event.span, event.settled)
}

/// What `event` is to a reader of `frame` that has read `taken` bytes of it.
fn head_of(event: &Event<CanEvent>, frame: FrameId, taken: usize) -> Head {
    if event.payload.frame() != frame {
        return Head::Foreign;
    }
    match &event.payload {
        CanEvent::Begin { .. } => Head::Foreign,
        CanEvent::Id { id, extended, .. } => Head::Id(Identifier {
            id: *id,
            extended: *extended,
        }),
        CanEvent::Control {
            remote,
            fd,
            brs,
            esi,
            dlc,
            length,
            ..
        } => Head::Control(Control {
            remote: *remote,
            fd: *fd,
            brs: *brs,
            esi: *esi,
            dlc: *dlc,
            length: *length,
        }),
        CanEvent::Data { bytes, .. } => Head::Data {
            byte: bytes.get(taken).copied(),
            last: taken + 1 >= bytes.len(),
        },
        CanEvent::Crc {
            value,
            ok,
            stuff_count,
            ..
        } => Head::Crc(CrcField {
            value: *value,
            ok: *ok,
            stuff_count: *stuff_count,
        }),
        CanEvent::Ack { acked, .. } => Head::Ack(*acked),
        CanEvent::End { reason, .. } => Head::End(*reason),
    }
}

/// A cursor over the events of one frame.
#[derive(Debug)]
pub struct Frame<'a> {
    bus: &'a mut CanBus,
}

impl Frame<'_> {
    /// The frame's id.
    pub fn id(&self) -> FrameId {
        self.bus
            .current
            .as_ref()
            .map_or(FrameId(0), |current| current.frame)
    }

    /// Whether the frame was already under way when the samples began or resumed after a
    /// gap: none of it was decoded.
    pub fn is_partial(&self) -> bool {
        self.bus
            .current
            .as_ref()
            .is_some_and(|current| current.partial)
    }

    /// Whether the frame's end has been read, or [`Frame::end`] found the stream cut where the
    /// end was due.
    pub fn is_ended(&self) -> bool {
        self.bus
            .current
            .as_ref()
            .is_none_or(|current| current.ended)
    }

    /// The ticks of what was consumed since this was last called; see [`CanBus::consumed`].
    pub fn consumed(&mut self) -> Provenance {
        self.bus.consumed()
    }

    /// Reads the identifier.
    pub async fn identifier(&mut self) -> Result<Identifier, ReadError> {
        match self.bus.seek(RANK_ID).await? {
            (Head::Id(identifier), at) => {
                self.bus.take(at);
                Ok(identifier)
            }
            _ => Err(ReadError::Missing),
        }
    }

    /// Reads the control field.
    pub async fn control(&mut self) -> Result<Control, ReadError> {
        match self.bus.seek(RANK_CONTROL).await? {
            (Head::Control(control), at) => {
                self.bus.take(at);
                Ok(control)
            }
            _ => Err(ReadError::Missing),
        }
    }

    /// Reads the next data byte. A sequence of reads that pauses between them keeps what it
    /// has read; nothing is read twice.
    pub async fn byte(&mut self) -> Result<u8, ReadError> {
        loop {
            let (head, at) = self.bus.seek(RANK_DATA).await?;
            let Head::Data { byte, last } = head else {
                return Err(ReadError::Missing);
            };
            let Some(current) = self.bus.current.as_mut() else {
                return Err(ReadError::Missing);
            };
            let first = current.taken == 0;
            current.taken += 1;
            if last {
                current.taken = 0;
                self.bus.inbox.take();
            } else {
                // A byte from inside a batch is a read like any other.
                self.bus.inbox.cx().spend();
            }
            // A batch's ticks are recorded with its first byte.
            if first && let Some(at) = at {
                self.bus.record(at);
            }
            if let Some(byte) = byte {
                return Ok(byte);
            }
        }
    }

    /// Reads to the frame's end, past whatever was not read, and tells how the frame ended.
    pub async fn end(&mut self) -> Result<Ending, ReadError> {
        if self.is_ended() {
            return Err(ReadError::Missing);
        }
        loop {
            let (head, at) = self.bus.head().await;
            let reason = match head {
                Head::End(reason) => {
                    self.bus.take(at);
                    reason
                }
                // Another frame's event: this one never ended, its decoder lost it. The event
                // stays for `next`.
                Head::Foreign => EndReason::Aborted,
                Head::Progress => {
                    self.bus.take(None);
                    continue;
                }
                Head::Gap(range) => {
                    self.bus.end_current();
                    return Err(ReadError::Gap(range));
                }
                Head::EndOfInput(tick) => {
                    self.bus.end_current();
                    return Err(ReadError::EndOfInput(tick));
                }
                // A batch partly read was recorded with its first byte; recording it again
                // changes nothing.
                _ => {
                    self.bus.pass(head, at);
                    continue;
                }
            };
            self.bus.end_current();
            let (crc, acked) = self
                .bus
                .current
                .as_ref()
                .map_or((None, None), |current| (current.crc, current.acked));
            return Ok(Ending { reason, crc, acked });
        }
    }
}
