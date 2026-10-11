# PWM

The `pwm` decoder measures the cycles of a pulse-width modulated line and writes them to a `pwm.events` stream, in ticks of the source's clock.

## Configuration

| Option     | Values                        | Default       |
| ---------- | ----------------------------- | ------------- |
| `channel`  | The line's channel            | 0             |
| `polarity` | `active-high` or `active-low` | `active-high` |

## Events (`pwm.events` v1)

| Kind    | Fields                         | Meaning                                                             |
| ------- | ------------------------------ | ------------------------------------------------------------------- |
| `cycle` | `period_ticks`, `active_ticks` | One cycle, from a leading edge (inactive to active) to the next one |

| Field          | Meaning                                                                          |
| -------------- | -------------------------------------------------------------------------------- |
| `period_ticks` | Ticks from the leading edge to the next one                                      |
| `active_ticks` | Ticks the line was active: from the leading edge to the trailing edge in between |

Duty cycle (`active_ticks / period_ticks`) and frequency (the source's `rateHz / period_ticks`) are the reader's to compute. A cycle covers its ticks, from its leading edge up to the next one, and is settled at that next leading edge, where it became certain.

## Props

| Prop       | Value                             |
| ---------- | --------------------------------- |
| `polarity` | `"active-high"` or `"active-low"` |

The logic input requires `channels` of at least the line's channel plus one.

## Decoding rules

- The leading edge is the change from inactive to active, the trailing edge the change back.
- A cycle is reported once the next leading edge is seen.
- The first sample of the stream and the first after a gap are never edges: the first cycle begins at the first leading edge after them.
- A cycle cut short by a gap or by the end of the samples is not reported. A line that stays at one level has no cycles.

## Limits

- Every change of level is an edge: a glitch makes cycles of its own.
