// SPDX-License-Identifier: Apache-2.0

//! The JTAG decoder on synthesized waveforms.

use serde::Serialize;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{
    EventSchema, GraphConfig, GraphError, InputRef, InputSpec, Io, Item, Levels, LogicBlock,
    LogicBlockBuilder, Node, NodeFuture, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Requires,
    SchemaId, Tick, TickRange, TimeBase,
};
use uiharu_protocols::jtag::synth::JtagWaveform;
use uiharu_protocols::jtag::{
    InitialState, Jtag, JtagBus, JtagConfig, JtagEvent, Next, Read, Register, ResetBy,
    SHIFT_BATCH_BITS, TapState,
};

/// TCK, TMS, TDI, TDO and TRST on channels 0 to 4.
const CHANNELS: u8 = 5;
/// Samples per half TCK period: a clock takes four samples and rises at its third.
const HALF: u64 = 2;
/// Samples before the first clock.
const LEAD: u64 = 4;

fn config() -> JtagConfig {
    JtagConfig {
        trst: Some(4),
        ..JtagConfig::default()
    }
}

/// The tick at which clock `n` (counted from 0 after the lead) rises.
fn rise(n: u64) -> u64 {
    LEAD + 4 * n + HALF
}

fn waveform(config: &JtagConfig) -> JtagWaveform {
    let mut waveform = JtagWaveform::new(config.clone(), CHANNELS, HALF, Tick::ZERO);
    waveform.idle(LEAD);
    waveform
}

fn harness(config: JtagConfig, graph: GraphConfig) -> Harness<JtagEvent> {
    Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        graph,
        |graph, source| {
            let jtag = graph.add(Jtag::new(config).expect("a valid configuration"));
            graph.connect(source.samples, Jtag::input(jtag))?;
            Ok(Jtag::events(jtag))
        },
    )
    .expect("builds")
}

fn decode(config: &JtagConfig, block: &LogicBlock) -> Vec<String> {
    let mut harness = harness(config.clone(), GraphConfig::default());
    harness.run_all(block);
    describe(&harness.without_progress())
}

fn decode_with_gap(config: &JtagConfig, block: &LogicBlock, gap: TickRange) -> Vec<String> {
    let mut harness = harness(config.clone(), GraphConfig::default());
    harness.feed_with_gaps(block, &[gap], &[97]);
    harness.end();
    describe(&harness.without_progress())
}

fn name(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .expect("serializes")
        .as_str()
        .expect("a name")
        .to_owned()
}

fn hex(bytes: &Option<Vec<u8>>) -> String {
    match bytes {
        Some(bytes) => bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        None => "--".to_owned(),
    }
}

fn at(span: TickRange, settled: Tick) -> String {
    if span.is_empty() {
        format!("@{settled}")
    } else {
        format!("[{},{})@{settled}", span.start, span.end)
    }
}

/// One line per item, readable in assertions.
fn describe(items: &[Item<JtagEvent>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Event(event) => {
                let what = match &event.payload {
                    JtagEvent::State { state } => format!("state {}", name(state)),
                    JtagEvent::Reset { by } => format!("reset {}", name(by)),
                    JtagEvent::ScanBegin { scan, register } => {
                        format!("scan-begin s{} {}", scan.0, name(register))
                    }
                    JtagEvent::Shift {
                        scan,
                        offset,
                        tdi,
                        tdo,
                        bits,
                    } => format!(
                        "shift s{} +{offset} {bits} {}/{}",
                        scan.0,
                        hex(tdi),
                        hex(tdo)
                    ),
                    JtagEvent::ScanEnd { scan, bits, reason } => {
                        format!("scan-end s{} {bits} {}", scan.0, name(reason))
                    }
                };
                format!("{what} {}", at(event.span, event.settled))
            }
            Item::Gap(range) => format!("gap {range}"),
            Item::Progress(tick) => format!("progress {tick}"),
            Item::End(tick) => format!("eof {tick}"),
        })
        .collect()
}

/// An event decided at `tick`, a sample.
fn one(what: &str, tick: u64) -> String {
    format!("{what} [{tick},{})@{tick}", tick + 1)
}

/// An event decided at the rising edge of clock `n`.
fn on(what: &str, n: u64) -> String {
    one(what, rise(n))
}

/// Bits `from..to` of `bytes`, least significant bit first, packed from bit 0.
fn bits_of(bytes: &[u8], from: u32, to: u32) -> Vec<u8> {
    let mut packed = vec![0u8; (to - from).div_ceil(8) as usize];
    for i in from..to {
        let bit = bytes[i as usize / 8] >> (i % 8) & 1;
        packed[(i - from) as usize / 8] |= bit << ((i - from) % 8);
    }
    packed
}

/// A shift batch of bits `from..to` of a scan whose first bit rose with clock `first`, settled
/// at `settled`.
fn batch(
    scan: u64,
    lines: (Option<&[u8]>, Option<&[u8]>),
    from: u32,
    to: u32,
    first: u64,
    settled: u64,
) -> String {
    let packed = |bytes: Option<&[u8]>| bytes.map(|bytes| bits_of(bytes, from, to));
    format!(
        "shift s{scan} +{from} {} {}/{} [{},{})@{settled}",
        to - from,
        hex(&packed(lines.0)),
        hex(&packed(lines.1)),
        rise(first + u64::from(from)),
        rise(first + u64::from(to) - 1) + 1
    )
}

fn pattern(len: usize, multiplier: usize, offset: usize) -> Vec<u8> {
    (0..len).map(|i| (i * multiplier + offset) as u8).collect()
}

#[test]
fn every_state_is_reported_along_a_path_through_all_sixteen() {
    let config = config();
    let mut waveform = waveform(&config);
    waveform.reset();
    let path = [
        false, true, false, false, true, false, true, false, true, true, true, true, false, false,
        true, false, true, false, true, true, false, true, true, true,
    ];
    waveform.clocks(&path);
    waveform.idle(6);
    let block = waveform.finish();
    let lines = decode(&config, &block);
    // The fifth clock with TMS high resets the controller from the unknown state; the path
    // starts with clock 5. TDI and TDO idle high, so every bit shifted is a one.
    let expected = vec![
        on("reset tms", 4),
        on("state test-logic-reset", 4),
        on("state run-test-idle", 5),
        on("state select-dr-scan", 6),
        on("state capture-dr", 7),
        on("state shift-dr", 8),
        on("scan-begin s1 dr", 8),
        on("shift s1 +0 1 01/01", 9),
        on("state exit1-dr", 9),
        on("state pause-dr", 10),
        on("state exit2-dr", 11),
        // Back in Shift-DR, the paused scan goes on.
        on("state shift-dr", 12),
        on("shift s1 +1 1 01/01", 13),
        on("state exit1-dr", 13),
        on("scan-end s1 2 complete", 14),
        on("state update-dr", 14),
        on("state select-dr-scan", 15),
        on("state select-ir-scan", 16),
        on("state capture-ir", 17),
        on("state shift-ir", 18),
        on("scan-begin s2 ir", 18),
        on("shift s2 +0 1 01/01", 19),
        on("state exit1-ir", 19),
        on("state pause-ir", 20),
        on("state exit2-ir", 21),
        on("state shift-ir", 22),
        on("shift s2 +1 1 01/01", 23),
        on("state exit1-ir", 23),
        on("scan-end s2 2 complete", 24),
        on("state update-ir", 24),
        on("state run-test-idle", 25),
        on("state select-dr-scan", 26),
        on("state select-ir-scan", 27),
        on("reset tms", 28),
        on("state test-logic-reset", 28),
        format!("eof {}", block.end()),
    ];
    assert_eq!(lines, expected);
    for state in TapState::ALL {
        let line = format!("state {} ", name(&state));
        assert!(
            lines.iter().any(|seen| seen.starts_with(&line)),
            "{state:?}"
        );
    }
}

#[test]
fn scans_of_any_length_come_in_batches_on_the_lines_connected() {
    let tdi = pattern(38, 37, 11);
    let tdo = pattern(38, 91, 5);
    for register in [Register::Ir, Register::Dr] {
        for bits in [1u32, 8, 300] {
            for (has_tdi, has_tdo) in [(true, true), (true, false), (false, true)] {
                let config = JtagConfig {
                    tdi: has_tdi.then_some(2),
                    tdo: has_tdo.then_some(3),
                    ..config()
                };
                let mut waveform = waveform(&config);
                waveform.reset();
                waveform.clock(false, true);
                waveform.scan(register, &tdi, &tdo, bits);
                waveform.idle(4);
                let block = waveform.finish();
                let lines = decode(&config, &block);
                // Clocks 0 to 4 reset, clock 5 goes to Run-Test/Idle; the IR column is one
                // clock further away than the DR column.
                let (column, begin) = match register {
                    Register::Ir => ("ir", 9),
                    Register::Dr => ("dr", 8),
                };
                let mut expected = vec![
                    on("reset tms", 4),
                    on("state test-logic-reset", 4),
                    on("state run-test-idle", 5),
                    on("state select-dr-scan", 6),
                ];
                if register == Register::Ir {
                    expected.push(on("state select-ir-scan", 7));
                }
                expected.push(on(&format!("state capture-{column}"), begin - 1));
                expected.push(on(&format!("state shift-{column}"), begin));
                expected.push(on(&format!("scan-begin s1 {column}"), begin));
                let first = begin + 1;
                let exit = first + u64::from(bits) - 1;
                let lines_of = (has_tdi.then_some(&tdi[..]), has_tdo.then_some(&tdo[..]));
                let mut from = 0;
                while from < bits {
                    let to = (from + u32::from(SHIFT_BATCH_BITS)).min(bits);
                    let last = rise(first + u64::from(to) - 1);
                    expected.push(batch(1, lines_of, from, to, first, last));
                    from = to;
                }
                expected.push(on(&format!("state exit1-{column}"), exit));
                expected.push(on(&format!("scan-end s1 {bits} complete"), exit + 1));
                expected.push(on(&format!("state update-{column}"), exit + 1));
                expected.push(on("state run-test-idle", exit + 2));
                expected.push(format!("eof {}", block.end()));
                assert_eq!(
                    lines, expected,
                    "{register:?}, {bits} bits, TDI {has_tdi}, TDO {has_tdo}"
                );
            }
        }
    }
}

#[test]
fn a_capture_without_shifting_is_no_scan_and_a_scan_may_begin_after_a_pause() {
    let config = config();
    let mut waveform = waveform(&config);
    waveform.reset();
    waveform.clock(false, true);
    // Select-DR, Capture-DR, Exit1-DR, Update-DR, Run-Test/Idle: nothing shifted (clocks 6
    // to 10).
    waveform.clocks(&[true, false, true, true, false]);
    // Select-DR, Capture-DR, Exit1-DR, Pause-DR, Exit2-DR, Shift-DR (clocks 11 to 16): the
    // scan begins in Shift-DR.
    waveform.load_tdo(&[0b10], 2);
    waveform.clocks(&[true, false, true, false, true, false]);
    waveform.shift(&[0b01], 2, true);
    waveform.go_to(TapState::RunTestIdle);
    waveform.idle(4);
    let block = waveform.finish();
    let lines = decode(&config, &block);
    let others: Vec<&String> = lines
        .iter()
        .filter(|line| !line.starts_with("state "))
        .collect();
    assert_eq!(
        others,
        vec![
            &on("reset tms", 4),
            &on("scan-begin s1 dr", 16),
            &batch(1, (Some(&[0b01]), Some(&[0b10])), 0, 2, 17, rise(18)),
            &on("scan-end s1 2 complete", 19),
            &format!("eof {}", block.end()),
        ]
    );
    assert!(lines.contains(&on("state update-dr", 9)), "{lines:?}");
    assert!(lines.contains(&on("state exit2-dr", 15)), "{lines:?}");
    assert!(lines.contains(&on("state shift-dr", 16)), "{lines:?}");
}

#[test]
fn trst_aborts_the_scan_and_holds_the_controller_in_reset() {
    let config = config();
    let mut waveform = waveform(&config);
    waveform.reset();
    waveform.clock(false, true);
    waveform.go_to(TapState::ShiftDr);
    waveform.shift(&[0xff, 0x03], 10, false);
    let trst = waveform.position().get();
    waveform.assert_trst();
    // Clocks while TRST is low do not move the controller, whatever TMS says.
    waveform.clocks(&[false, false, true]);
    waveform.release_trst();
    let next = waveform.position().get() + HALF;
    waveform.clock(false, true);
    waveform.idle(4);
    let block = waveform.finish();
    let lines = decode(&config, &block);
    let expected = vec![
        on("reset tms", 4),
        on("state test-logic-reset", 4),
        on("state run-test-idle", 5),
        on("state select-dr-scan", 6),
        on("state capture-dr", 7),
        on("state shift-dr", 8),
        on("scan-begin s1 dr", 8),
        // The ten bits shifted so far go out where TRST cut the scan short.
        batch(
            1,
            (Some(&[0xff, 0x03]), Some(&[0xff, 0x03])),
            0,
            10,
            9,
            trst,
        ),
        one("scan-end s1 10 aborted", trst),
        one("reset trst", trst),
        one("state test-logic-reset", trst),
        one("state run-test-idle", next),
        format!("eof {}", block.end()),
    ];
    assert_eq!(lines, expected);
}

#[test]
fn trst_low_at_the_first_sample_is_a_reset_there() {
    let config = config();
    let mut waveform = JtagWaveform::new(config.clone(), CHANNELS, HALF, Tick::ZERO);
    waveform.assert_trst();
    waveform.idle(2);
    waveform.release_trst();
    let next = waveform.position().get() + HALF;
    waveform.clock(false, true);
    waveform.idle(4);
    let block = waveform.finish();
    // The assumed state gives way to the reset TRST holds the controller in.
    for initial_state in [InitialState::Unknown, InitialState::RunTestIdle] {
        let config = JtagConfig {
            initial_state,
            ..config.clone()
        };
        assert_eq!(
            decode(&config, &block),
            vec![
                one("reset trst", 0),
                one("state test-logic-reset", 0),
                one("state run-test-idle", next),
                format!("eof {}", block.end()),
            ],
            "{initial_state:?}"
        );
    }
}

#[test]
fn a_tck_edge_as_trst_changes_does_not_clock_the_controller() {
    let config = JtagConfig {
        initial_state: InitialState::RunTestIdle,
        ..config()
    };
    let (tck, tms, trst) = (1u64, 1u64 << 1, 1u64 << 4);
    let mut builder = LogicBlockBuilder::new(Tick::ZERO, CHANNELS);
    builder.push_repeated(Levels(trst | tms), 4);
    // TCK rises with TMS high as TRST is asserted: no move to Select-DR-Scan.
    builder.push_repeated(Levels(tck | tms), 4);
    builder.push_repeated(Levels(0), 4);
    // TCK rises with TMS low as TRST is released: no move to Run-Test/Idle.
    builder.push_repeated(Levels(trst | tck), 4);
    builder.push_repeated(Levels(trst), 4);
    // The next rising edge clocks the controller.
    builder.push_repeated(Levels(trst | tck), 4);
    let block = builder.finish().expect("a block");
    assert_eq!(
        decode(&config, &block),
        vec![
            one("state run-test-idle", 0),
            one("reset trst", 4),
            one("state test-logic-reset", 4),
            one("state run-test-idle", 20),
            "eof 24".to_owned(),
        ]
    );
}

#[test]
fn the_state_is_reported_only_once_known_or_assumed() {
    let tdi = [0xa5];
    let tdo = [0x3c];
    // From power-up, in Test-Logic-Reset, without the five clocks of a reset.
    let mut power_up = waveform(&config());
    power_up.clock(false, true);
    power_up.scan(Register::Dr, &tdi, &tdo, 8);
    power_up.idle(4);
    let from_power_up = power_up.finish();
    let unknown = config();
    assert_eq!(
        decode(&unknown, &from_power_up),
        vec![format!("eof {}", from_power_up.end())]
    );
    let assumed = JtagConfig {
        initial_state: InitialState::TestLogicReset,
        ..config()
    };
    let lines = decode(&assumed, &from_power_up);
    assert_eq!(lines[0], one("state test-logic-reset", 0));
    assert_eq!(lines[1], on("state run-test-idle", 0));
    assert!(lines.contains(&on("scan-begin s1 dr", 3)), "{lines:?}");
    assert!(
        lines.contains(&batch(1, (Some(&tdi), Some(&tdo)), 0, 8, 4, rise(11))),
        "{lines:?}"
    );
    assert!(
        lines.contains(&on("scan-end s1 8 complete", 12)),
        "{lines:?}"
    );

    // A capture that starts with the controller in Run-Test/Idle: the samples begin with the
    // first clock of the scan.
    let mut waveform = waveform(&config());
    waveform.reset();
    waveform.clock(false, true);
    let cut = waveform.position();
    waveform.scan(Register::Ir, &[0b0110], &[0b0001], 4);
    waveform.idle(4);
    let whole = waveform.finish();
    let tail = whole
        .slice(TickRange::new(cut, whole.end()))
        .expect("a slice");
    assert_eq!(decode(&unknown, &tail), vec![format!("eof {}", tail.end())]);
    let idle = JtagConfig {
        initial_state: InitialState::RunTestIdle,
        ..config()
    };
    let lines = decode(&idle, &tail);
    assert_eq!(lines[0], one("state run-test-idle", cut.get()));
    assert_eq!(lines[1], on("state select-dr-scan", 6));
    assert!(lines.contains(&on("scan-begin s1 ir", 9)), "{lines:?}");
    assert!(
        lines.contains(&batch(
            1,
            (Some(&[0b0110]), Some(&[0b0001])),
            0,
            4,
            10,
            rise(13)
        )),
        "{lines:?}"
    );
    assert!(
        lines.contains(&on("scan-end s1 4 complete", 14)),
        "{lines:?}"
    );
}

#[test]
fn only_five_clocks_in_a_row_with_tms_high_reset_an_unknown_controller() {
    let config = config();
    let mut waveform = waveform(&config);
    waveform.clocks(&[true, true, true, true, false, true, true, true, true]);
    waveform.clocks(&[true, false]);
    waveform.idle(4);
    let block = waveform.finish();
    assert_eq!(
        decode(&config, &block),
        vec![
            on("reset tms", 9),
            on("state test-logic-reset", 9),
            on("state run-test-idle", 10),
            format!("eof {}", block.end()),
        ]
    );
}

#[test]
fn the_end_of_the_samples_aborts_the_scan() {
    let config = config();
    let tdi = pattern(13, 29, 3);
    let tdo = pattern(13, 7, 1);
    let mut waveform = waveform(&config);
    waveform.reset();
    waveform.clock(false, true);
    waveform.load_tdo(&tdo, 100);
    waveform.go_to(TapState::ShiftDr);
    waveform.shift(&tdi, 100, false);
    let block = waveform.finish();
    let end = block.end().get();
    let lines = decode(&config, &block);
    let n = lines.len();
    assert_eq!(
        lines[n - 3..],
        [
            batch(1, (Some(&tdi), Some(&tdo)), 0, 100, 9, end),
            format!("scan-end s1 100 aborted @{end}"),
            format!("eof {end}"),
        ]
    );
}

#[test]
fn a_gap_aborts_the_scan_and_the_state_is_unknown_until_the_next_reset() {
    let config = config();
    let tdi = pattern(38, 37, 11);
    let tdo = pattern(38, 91, 5);
    let mut waveform = waveform(&config);
    waveform.reset();
    waveform.clock(false, true);
    waveform.scan(Register::Dr, &tdi, &tdo, 300);
    waveform.reset();
    waveform.clock(false, true);
    waveform.scan(Register::Ir, &[0b1001], &[0b0101], 4);
    waveform.idle(4);
    let block = waveform.finish();
    // The DR scan begins on clock 8 and shifts on clocks 9 to 308; lose clocks 101 to 119.
    let gap = TickRange::new(Tick(rise(100) + 1), Tick(rise(120)));
    let lines = decode_with_gap(&config, &block, gap);
    let expected = vec![
        on("reset tms", 4),
        on("state test-logic-reset", 4),
        on("state run-test-idle", 5),
        on("state select-dr-scan", 6),
        on("state capture-dr", 7),
        on("state shift-dr", 8),
        on("scan-begin s1 dr", 8),
        batch(1, (Some(&tdi), Some(&tdo)), 0, 92, 9, gap.start.get()),
        format!("scan-end s1 92 aborted @{}", gap.start),
        format!("gap {gap}"),
        // Exit1 and Update with TMS high, Run-Test/Idle with TMS low, then the five clocks of
        // the reset, 311 to 315.
        on("reset tms", 315),
        on("state test-logic-reset", 315),
        on("state run-test-idle", 316),
        on("state select-dr-scan", 317),
        on("state select-ir-scan", 318),
        on("state capture-ir", 319),
        on("state shift-ir", 320),
        on("scan-begin s2 ir", 320),
        batch(2, (Some(&[0b1001]), Some(&[0b0101])), 0, 4, 321, rise(324)),
        on("state exit1-ir", 324),
        on("scan-end s2 4 complete", 325),
        on("state update-ir", 325),
        on("state run-test-idle", 326),
        format!("eof {}", block.end()),
    ];
    assert_eq!(lines, expected);
}

#[test]
fn an_assumed_state_holds_again_after_a_gap() {
    let config = JtagConfig {
        initial_state: InitialState::RunTestIdle,
        ..config()
    };
    // The device leaves Test-Logic-Reset with TMS low, which keeps an assumed Run-Test/Idle
    // where it is; the host returns there after every scan.
    let mut waveform = waveform(&config);
    waveform.clock(false, true);
    waveform.scan(Register::Dr, &[0x11], &[0x22], 8);
    let idle_from = waveform.position();
    waveform.idle(40);
    let resume = waveform.position();
    waveform.scan(Register::Dr, &[0x33], &[0x44], 8);
    waveform.idle(4);
    let block = waveform.finish();
    let gap = TickRange::new(idle_from.offset(10), idle_from.offset(30));
    let lines = decode_with_gap(&config, &block, gap);
    let gap_line = lines
        .iter()
        .position(|line| line.starts_with("gap "))
        .expect("the gap");
    assert_eq!(
        lines[gap_line + 1],
        one("state run-test-idle", gap.end.get())
    );
    let second = resume.get() + HALF;
    assert_eq!(lines[gap_line + 2], one("state select-dr-scan", second));
    assert!(
        lines[gap_line..]
            .iter()
            .any(|line| line.starts_with("shift s2 +0 8 33/44 ")),
        "{lines:?}"
    );
    assert!(
        lines[gap_line..]
            .iter()
            .any(|line| line.starts_with("scan-end s2 8 complete ")),
        "{lines:?}"
    );
}

#[test]
fn batches_show_up_before_the_scan_ends() {
    let config = config();
    let tdi = pattern(88, 13, 7);
    let tdo = pattern(88, 51, 9);
    let mut waveform = waveform(&config);
    waveform.reset();
    waveform.clock(false, true);
    waveform.scan(Register::Dr, &tdi, &tdo, 700);
    waveform.idle(4);
    let block = waveform.finish();
    // Feed up to the middle of bit 600 (clock 609).
    let cut = Tick(rise(609));
    let head = block
        .slice(TickRange::new(Tick::ZERO, cut))
        .expect("a slice");
    let mut harness = harness(config, GraphConfig::default());
    harness.feed(&head, &[61]);
    harness.collect();
    let lines = describe(&harness.without_progress());
    let lines_of = (Some(&tdi[..]), Some(&tdo[..]));
    assert_eq!(
        lines[lines.len() - 2..],
        [
            batch(1, lines_of, 0, 256, 9, rise(264)),
            batch(1, lines_of, 256, 512, 9, rise(520)),
        ]
    );
    let tail = block
        .slice(TickRange::new(cut, block.end()))
        .expect("a slice");
    harness.feed(&tail, &[61]);
    harness.end();
    let lines = describe(&harness.without_progress());
    assert!(
        lines.contains(&batch(1, lines_of, 512, 700, 9, rise(708))),
        "{lines:?}"
    );
    assert!(
        lines.contains(&on("scan-end s1 700 complete", 709)),
        "{lines:?}"
    );
}

#[test]
fn progress_is_held_back_to_the_batch_under_way() {
    let config = config();
    let mut waveform = waveform(&config);
    waveform.reset();
    waveform.clock(false, true);
    waveform.scan(Register::Dr, &pattern(88, 13, 7), &pattern(88, 51, 9), 700);
    waveform.idle(4);
    let block = waveform.finish();
    let mut harness = harness(config, GraphConfig::default());
    let mut from = Tick::ZERO;
    // Bit n of the scan rises with clock 9 + n; the first batch is full with bit 255, and the
    // second starts with the next.
    let after = |n: u64| Tick(rise(n) + 1);
    for (cut, held) in [
        (after(109), Tick(rise(9))),
        (after(264), after(264)),
        (after(300), Tick(rise(265))),
    ] {
        let piece = block.slice(TickRange::new(from, cut)).expect("a slice");
        harness.feed(&piece, &[u64::MAX]);
        harness.collect();
        assert_eq!(
            harness.items.last(),
            Some(&Item::Progress(held)),
            "up to {cut}"
        );
        from = cut;
    }
    harness.assert_progress_promise();
}

/// Resets, scans of many lengths on both registers, a paused scan, TRST and idle time.
fn rich_waveform(config: &JtagConfig) -> LogicBlock {
    let mut waveform = waveform(config);
    for n in 0..6u32 {
        let tdi = pattern(40, 17 + n as usize, n as usize);
        let tdo = pattern(40, 23, 3 * n as usize);
        waveform.reset();
        waveform.clock(false, true);
        waveform.scan(Register::Ir, &tdi, &tdo, 3 + n);
        waveform.scan(
            Register::Dr,
            &tdi,
            &tdo,
            [1, 8, 35, 257, 300, 64][n as usize],
        );
        waveform.idle(u64::from(n) * 3 + 1);
    }
    // A scan paused twice.
    waveform.load_tdo(&[0x5a; 8], 60);
    waveform.go_to(TapState::ShiftDr);
    waveform.shift(&[0xa5; 8], 20, true);
    waveform.go_to(TapState::PauseDr);
    waveform.clocks(&[false, false]);
    waveform.go_to(TapState::ShiftDr);
    waveform.shift(&[0x0f; 8], 30, true);
    waveform.go_to(TapState::PauseDr);
    waveform.go_to(TapState::ShiftDr);
    waveform.shift(&[0xf0; 8], 10, true);
    waveform.go_to(TapState::RunTestIdle);
    // TRST in the middle of a scan, then a reset by TMS.
    waveform.go_to(TapState::ShiftIr);
    waveform.shift(&[0xff], 5, false);
    waveform.assert_trst();
    waveform.clocks(&[true, false]);
    waveform.release_trst();
    waveform.clock(false, true);
    waveform.scan(Register::Dr, &[0x81; 4], &[0x18; 4], 32);
    waveform.reset();
    waveform.idle(5);
    waveform.finish()
}

#[test]
fn every_cut_and_pause_yields_the_same_events() {
    for initial_state in [InitialState::Unknown, InitialState::RunTestIdle] {
        let config = JtagConfig {
            initial_state,
            ..config()
        };
        let block = rich_waveform(&config);
        let len = block.len();
        // Gaps inside two long DR scans, and one through the final reset.
        let gaps = [
            TickRange::new(Tick(len / 5), Tick(len / 5 + 37)),
            TickRange::new(Tick(len / 2), Tick(len / 2 + 3)),
            TickRange::new(Tick(len - 30), Tick(len - 12)),
        ];
        let reference = {
            let mut harness = harness(config.clone(), GraphConfig::default());
            harness.feed_with_gaps(&block, &gaps, &[u64::MAX]);
            harness.end();
            harness.without_progress()
        };
        let lines = describe(&reference);
        assert!(lines.len() > 150, "{lines:?}");
        for what in [
            "reset trst",
            "scan-end s13 60 complete",
            "scan-end s8 56 aborted",
        ] {
            assert!(
                lines.iter().any(|line| line.starts_with(what)),
                "{what}: {lines:?}"
            );
        }
        assert_block_invariant(|graph| harness(config.clone(), graph), &block, &gaps);
    }
}

/// What the test reader makes of a JTAG stream.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Seen {
    State(TapState),
    Reset(ResetBy),
    /// A scan: its register, the bits read, the first 64 of them on each line, and how the
    /// reading ended.
    Scan {
        register: Register,
        bits: u32,
        tdi: u64,
        tdo: u64,
        end: String,
    },
}

impl EventSchema for Seen {
    const SCHEMA: SchemaId = SchemaId::new("test.jtag-reader", 1);
}

/// A decoder built on JTAG: it reads each scan through the cursor, at most `limit` bits of it,
/// and leaves the rest to [`JtagBus::finish`].
struct Reader {
    limit: Option<u32>,
}

impl Node for Reader {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "jtag-reader".into(),
            inputs: vec![InputSpec::new::<JtagEvent>(
                "bus",
                Requires::new().equals("tdi", true).equals("tdo", true),
            )],
            outputs: vec![OutputSpec::new::<Seen>("seen", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let out = io.output(0);
        Box::pin(read(
            JtagBus::new(io.input(0), out.progress_sink()),
            out,
            self.limit,
        ))
    }
}

async fn read(mut bus: JtagBus, mut out: Outbox<Seen>, limit: Option<u32>) {
    loop {
        let seen = match bus.next().await {
            Next::Scan(mut scan) => {
                let register = scan.register();
                let (mut bits, mut tdi, mut tdo) = (0u32, 0u64, 0u64);
                let end = loop {
                    if limit.is_some_and(|limit| bits >= limit) {
                        break "stopped".to_owned();
                    }
                    match scan.bit_or_end().await {
                        Ok(Read::Bit(bit)) => {
                            if bits < 64 {
                                tdi |= u64::from(bit.tdi().expect("TDI is connected")) << bits;
                                tdo |= u64::from(bit.tdo().expect("TDO is connected")) << bits;
                            }
                            bits += 1;
                        }
                        Ok(Read::End) => break "complete".to_owned(),
                        Err(error) => break format!("{error:?}"),
                    }
                };
                Seen::Scan {
                    register,
                    bits,
                    tdi,
                    tdo,
                    end,
                }
            }
            Next::State(state) => Seen::State(state),
            Next::Reset(by) => Seen::Reset(by),
            Next::Gap(range) => {
                out.gap(range).await;
                continue;
            }
            Next::Progress(tick) => {
                out.progress(tick).await;
                continue;
            }
            Next::End(tick) => {
                out.end(tick).await;
                return;
            }
        };
        let seen = match (bus.finish().await, seen) {
            (
                Err(error),
                Seen::Scan {
                    register,
                    bits,
                    tdi,
                    tdo,
                    end,
                },
            ) => Seen::Scan {
                register,
                bits,
                tdi,
                tdo,
                end: format!("{end}, then {error:?}"),
            },
            (_, seen) => seen,
        };
        out.emit(bus.consumed(), seen).await;
    }
}

fn reader_harness(config: JtagConfig, limit: Option<u32>, graph: GraphConfig) -> Harness<Seen> {
    Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        graph,
        |graph, source| {
            let jtag = graph.add(Jtag::new(config).expect("a valid configuration"));
            graph.connect(source.samples, Jtag::input(jtag))?;
            let reader = graph.add(Reader { limit });
            graph.connect(Jtag::events(jtag), InputRef::of_node(reader, 0))?;
            Ok(OutputRef::of_node(reader, 0))
        },
    )
    .expect("builds")
}

fn low_bits(bytes: &[u8], bits: u32) -> u64 {
    (0..bits.min(64)).fold(0, |value, i| {
        value | u64::from(bytes[i as usize / 8] >> (i % 8) & 1) << i
    })
}

/// A reset, then scans of 35, 4 and 300 bits.
fn scans_waveform(tdi: &[u8], tdo: &[u8]) -> LogicBlock {
    let mut waveform = waveform(&config());
    waveform.reset();
    waveform.clock(false, true);
    waveform.scan(Register::Dr, tdi, tdo, 35);
    waveform.scan(Register::Ir, tdi, tdo, 4);
    waveform.scan(Register::Dr, tdi, tdo, 300);
    waveform.idle(4);
    waveform.finish()
}

#[test]
fn a_decoder_built_on_jtag_reads_scans_bit_by_bit() {
    let tdi = pattern(38, 37, 11);
    let tdo = pattern(38, 91, 5);
    let block = scans_waveform(&tdi, &tdo);
    let scan = |register, bits, end: &str| Seen::Scan {
        register,
        bits,
        tdi: low_bits(&tdi, bits),
        tdo: low_bits(&tdo, bits),
        end: end.to_owned(),
    };
    let to_shift = |register| match register {
        Register::Dr => vec![
            Seen::State(TapState::SelectDrScan),
            Seen::State(TapState::CaptureDr),
            Seen::State(TapState::ShiftDr),
        ],
        Register::Ir => vec![
            Seen::State(TapState::SelectDrScan),
            Seen::State(TapState::SelectIrScan),
            Seen::State(TapState::CaptureIr),
            Seen::State(TapState::ShiftIr),
        ],
    };
    let after = |update| vec![Seen::State(update), Seen::State(TapState::RunTestIdle)];
    let mut expected = vec![
        Seen::Reset(ResetBy::Tms),
        Seen::State(TapState::TestLogicReset),
        Seen::State(TapState::RunTestIdle),
    ];
    expected.extend(to_shift(Register::Dr));
    expected.push(scan(Register::Dr, 35, "complete"));
    expected.extend(after(TapState::UpdateDr));
    expected.extend(to_shift(Register::Ir));
    expected.push(scan(Register::Ir, 4, "complete"));
    expected.extend(after(TapState::UpdateIr));
    expected.extend(to_shift(Register::Dr));
    expected.push(scan(Register::Dr, 300, "complete"));
    expected.extend(after(TapState::UpdateDr));

    let mut harness = reader_harness(config(), None, GraphConfig::default());
    harness.run_all(&block);
    let events = harness.events();
    let seen: Vec<Seen> = events.iter().map(|event| event.payload.clone()).collect();
    assert_eq!(seen, expected);
    // The first scan covers its beginning through its end: clocks 8 to 44.
    let first = events
        .iter()
        .find(|event| matches!(event.payload, Seen::Scan { .. }))
        .expect("a scan");
    assert_eq!(
        (first.span, first.settled),
        (
            TickRange::new(Tick(rise(8)), Tick(rise(44) + 1)),
            Tick(rise(44))
        )
    );

    // A reader that stops after four bits leaves the rest of each scan to `finish`.
    let mut harness = reader_harness(config(), Some(4), GraphConfig::default());
    harness.run_all(&block);
    let seen: Vec<Seen> = harness
        .events()
        .into_iter()
        .map(|event| event.payload)
        .collect();
    let stopped: Vec<Seen> = expected
        .iter()
        .map(|seen| match seen {
            Seen::Scan { register, .. } => scan(*register, 4, "stopped"),
            other => other.clone(),
        })
        .collect();
    assert_eq!(seen, stopped);
}

#[test]
fn a_reader_learns_how_a_scan_was_cut_off() {
    let tdi = pattern(38, 37, 11);
    let tdo = pattern(38, 91, 5);
    let block = scans_waveform(&tdi, &tdo);
    // The 300-bit scan begins on clock 58 and shifts from clock 59; lose clocks 101 to 109.
    // The JTAG decoder aborts the scan before it passes the gap on.
    let gap = TickRange::new(Tick(rise(100) + 1), Tick(rise(110)));
    for limit in [None, Some(200)] {
        let mut harness = reader_harness(config(), limit, GraphConfig::default());
        harness.feed_with_gaps(&block, &[gap], &[53]);
        harness.end();
        let items = harness.without_progress();
        let cut = items
            .iter()
            .filter_map(Item::event)
            .filter_map(|event| match &event.payload {
                Seen::Scan { bits: 42, end, .. } => Some(end.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(cut, vec!["Aborted".to_owned()], "limit {limit:?}");
        assert!(
            items.contains(&Item::Gap(gap)),
            "the gap is passed on: {items:?}"
        );
    }
}

#[test]
fn a_reader_requires_the_lines_it_reads() {
    let error = Harness::build(
        CHANNELS,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let config = JtagConfig {
                tdo: None,
                ..config()
            };
            let jtag = graph.add(Jtag::new(config).expect("a valid configuration"));
            graph.connect(source.samples, Jtag::input(jtag))?;
            let reader = graph.add(Reader { limit: None });
            graph.connect(Jtag::events(jtag), InputRef::of_node(reader, 0))?;
            Ok(OutputRef::<Seen>::of_node(reader, 0))
        },
    )
    .expect_err("rejected");
    assert_eq!(
        error,
        GraphError::Requirement {
            from: "jtag#0.events".into(),
            to: "jtag-reader#1.bus".into(),
            key: "tdo",
            requirement: "= true".into(),
            found: "false".into(),
        }
    );
}

#[test]
fn the_reader_sees_the_same_whatever_the_blocks() {
    let config = config();
    let block = rich_waveform(&config);
    let len = block.len();
    let gaps = [
        TickRange::new(Tick(len / 5), Tick(len / 5 + 37)),
        TickRange::new(Tick(len * 2 / 3), Tick(len * 2 / 3 + 5)),
    ];
    for limit in [None, Some(3)] {
        assert_block_invariant(
            |graph| reader_harness(config.clone(), limit, graph),
            &block,
            &gaps,
        );
    }
}
