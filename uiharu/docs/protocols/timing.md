# Edge timing

The `timing` decoder reports the edges of one line, each with the ticks since the previous one, and writes them to a `timing.events` stream.

## Configuration

| Option    | Values                                           | Default |
| --------- | ------------------------------------------------ | ------- |
| `channel` | The line's channel                               | 0       |
| `edges`   | `any`, `rising` or `falling`: the edges reported | `any`   |

## Events (`timing.events` v1)

| Kind   | Fields                    | Meaning                |
| ------ | ------------------------- | ---------------------- |
| `edge` | `slope`, `since_previous` | The line changed level |

| Field            | Meaning                                                                                                              |
| ---------------- | -------------------------------------------------------------------------------------------------------------------- |
| `slope`          | `rising` (low to high) or `falling` (high to low)                                                                    |
| `since_previous` | Ticks since the previous edge the stream reported; `null` for the first edge after the start of the samples or a gap |

With every edge reported, the interval is the time the line spent at a level; with only rising or only falling edges, it is the signal's period. Periods and frequencies in seconds are the reader's to compute, with the source's `rateHz`. An edge covers the single tick where the line has its new level and is settled there.

## Props

| Prop    | Value                              |
| ------- | ---------------------------------- |
| `edges` | `"any"`, `"rising"` or `"falling"` |

The logic input requires `channels` of at least the line's channel plus one.

## Decoding rules

- An edge is a change of level from one sample to the next, placed at the first sample with the new level.
- The first sample of the stream and the first after a gap are never edges, and the first edge reported after either has no interval: the edge before it was not seen.

## Limits

- Every change of level is an edge: a glitch makes two of them.
