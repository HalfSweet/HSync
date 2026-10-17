# uiharu performance

Where the engine spends its time, what was changed to spend less of it, how it compares with the decoders HSync runs today, and what is left to do. The numbers come from one machine and one day; the methodology below says how to take them again.

## 1. Machine and toolchain

|                          |                                                                                                                            |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| Machine                  | Apple M4 Pro, 14 cores, 24 GiB, macOS 27.0.1                                                                               |
| Rust                     | rustc 1.98.1 (stable), release profile of the workspace: thin LTO, one codegen unit                                        |
| Python (for `hsync-srd`) | CPython 3.14.7, the Homebrew framework build                                                                               |
| Profiler                 | samply 0.13.1 at 5 kHz, `--unstable-presymbolicate`, on a release build with line tables (`CARGO_PROFILE_RELEASE_DEBUG=1`) |
| sigrok-cli               | not installed; the harness reports it as skipped                                                                           |

## 2. Methodology

Two workloads, both SPI mode 0, eight-bit words, MSB first, chip select active low:

- **dense** — `uiharu bench --samples 50000000`: synthetic SPI NOR `READ` traffic from `spi::synth`, 4 channels, a clock edge every 2 samples, blocks of 65 648 samples, the SPI → SPI NOR pipeline with both outputs read and discarded. 25.6 M clock edges and 54 k chip-select edges in 50 M samples; 1 600 200 SPI words, 114 300 NOR events. `hsync-bench`'s `nor` workload is the same traffic (50 016 000 samples).
- **demo** — DSView's protocol demo capture (`src-tauri/resources/demo/protocol.dsl`, 16 channels at 25 MHz, SPI on D12–D15) repeated 128 times: 16.8 M samples, 36 864 words, one clock edge per 28 samples. The sparse case, and the one with 16 channels.

Rules followed for every number here: release builds; the machine's load average checked before a run (below 6; a run during a load spike was discarded — this machine shares its cores with other sessions, and under a load of 100 the same binary measured 3× slower); `uiharu bench` reported as the median of five runs; `hsync-bench` as the median of three, each case in a child process under `/usr/bin/time -l` for its peak resident set; A/B comparisons run the two binaries alternately (A, B, A, B, …) within the same minute. The engine's cost is per **edge**, not per sample — a clock edge every 2 or every 8 samples costs the same 15–20 ns — so the tables also give nanoseconds per clock edge, computed as seconds / (SPI words × 16 + chip-select edges).

Reproducing:

```sh
cargo run --release -p uiharu-cli -- bench --samples 50000000          # dense, SPI -> NOR
cargo run --release -p hsync-bench -- --repeats 128 --samples 50000000 --runs 3
CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release -p uiharu-cli --target-dir target/prof
samply record --save-only --unstable-presymbolicate -r 5000 -o profile.json.gz \
  target/prof/release/uiharu bench --samples 100000000
```

## 3. The profile before

Baseline (commit `bdceca1`): 100 Msamples/s on the dense workload, 19.5 ns per clock edge. Self time by symbol, dense workload:

| Symbol                                                                                    | Self time |
| ----------------------------------------------------------------------------------------- | --------: |
| `LogicBlock::next_change` (the per-word edge scan)                                        |    50.5 % |
| `LogicBlock::levels` (gather of all channels at one tick) — called three times per change |    18.2 % |
| `SignalCursor::poll_change` (budget, block bookkeeping, `Rc`/`Arc` clones)                |    14.5 % |
| SPI decoder `run` future                                                                  |     5.6 % |
| `Decoder::shift`                                                                          |     3.6 % |
| `Outbox::emit` → `Broadcast::try_push`                                                    |     2.8 % |
| SPI NOR (`SpiBus::next_word`, `run`, emits)                                               |     2.5 % |
| `Broadcast::advance` (inbox and exposed reader)                                           |     1.3 % |
| `Pipeline::run`, allocation, everything else                                              |     < 1 % |

83 % of the time went into finding the next change. The executor's polling loop, the budget refills and the queues are not visible; the NOR layer and its `Vec<u8>` batches cost under 3 %.

The CLI's input path (`uiharu decode` on a 71 M-sample raw file, SPI pipeline, nothing written) took 845 ms, of which the engine needs about 550 ms: `LogicBlock::from_interleaved`, a bit transpose done sample by sample through `LogicBlockBuilder`, cost ~270 ms (3.8 ns per sample); writing 2.3 M SPI events as NDJSON costs another ~260 ms (115 ns per event line).

## 4. Optimizations

Each is one commit with its measurement; the first three columns are `uiharu bench` on the dense workload.

| Commit    | Change                                                                                                                                                                                                                                                                                                                                                                  |         Before |          After | Other measurements                                                                                                                                                                                                 |
| --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------: | -------------: | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `ea27209` | `next_change`: the changed channels come from the same per-channel edge words that found the edge, instead of two more `levels()` gathers; the watched channels are walked with a bit loop instead of an iterator closure; channels the block does not have are masked once, not per word; the cursor borrows its current block instead of cloning the `Arc` per change | 100 Msamples/s | 128 Msamples/s | demo 586 → 910 Msamples/s; nor (harness) 98 → 130                                                                                                                                                                  |
| `104e544` | `from_interleaved`: 64 samples of a byte group as a 64 × 8 bit matrix — eight 8 × 8 bit transposes (masks `0x00aa…`, `0x0000cccc…`, `0xf0f0f0f0` with shifts 7, 14, 28), then one 8 × 8 byte transpose — storing each channel word once; a proptest checks it against the old construction                                                                              |              — |              — | 4 M random samples: 4 channels 10.9 → 0.21 ns/sample (52×), 8 channels 22.4 → 0.25 (89×), 16 channels 41.8 → 1.08 (39×), 64 channels 173 → 4.1 (42×); `uiharu decode`, 71 M samples, nothing written: 840 → 585 ms |
| `1f92e7f` | `levels()` back to a plain indexed loop: the `iter().step_by(stride)` form the first commit introduced measured slower in the pipeline                                                                                                                                                                                                                                  |          128.8 |          134.0 |                                                                                                                                                                                                                    |

Result on the dense workload: **100 → 133 Msamples/s (1.33×)**, 19.5 → 14.6 ns per clock edge, block latency p50 637 → 482 µs for 65 648-sample blocks; with a clock edge every 8 samples 390 → 511 Msamples/s; SPI alone 105 → 141. On the demo capture **586 → 908 Msamples/s (1.55×)**. Memory is unchanged: 3 MiB resident for the whole `uiharu bench` process, +0.1 MiB during decoding in the harness.

Tried and reverted, each measured with five interleaved A/B pairs (noise about ±1 %):

| Change                                                                                                                            | Result                                    |
| --------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------- |
| `Decoder::shift`: precomputed MOSI/MISO level masks instead of `Option<u8>` checks per bit                                        | 131.9 vs 133.4 Msamples/s median: no gain |
| `poll_change` without the `Rc<NodeCx>` clone per call (`self.inbox.cx()` at each site)                                            | 131.9 vs 132.6: no gain                   |
| `next_change` in one pass, the first two watched channels' edge words kept in registers instead of recomputed for the changed set | 134.2 vs 132.6: +1 %, within noise        |

These confirm what the profile only hints at: the scan is bound by the latency of a loop-carried dependency, not by instruction count. Each edge waits for the previous one: `prev` levels → edge words → `trailing_zeros` → tick → gather of the levels at that tick → `prev`. Removing instructions off that chain changes nothing; only shortening the chain or doing more per step does.

## 5. Where the time goes now

Self time at HEAD, dense workload (14.6 ns per edge): `next_change` 38.7 %, `levels` 20.2 %, `poll_change` 15.1 %, SPI `run` 10.8 %, `shift` 5.1 %, `Outbox::emit` 2.6 %, `SpiBus::next_word` 2.5 %, NOR `run` 1.3 %, `advance` 2.0 %.

Decomposition with microbenchmarks on the same traffic (nanoseconds per clock edge):

| What runs                                                                                                    | ns / edge |
| ------------------------------------------------------------------------------------------------------------ | --------: |
| The whole SPI → SPI NOR pipeline (`uiharu bench`)                                                            |      14.6 |
| The runtime alone: a node that awaits `next_change` on CLK and CS and only counts, through a real `Pipeline` | 13.4–14.0 |
| `LogicBlock::next_change` + `levels` in a plain loop, outside the runtime                                    |      9–11 |

So the decoders — SPI, SPI NOR, their queues and emits — cost about 1 ns per edge; the cursor, executor and async machinery about 2.5 ns; the stateless word scan and gather about 11 ns. On the sparse 16-channel demo the engine costs 31 ns per edge (1.1 ns per sample): each `next_change` call also walks the words without edges, and the gather reads 16 channels instead of 4.

## 6. Comparison with the sigrok decoders

`hsync-bench` (`crates/hsync-bench`) runs every decoder on the same captures in child processes: uiharu's SPI and SPI → SPI NOR pipelines; `hsync-decode`'s native SPI decoder (a Rust port of DSView's `1:spi`); DSView's own Python `1:spi` decoder in the embedded interpreter through `hsync-srd`, which is libsigrokdecode's actual behaviour; and `sigrok-cli`, when installed. The Python decoder walks the samples one edge at a time in the interpreter and gets 1/16 of the samples (`--python-divisor 16`); the rates are comparable, the memory is not. "Events" are what each decoder puts out: uiharu's begin/word/end events, the native decoder's words and transfers, the Python decoder's annotations — about 29 per word, as it annotates every bit and every transfer as well.

The Python decoder host (`hsync-srd`) and DSView's decoders left the repository after this study; the `sigrok-spi` rows stay as they were measured and cannot be reproduced from the current tree.

Before the optimizations (`bdceca1`):

| Decoder             | Workload |    Samples | Msamples/s | kwords/s | ns / edge |     Time | Peak RSS | RSS growth while decoding |
| ------------------- | -------- | ---------: | ---------: | -------: | --------: | -------: | -------: | ------------------------: |
| uiharu-spi          | demo     | 16 777 216 |        586 |    1 288 |        48 |  28.6 ms | 39.3 MiB |                  +0.2 MiB |
| uiharu-spi-nor      | demo     | 16 777 216 |        604 |    1 326 |        47 |  27.8 ms | 39.3 MiB |                  +0.2 MiB |
| hsync-decode-spi    | demo     | 16 777 216 |      1 916 |    4 210 |        15 |   8.8 ms | 41.4 MiB |                  +1.2 MiB |
| sigrok-spi (Python) | demo     |  1 048 576 |       34.8 |       76 |       820 |  30.2 ms | 45.3 MiB |                  +7.0 MiB |
| uiharu-spi          | nor      | 50 016 000 |         98 |    3 051 |      19.8 |   508 ms | 30.7 MiB |                  +0.1 MiB |
| uiharu-spi-nor      | nor      | 50 016 000 |         95 |    2 936 |      20.6 |   528 ms | 30.7 MiB |                  +0.2 MiB |
| hsync-decode-spi    | nor      | 50 016 000 |        186 |    5 776 |      10.5 |   269 ms | 81.9 MiB |                 +50.3 MiB |
| sigrok-spi (Python) | nor      |  3 134 336 |        2.5 |       79 |       780 | 1 237 ms |  329 MiB |                  +291 MiB |

After (`1f92e7f`):

| Decoder             | Workload |    Samples | Msamples/s | kwords/s | ns / edge |     Time | Peak RSS | RSS growth while decoding |
| ------------------- | -------- | ---------: | ---------: | -------: | --------: | -------: | -------: | ------------------------: |
| uiharu-spi          | demo     | 16 777 216 |        908 |    1 996 |        31 |  18.5 ms | 39.3 MiB |                  +0.1 MiB |
| uiharu-spi-nor      | demo     | 16 777 216 |        930 |    2 044 |        30 |  18.0 ms | 39.3 MiB |                  +0.2 MiB |
| hsync-decode-spi    | demo     | 16 777 216 |      1 873 |    4 116 |        15 |   9.0 ms | 41.4 MiB |                  +1.2 MiB |
| sigrok-spi (Python) | demo     |  1 048 576 |       34.5 |       76 |       825 |  30.4 ms | 45.7 MiB |                  +7.1 MiB |
| uiharu-spi          | nor      | 50 016 000 |        137 |    4 248 |      14.2 |   365 ms | 30.7 MiB |                  +0.1 MiB |
| uiharu-spi-nor      | nor      | 50 016 000 |        129 |    3 986 |      15.2 |   389 ms | 30.7 MiB |                  +0.2 MiB |
| hsync-decode-spi    | nor      | 50 016 000 |        186 |    5 752 |      10.5 |   270 ms | 81.9 MiB |                 +50.3 MiB |
| sigrok-spi (Python) | nor      |  3 134 336 |        2.5 |       78 |       780 | 1 252 ms |  329 MiB |                  +291 MiB |

Ratios, by samples per second: uiharu's SPI decoder runs at **0.48×** the native `hsync-decode` decoder on the demo capture and **0.74×** on dense traffic (0.31× and 0.53× before), and **26×** and **55×** the Python sigrok decoder (17× and 39× before). The peak RSS of a case is mostly the workload itself (25–34 MiB of samples); the growth during decoding is the decoder's own: uiharu's stays at 0.1–0.2 MiB whatever the length, because it streams through bounded queues, while the native decoder materializes its words (40 bytes each, 50 MiB here) and the Python decoder its annotations (291 MiB for 3 M samples).

What the gap to the native decoder is: on dense traffic 14.2 against 10.5 ns per edge, the cost of resumable decoders with backpressure (section 5: about 3.5 ns per edge for the cursor, executor and async wrapper) plus the per-edge gather of all channels. On the sparse 16-channel capture the native decoder keeps per-channel summaries that skip quiet stretches of samples and reads only the lines it needs, where uiharu scans every word and gathers 16 channels at every edge.

## 7. Proposals

Not implemented here, either because they change public APIs that other work builds on right now, or because they are not worth their cost yet. Expected gains are from prototypes in a scratch microbenchmark or from the measurements above.

| Proposal                                                                                                                                                                                                                                                                                                                                                                                  | Expected gain                                                                                                                                                                                      | Notes                                                                                                                                                                                                                                                                                                                                   |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **A word-batch read on the cursor** (API addition): `next_word(mask)` resolving to the 64-sample word at the cursor — the edge bits of each watched channel and the level words of the channels the decoder reads — so a decoder handles up to 64 ticks per `await` and takes bits with shifts, e.g. SPI: `rising = clk & !(clk << 1 \| carry)`, then `mosi >> bit & 1` per sampling edge | SPI on dense traffic 5–10×: the prototype of the extraction alone runs at 1.0 ns per sampling edge, 8 ns per 8-bit word, against ~235 ns per word today; polls, budget and future resumes fall 64× | Keeps determinism (the cursor still carries the levels before the word across blocks, and a word cut by a block boundary is served as the block's partial word). It is also the shape the DSL compiler should emit for "bit on edge" decoders, and the real vectorization of this engine: 64 samples per step with no per-sample branch |
| **An edge cache in the cursor**: the edge words of the current word for the current mask, keyed by block, word and mask, so that the next call in the same word is a masked `trailing_zeros` instead of a rescan                                                                                                                                                                          | Scan 9–11 → 5.7 ns per edge in the prototype; pipeline ≈ 14.6 → 10–11 ns per edge, about 1.4× on dense traffic                                                                                     | One new private struct and a one-line change at the call site in `poll_change`; not done now because `poll_change` is being extended (`seek`, `next_change_before`, per-block progress), and `seek` must invalidate the cache                                                                                                           |
| **A read mask for the levels**: `next_change(watch, read)` so the gather covers only the channels the decoder reads                                                                                                                                                                                                                                                                       | About 20 % on 16-channel captures (the 16-channel gather costs ~7 ns per edge against ~2.5 for 4)                                                                                                  | Subsumed by the word-batch read                                                                                                                                                                                                                                                                                                         |
| Hand-written NDJSON in `uiharu decode` instead of `serde_json::to_writer` per line                                                                                                                                                                                                                                                                                                        | 3–4× on the output part (115 ns per event line today, 22 % of a run that writes every stream)                                                                                                      | CLI code, outside this round                                                                                                                                                                                                                                                                                                            |
| SIMD for `from_interleaved` with 16 or more channels: NEON `tbl`/`vld4` to de-interleave the byte groups before the bit transposes (`std::arch` with runtime detection, one module, scalar fallback, differential tests)                                                                                                                                                                  | 2–4× on the function, which is now ≤ 7 % of `uiharu decode`                                                                                                                                        | Needs `unsafe`; not proposed for adoption — the gain no longer justifies it. SIMD does not apply to the edge scan itself: it is latency-bound per edge; the word-batch read is where the data parallelism is                                                                                                                            |
| `NorEvent::Data` without a `Vec<u8>` per batch (an inline `[u8; 32]` with a length, or a shared buffer)                                                                                                                                                                                                                                                                                   | < 1 %: one allocation per 32 bytes, the NOR layer is 1.3 % of the time                                                                                                                             | API change                                                                                                                                                                                                                                                                                                                              |
| `Broadcast::trim` remembering the slowest reader instead of scanning the readers on every `advance`                                                                                                                                                                                                                                                                                       | < 1 % at today's reader counts                                                                                                                                                                     |                                                                                                                                                                                                                                                                                                                                         |
| `uiharu bench`'s "peak memory" on macOS reads `ps -o rss=`, the _current_ resident set; the peak needs `getrusage` (libc) or `/usr/bin/time -l`, which `hsync-bench` uses                                                                                                                                                                                                                 | Correctness of the report, not speed                                                                                                                                                               |                                                                                                                                                                                                                                                                                                                                         |
| The acceptance suite sets `cases: 64` explicitly, which overrides `PROPTEST_CASES`; reading the variable when it is set would let `PROPTEST_CASES=2000` mean 2000 cases                                                                                                                                                                                                                   | Test workflow                                                                                                                                                                                      | The suite takes 0.3 s at 64 cases; `LogicBlock::slice` and `LogicBlockBuilder::push_repeated` are sample-by-sample but no longer worth speeding up for it                                                                                                                                                                               |

No `unsafe` was introduced; the uiharu crates keep `#![forbid(unsafe_code)]`.

## 8. The interpreter

`uiharu-interp` ([dsl.md](dsl.md) §8) runs a decoder's IR on the same runtime without generating its Rust, for a decoder under work or one a user brings. Measured on the synthetic capture of `uiharu gen --reads 3000 --bytes 256` (40 233 640 samples, 632 400 SPI words, 28 200 NOR events), `uiharu decode` writing every stream, release build, on the machine of section 1:

| Pipeline                          |   Time | Ratio |
| --------------------------------- | -----: | ----: |
| SPI and SPI NOR compiled          | 0.47 s |     1 |
| SPI compiled, SPI NOR interpreted | 1.16 s |  2.5× |
| SPI interpreted, SPI NOR compiled | 6.13 s |   13× |
| SPI and SPI NOR interpreted       | 6.58 s |   14× |

The output is the same to the byte in every row. The edge-driven bus decoder is where the cost is: about 600 ns per clock edge against 15 compiled — one dispatch per instruction (some fifty per edge), every value on a stack and every record behind a reference count, the `Edge` record made for each `next_change`, the decoder's fields behind a `RefCell`, and the JSON of every event on the way out. A device decoder over a compiled bus pays the bridge (serde to JSON and back once per word) and its interpreted body, about 1 µs per word. None of it is optimized: the interpreter is for trying a decoder; a decoder that is kept is generated.
