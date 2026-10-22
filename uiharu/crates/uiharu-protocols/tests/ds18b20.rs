// SPDX-License-Identifier: Apache-2.0

//! The DS18B20 decoder on synthesized 1-Wire waveforms, through the link and network layers.

use uiharu_core::testing::{self, without_progress};
use uiharu_core::{
    Graph, GraphConfig, GraphError, InputRef, Item, LogicBlock, OutputRef, SchemaId, SourceSpec,
    Tick, TickRange, TimeBase,
};
use uiharu_protocols::ds18b20::synth::{
    command, convert_t, read_power_supply, read_scratchpad, write_scratchpad,
};
use uiharu_protocols::ds18b20::{
    COPY_SCRATCHPAD, Ds18b20, Ds18b20Event, READ_POWER_SUPPLY, READ_SCRATCHPAD, RECALL_E2,
};
use uiharu_protocols::onewire::rom::{
    MATCH_ROM, OVERDRIVE_MATCH_ROM, OVERDRIVE_SKIP_ROM, READ_ROM, SEARCH_ROM,
};
use uiharu_protocols::onewire::synth::{OneWireWaveform, Slot};
use uiharu_protocols::onewire::{
    LinkConfig, LinkEvent, ONEWIRE_EVENTS, OneWireLink, OneWireNetwork, Speed,
};
use uiharu_protocols::spi::{SPI_EVENTS, Spi, SpiConfig, SpiEvent};

const CHANNELS: u8 = 1;
const DATA: u8 = 0;
const MHZ: u64 = 1_000_000;

fn config(speed: Speed) -> LinkConfig {
    LinkConfig { data: DATA, speed }
}

fn waveform(speed: Speed, rate: u64) -> OneWireWaveform {
    let mut waveform =
        OneWireWaveform::new(config(speed), CHANNELS, TimeBase::new(rate), Tick::ZERO);
    waveform.idle(100);
    waveform
}

/// Source → link layer → network layer → DS18B20.
fn stack(speed: Speed, rate: u64, graph: GraphConfig) -> testing::Harness<Ds18b20Event> {
    testing::Harness::build(CHANNELS, TimeBase::new(rate), graph, |graph, source| {
        let link = graph.add(OneWireLink::new(config(speed)).expect("a valid configuration"));
        graph.connect(source.samples, OneWireLink::input(link))?;
        let net = graph.add(OneWireNetwork::new());
        graph.connect(OneWireLink::events(link), OneWireNetwork::input(net))?;
        let thermometer = graph.add(Ds18b20::new());
        graph.connect(OneWireNetwork::events(net), Ds18b20::input(thermometer))?;
        Ok(Ds18b20::events(thermometer))
    })
    .expect("builds")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn describe(event: &Ds18b20Event) -> String {
    match event {
        Ds18b20Event::Command { tx, code } => format!("command tx{} {code:02x}", tx),
        Ds18b20Event::Scratchpad {
            tx,
            bytes,
            crc_ok,
            temperature_raw,
            th,
            tl,
            config,
            resolution,
        } => format!(
            "scratchpad tx{} {} crc {crc_ok} t {temperature_raw} th {th} tl {tl} config \
             {config:02x} res {resolution}",
            tx,
            hex(bytes)
        ),
        Ds18b20Event::PartialScratchpad {
            tx,
            bytes,
            temperature_raw,
        } => format!("partial tx{} {} t {temperature_raw:?}", tx, hex(bytes)),
        Ds18b20Event::WriteScratchpad { tx, th, tl, config } => {
            format!("write tx{} th {th} tl {tl} config {config:02x}", tx)
        }
        Ds18b20Event::PowerSupply { tx, parasite } => {
            format!("power tx{} parasite {parasite}", tx)
        }
        Ds18b20Event::UnknownCommand { tx, code } => format!("unknown tx{} {code:02x}", tx),
        Ds18b20Event::End { tx, reason } => format!("end tx{} {reason:?}", tx),
    }
}

/// One line per item, progress left out: it depends on how the samples were cut.
fn lines(items: &[Item<Ds18b20Event>]) -> Vec<String> {
    without_progress(items)
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let at = if event.span.is_empty() {
                    format!("@{}", event.settled)
                } else {
                    format!(
                        "[{},{})@{}",
                        event.span.start, event.span.end, event.settled
                    )
                };
                format!("{} {at}", describe(&event.payload))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// The lines without their ticks.
fn what(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            let cut = line.find(" [").or_else(|| line.find(" @"));
            cut.map_or(line.clone(), |cut| line[..cut].to_owned())
        })
        .collect()
}

/// Decodes `slots` at standard speed, 1 MHz, closed by a reset the device answers.
fn decode(slots: impl IntoIterator<Item = Slot>) -> Vec<String> {
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.slots(slots);
    waveform.reset(true);
    waveform.idle(10);
    let mut harness = stack(Speed::Standard, MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    lines(&harness.items)
}

/// The CRC-8 of the 1-Wire standard, computed here independently of the decoder: most
/// significant bit first with the polynomial 0x31 on reflected bytes, the result reflected.
fn crc(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in bytes {
        crc ^= byte.reverse_bits();
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x31
            } else {
                crc << 1
            };
        }
    }
    crc.reverse_bits()
}

/// A scratchpad holding these registers and its CRC.
fn scratchpad(temperature_raw: i16, th: i8, tl: i8, config: u8) -> [u8; 9] {
    let [lsb, msb] = temperature_raw.to_le_bytes();
    let mut bytes = [
        lsb,
        msb,
        th.to_le_bytes()[0],
        tl.to_le_bytes()[0],
        config,
        0xff,
        0x0c,
        0x10,
        0,
    ];
    bytes[8] = crc(&bytes[..8]);
    bytes
}

/// A ROM code of `family` with its CRC.
fn rom_code(family: u8, serial: u64) -> u64 {
    let mut bytes = [0u8; 8];
    bytes[0] = family;
    bytes[1..7].copy_from_slice(&serial.to_le_bytes()[..6]);
    bytes[7] = crc(&bytes[..7]);
    u64::from_le_bytes(bytes)
}

#[test]
fn the_tests_crc_is_the_standards() {
    assert_eq!(crc(b"123456789"), 0xa1);
    // The scratchpad of a DS18B20 at power-up: 85 °C, TH 75, TL 70, 12 bits.
    assert_eq!(
        scratchpad(0x0550, 75, 70, 0x7f),
        [0x50, 0x05, 0x4b, 0x46, 0x7f, 0xff, 0x0c, 0x10, 0x1c]
    );
}

#[test]
fn convert_t_with_skip_rom() {
    let lines = decode(convert_t(None));
    // The reset falls at 100; Skip ROM and the command take eight slots of 70 µs each; the
    // last slot of 0x44 is a 0, released 60 µs after it falls.
    let command_end = 100 + 980 + 15 * 70 + 60;
    // The next reset falls at 2200 and its presence pulse ends 650 µs later.
    assert_eq!(
        lines,
        vec![
            format!("command tx1 44 [100,{})@{command_end}", command_end + 1),
            "end tx1 Complete @2850".to_owned(),
            "eof 3190".to_owned(),
        ]
    );
}

#[test]
fn read_scratchpad_with_match_rom() {
    let rom = rom_code(0x28, 0x0000_0612_3456);
    // 25.0625 °C.
    let bytes = scratchpad(0x0191, 75, -10, 0x5f);
    let lines = decode(read_scratchpad(Some(rom), &bytes));
    assert_eq!(
        what(&lines),
        vec![
            "command tx1 be".to_owned(),
            format!(
                "scratchpad tx1 {} crc true t 401 th 75 tl -10 config 5f res 11",
                hex(&bytes)
            ),
            "end tx1 Complete".to_owned(),
            lines[3].clone(),
        ]
    );
    // The command spans the reset, Match ROM, the ROM code and itself, and settles at the
    // sampling point of its last slot, a 1; the scratchpad spans its 72 read slots and
    // settles with the last.
    let first = 100 + 980 + (8 + 64 + 8) * 70;
    let last = first + 71 * 70;
    let settled = last + if bytes[8] >> 7 == 1 { 15 } else { 30 };
    assert_eq!(
        lines[0],
        format!("command tx1 be [100,{})@{}", first - 54, first - 55)
    );
    assert!(
        lines[1].ends_with(&format!("[{first},{})@{settled}", settled + 1)),
        "{}",
        lines[1]
    );
}

#[test]
fn a_scratchpad_with_a_bad_crc_is_still_reported() {
    let mut bytes = scratchpad(-0x0190, 20, 10, 0x1f);
    bytes[8] ^= 0x01;
    let lines = what(&decode(read_scratchpad(None, &bytes)));
    // -25 °C, 9 bits.
    assert_eq!(
        lines[1],
        format!(
            "scratchpad tx1 {} crc false t -400 th 20 tl 10 config 1f res 9",
            hex(&bytes)
        )
    );
    assert_eq!(lines[2], "end tx1 Complete");
}

#[test]
fn write_scratchpad_carries_the_registers() {
    let lines = what(&decode(write_scratchpad(None, -10, -55, 0x3f)));
    assert_eq!(
        lines[..3],
        [
            "command tx1 4e",
            "write tx1 th -10 tl -55 config 3f",
            "end tx1 Complete"
        ]
    );
}

#[test]
fn read_power_supply_reads_one_slot() {
    for parasite in [true, false] {
        let lines = decode(read_power_supply(None, parasite));
        assert_eq!(
            what(&lines)[..3],
            [
                "command tx1 b4".to_owned(),
                format!("power tx1 parasite {parasite}"),
                "end tx1 Complete".to_owned()
            ]
        );
        // The single slot is reported when the reset ends the transaction.
        let slot = 100 + 980 + 16 * 70;
        let reset = slot + 70;
        let end = if parasite { slot + 31 } else { slot + 16 };
        assert_eq!(
            lines[1],
            format!(
                "power tx1 parasite {parasite} [{slot},{end})@{}",
                reset + 650
            )
        );
    }
    // A master that reads a whole byte gets the answer in its first slot.
    let mut slots = command(None, READ_POWER_SUPPLY);
    slots.extend((0..8).map(|_| Slot::Read(false)));
    assert_eq!(
        what(&decode(slots))[1],
        "power tx1 parasite true".to_owned()
    );
    // Without the read slot the command is incomplete.
    assert_eq!(
        what(&decode(command(None, READ_POWER_SUPPLY)))[..2],
        ["command tx1 b4", "end tx1 Incomplete"]
    );
}

#[test]
fn a_scratchpad_cut_short_by_a_reset_is_partial() {
    let bytes = scratchpad(0x0191, 75, 70, 0x7f);
    let lines = decode(read_scratchpad(None, &bytes[..2]));
    let first = 100 + 980 + 16 * 70;
    let reset = first + 16 * 70;
    assert_eq!(
        lines[..3],
        [
            format!("command tx1 be [100,{})@{}", first - 54, first - 55),
            // The bytes read, the last bit a 0 the slave released after 30 µs, settled where
            // the reset ended the transaction.
            format!(
                "partial tx1 9101 t Some(401) [{first},{})@{}",
                reset - 39,
                reset + 650
            ),
            format!("end tx1 Incomplete @{}", reset + 650),
        ]
    );
    let lines = what(&decode(read_scratchpad(None, &bytes[..1])));
    assert_eq!(
        lines[..3],
        [
            "command tx1 be",
            "partial tx1 91 t None",
            "end tx1 Incomplete"
        ]
    );
    // Nothing read at all: the end alone.
    let lines = what(&decode(read_scratchpad(None, &[])));
    assert_eq!(lines[..2], ["command tx1 be", "end tx1 Incomplete"]);
}

#[test]
fn transactions_for_other_devices_are_ignored() {
    let other = rom_code(0x10, 0x0000_0812_3456);
    let device = rom_code(0x28, 0x0000_0612_3456);
    let mut slots = command(Some(other), READ_SCRATCHPAD);
    slots.extend((0..9).map(|_| Slot::Read(true)));
    // Read ROM selects the only device on the bus: here a DS18B20.
    slots.push(Slot::Reset { presence: true });
    slots.extend((0..8).map(|n| Slot::Write(READ_ROM >> n & 1 == 1)));
    slots.extend((0..64).map(|n| Slot::Read(device >> n & 1 == 1)));
    slots.extend((0..8).map(|n| Slot::Write(0x44 >> n & 1 == 1)));
    // A search is followed by a reset, not by a function command.
    slots.push(Slot::Reset { presence: true });
    slots.extend((0..8).map(|n| Slot::Write(SEARCH_ROM >> n & 1 == 1)));
    slots.extend(uiharu_protocols::onewire::synth::search_rom(&[device]).0);
    slots.extend((0..8).map(|n| Slot::Write(0x44 >> n & 1 == 1)));
    let lines = what(&decode(slots));
    assert_eq!(lines[..2], ["command tx2 44", "end tx2 Complete"]);
    assert_eq!(lines.len(), 3, "{lines:?}");
}

#[test]
fn an_unknown_command_is_reported_and_its_transaction_skipped() {
    let mut slots = command(None, 0xaa);
    slots.extend((0..16).map(|n| Slot::Read(n % 2 == 0)));
    slots.extend(command(None, COPY_SCRATCHPAD));
    slots.extend(command(None, RECALL_E2));
    slots.extend((0..5).map(|_| Slot::Read(false)));
    let lines = what(&decode(slots));
    // Four resets, 69 slots and the 10 µs after the last reset.
    let end = 100 + 4 * 980 + (32 + 16 + 16 + 5) * 70 + 10;
    assert_eq!(
        lines,
        [
            "unknown tx1 aa".to_owned(),
            "command tx2 48".to_owned(),
            "end tx2 Complete".to_owned(),
            // The busy bits after Recall E² are skipped.
            "command tx3 b8".to_owned(),
            "end tx3 Complete".to_owned(),
            format!("eof {end}"),
        ]
    );
}

#[test]
fn the_overdrive_rom_commands_address_the_device_too() {
    let device = rom_code(0x28, 0x0000_0612_3456);
    let bytes = scratchpad(0x0550, 75, 70, 0x7f);
    let mut waveform = waveform(Speed::Overdrive, 10 * MHZ);
    waveform.reset(true);
    waveform.byte(OVERDRIVE_SKIP_ROM);
    waveform.byte(READ_SCRATCHPAD);
    for byte in bytes {
        waveform.read_byte(byte);
    }
    waveform.reset(true);
    waveform.byte(OVERDRIVE_MATCH_ROM);
    waveform.rom(device);
    waveform.byte(0x44);
    waveform.reset(true);
    waveform.byte(MATCH_ROM);
    waveform.rom(device ^ 0x01);
    waveform.byte(0x44);
    waveform.reset(false);
    let mut harness = stack(Speed::Overdrive, 10 * MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    let lines = what(&lines(&harness.items));
    assert_eq!(
        lines[..lines.len() - 1],
        [
            "command tx1 be".to_owned(),
            format!(
                "scratchpad tx1 {} crc true t 1360 th 75 tl 70 config 7f res 12",
                hex(&bytes)
            ),
            "end tx1 Complete".to_owned(),
            "command tx2 44".to_owned(),
            "end tx2 Complete".to_owned(),
        ]
    );
}

#[test]
fn a_gap_aborts_the_command_and_the_next_transaction_decodes() {
    let bytes = scratchpad(0x0191, 75, 70, 0x7f);
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.slots(read_scratchpad(None, &bytes));
    waveform.slots(convert_t(None));
    waveform.reset(true);
    waveform.idle(10);
    let block = waveform.finish();
    // Lose the fifth scratchpad byte.
    let fifth = 100 + 980 + (16 + 4 * 8) * 70;
    let gap = TickRange::new(Tick(fifth + 20), Tick(fifth + 400));
    let mut harness = stack(Speed::Standard, MHZ, GraphConfig::default());
    harness.feed_with_gaps(&block, &[gap], &[333]);
    harness.end();
    let lines = lines(&harness.items);
    assert_eq!(
        what(&lines)[..6],
        [
            "command tx1 be".to_owned(),
            format!("partial tx1 {} t Some(401)", hex(&bytes[..4])),
            "end tx1 Aborted".to_owned(),
            format!("gap {gap}"),
            "command tx2 44".to_owned(),
            "end tx2 Complete".to_owned(),
        ]
    );
    assert!(
        lines[1].ends_with(&format!(")@{}", gap.start)),
        "{}",
        lines[1]
    );
    // The end covers the first slot of the fifth byte, read before the gap, and is placed
    // at the gap.
    assert_eq!(
        lines[2],
        format!("end tx1 Aborted [{fifth},{})@{}", gap.start, gap.start)
    );
}

#[test]
fn the_end_of_the_samples_aborts_the_command() {
    let bytes = scratchpad(0x0191, 75, 70, 0x7f);
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.slots(read_scratchpad(None, &bytes[..5]));
    let block = waveform.finish();
    let mut harness = stack(Speed::Standard, MHZ, GraphConfig::default());
    harness.run_all(&block);
    let end = block.end();
    assert_eq!(
        what(&lines(&harness.items)),
        vec![
            "command tx1 be".to_owned(),
            format!("partial tx1 {} t Some(401)", hex(&bytes[..5])),
            "end tx1 Aborted".to_owned(),
            format!("eof {end}"),
        ]
    );
    assert_eq!(lines(&harness.items)[2], format!("end tx1 Aborted @{end}"));
}

#[test]
fn a_bad_slot_aborts_the_command() {
    let bytes = scratchpad(0x0191, 75, 70, 0x7f);
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.slots(read_scratchpad(None, &bytes[..3]));
    waveform.bad_slot(250_000);
    waveform.slots(read_scratchpad(None, &bytes));
    waveform.reset(true);
    waveform.idle(10);
    let mut harness = stack(Speed::Standard, MHZ, GraphConfig::default());
    harness.run_all(&waveform.finish());
    let lines = what(&lines(&harness.items));
    assert_eq!(
        lines[..5],
        [
            "command tx1 be".to_owned(),
            format!("partial tx1 {} t Some(401)", hex(&bytes[..3])),
            "end tx1 Aborted".to_owned(),
            "command tx2 be".to_owned(),
            format!(
                "scratchpad tx2 {} crc true t 401 th 75 tl 70 config 7f res 12",
                hex(&bytes)
            ),
        ]
    );
}

/// Every command, partial scratchpads and an unknown command, at 1 MHz, with two gaps: one in
/// the fourth byte of a scratchpad, one in the busy bits after Copy Scratchpad.
fn rich_traffic() -> (LogicBlock, [TickRange; 2]) {
    let device = rom_code(0x28, 0x0000_0612_3456);
    let bytes = scratchpad(0x0191, 75, 70, 0x7f);
    let mut waveform = waveform(Speed::Standard, MHZ);
    waveform.slots(convert_t(None));
    waveform.slots(command(Some(device), READ_SCRATCHPAD));
    let mut in_scratchpad = Tick::ZERO;
    for (n, byte) in bytes.into_iter().enumerate() {
        if n == 3 {
            in_scratchpad = waveform.position().offset(25);
        }
        waveform.read_byte(byte);
    }
    waveform.slots(write_scratchpad(None, 30, -5, 0x3f));
    waveform.slots(read_power_supply(Some(device), true));
    waveform.slots(read_scratchpad(None, &bytes[..3]));
    waveform.slots(command(None, 0x99));
    waveform.slots(command(None, COPY_SCRATCHPAD));
    let in_busy_bits = waveform.position().offset(100);
    waveform.slots((0..8).map(|_| Slot::Read(false)));
    waveform.slots(read_scratchpad(None, &bytes));
    waveform.slots(command(None, RECALL_E2));
    waveform.reset(true);
    waveform.idle(20);
    let gaps = [
        TickRange::new(in_scratchpad, in_scratchpad.offset(150)),
        TickRange::new(in_busy_bits, in_busy_bits.offset(31)),
    ];
    (waveform.finish(), gaps)
}

#[test]
fn events_do_not_depend_on_blocks_pauses_or_queues() {
    let (block, gaps) = rich_traffic();
    let mut whole = stack(Speed::Standard, MHZ, GraphConfig::default());
    whole.feed_with_gaps(&block, &gaps, &[u64::MAX]);
    whole.end();
    let kinds = what(&lines(&whole.items));
    for kind in [
        "command ",
        "scratchpad ",
        "write ",
        "power ",
        "partial ",
        "unknown ",
    ] {
        assert!(
            kinds.iter().any(|line| line.starts_with(kind)),
            "no {kind}: {kinds:?}"
        );
    }
    assert!(kinds.iter().any(|line| line.ends_with("Aborted")));
    testing::assert_block_invariant(|graph| stack(Speed::Standard, MHZ, graph), &block, &gaps);
}

#[test]
fn only_a_1_wire_network_stream_is_accepted() {
    let refused = |wire: &dyn Fn(&mut Graph, uiharu_core::NodeId) -> Result<(), GraphError>,
                   from: &str,
                   output: SchemaId| {
        let mut graph = Graph::new();
        graph
            .source(SourceSpec::new("logic", 4, TimeBase::new(MHZ)))
            .expect("a source");
        let thermometer = graph.add(Ds18b20::new());
        let error = wire(&mut graph, thermometer).expect_err("refused");
        assert_eq!(
            error,
            GraphError::SchemaMismatch {
                from: from.into(),
                to: "ds18b20#0.bus".into(),
                output,
                input: ONEWIRE_EVENTS,
            }
        );
    };
    // The SPI decoder's words.
    refused(
        &|graph, thermometer| {
            let spi = graph.add(Spi::new(SpiConfig::default()).expect("valid"));
            graph.connect(
                OutputRef::<SpiEvent>::of_node(spi, 0),
                InputRef::<SpiEvent>::of_node(thermometer, 0),
            )
        },
        "spi#1.events",
        SPI_EVENTS,
    );
    // The 1-Wire link layer's slots, below the network layer.
    refused(
        &|graph, thermometer| {
            let link = graph.add(OneWireLink::new(config(Speed::Standard)).expect("valid"));
            graph.connect(
                OneWireLink::events(link),
                InputRef::<LinkEvent>::of_node(thermometer, 0),
            )
        },
        "onewire-link#1.events",
        uiharu_protocols::onewire::ONEWIRE_LINK,
    );
}
