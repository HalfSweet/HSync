// SPDX-License-Identifier: Apache-2.0

//! Builds the same SPI pipeline from a saved JSON description.

use std::sync::Arc;

use uiharu::protocols::spi::SpiConfig;
use uiharu::protocols::spi::synth::SpiWaveform;
use uiharu::registry::{Description, DynItem, build};
use uiharu::{SignalItem, Tick};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let description: Description = serde_json::from_str(
        r#"{
            "source": { "channels": 4, "rate": 25000000 },
            "nodes": { "spi": { "type": "spi" } },
            "connections": [{ "from": "source", "to": "spi.logic" }],
            "expose": { "spi": "spi.events" }
        }"#,
    )?;
    let mut built = build(&description)?;
    let mut waveform = SpiWaveform::new(SpiConfig::default(), 4, 5, Tick::ZERO);
    waveform.idle(20);
    waveform.transaction([(0x03, 0xff), (0xa5, 0x5a)]);
    waveform.idle(20);
    let block = waveform.finish();
    let end = block.end();
    built
        .pipeline
        .push(built.source, SignalItem::Data(Arc::new(block)))?;
    built.pipeline.push(built.source, SignalItem::End(end))?;
    built.pipeline.run();

    let mut count = 0;
    let mut ended = false;
    for stream in &mut built.streams {
        while let Some(item) = stream.reader.next() {
            println!("{item:?}");
            count += usize::from(matches!(item, DynItem::Event { .. }));
            ended |= matches!(item, DynItem::End(_));
        }
    }
    assert_eq!(count, 4);
    assert!(ended);
    assert!(built.faults().is_empty());
    Ok(())
}
