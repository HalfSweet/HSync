// SPDX-License-Identifier: Apache-2.0

//! `uiharu gen`: writes a synthetic SPI NOR capture to try the decoder on. (The module is
//! `generate` because `gen` is a reserved word.)

use std::fs::File;
use std::io::{self, BufWriter, Write};

use uiharu_core::{LogicBlock, Tick};
use uiharu_protocols::spi::synth::{SpiWaveform, nor_read_words};

use crate::Failure;
use crate::args::{Args, UsageError};
use crate::setup::{self, Setup};

/// The options of the command, beyond the shared ones.
pub const OPTIONS: &[&str] = &["reads", "bytes", "half-period", "idle", "seed"];

pub const USAGE: &str = "\
uiharu gen [options] <output>

  Writes a synthetic capture of SPI NOR traffic in the raw binary layout `decode` reads (`-`
  for standard output): READ commands of random addresses and data, with a JEDEC ID read
  (opcode 0x9f, which the decoder does not know) every fifth transaction.

      --reads <N>               transactions to write (default 16)
      --bytes <N>               data bytes per read (default 64)
      --half-period <SAMPLES>   samples per half clock period (default 4)
      --idle <SAMPLES>          idle samples between transactions (default 40)
      --seed <N>                seed of the addresses and data (default 1)
";

/// A small deterministic generator (xorshift64*).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn byte(&mut self) -> u8 {
        (self.next() >> 56) as u8
    }
}

/// Appends `reads` transactions to `waveform`.
pub fn traffic(waveform: &mut SpiWaveform, rng: &mut Rng, reads: u64, bytes: usize, idle: u64) {
    for n in 0..reads {
        if n % 5 == 4 {
            waveform.transaction([(0x9f, 0xff), (0x00, 0xef), (0x00, 0x40), (0x00, 0x18)]);
        } else {
            let address = (rng.next() >> 40) as u32 & 0x00ff_ffff;
            let data: Vec<u8> = (0..bytes).map(|_| rng.byte()).collect();
            waveform.transaction(nor_read_words(address, &data));
        }
        waveform.idle(idle);
    }
}

/// The block's samples in the raw binary layout.
pub fn interleave(block: &LogicBlock, channels: u8) -> Vec<u8> {
    let unit = usize::from(channels).div_ceil(8);
    let mut bytes = Vec::with_capacity(usize::try_from(block.len()).unwrap_or(0) * unit);
    let mut tick = block.start();
    while tick < block.end() {
        let levels = block.levels(tick).map_or(0, |levels| levels.0);
        for group in 0..unit {
            bytes.push((levels >> (8 * group)) as u8);
        }
        tick = tick.next();
    }
    bytes
}

pub fn run(args: &Args) -> Result<(), Failure> {
    let mut known: Vec<&str> = setup::OPTIONS.to_vec();
    known.extend(OPTIONS);
    args.only(&known)?;
    let setup = Setup::from_args(args)?;
    let path = args.positional().get(1).ok_or_else(|| {
        UsageError("gen needs the file to write, or - for standard output".into())
    })?;
    let reads: u64 = args.parsed("reads", 16)?;
    let bytes: usize = args.parsed("bytes", 64)?;
    let half_period: u64 = args.parsed("half-period", 4)?;
    let idle: u64 = args.parsed("idle", 40)?;
    let seed: u64 = args.parsed("seed", 1)?;

    let mut waveform = SpiWaveform::new(setup.spi.clone(), setup.channels, half_period, Tick::ZERO);
    waveform.idle(idle);
    traffic(&mut waveform, &mut Rng::new(seed), reads, bytes, idle);
    let block = waveform.finish();
    let data = interleave(&block, setup.channels);

    let mut out: Box<dyn Write> = if path == "-" {
        Box::new(BufWriter::new(io::stdout().lock()))
    } else {
        Box::new(BufWriter::new(
            File::create(path).map_err(|error| Failure::Io(format!("{path}: {error}")))?,
        ))
    };
    out.write_all(&data)?;
    out.flush()?;
    eprintln!(
        "{path}: {} samples of {} channels ({} bytes), {reads} transactions",
        block.len(),
        setup.channels,
        data.len()
    );
    Ok(())
}
