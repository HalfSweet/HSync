//! The captures every decoder is measured on, in both layouts the decoders read: uiharu's
//! `LogicBlock`s and the hsync crates' `LogicCapture`.
//!
//! A workload is one unit of samples, a multiple of 64 long, repeated until it is long enough:
//! DSView's protocol demo (16 channels at 25 MHz, SPI on D12–D15) or synthetic SPI NOR traffic
//! from uiharu's waveform synthesizer (4 channels, a clock edge every two samples).

use std::path::Path;
use std::sync::Arc;

use hsync_capture::LogicCapture;
use hsync_core::ChannelIndex;
use uiharu_core::{LogicBlock, Tick};
use uiharu_protocols::spi::SpiConfig;
use uiharu_protocols::spi::synth::{SpiWaveform, nor_read_words};

/// Which capture a case runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workload {
    /// DSView's protocol demo, repeated.
    Demo,
    /// Synthetic SPI NOR `READ` traffic, repeated.
    Nor,
}

impl Workload {
    pub const ALL: [Workload; 2] = [Workload::Demo, Workload::Nor];

    pub fn name(self) -> &'static str {
        match self {
            Workload::Demo => "demo",
            Workload::Nor => "nor",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|workload| workload.name() == name)
    }
}

/// The SPI lines of a workload, as channel indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bus {
    pub clk: ChannelIndex,
    pub cs: ChannelIndex,
    pub mosi: ChannelIndex,
    pub miso: ChannelIndex,
}

impl Bus {
    /// uiharu's view of the bus: mode 0, eight-bit words, most significant bit first, chip
    /// select active low — what DSView's demo and the synthesizer both use.
    pub fn spi_config(&self) -> SpiConfig {
        SpiConfig {
            clk: self.clk as u8,
            cs: Some(self.cs as u8),
            mosi: Some(self.mosi as u8),
            miso: Some(self.miso as u8),
            ..SpiConfig::default()
        }
    }
}

/// A workload's samples: one unit, repeated.
#[derive(Debug)]
pub struct Source {
    pub channels: u8,
    pub rate_hz: u64,
    pub bus: Bus,
    /// The words of each channel over one unit.
    unit: Vec<Vec<u64>>,
    /// Samples per unit, a multiple of 64.
    unit_len: u64,
    repeats: u64,
}

/// A small deterministic generator (xorshift64*), for the synthetic traffic.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn byte(&mut self) -> u8 {
        (self.next() >> 56) as u8
    }
}

impl Source {
    /// DSView's protocol demo, `repeats` times over.
    pub fn demo(root: &Path, repeats: u64) -> Result<Self, String> {
        let path = root.join("src-tauri/resources/demo/protocol.dsl");
        let document = hsync_fileio::read_dsl_file(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let capture = document.capture;
        let channels = capture
            .channels()
            .map(|(index, _)| index + 1)
            .max()
            .unwrap_or(0);
        let unit_len = capture.len();
        if unit_len == 0 || !unit_len.is_multiple_of(64) {
            return Err(format!(
                "the demo capture holds {unit_len} samples; a multiple of 64 is needed"
            ));
        }
        let unit = (0..channels)
            .map(|index| {
                let Some(data) = capture.channel(index) else {
                    return vec![0; (unit_len / 64) as usize];
                };
                (0..data.word_count()).map(|word| data.word(word)).collect()
            })
            .collect();
        Ok(Self {
            channels: u8::try_from(channels).map_err(|_| "too many channels".to_owned())?,
            rate_hz: capture.samplerate(),
            // The demo's SPI decoder entry: CLK on D12, CS# on D13, MOSI on D14, MISO on D15.
            bus: Bus {
                clk: 12,
                cs: 13,
                mosi: 14,
                miso: 15,
            },
            unit,
            unit_len,
            repeats: repeats.max(1),
        })
    }

    /// Synthetic SPI NOR traffic of at least `samples` samples: `READ` commands of `bytes`
    /// random bytes with a JEDEC ID read every fifth transaction, a clock edge every
    /// `half_period` samples, as `uiharu bench` makes it.
    pub fn nor(samples: u64, bytes: usize, half_period: u64) -> Self {
        let config = SpiConfig::default();
        let bus = Bus {
            clk: config.clk.into(),
            cs: config.cs.unwrap_or(1).into(),
            mosi: config.mosi.unwrap_or(2).into(),
            miso: config.miso.unwrap_or(3).into(),
        };
        let channels = 4;
        let mut waveform = SpiWaveform::new(config, channels, half_period, Tick::ZERO);
        let mut rng = Rng(7);
        waveform.idle(8);
        let mut n = 0;
        while waveform.position().get() < 65_536 {
            if n % 5 == 4 {
                waveform.transaction([(0x9f, 0xff), (0x00, 0xef), (0x00, 0x40), (0x00, 0x18)]);
            } else {
                let address = (rng.next() >> 40) as u32 & 0x00ff_ffff;
                let data: Vec<u8> = (0..bytes).map(|_| rng.byte()).collect();
                waveform.transaction(nor_read_words(address, &data));
            }
            waveform.idle(8);
            n += 1;
        }
        // Pad to whole words so that the unit tiles the capture word by word.
        let short = waveform.position().get().next_multiple_of(64) - waveform.position().get();
        waveform.idle(short);
        let block = waveform.finish();
        let unit_len = block.len();
        let unit = (0..channels).map(|c| block.words(c).to_vec()).collect();
        Self {
            channels,
            rate_hz: 25_000_000,
            bus,
            unit,
            unit_len,
            repeats: samples.div_ceil(unit_len).max(1),
        }
    }

    /// The samples of the whole workload.
    pub fn samples(&self) -> u64 {
        self.unit_len * self.repeats
    }

    fn unit_words(&self) -> usize {
        (self.unit_len / 64) as usize
    }

    /// The workload as blocks of `chunk` samples (rounded up to whole words), for uiharu.
    pub fn blocks(&self, chunk: u64) -> Vec<Arc<LogicBlock>> {
        let chunk = chunk.max(1).next_multiple_of(64);
        let total = self.samples();
        let unit_words = self.unit_words();
        let mut blocks = Vec::with_capacity(total.div_ceil(chunk) as usize);
        let mut start = 0;
        while start < total {
            let len = chunk.min(total - start);
            let first_word = (start / 64) as usize;
            let words_per_channel = len.div_ceil(64) as usize;
            let mut words = Vec::with_capacity(words_per_channel * usize::from(self.channels));
            for lane in &self.unit {
                words.extend(
                    (first_word..first_word + words_per_channel)
                        .map(|word| lane[word % unit_words]),
                );
            }
            let block = LogicBlock::from_words(Tick(start), len, self.channels, words)
                .expect("the words match the block");
            blocks.push(Arc::new(block));
            start += len;
        }
        blocks
    }

    /// The workload as a capture, for the hsync decoders.
    pub fn capture(&self) -> LogicCapture {
        let mut capture = LogicCapture::new(self.rate_hz, 0..ChannelIndex::from(self.channels));
        for (index, lane) in self.unit.iter().enumerate() {
            let channel = capture
                .channel_mut(index as ChannelIndex)
                .expect("the channel was created");
            for _ in 0..self.repeats {
                channel.append_words(lane);
            }
        }
        capture
    }
}
