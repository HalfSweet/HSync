# Streaming decode engine for logic analyzers — PRD v0.2

**Scope of this version:** prove the whole chain **TypeScript DSL → resumable state machine → Rust execution**, with SPI → SPI NOR as the first layered decoding scenario.

The TypeScript API in this document is a proposal that pins down what the compiler and the executor must do; it is not an SDK that exists yet. The first step, which the code in this directory implements, builds the chain's Rust end by hand in the shape the DSL will take; [design.md](design.md) describes how.

## 1. Product goal

A Rust decode engine that embeds in logic analyzer software, automated test tools and waveform analysis tools.

Protocol authors write decoding rules in a restricted TypeScript; users compose decode chains through typed ports. **Decoding keeps up with a capture as it runs, without waiting for the capture or for a whole transaction to finish; the execution state and the memory it takes are under control.**

Live capture and offline replay use the same decoding logic.

## 2. Overall approach

```text
Compile chain:
TypeScript → type and subset checks → Decoder IR → state machine → native Rust code

Run chain:
sample blocks → signal queries and sampling → SPI event stream → SPI NOR event stream → UI / export
```

The MVP compiles ahead of time and brings in no JavaScript runtime. The Decoder IR is independent of TypeScript syntax and of Rust source, leaving room for other front ends and for dynamic loading later.

## 3. MVP scope

| Module              | Requirements                                                                                                                                        |
| ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Data input          | Takes blocks of digital samples with a time base continuously and supports replay in blocks; hardware captures come in through an external adapter. |
| Signal core         | Waiting for edges, reading levels, sampling at a given instant, and a consistent treatment of channels that change at the same instant.             |
| DSL compiler        | Parses TypeScript, checks the restricted subset, generates the IR and Rust; diagnostics point at the source file.                                   |
| Rust executor       | Keeps state across blocks; supports waiting for input, output backpressure, execution budgets and batching.                                         |
| Decode graph        | Typed ports, explicit connections, compatibility checks, cycle checks, and sharing of an upstream node's results.                                   |
| Reference protocols | Plain single-line SPI in all four modes with eight-bit words; a simplified SPI NOR that carries the three-byte-address `READ` through.              |
| Tools               | A CLI that replays, decodes and exports NDJSON, without a graphical interface.                                                                      |

## 4. The TypeScript DSL, by example

### 4.1 Typed events

Protocol layers pass structured events to each other, not UI annotation text and not arrays holding whole transactions.

The eight-bit SPI events of the MVP:

```ts
// spi-schema.ts
import { eventSchema, type U8, type TxId } from '@decoder/dsl'

export type SpiEvent =
  | { kind: 'begin'; tx: TxId }
  | {
      kind: 'word'
      tx: TxId
      mosi: U8 | null
      miso: U8 | null
    }
  | {
      kind: 'end'
      tx: TxId
      reason: 'cs-release' | 'aborted'
    }

export const spiEventsV1 = eventSchema<SpiEvent>('spi.events', 1)
```

`eventSchema<T>()` is a compiler built-in: it derives the schema and the Rust types from the type declaration statically, without runtime reflection. `U8` is the DSL's eight-bit unsigned integer; a line that is not connected reads `null`, never a made-up zero.

Every event carries the same envelope — the time range on the signal, the time at which the result became certain, and its number within the stream — so protocol authors do not declare these on each event type.

### 4.2 A streaming SPI NOR decoder

The example covers only `0x03` + three address bytes + data; it is not a complete SPI NOR implementation.

```ts
// spi-nor-read24.ts
import { defineDecoder } from '@decoder/dsl'
import { spiEventsV1 } from './spi-schema'
import { norEventsV1 } from './nor-schema'

export const SpiNorRead24 = defineDecoder({
  id: 'spi-nor-read24',

  input: {
    name: 'bus',
    schema: spiEventsV1,
    requires: {
      wordBits: 8,
      bitOrder: 'msb-first',
      cs: true,
      mosi: true,
      miso: true,
    },
  },

  output: {
    name: 'operations',
    schema: norEventsV1,
  },

  async transaction(tx, out) {
    const opcode = (await tx.word()).mosi

    if (opcode !== 0x03) {
      await out.unknownCommand(tx.id, opcode)
      await tx.drain()
      return
    }

    const op = await out.begin('read', tx.id)

    let address = 0
    for (let i = 0; i < 3; i++) {
      address = (address << 8) | (await tx.word()).mosi
    }
    await out.address(op, address)

    while (true) {
      const item = await tx.wordOrEnd()

      if (item.kind === 'end') {
        await out.end(op, 'complete')
        return
      }

      await out.data(op, item.miso)
    }
  },

  recovery: {
    truncated: 'abort-current',
    gap: 'abort-invalidate-resync',
  },
})
```

`norEventsV1` defines the events for an operation's beginning, its address, pieces of data, its end and an unknown command; `out` is the typed writer derived from the output schema.

The cursor `tx` offers the contract's primitives and nothing more: the next word with every line's value (`word()`), the next word or the transaction's normal end (`wordOrEnd()`), and draining (`drain()`). Which line a protocol reads, byte order and field widths are the decoder's own code or shared helper functions, never operations of the bus contract: a contract is the more useful the less it knows about the protocols above it. Because the input requires `mosi: true` and `miso: true`, the compiler types `mosi` and `miso` as `U8` in the body, not `U8 | null`.

The code must have these semantics:

| Operation       | Required behaviour                                                                                                                                                                                                                          |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `transaction()` | Starts when an SPI `begin` arrives; `tx` is a cursor over the stream, not a copy of the whole transaction.                                                                                                                                  |
| `word()`        | May wait across input blocks; a sequence of reads that pauses between them (here, the address bytes) keeps what it has read so far and never consumes input twice. The transaction's end here is an error that goes down the recovery path. |
| `wordOrEnd()`   | Returns `end` only for a normal chip select release; an abnormal end or a gap goes down the recovery path.                                                                                                                                  |
| `out.data()`    | Emits data continuously, without waiting for the read to finish; the physical transfer may batch.                                                                                                                                           |
| `await`         | Saves the state when input is short or output has no room, and continues once the condition holds — without a JavaScript runtime.                                                                                                           |
| `recovery`      | Aborts the operation under way and emits an abnormal end; a gap in addition invalidates the related state and resynchronizes.                                                                                                               |

**Both reading and writing can pause.** After pausing on output backpressure, a decoder must neither read its input again nor send an event twice; when both input and output are available, several `await`s complete in one run.

### 4.3 A layered decode graph

Protocol implementations are separate from the wiring. A pipeline file describes the capture channels and the connections between decoders:

```ts
// pipeline.ts
import { definePipeline } from '@decoder/dsl'
import { SpiDecoder } from '@decoder/protocols'
import { SpiNorRead24 } from './spi-nor-read24'

export default definePipeline((g) => {
  const capture = g.source('logic')

  const spi = g.node(SpiDecoder, {
    clk: capture.channel(0),
    cs: capture.channel(1),
    mosi: capture.channel(2),
    miso: capture.channel(3),
    mode: 0,
    wordBits: 8,
    bitOrder: 'msb-first',
  })

  const flash = g.node(SpiNorRead24)

  g.connect(spi.outputs.events, flash.inputs.bus)
  g.expose('flash-operations', flash.outputs.operations)
})
```

This generates the Graph IR statically. Connecting checks the schema identity and version, the word width, the bit order, the required channels, the transaction framing and the time domain.

SPI NOR depends on the `spi.events` contract, not on a particular SPI implementation: a test event source, a native Rust decoder or another DSL decoder all connect as long as they satisfy the contract. When one SPI output feeds several consumers, the decoded results are shared.

### 4.4 The subset

Supported: explicit integer types, structs, discriminated unions, conditionals, bounded computation loops, stream loops that advance the input, and helper functions that can be resolved statically.

Not supported: arbitrary npm modules, dynamic execution, network and file access, arbitrary Promises, recursion and containers that grow without bound. Buffers and any data kept across a pause point must have an explicit capacity or resource policy.

Integer overflow, shifts and conversions are defined by the DSL; time has a dedicated type. **Passing the TypeScript type checker is not passing the DSL checker.** Programs that use unsupported features or cannot meet the resource constraints must be rejected explicitly.

## 5. Runtime constraints

**Input lifecycle.** A stream carries `Data / Progress / Gap / End`. Exhausting the current block is only a pause; `End` means the input is over for good; `Gap` means data is missing. Timeouts follow capture time, not the host's callback intervals.

**Incremental results and provenance.** Every layer emits immutable events continuously, ties lifecycles together through transaction or operation ids, and traces its sources through time ranges and source identifiers. No layer may cache a whole transaction by default, nor hold on to large upstream objects to keep the relation.

**Backpressure and resource limits.** Queues, decoder state and retained buffers have explicit limits. When a capture source cannot pause and its buffer runs out, it must report a gap or a reason for stopping; it must never drop samples silently. Archiving results and the decoders' working memory are managed separately.

**Determinism.** The same input and configuration produce the same protocol results; how the input is cut into blocks and where downstream pauses must not change the decoded meaning.

## 6. Acceptance criteria

| Scenario              | Passes when                                                                                                                                    |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| Incremental decoding  | SPI words, the NOR address and pieces of data that are already certain can be consumed before the transaction ends.                            |
| Arbitrary blocks      | Cutting the input at random inside bytes, addresses and transactions yields, batch boundaries aside, the same events and the same final state. |
| Downstream pressure   | Pausing and resuming the consumer at random yields no duplicate and no missing event.                                                          |
| Recovery              | Covers truncated fields, lost samples and unknown commands; an incomplete transaction is never marked complete.                                |
| Bounded memory        | Long transactions and continuous input never make the decoder state or the queues grow without bound.                                          |
| Compiler diagnostics  | Illegal syntax, types, resource use and port connections each produce a diagnostic that points at its place.                                   |
| Performance benchmark | Throughput, P95 event latency and peak memory are measured on named reference hardware under a named load, to show the engine keeps up.        |

## 7. Deliverables and order of work

The MVP delivers the Rust core library, the TypeScript DSL SDK and compiler, the reference protocols, the CLI, and the tests for consistency, backpressure, recovery and performance.

The order: **first carry the SPI → NOR stream contract through in handwritten Rust; then extract the IR and connect the TypeScript compiler; last, verify correctness and optimize performance.**

Out of scope for this version: a full GUI, capture hardware drivers, full TypeScript compatibility, a JavaScript runtime, dynamic loading at runtime, a sigrok compatibility layer, QSPI/OSPI and clock synchronization across devices. The signal interface underneath is not limited to byte streams, so that wider buses and multi-phase protocols can follow.
