// SPDX-License-Identifier: Apache-2.0

//! Events as NDJSON, one item per line:
//!
//! ```json
//! {"stream":"spi","seq":0,"span":[20,21],"settled":20,"event":{"kind":"begin","tx":1,"partial":false}}
//! {"stream":"nor","gap":[100,200]}
//! {"stream":"nor","end":5008}
//! ```
//!
//! Progress lines (`{"stream":"spi","progress":123}`) are written only when asked, as they
//! depend on how the samples were cut into blocks.

use std::io::{self, Write};

use serde::Serialize;
use uiharu_core::{Exposed, Item, Tick, TickRange};

#[derive(Serialize)]
struct EventLine<'a, T> {
    stream: &'a str,
    seq: u64,
    span: TickRange,
    settled: Tick,
    event: &'a T,
}

#[derive(Serialize)]
struct GapLine<'a> {
    stream: &'a str,
    gap: TickRange,
}

#[derive(Serialize)]
struct ProgressLine<'a> {
    stream: &'a str,
    progress: Tick,
}

#[derive(Serialize)]
struct EndLine<'a> {
    stream: &'a str,
    end: Tick,
}

/// Writes one item.
pub fn write_item<T: Serialize>(
    out: &mut impl Write,
    stream: &str,
    item: &Item<T>,
    progress: bool,
) -> io::Result<()> {
    match item {
        Item::Event(event) => serde_json::to_writer(
            &mut *out,
            &EventLine {
                stream,
                seq: event.seq,
                span: event.span,
                settled: event.settled,
                event: &event.payload,
            },
        )?,
        Item::Gap(range) => serde_json::to_writer(
            &mut *out,
            &GapLine {
                stream,
                gap: *range,
            },
        )?,
        Item::Progress(tick) => {
            if !progress {
                return Ok(());
            }
            serde_json::to_writer(
                &mut *out,
                &ProgressLine {
                    stream,
                    progress: *tick,
                },
            )?;
        }
        Item::End(tick) => serde_json::to_writer(&mut *out, &EndLine { stream, end: *tick })?,
    }
    out.write_all(b"\n")
}

/// A writer for one exposed stream that remembers whether its end was written.
#[derive(Debug)]
pub struct StreamWriter<T> {
    name: String,
    exposed: Exposed<T>,
    ended: bool,
    /// Events written.
    pub events: u64,
}

impl<T: Serialize> StreamWriter<T> {
    /// A writer for `exposed`, named `name` in the output.
    pub fn new(name: impl Into<String>, exposed: Exposed<T>) -> Self {
        Self {
            name: name.into(),
            exposed,
            ended: false,
            events: 0,
        }
    }

    /// Whether the stream's end was written.
    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// Writes every item waiting, up to and including the end of the stream.
    pub fn drain(&mut self, out: &mut impl Write, progress: bool) -> io::Result<()> {
        while !self.ended {
            let Some((end, written)) = self.exposed.with_next(|item| {
                let is_event = matches!(item, Item::Event(_));
                (
                    item.is_end(),
                    write_item(out, &self.name, item, progress).map(|()| is_event),
                )
            }) else {
                break;
            };
            if written? {
                self.events += 1;
            }
            self.exposed.advance();
            self.ended = end;
        }
        Ok(())
    }

    /// Reads every item waiting without writing it, counting the events.
    pub fn discard(&mut self) {
        while !self.ended {
            let Some((end, is_event)) = self
                .exposed
                .with_next(|item| (item.is_end(), item.event().is_some()))
            else {
                break;
            };
            if is_event {
                self.events += 1;
            }
            self.exposed.advance();
            self.ended = end;
        }
    }
}
