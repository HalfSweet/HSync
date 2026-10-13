// SPDX-License-Identifier: Apache-2.0

//! The NDJSON the command line writes: one line per item of an exposed output, read through
//! the registry's type-erased reader.

use std::io::{self, Write};

use serde::Serialize;
use uiharu_core::{Tick, TickRange};
use uiharu_registry::{DynItem, DynReader};

#[derive(Serialize)]
struct EventLine<'a> {
    stream: &'a str,
    seq: u64,
    span: TickRange,
    settled: Tick,
    event: &'a serde_json::Value,
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

/// Writes `item` of stream `stream` as one line; progress only when `progress` asks for it.
pub fn write_item(
    out: &mut dyn Write,
    stream: &str,
    item: &DynItem,
    progress: bool,
) -> io::Result<()> {
    match item {
        DynItem::Event {
            seq,
            span,
            settled,
            payload,
        } => serde_json::to_writer(
            &mut *out,
            &EventLine {
                stream,
                seq: *seq,
                span: *span,
                settled: *settled,
                event: payload,
            },
        )?,
        DynItem::Gap(range) => serde_json::to_writer(
            &mut *out,
            &GapLine {
                stream,
                gap: *range,
            },
        )?,
        DynItem::Progress(tick) => {
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
        DynItem::End(tick) => serde_json::to_writer(&mut *out, &EndLine { stream, end: *tick })?,
    }
    out.write_all(b"\n")
}

/// A stream being written, or discarded.
pub struct Writer {
    reader: Box<dyn DynReader>,
}

impl Writer {
    /// A writer over `reader`.
    pub fn new(reader: Box<dyn DynReader>) -> Self {
        Self { reader }
    }

    /// The stream's name.
    pub fn name(&self) -> &str {
        self.reader.name()
    }

    /// Whether the end of the stream was read.
    pub fn is_ended(&self) -> bool {
        self.reader.is_ended()
    }

    /// Events read so far.
    pub fn event_count(&self) -> u64 {
        self.reader.events()
    }

    /// Writes everything the stream holds.
    pub fn drain(&mut self, out: &mut dyn Write, progress: bool) -> io::Result<()> {
        while let Some(item) = self.reader.next() {
            write_item(out, self.reader.name(), &item, progress)?;
        }
        Ok(())
    }

    /// Reads everything the stream holds without writing it.
    pub fn discard(&mut self) {
        while self.reader.next().is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_are_written_as_one_line_each() {
        let mut out = Vec::new();
        let event = DynItem::Event {
            seq: 3,
            span: TickRange::new(Tick(10), Tick(20)),
            settled: Tick(20),
            payload: serde_json::json!({"kind": "word", "tx": 1, "mosi": 3, "miso": null}),
        };
        write_item(&mut out, "spi", &event, false).expect("writes");
        write_item(
            &mut out,
            "spi",
            &DynItem::Gap(TickRange::new(Tick(20), Tick(30))),
            false,
        )
        .expect("writes");
        write_item(&mut out, "spi", &DynItem::Progress(Tick(40)), false).expect("writes");
        write_item(&mut out, "spi", &DynItem::Progress(Tick(40)), true).expect("writes");
        write_item(&mut out, "spi", &DynItem::End(Tick(50)), false).expect("writes");
        let text = String::from_utf8(out).expect("utf-8");
        assert_eq!(
            text,
            concat!(
                r#"{"stream":"spi","seq":3,"span":[10,20],"settled":20,"event":{"kind":"word","tx":1,"mosi":3,"miso":null}}"#,
                "\n",
                r#"{"stream":"spi","gap":[20,30]}"#,
                "\n",
                r#"{"stream":"spi","progress":40}"#,
                "\n",
                r#"{"stream":"spi","end":50}"#,
                "\n",
            )
        );
    }
}
