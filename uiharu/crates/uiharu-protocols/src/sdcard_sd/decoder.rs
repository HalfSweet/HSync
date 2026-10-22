// SPDX-License-Identifier: Apache-2.0

//! Rising CLK samples CMD and DAT simultaneously; CMD is processed first on a tie.
//! Initial/gap-low CMD is partial and drained to eight sampled ones. Normal tokens
//! carry 48 bits, or 136 for the response to CMD2/9/10. CRC7 excludes the R2 header.
//! DAT reads require a valid non-error R1 to CMD17/18 and do not infer write/busy traffic.

use super::{EndReason, SdcardSdEvent as E, crc7_bit, crc16_bit};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, LogicBlock, MAX_CHANNELS, Node, NodeFuture,
    NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires, Sample,
    SignalCursor, Tick, TickRange,
};

/// Native SD wiring. Empty `data` disables read block decoding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SdcardSdConfig {
    /// CLK channel.
    pub clk: u8,
    /// Bidirectional CMD channel.
    pub cmd: u8,
    /// DAT0, or DAT0 through DAT3 in that order; empty for CMD only.
    pub data: Vec<u8>,
    /// Read block length in bytes; fixed for this decoder instance.
    pub block_bytes: u32,
}
impl Default for SdcardSdConfig {
    fn default() -> Self {
        Self {
            clk: 0,
            cmd: 1,
            data: vec![],
            block_bytes: 512,
        }
    }
}
impl SdcardSdConfig {
    /// Validates channels, width and bounded block length.
    pub fn validate(&self) -> Result<(), SdcardSdConfigError> {
        if !matches!(self.data.len(), 0 | 1 | 4) {
            return Err(SdcardSdConfigError::Width);
        }
        if !(1..=4096).contains(&self.block_bytes) {
            return Err(SdcardSdConfigError::BlockLength);
        }
        let mut seen = ChannelMask::EMPTY;
        for c in self.channels() {
            if c >= MAX_CHANNELS {
                return Err(SdcardSdConfigError::Channel(c));
            }
            if seen.has(c) {
                return Err(SdcardSdConfigError::Duplicate(c));
            }
            seen = seen.with(c);
        }
        Ok(())
    }
    fn channels(&self) -> impl Iterator<Item = u8> + '_ {
        [self.clk, self.cmd]
            .into_iter()
            .chain(self.data.iter().copied())
    }
    /// Properties of the native transport.
    pub fn props(&self) -> Props {
        Props::new()
            .with("dataWidth", self.data.len() as i64)
            .with("blockBytes", i64::from(self.block_bytes))
            .with("sampling", "rising")
            .with("readOnly", true)
    }
}
/// Invalid decoder configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SdcardSdConfigError {
    /// Unsupported DAT width.
    #[error("sdcard-sd.config.width")]
    Width,
    /// Unsupported read block length.
    #[error("sdcard-sd.config.block-length")]
    BlockLength,
    /// Out-of-range channel.
    #[error("sdcard-sd.config.channel:{0}")]
    Channel(u8),
    /// Aliased lines.
    #[error("sdcard-sd.config.duplicate:{0}")]
    Duplicate(u8),
}
/// Native SD transport node.
#[derive(Clone, Debug)]
pub struct SdcardSd {
    config: SdcardSdConfig,
}
impl SdcardSd {
    /// Constructs a validated node.
    pub fn new(config: SdcardSdConfig) -> Result<Self, SdcardSdConfigError> {
        config.validate()?;
        Ok(Self { config })
    }
    /// Decoder configuration.
    pub fn config(&self) -> &SdcardSdConfig {
        &self.config
    }
    /// Logic input.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }
    /// Native events output.
    pub fn events(node: NodeId) -> OutputRef<E> {
        OutputRef::of_node(node, 0)
    }
}
impl Node for SdcardSd {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "sdcard-sd".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least(
                    "channels",
                    i64::from(self.config.channels().max().unwrap_or(0)) + 1,
                ),
            )],
            outputs: vec![OutputSpec::new::<E>("events", self.config.props())],
        }
    }
    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.signal(0), io.output(0), self.config))
    }
}
struct Token {
    id: u64,
    partial: bool,
    ones: u8,
    count: u16,
    total: u16,
    host: bool,
    index: u8,
    crc: u8,
    received: u8,
    value: u32,
    used: u8,
    offset: u16,
    start: Tick,
    no_crc: bool,
    argument: u32,
}
impl Token {
    fn new(id: u64, tick: Tick, partial: bool) -> Self {
        Self {
            id,
            partial,
            ones: 0,
            count: 1,
            total: 48,
            host: false,
            index: 0,
            crc: 0,
            received: 0,
            value: 0,
            used: 0,
            offset: 0,
            start: tick,
            no_crc: false,
            argument: 0,
        }
    }
}
struct Data {
    id: u64,
    clocks: u32,
    value: u8,
    used: u8,
    crc: [u16; 4],
    received: [u16; 4],
    start: Tick,
    crc_start: Tick,
}
fn over(start: Tick, end: Tick) -> Provenance {
    Provenance::over(TickRange::new(start, end.next()), end)
}
async fn close(
    out: &mut Outbox<E>,
    token: &mut Option<Token>,
    data: &mut Option<Data>,
    tick: Tick,
) {
    if let Some(t) = token.take() {
        out.emit(
            Provenance::at(tick),
            E::End {
                token: t.id,
                reason: EndReason::Aborted,
            },
        )
        .await;
    }
    if let Some(d) = data.take() {
        out.emit(
            Provenance::at(tick),
            E::DataEnd {
                block: d.id,
                reason: EndReason::Aborted,
            },
        )
        .await;
    }
}
async fn run(mut signal: SignalCursor, mut out: Outbox<E>, c: SdcardSdConfig) {
    let mut id = 0u64;
    let mut token: Option<Token> = None;
    let mut data: Option<Data> = None;
    let mut last_command = None;
    let mut last_app = false;
    let mut app_next = false;
    let mut pending_read = None;
    // After lost samples, high CMD alone does not establish token alignment.
    let mut unknown_alignment = false;
    let mut idle_ones = 0u8;
    let mut read = false;
    let mut multiple = false;
    'resync: loop {
        loop {
            match signal.sample().await {
                Sample::At { tick, levels } => {
                    if !levels.high(c.cmd) {
                        id += 1;
                        token = Some(Token::new(id, tick, true));
                        unknown_alignment = false;
                        out.emit(
                            Provenance::one(tick),
                            E::Begin {
                                token: id,
                                partial: true,
                            },
                        )
                        .await;
                    }
                    break;
                }
                Sample::Gap(r) => out.gap(r).await,
                Sample::Progress(t) => out.progress(t).await,
                Sample::End(t) => {
                    out.end(t).await;
                    return;
                }
            }
        }
        loop {
            match signal.next_change(ChannelMask::one(c.clk)).await {
                Change::At { tick, levels, .. } => {
                    if !levels.high(c.clk) {
                        continue;
                    }
                    let bit = levels.high(c.cmd);
                    if unknown_alignment && bit && token.is_none() {
                        idle_ones += 1;
                        if idle_ones == 8 {
                            unknown_alignment = false;
                        }
                        continue;
                    }
                    if token.is_none() && !bit {
                        id += 1;
                        let partial = unknown_alignment;
                        unknown_alignment = false;
                        idle_ones = 0;
                        token = Some(Token::new(id, tick, partial));
                        out.emit(Provenance::one(tick), E::Begin { token: id, partial })
                            .await;
                    } else if let Some(t) = token.as_mut() {
                        if t.partial {
                            t.ones = if bit { t.ones + 1 } else { 0 };
                            if t.ones == 8 {
                                out.emit(
                                    Provenance::one(tick),
                                    E::End {
                                        token: t.id,
                                        reason: EndReason::Resynchronized,
                                    },
                                )
                                .await;
                                token = None;
                            }
                        } else {
                            let n = t.count;
                            if n == 1 {
                                t.host = bit;
                                if bit {
                                    read = false;
                                    pending_read = None;
                                }
                                t.total = if !bit && matches!(last_command, Some(2 | 9 | 10)) {
                                    136
                                } else {
                                    48
                                };
                                t.no_crc = !bit && last_command == Some(41) && last_app;
                            }
                            if n < 8 {
                                if n >= 2 {
                                    t.index = (t.index << 1) | u8::from(bit);
                                }
                                if n == 7 {
                                    out.emit(
                                        over(t.start, tick),
                                        E::Header {
                                            token: t.id,
                                            host: t.host,
                                            index: t.index,
                                            bits: t.total,
                                        },
                                    )
                                    .await;
                                    t.start = tick.next();
                                }
                            }
                            if (t.total == 48 && n < 40)
                                || (t.total == 136 && (8..128).contains(&n))
                            {
                                t.crc = crc7_bit(t.crc, bit);
                            }
                            if (8..t.total - 8).contains(&n) {
                                if t.used == 0 {
                                    t.start = tick;
                                }
                                t.value = (t.value << 1) | u32::from(bit);
                                t.used += 1;
                                if t.used == 32 || n == t.total - 9 {
                                    out.emit(
                                        over(t.start, tick),
                                        E::Payload {
                                            token: t.id,
                                            offset: t.offset,
                                            value: t.value,
                                            bits: t.used,
                                        },
                                    )
                                    .await;
                                    if t.total == 48 {
                                        t.argument = t.value;
                                    }
                                    t.offset += u16::from(t.used);
                                    t.used = 0;
                                    t.value = 0;
                                    t.start = tick.next();
                                }
                            } else if (t.total - 8..t.total - 1).contains(&n) {
                                if n == t.total - 8 {
                                    t.start = tick;
                                }
                                t.received = (t.received << 1) | u8::from(bit);
                                if n == t.total - 2 {
                                    let expected = (!t.no_crc).then_some(t.crc);
                                    out.emit(
                                        over(t.start, tick),
                                        E::Crc {
                                            token: t.id,
                                            value: t.received,
                                            expected,
                                            ok: expected.map(|v| v == t.received),
                                        },
                                    )
                                    .await;
                                    t.start = tick.next();
                                }
                            }
                            t.count += 1;
                            if t.count == t.total {
                                let valid = bit && t.crc == t.received;
                                if t.host {
                                    last_command = valid.then_some(t.index);
                                    last_app = app_next;
                                    app_next = false;
                                    read = false;
                                    pending_read = if valid && matches!(t.index, 17 | 18) {
                                        Some(t.index == 18)
                                    } else {
                                        None
                                    };
                                } else {
                                    if valid
                                        && last_command == Some(t.index)
                                        && t.argument & 0xfff9_8008 == 0
                                    {
                                        if let Some(is_multiple) = pending_read.take() {
                                            read = true;
                                            multiple = is_multiple;
                                        }
                                        app_next = t.index == 55 && t.argument & 0x20 != 0;
                                    } else {
                                        pending_read = None;
                                        read = false;
                                        app_next = false;
                                    }
                                    last_command = None;
                                }
                                out.emit(
                                    Provenance::one(tick),
                                    E::End {
                                        token: t.id,
                                        reason: if bit {
                                            EndReason::Complete
                                        } else {
                                            EndReason::InvalidEnd
                                        },
                                    },
                                )
                                .await;
                                token = None;
                            }
                        }
                    }
                    if !c.data.is_empty() {
                        let width = c.data.len();
                        let symbol = c
                            .data
                            .iter()
                            .enumerate()
                            .fold(0u8, |v, (i, &line)| v | (u8::from(levels.high(line)) << i));
                        if data.is_none() && read && symbol == 0 {
                            id += 1;
                            data = Some(Data {
                                id,
                                clocks: 0,
                                value: 0,
                                used: 0,
                                crc: [0; 4],
                                received: [0; 4],
                                start: tick.next(),
                                crc_start: tick,
                            });
                            out.emit(
                                Provenance::one(tick),
                                E::DataBegin {
                                    block: id,
                                    partial: false,
                                },
                            )
                            .await;
                        } else if let Some(d) = data.as_mut() {
                            let payload_clocks = c.block_bytes * 8 / width as u32;
                            if d.clocks < payload_clocks {
                                if d.used == 0 {
                                    d.start = tick;
                                }
                                for (lane, crc) in d.crc.iter_mut().enumerate().take(width) {
                                    *crc = crc16_bit(*crc, symbol & (1 << lane) != 0);
                                }
                                d.value = (d.value << width) | symbol;
                                d.used += width as u8;
                                if d.used == 8 {
                                    out.emit(
                                        over(d.start, tick),
                                        E::Data {
                                            block: d.id,
                                            offset: d.clocks * width as u32 / 8,
                                            value: d.value,
                                        },
                                    )
                                    .await;
                                    d.value = 0;
                                    d.used = 0;
                                    d.start = tick.next();
                                }
                            } else if d.clocks < payload_clocks + 16 {
                                if d.clocks == payload_clocks {
                                    d.crc_start = tick;
                                    d.start = tick;
                                }
                                for lane in 0..width {
                                    d.received[lane] = (d.received[lane] << 1)
                                        | u16::from(symbol & (1 << lane) != 0);
                                }
                                if d.clocks == payload_clocks + 15 {
                                    for lane in 0..width {
                                        out.emit(
                                            over(d.crc_start, tick),
                                            E::DataCrc {
                                                block: d.id,
                                                lane: lane as u8,
                                                value: d.received[lane],
                                                expected: d.crc[lane],
                                                ok: d.received[lane] == d.crc[lane],
                                            },
                                        )
                                        .await;
                                    }
                                    d.start = tick.next();
                                }
                            } else {
                                out.emit(
                                    Provenance::one(tick),
                                    E::DataEnd {
                                        block: d.id,
                                        reason: if symbol == (1 << width) - 1 {
                                            EndReason::Complete
                                        } else {
                                            EndReason::InvalidEnd
                                        },
                                    },
                                )
                                .await;
                                data = None;
                                if !multiple {
                                    read = false;
                                }
                                continue;
                            }
                            d.clocks += 1;
                        }
                    }
                }
                Change::Progress(tick) => {
                    let hold = token
                        .as_ref()
                        .map(|t| t.start)
                        .into_iter()
                        .chain(data.as_ref().map(|d| d.start))
                        .min()
                        .map_or(tick, |start| tick.min(start));
                    out.progress(hold).await;
                }
                Change::Gap(r) => {
                    close(&mut out, &mut token, &mut data, r.start).await;
                    last_command = None;
                    pending_read = None;
                    app_next = false;
                    last_app = false;
                    unknown_alignment = true;
                    idle_ones = 0;
                    read = false;
                    out.gap(r).await;
                    continue 'resync;
                }
                Change::End(t) => {
                    close(&mut out, &mut token, &mut data, t).await;
                    out.end(t).await;
                    return;
                }
                Change::Quiet(_) => {}
            }
        }
    }
}
