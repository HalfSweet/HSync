// SPDX-License-Identifier: Apache-2.0

//! Generates and decodes a complete SPI transaction using the public facade.

use std::sync::Arc;

use uiharu::protocols::spi::synth::SpiWaveform;
use uiharu::protocols::spi::{Spi, SpiConfig};
use uiharu::{Graph, Item, SignalItem, SourceSpec, Tick, TimeBase};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = SpiConfig::default();
    let mut waveform = SpiWaveform::new(config, 4, 5, Tick::ZERO);
    waveform.idle(20);
    waveform.transaction([(0x03, 0xff), (0xa5, 0x5a)]);
    waveform.idle(20);
    let block = waveform.finish();
    let end = block.end();

    let mut graph = Graph::new();
    let source = graph.source(SourceSpec::new("logic", 4, TimeBase::new(25_000_000)))?;
    let spi = graph.add(Spi::new(config)?);
    graph.connect(source.samples, Spi::input(spi))?;
    let output = graph.expose("spi", Spi::events(spi))?;
    let mut pipeline = graph.build()?;
    let events = pipeline.exposed(output).expect("exposed once");

    pipeline.push(source.id, SignalItem::Data(Arc::new(block)))?;
    pipeline.push(source.id, SignalItem::End(end))?;
    pipeline.run();

    let mut count = 0;
    let mut ended = false;
    while let Some(item) = events.next() {
        println!("{item:?}");
        count += usize::from(matches!(item, Item::Event(_)));
        if matches!(item, Item::End(_)) {
            ended = true;
            break;
        }
    }
    // Begin, two words and end, followed by end-of-stream.
    assert_eq!(count, 4);
    assert!(ended);
    Ok(())
}
