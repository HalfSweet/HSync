// SPDX-License-Identifier: Apache-2.0
//! Independently specified OScan1, SW-DP posting and AVR signature vectors.
mod dsl_spi_support;
use serde_json::json;
use uiharu_core::testing::{Harness, assert_block_invariant};
use uiharu_core::{
    GraphConfig, InputRef, Levels, LogicBlock, LogicBlockBuilder, Node, OutputRef, Tick, TickRange,
    TimeBase,
};
use uiharu_dsl_protocols::{avr_isp_device, cjtag, swd_dp};
use uiharu_interp::{DynEvent, Interpreter, ToDyn};
use uiharu_protocols::avr_isp::{AvrIsp, IspEvent};
use uiharu_protocols::jtag::{EndReason, JtagEvent};
use uiharu_protocols::spi::{Spi, SpiConfig, synth::SpiWaveform};
use uiharu_protocols::swd::{
    Ack, Swd, SwdConfig, SwdEvent,
    synth::{SwdPacket, SwdWaveform},
};

struct Compact {
    b: LogicBlockBuilder,
    level: u64,
}
impl Compact {
    fn new() -> Self {
        let mut w = Self {
            b: LogicBlockBuilder::new(Tick::ZERO, 8),
            level: 2,
        };
        w.hold();
        w
    }
    fn hold(&mut self) {
        self.b.push_repeated(Levels(self.level), 2);
    }
    fn bit(&mut self, bit: bool) {
        self.level = if bit { 2 } else { 0 };
        self.hold();
        self.level |= 1;
        self.hold();
    }
    fn raw(&mut self, bits: &[bool]) {
        for &bit in bits {
            self.bit(bit);
        }
    }
    fn value(&mut self, value: u64, bits: u32) {
        for n in 0..bits {
            self.bit((value >> n) & 1 != 0);
        }
    }
    fn escape(&mut self, changes: u8) {
        self.level |= 1;
        self.hold();
        for _ in 0..changes {
            self.level ^= 2;
            self.hold();
        }
        self.level &= !1;
        self.hold();
    }
    fn oscan(&mut self, tms: bool, tdi: bool, tdo: bool) {
        self.raw(&[!tdi, tms, tdo]);
    }
    fn tap(&mut self, bits: &[bool]) {
        for &tms in bits {
            self.oscan(tms, false, true);
        }
    }
    fn ir(&mut self, input: u8, output: u8) {
        self.tap(&[true, true, false, false]);
        for n in 0..4 {
            self.oscan(n == 3, (input >> n) & 1 != 0, (output >> n) & 1 != 0);
        }
        self.tap(&[true, false]);
    }
    fn short(&mut self) {
        self.raw(&[true, true, true, true, true, false]);
        self.escape(7);
        self.value(0x08c, 12);
    }
    fn zero(&mut self) {
        self.raw(&[true, true, true, false, true, false]);
    }
    fn paused_scan(&mut self, bits: usize) {
        self.raw(&[true, false]);
        for n in 0..bits {
            self.bit(n + 1 == bits);
        }
        self.bit(false);
    }
    fn long(&mut self) {
        self.raw(&[true, true, true, true, true, false]);
        self.escape(7);
        self.value(0, 36);
        // TI SWRU319C §6.3: two zero-bit scans, one-bit lock, STFMT3/9.
        self.raw(&[true, false, true, false]);
        self.zero();
        self.zero();
        self.paused_scan(1);
        self.paused_scan(3);
        self.zero();
        self.paused_scan(9);
        self.zero();
        self.value(0, 4);
        // Close command window through a benign IR scan; finish at Idle.
        self.tap(&[true, true, true, true, false, false]);
        for n in 0..4 {
            self.oscan(n == 3, true, true);
        }
        self.tap(&[true, false]);
    }
    fn finish(mut self) -> LogicBlock {
        self.hold();
        self.b.finish().expect("waveform")
    }
}
fn compact(config: cjtag::CjtagConfig, g: GraphConfig) -> Harness<JtagEvent> {
    Harness::build(8, TimeBase::new(1_000_000), g, |graph, source| {
        let node = graph.add(cjtag::Cjtag::new(config).expect("cJTAG"));
        graph.connect(source.samples, cjtag::Cjtag::input(node))?;
        Ok(cjtag::Cjtag::events(node))
    })
    .expect("cJTAG graph")
}
fn compact_wave(long: bool) -> LogicBlock {
    let mut w = Compact::new();
    if long {
        w.long();
    } else {
        w.short();
    }
    w.ir(10, 1);
    w.finish()
}
fn shift_value(events: &[JtagEvent], last: bool) -> u32 {
    let scan = events
        .iter()
        .filter_map(|e| {
            if let JtagEvent::ScanEnd {
                scan,
                bits: 4,
                reason: EndReason::Complete,
            } = e
            {
                Some(*scan)
            } else {
                None
            }
        })
        .next_back()
        .expect("complete IR");
    events
        .iter()
        .filter_map(|e| {
            if let JtagEvent::Shift {
                scan: s,
                offset,
                tdi,
                tdo,
                ..
            } = e
            {
                if *s == scan {
                    Some(
                        u32::from(if last { tdo } else { tdi }.as_ref().expect("line")[0])
                            << offset,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        })
        .fold(0, |a, b| a | b)
}
#[test]
fn oscan1_short_and_standard_activation_emit_the_shared_jtag_contract() {
    for long in [false, true] {
        let block = compact_wave(long);
        let config = cjtag::CjtagConfig::default();
        let mut h = compact(config, GraphConfig::default());
        h.run_all(&block);
        let events: Vec<_> = h.events().into_iter().map(|e| e.payload).collect();
        assert_eq!(shift_value(&events, false), 10, "long={long}");
        assert_eq!(shift_value(&events, true), 1, "long={long}");
        dsl_spi_support::logic_parity(
            "cjtag",
            json!(config),
            GraphConfig::default(),
            &block,
            &[],
            &[1, 7, 3],
            &h.items,
        );
        assert_block_invariant(|g| compact(config, g), &block, &[]);
    }
}
#[test]
fn compact_escape_and_capture_loss_abort_scans_and_require_reactivation() {
    let mut w = Compact::new();
    w.short();
    w.tap(&[true, false, false]);
    w.oscan(false, true, false);
    let cut = w.b.next_tick();
    w.escape(8);
    w.short();
    w.ir(3, 1);
    let block = w.finish();
    let gaps = [TickRange::new(cut, Tick(cut.0 + 2))];
    let config = cjtag::CjtagConfig::default();
    let mut h = compact(config, GraphConfig::default());
    h.feed_with_gaps(&block, &gaps, &[3, 1]);
    h.end();
    assert!(h.events().iter().any(|e| matches!(
        e.payload,
        JtagEvent::ScanEnd {
            reason: EndReason::Aborted,
            ..
        }
    )));
    assert_eq!(
        shift_value(
            &h.events()
                .into_iter()
                .map(|e| e.payload)
                .collect::<Vec<_>>(),
            false
        ),
        3
    );
    assert_block_invariant(|g| compact(config, g), &block, &gaps);
}
fn swd_harness(g: GraphConfig) -> Harness<swd_dp::Event> {
    Harness::build(8, TimeBase::new(1_000_000), g, |graph, source| {
        let bus = graph.add(Swd::new(SwdConfig::default()).expect("SWD"));
        graph.connect(source.samples, Swd::input(bus))?;
        let dp = graph.add(swd_dp::SwdDp::new(swd_dp::SwdDpConfig::default()).expect("DP"));
        graph.connect(Swd::events(bus), swd_dp::SwdDp::input(dp))?;
        Ok(swd_dp::SwdDp::events(dp))
    })
    .expect("SWD graph")
}
fn swd_wave() -> LogicBlock {
    let mut w = SwdWaveform::new(SwdConfig::default(), 8, 2, Tick::ZERO);
    w.idle(8);
    w.connect();
    for p in [
        SwdPacket::write(false, 8, 0),
        SwdPacket::write(true, 0, 0x12),
        SwdPacket::write(true, 4, 0x200003fc),
        SwdPacket::read(true, 12, 0xdeadbeef), // first return is unknown; issue read1
        SwdPacket::read(true, 12, 0).with_ack(Ack::Wait), // keep read1 and TAR
        SwdPacket::read(true, 12, 0x11223344), // completes read1; issue read2
        SwdPacket::read(false, 8, 0x11223344), // RESEND repeats read1, does not consume read2
        SwdPacket::read(false, 12, 0x55667788), // RDBUFF completes read2
        SwdPacket::write(true, 12, 0xa5).with_ack(Ack::Fault),
        SwdPacket::write(false, 8, 0),
        SwdPacket::write(true, 12, 0x99), // lost CSW/TAR
    ] {
        w.packet(&p);
        w.idle_cycles(2);
    }
    w.finish()
}
#[test]
fn swd_posted_reads_keep_waits_separate_from_results_resend_and_fault_context() {
    let block = swd_wave();
    let mut h = swd_harness(GraphConfig::default());
    h.run_all(&block);
    let events: Vec<_> = h.events().into_iter().map(|e| e.payload).collect();
    let results: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let swd_dp::Event::MemoryResult {
                planned_address,
                raw_data,
                ..
            } = e
            {
                Some((*planned_address, *raw_data))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        results,
        vec![
            (Some(0x200003fc), 0x11223344),
            (Some(0x20000000), 0x55667788)
        ]
    );
    assert!(events.iter().any(|e| matches!(
        e,
        swd_dp::Event::ResendResult {
            original_tx: Some(_),
            matches_previous: Some(true),
            ..
        }
    )));
    let planned: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let swd_dp::Event::MemoryRequest {
                planned_address, ..
            } = e
            {
                Some(*planned_address)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(planned, vec![Some(0x200003fc), Some(0x20000000), None]);
    let mut i = Harness::<DynEvent>::build(
        8,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let swd = Swd::new(SwdConfig::default()).expect("SWD");
            let props = swd.spec().outputs[0].props.clone();
            let swd = graph.add(swd);
            graph.connect(source.samples, Swd::input(swd))?;
            let bridge = graph.add(ToDyn::<SwdEvent>::new(props));
            graph.connect(Swd::events(swd), InputRef::of_node(bridge, 0))?;
            let dp = graph.add(
                Interpreter::new(
                    dsl_spi_support::program("swd-dp"),
                    &json!(swd_dp::SwdDpConfig::default()),
                )
                .expect("interpreted"),
            );
            graph.connect(
                OutputRef::<DynEvent>::of_node(bridge, 0),
                Interpreter::bus_input(dp),
            )?;
            Ok(Interpreter::events(dp))
        },
    )
    .expect("interpreted graph");
    i.run_all(&block);
    dsl_spi_support::assert_items(&h.items, &i.items, "swd-dp");
    assert_block_invariant(swd_harness, &block, &[]);
}
fn isp_harness(g: GraphConfig) -> Harness<avr_isp_device::Event> {
    Harness::build(8, TimeBase::new(1_000_000), g, |graph, source| {
        let spi = graph.add(Spi::new(SpiConfig::default()).expect("SPI"));
        graph.connect(source.samples, Spi::input(spi))?;
        let isp = graph.add(AvrIsp::new());
        graph.connect(Spi::events(spi), AvrIsp::input(isp))?;
        let dev = graph.add(
            avr_isp_device::AvrIspDevice::new(avr_isp_device::AvrIspDeviceConfig {})
                .expect("AVR device"),
        );
        graph.connect(
            AvrIsp::events(isp),
            avr_isp_device::AvrIspDevice::input(dev),
        )?;
        Ok(avr_isp_device::AvrIspDevice::events(dev))
    })
    .expect("AVR graph")
}
fn isp_wave() -> LogicBlock {
    let mut w = SpiWaveform::new(SpiConfig::default(), 8, 2, Tick::ZERO);
    w.idle(5);
    for signature in [[0x1e, 0x95, 0x16], [0x42, 0x34, 0x56]] {
        w.select();
        w.words([(0xac, 0), (0x53, 0), (0, 0x22), (0, 0)]);
        for (index, value) in signature.into_iter().enumerate() {
            w.words([(0x30, 0), (0, 0x30), (index as u8, 0), (0, value)]);
        }
        w.release();
        w.idle(5);
    }
    w.finish()
}
#[test]
fn avr_signature_tuple_contract_preserves_native_instructions_and_names_observed_ids() {
    let block = isp_wave();
    let mut h = isp_harness(GraphConfig::default());
    h.run_all(&block);
    assert!(h.events().iter().any(|e| matches!(
        e.payload,
        avr_isp_device::Event::Signature {
            value: 0x1e9516,
            device: avr_isp_device::Device::Atmega328pb,
            ..
        }
    )));
    assert!(h.events().iter().any(|e| matches!(
        e.payload,
        avr_isp_device::Event::Signature {
            value: 0x423456,
            device: avr_isp_device::Device::Unknown,
            vendor: avr_isp_device::Vendor::Unknown,
            ..
        }
    )));
    assert_eq!(
        h.events()
            .iter()
            .filter(|e| matches!(e.payload, avr_isp_device::Event::SyncWarning { .. }))
            .count(),
        2
    );
    let mut i = Harness::<DynEvent>::build(
        8,
        TimeBase::new(1_000_000),
        GraphConfig::default(),
        |graph, source| {
            let spi = graph.add(Spi::new(SpiConfig::default()).expect("SPI"));
            graph.connect(source.samples, Spi::input(spi))?;
            let isp = graph.add(AvrIsp::new());
            graph.connect(Spi::events(spi), AvrIsp::input(isp))?;
            let bridge = graph.add(ToDyn::<IspEvent>::new(Default::default()));
            graph.connect(AvrIsp::events(isp), InputRef::of_node(bridge, 0))?;
            let dev = graph.add(
                Interpreter::new(dsl_spi_support::program("avr-isp-device"), &json!({}))
                    .expect("interpreted"),
            );
            graph.connect(
                OutputRef::<DynEvent>::of_node(bridge, 0),
                Interpreter::bus_input(dev),
            )?;
            Ok(Interpreter::events(dev))
        },
    )
    .expect("interpreted graph");
    i.run_all(&block);
    dsl_spi_support::assert_items(&h.items, &i.items, "avr-isp-device");
    assert_block_invariant(isp_harness, &block, &[]);
}

#[test]
fn oscan0_stalls_ready_repetitions_and_fixed_delays_preserve_scan_bits() {
    for (ready, delay) in [(1, 0), (4, 2)] {
        let mut w = Compact::new();
        let mut packet = |tms: bool, tdi: bool, tdo: bool| {
            w.raw(&[!tdi, tms]);
            w.raw(&[false, false, false]);
            for _ in 0..ready {
                w.bit(true);
            }
            w.bit(tdo);
            for _ in 0..delay {
                w.bit(false);
            }
        };
        for tms in [
            true, true, true, true, true, false, true, true, false, false,
        ] {
            packet(tms, false, true);
        }
        for n in 0..4 {
            packet(n == 3, (10 >> n) & 1 != 0, (1 >> n) & 1 != 0);
        }
        packet(true, false, true);
        packet(false, false, true);
        let block = w.finish();
        let config = cjtag::CjtagConfig {
            start_aligned: true,
            format: cjtag::Format::Oscan0,
            ready_ones: ready,
            delay_cycles: delay,
            ..Default::default()
        };
        let mut h = compact(config, GraphConfig::default());
        h.run_all(&block);
        let events: Vec<_> = h.events().into_iter().map(|e| e.payload).collect();
        assert_eq!(shift_value(&events, false), 10);
        assert_eq!(shift_value(&events, true), 1);
        dsl_spi_support::logic_parity(
            "cjtag",
            json!(config),
            GraphConfig::default(),
            &block,
            &[],
            &[1, 3, 7],
            &h.items,
        );
        assert_block_invariant(|g| compact(config, g), &block, &[]);
    }
}

#[test]
fn swd_nonstrict_start_scans_without_a_reset_but_strict_default_waits() {
    let loose = SwdConfig {
        strict_start: false,
        ..Default::default()
    };
    let mut w = SwdWaveform::new(loose.clone(), 8, 2, Tick::ZERO);
    w.idle(8);
    w.idle_cycles(2);
    w.packet(&SwdPacket::read(false, 0, 0x2ba01477));
    w.idle_cycles(2);
    let block = w.finish();
    for (config, expected) in [(SwdConfig::default(), false), (loose, true)] {
        let mut h = Harness::<SwdEvent>::build(
            8,
            TimeBase::new(1_000_000),
            GraphConfig::default(),
            |graph, source| {
                let n = graph.add(Swd::new(config.clone()).expect("SWD"));
                graph.connect(source.samples, Swd::input(n))?;
                Ok(Swd::events(n))
            },
        )
        .expect("SWD graph");
        h.run_all(&block);
        assert_eq!(
            h.events().iter().any(|e| matches!(
                e.payload,
                SwdEvent::Data {
                    value: 0x2ba01477,
                    ..
                }
            )),
            expected
        );
        assert_block_invariant(
            |g| {
                Harness::build(8, TimeBase::new(1_000_000), g, |graph, source| {
                    let n = graph.add(Swd::new(config.clone()).expect("SWD"));
                    graph.connect(source.samples, Swd::input(n))?;
                    Ok(Swd::events(n))
                })
                .expect("SWD graph")
            },
            &block,
            &[],
        );
    }
}

#[test]
fn corrupt_posted_read_and_line_reset_never_leave_a_candidate_memory_result() {
    let mut w = SwdWaveform::new(SwdConfig::default(), 8, 2, Tick::ZERO);
    w.idle(8);
    w.connect();
    for p in [
        SwdPacket::write(false, 8, 0),
        SwdPacket::write(true, 0, 0x12),
        SwdPacket::write(true, 4, 0x20000000),
        SwdPacket::read(true, 12, 0),
    ] {
        w.packet(&p);
        w.idle_cycles(2);
    }
    w.packet(&SwdPacket {
        bad_data_parity: true,
        ..SwdPacket::read(true, 12, 0x12345678)
    });
    w.line_reset(56);
    w.idle_cycles(2);
    w.packet(&SwdPacket::read(false, 12, 0x55667788));
    w.idle_cycles(2);
    let block = w.finish();
    let mut h = swd_harness(GraphConfig::default());
    h.run_all(&block);
    assert!(
        !h.events()
            .iter()
            .any(|e| matches!(e.payload, swd_dp::Event::MemoryResult { .. }))
    );
    assert_block_invariant(swd_harness, &block, &[]);
}
