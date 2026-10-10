//! A demo capture in real time: waiting for the trigger like a device in stream mode, then
//! the samples from before the trigger at once and the rest at the sample rate.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use hsync_core::{ChannelIndex, TriggerCondition};
use hsync_driver::{Acquisition, EndReason, Packet};

use crate::pattern::Pattern;

/// How often the acquisition wakes up.
const TICK: Duration = Duration::from_millis(10);
/// How often progress is reported while waiting for the trigger.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// Largest data packet.
const MAX_PACKET: usize = 4 << 20;

#[derive(Debug, Clone)]
pub(crate) struct DemoParams {
    pub pattern: Pattern,
    pub channels: Vec<ChannelIndex>,
    pub samplerate: u64,
    /// Samples to capture, a multiple of 64.
    pub samples: u64,
    /// Conditions by position in `channels`.
    pub triggers: Vec<(usize, TriggerCondition)>,
    /// Samples kept before the trigger, a multiple of 64.
    pub pre_trigger: u64,
}

/// Samples of one word where `condition` holds; `previous` is the sample before the word.
fn condition_mask(word: u64, previous: bool, condition: TriggerCondition) -> u64 {
    let before = (word << 1) | u64::from(previous);
    match condition {
        TriggerCondition::None => u64::MAX,
        TriggerCondition::High => word,
        TriggerCondition::Low => !word,
        TriggerCondition::Rise => word & !before,
        TriggerCondition::Fall => !word & before,
        TriggerCondition::Both => word ^ before,
    }
}

/// Words the sample rate has produced since `since`.
fn words_due(since: Instant, samplerate: u64) -> u64 {
    let samples = since.elapsed().as_secs_f64() * samplerate as f64;
    (samples / 64.0) as u64
}

pub(crate) fn start(
    params: DemoParams,
    running: Arc<AtomicBool>,
    sink: Sender<Packet>,
) -> Box<dyn Acquisition> {
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    running.store(true, Ordering::Release);
    let _ = sink.send(Packet::Start {
        channels: params.channels.clone(),
        samplerate: params.samplerate,
    });
    let thread = std::thread::Builder::new()
        .name("demo-acquisition".into())
        .spawn(move || {
            let reason = run(params, &sink, &stopping);
            running.store(false, Ordering::Release);
            let _ = sink.send(Packet::End(reason));
        })
        .ok();
    Box::new(DemoAcquisition { stop, thread })
}

struct DemoAcquisition {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Acquisition for DemoAcquisition {
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    fn wait(mut self: Box<Self>) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for DemoAcquisition {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn run(params: DemoParams, sink: &Sender<Packet>, stop: &AtomicBool) -> EndReason {
    let channels = params.channels.len();
    let mut capture = params.pattern.clone();
    let mut words = vec![0u64; channels];
    let mut backlog = 0;

    if !params.triggers.is_empty() {
        match wait_for_trigger(&params, &mut capture, sink, stop) {
            Some(trigger) => {
                let _ = sink.send(Packet::Trigger { sample: trigger });
                backlog = trigger / 64 + 1;
            }
            None => return EndReason::Stopped,
        }
    }

    let total = params.samples / 64;
    let begin = Instant::now();
    let mut emitted = 0u64;
    loop {
        if stop.load(Ordering::Acquire) {
            return EndReason::Stopped;
        }
        let due = (backlog + words_due(begin, params.samplerate)).min(total);
        let mut bytes = Vec::new();
        while emitted < due && bytes.len() < MAX_PACKET {
            capture.next_words(&params.channels, &mut words);
            for word in &words {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
            emitted += 1;
        }
        if !bytes.is_empty() {
            let _ = sink.send(Packet::Logic(bytes));
        }
        if emitted >= total {
            return EndReason::Completed;
        }
        if emitted >= due {
            std::thread::sleep(TICK);
        }
    }
}

/// Runs the signal in real time until the trigger conditions hold past the pre-trigger
/// samples. Leaves `capture` at the first word to emit and returns the trigger's sample within
/// the capture; `None` if stopped first.
fn wait_for_trigger(
    params: &DemoParams,
    capture: &mut Pattern,
    sink: &Sender<Packet>,
    stop: &AtomicBool,
) -> Option<u64> {
    let mut scan = params.pattern.clone();
    let channels = params.channels.len();
    let mut words = vec![0u64; channels];
    let mut discard = vec![0u64; channels];
    let mut previous = vec![false; channels];
    let pre_words = params.pre_trigger / 64;
    let mut scanned = 0u64;
    let mut start = 0u64;
    let begin = Instant::now();
    let mut reported = Instant::now();
    loop {
        if stop.load(Ordering::Acquire) {
            return None;
        }
        let due = words_due(begin, params.samplerate);
        while scanned < due {
            scan.next_words(&params.channels, &mut words);
            let base = scanned * 64;
            let mut matches = u64::MAX;
            for &(index, condition) in &params.triggers {
                matches &= condition_mask(words[index], previous[index], condition);
            }
            if base < params.pre_trigger {
                let skip = params.pre_trigger - base;
                matches &= if skip >= 64 { 0 } else { u64::MAX << skip };
            }
            for (last, word) in previous.iter_mut().zip(&words) {
                *last = word >> 63 != 0;
            }
            if matches != 0 {
                return Some(base + u64::from(matches.trailing_zeros()) - start * 64);
            }
            scanned += 1;
            while start + pre_words < scanned {
                capture.next_words(&params.channels, &mut discard);
                start += 1;
            }
        }
        if reported.elapsed() >= PROGRESS_INTERVAL {
            reported = Instant::now();
            let _ = sink.send(Packet::Progress {
                triggered: false,
                captured: (scanned * 64).min(params.pre_trigger),
            });
        }
        std::thread::sleep(TICK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_edges_across_words() {
        // Sample 0 rises from the previous word's low level.
        assert_eq!(
            condition_mask(0b0110, false, TriggerCondition::Rise),
            0b0010
        );
        assert_eq!(
            condition_mask(0b0110, false, TriggerCondition::Fall),
            0b1000
        );
        assert_eq!(
            condition_mask(0b0111, false, TriggerCondition::Rise),
            0b0001
        );
        assert_eq!(condition_mask(0, true, TriggerCondition::Fall), 1);
        assert_eq!(
            condition_mask(0b0110, false, TriggerCondition::Both),
            0b1010
        );
        assert_eq!(
            condition_mask(0b0110, false, TriggerCondition::High),
            0b0110
        );
    }
}
