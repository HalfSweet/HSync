# Parallel bus

The `parallel` decoder reads up to 64 data lines as one word — on the edges of a clock line, or whenever a data line changes — and writes the words to a `parallel.events` stream.

## Configuration

| Option  | Values                                                                             | Default      |
| ------- | ---------------------------------------------------------------------------------- | ------------ |
| `data`  | The data lines' channels, least significant bit first: 1 to 64 distinct channels   | channels 0–7 |
| `clock` | None, or a channel and the edges that read a word: `rising`, `falling` or `either` | none         |

The clock's channel must not be a data line.

## Events (`parallel.events` v1)

| Kind   | Fields                     | Meaning                          |
| ------ | -------------------------- | -------------------------------- |
| `word` | `value`, `bits`, `initial` | The data lines, read as one word |

| Field     | Meaning                                                                                                                                   |
| --------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `value`   | Bit `i` is the level of the `i`-th data line                                                                                              |
| `bits`    | The number of data lines, 1 to 64                                                                                                         |
| `initial` | On a bus without a clock, `true` for the levels found at the first sample or at the first sample after a gap: they were not seen changing |

A word covers the tick it was read at and is settled there.

## Props

| Prop      | Value                                |
| --------- | ------------------------------------ |
| `bits`    | The number of data lines             |
| `clocked` | Whether a clock line reads the words |

The logic input requires `channels` of at least the highest channel read plus one.

## Decoding rules

- With a clock, a word is the data lines' levels at the configured edge of the clock, read at the first sample with the clock's new level. The clock's other edge, and changes of the data lines between its edges, make no word.
- Without a clock, a word is reported at every tick where a data line changes, with the levels after the change. Lines that change at the same tick make one word, because the signal cursor reports them together; lines that change a tick apart make a word each, so skew between the lines shows up as words in between.
- The first sample of the stream and the first after a gap are never edges or changes. Without a clock, their levels are reported once, marked `initial`, so that the bus's value is known from the first sample on; with a clock, nothing is read there.

## Limits

- At most 64 data lines.
- Values of 2^53 and above, which only buses of more than 53 lines carry, are not kept exactly by JSON readers that parse numbers as doubles, such as JavaScript's `JSON.parse`.
