// SPDX-License-Identifier: GPL-3.0-or-later
//! The sample-by-sample oracle is independent of the production bit transposes.

use super::*;
use crate::assemble::{Packed, WireDecoder};
use crate::{SLOGIC32U3, UsbSpeed};

fn settings(enabled: u32, limit: u64) -> Settings {
    let mut settings = Settings::defaults(&SLOGIC32U3, UsbSpeed::SuperPlus);
    settings.enabled = enabled;
    settings.limit_samples = limit;
    settings
}

fn wire<F: WireFormat>(samples: &[u32]) -> Vec<u8> {
    let mut bytes = vec![0; (samples.len() * F::BITS).div_ceil(8)];
    for (tick, &sample) in samples.iter().enumerate() {
        for bit in 0..F::BITS {
            let offset = tick * F::BITS + bit;
            bytes[offset / 8] |= (((sample >> bit) & 1) as u8) << (offset % 8);
        }
    }
    bytes
}

fn logic(packets: &[Packet]) -> Vec<u8> {
    packets
        .iter()
        .flat_map(|packet| match packet {
            Packet::Logic(bytes) => bytes.as_slice(),
            _ => &[],
        })
        .copied()
        .collect()
}

fn analog(packets: &[Packet]) -> Vec<u16> {
    packets
        .iter()
        .flat_map(|packet| match packet {
            Packet::Analog(codes) => codes.as_slice(),
            _ => &[],
        })
        .copied()
        .collect()
}

fn expected(samples: &[u32], channels: &[u16]) -> Vec<u8> {
    let mut out = Vec::new();
    for block in samples.as_chunks::<64>().0 {
        for &channel in channels {
            let word = block.iter().enumerate().fold(0u64, |word, (tick, sample)| {
                word | (u64::from((sample >> channel) & 1) << tick)
            });
            out.extend_from_slice(&word.to_le_bytes());
        }
    }
    out
}

fn run<F: WireFormat>(
    settings: &Settings,
    plan: &CapturePlan,
    samples: &[u32],
    cut: usize,
    head: usize,
) -> Vec<Packet> {
    let mut bytes = vec![255; head];
    bytes.extend(wire::<F>(samples));
    let mut decoder = WireDecoder::<F>::new(head);
    let mut stream = Stream::<F>::new(settings, plan);
    let mut packets = Vec::new();
    for part in bytes.chunks(cut) {
        decoder.push(part, |view| stream.push(view, &mut packets));
        decoder.push(&[], |view| stream.push(view, &mut packets));
    }
    assert!(stream.done() || plan.looping);
    packets
}

fn check_format<F: WireFormat>() {
    let mask = u32::MAX >> (32 - F::BITS);
    let mut seed = 0x1234_5678u32;
    let samples: Vec<_> = (0..1024)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed & mask
        })
        .collect();
    for enabled in [mask, mask & 0xaaaa_aaaa, 1 << (F::BITS - 1)] {
        let channels: Vec<_> = (0..32).filter(|c| enabled & (1 << c) != 0).collect();
        for head in [0, 4] {
            // Includes partial words, partial blocks, and the direct whole-block path.
            for cut in [1, 2, 3, 7, 31, 32, 63, 64, 65, 127, 256, 4096] {
                let packets = run::<F>(
                    &settings(enabled, 960),
                    &CapturePlan::default(),
                    &samples,
                    cut,
                    head,
                );
                assert_eq!(
                    logic(&packets),
                    expected(&samples[..960], &channels),
                    "{} bits, mask {enabled:x}, cut {cut}, head {head}",
                    F::BITS
                );
            }
        }
    }
}

#[test]
fn all_widths_match_the_scalar_oracle_across_usb_boundaries() {
    check_format::<Packed<2>>();
    check_format::<Packed<4>>();
    check_format::<u8>();
    check_format::<u16>();
    check_format::<u32>();
}

fn check_trigger<F: WireFormat>() {
    let high = 1 << (F::BITS - 1);
    for position in [0, 1, 25, 90] {
        let pre = 128 * position / 100;
        let plan = CapturePlan {
            triggers: vec![
                (0, TriggerCondition::Rise),
                ((F::BITS - 1) as u16, TriggerCondition::High),
            ],
            trigger_position: position as u8,
            ..Default::default()
        };
        // Visit every packed-sample offset and both sides of a 64-sample boundary.
        for edge in [128, 129, 130, 131, 173, 191, 192, 193] {
            let samples: Vec<_> = (0..384)
                .map(|tick| high | u32::from(tick >= edge))
                .collect();
            for cut in [1, 3, 17, 64, 257, 2048] {
                let packets = run::<F>(&settings(high | 1, 128), &plan, &samples, cut, 4);
                assert_eq!(
                    packets
                        .iter()
                        .filter(|p| matches!(p, Packet::Trigger { .. }))
                        .count(),
                    1
                );
                assert!(packets.contains(&Packet::Trigger { sample: pre as u64 }));
                assert_eq!(
                    logic(&packets),
                    expected(
                        &samples[edge - pre..edge - pre + 128],
                        &[0, (F::BITS - 1) as u16]
                    ),
                    "{} bits, pre {pre}, edge {edge}, cut {cut}",
                    F::BITS
                );
            }
        }
    }
}

#[test]
fn triggering_preserves_unaligned_history_and_sample_positions_for_every_width() {
    check_trigger::<Packed<2>>();
    check_trigger::<Packed<4>>();
    check_trigger::<u8>();
    check_trigger::<u16>();
    check_trigger::<u32>();
}

fn check_history<F: WireFormat>() {
    let mut history = History::<F>::default();
    let mask = u32::MAX >> (32 - F::BITS);
    for tick in 0..1000 {
        if history.len == 115 {
            history.pop();
        }
        history.push(F::Word::from_u32(tick & mask));
        // A packed byte may straddle the head, but no sample expands to u32.
        assert!(
            history.words.len() * size_of::<F::Word>()
                <= (history.len * F::BITS).div_ceil(8) + size_of::<F::Word>()
        );
    }
    for tick in 885..1000 {
        assert_eq!(history.pop().map(Into::<u32>::into), Some(tick & mask));
    }
    assert!(history.pop().is_none());
}

#[test]
fn history_uses_the_wire_width_instead_of_expanding_each_sample() {
    check_history::<Packed<2>>();
    check_history::<Packed<4>>();
    check_history::<u8>();
    check_history::<u16>();
    check_history::<u32>();
}

#[test]
fn mixed_analog_and_logic_stay_aligned_through_a_trigger() {
    let samples: Vec<_> = (0u32..512)
        .map(|n| {
            u32::from((n as u8).reverse_bits())
                | u32::from((255 - n as u8).reverse_bits()) << 16
                | u32::from(n >= 173) << 9
        })
        .collect();
    let mut settings = settings(0xff_ffff, 128);
    settings.analog = 0b101;
    let plan = CapturePlan {
        triggers: vec![(9, TriggerCondition::Rise)],
        trigger_position: 1,
        ..Default::default()
    };
    for cut in [1, 3, 63, 64, 100, 2048] {
        let packets = run::<u32>(&settings, &plan, &samples, cut, 0);
        assert!(packets.contains(&Packet::Trigger { sample: 1 }));
        assert_eq!(
            logic(&packets),
            expected(&samples[172..300], &(8..16).collect::<Vec<_>>())
        );
        let codes: Vec<_> = (172u16..300)
            .flat_map(|n| [n % 256, 255 - n % 256])
            .collect();
        assert_eq!(analog(&packets), codes);
    }
}

#[test]
fn analog_only_and_instant_looping_keep_their_existing_behavior() {
    for enabled in [0, 0xff00] {
        let mut config = settings(enabled, 64);
        config.analog = 1;
        let samples: Vec<_> = (0u32..192)
            .map(|n| u32::from((n as u8).reverse_bits()) | (n & 3) << 8)
            .collect();
        let plan = CapturePlan {
            instant: true,
            looping: true,
            triggers: vec![(8, TriggerCondition::Rise)],
            ..Default::default()
        };
        for cut in [1, 3, 64, 384] {
            let packets = run::<u16>(&config, &plan, &samples, cut, 0);
            assert_eq!(analog(&packets), (0..192).collect::<Vec<u16>>());
            assert!(!packets.iter().any(|p| matches!(p, Packet::Trigger { .. })));
            assert_eq!(
                logic(&packets),
                expected(
                    &samples,
                    &if enabled == 0 {
                        vec![]
                    } else {
                        (8..16).collect()
                    }
                )
            );
        }
    }
}

#[test]
fn history_limit_and_initial_edge_semantics_are_unchanged() {
    let plan = CapturePlan {
        triggers: vec![(0, TriggerCondition::High)],
        trigger_position: 90,
        ..Default::default()
    };
    let stream = Stream::<Packed<4>>::new(&settings(1, 1 << 34), &plan);
    assert_eq!(stream.pre_limit, MAX_PRE_SAMPLES as usize);
    assert_eq!(stream.pre.words.capacity(), 0);
    let packets = run::<Packed<4>>(&settings(1, 128), &plan, &[1; 256], 1, 0);
    assert!(packets.contains(&Packet::Trigger { sample: 115 }));
    for condition in [
        TriggerCondition::Rise,
        TriggerCondition::Fall,
        TriggerCondition::Both,
    ] {
        let trigger = Trigger::new(&[(0, condition)], 1);
        assert!(!trigger.matches(None, 0));
        assert!(!trigger.matches(None, 1));
    }
    let trigger = Trigger::new(
        &[(0, TriggerCondition::Fall), (1, TriggerCondition::Low)],
        3,
    );
    assert!(trigger.matches(Some(1), 0));
    assert!(!trigger.matches(Some(1), 2));
    assert!(!trigger.matches(Some(0), 0));
    assert!(Trigger::new(&[(31, TriggerCondition::High)], 0xffff).empty());
    assert!(Trigger::new(&[(32, TriggerCondition::High)], u32::MAX).empty());
}

#[test]
fn large_transfers_keep_packets_bounded_and_stop_at_the_exact_sample_limit() {
    let input = vec![255; MAX_PACKET * 2];
    let count = (MAX_PACKET as u64 * 2) + 64;
    let mut stream = Stream::<Packed<4>>::new(&settings(15, count), &CapturePlan::default());
    let mut decoder = WireDecoder::<Packed<4>>::new(0);
    let mut packets = Vec::new();
    decoder.push(&input, |view| stream.push(view, &mut packets));
    assert!(stream.done());
    let mut bytes = 0;
    for packet in &packets {
        if let Packet::Logic(data) = packet {
            assert!(data.len() <= MAX_PACKET);
            assert!(data.iter().all(|&byte| byte == 255));
            bytes += data.len();
        }
    }
    assert_eq!(bytes as u64 * 2, count);
    packets.clear();
    decoder.push(&input, |view| stream.push(view, &mut packets));
    assert!(packets.is_empty());
}
