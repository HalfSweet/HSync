# HSync interface design

This document specifies how the HSync interface looks and behaves: layout, design tokens, the appearance and behaviour of each area, accessibility and motion. The interface follows this document; the `.hsync/` mockups are only a reference. Update this document whenever the interface changes.

## 1. Principles

- **Grouped by decoder**: a channel sits under the decoder it is bound to; unbound channels go into "Other channels", which collapses as a whole.
- **One job per colour**: decode blocks use the bus colour, traces use the channel colour; red and amber are only for errors and warnings.
- **Decoding lines up with time**: decode blocks and traces share one timeline; the complete results go into the data stream panel, one decoder at a time.
- **Know where you are**: an overview strip, plus a scale with the trigger point at zero.
- **Triggers set per channel**: each channel has one trigger slot, and its condition is drawn on its own row.
- **Single window**: no side panels, no modes to switch into first.
- **Flat**: no gradients, shadows or glows, and no outline around buttons, fields, menus, popovers or dialogs; hierarchy comes only from fill shades, and 1 px lines only divide.

## 2. Layout

From top to bottom: top bar, overview strip, timeline (ruler plus grouped rows, scrolling vertically), data stream panel (opens and closes), status bar.

```
┌ Top bar ──────────────────────────────────────────────────────────────────────────────────────────────┐
│ ⎍ [Device ▾] [25 MHz | 5.24 ms | 16 ch ▾] [⎍ Trigger Not set ▾] [▶ Capture|▾] … [🔍 Search /] [⤓] [⚙] │
├ Overview strip (58px) ────────────────────────────────────────────────────────────────────────────────┤
├ Label column 204px ─┬ Ruler (38px, sticky) ───────────────────────────────────────────────────────────┤
│ I²C group header    │ Decode blocks (bytes)                                                           │
│   SDA  D0        ▢  │ Trace                                                                           │
│ …                   │ …                                                                               │
├ Data stream panel (closed by default, drag to resize, timeline keeps at least 180px) ─────────────────┤
├ Status bar (28px): [≡ Data stream] Capture complete · trigger time · sample count … [− 50 µs/div + ↔] ┤
└───────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

- The app starts without a capture: the overview strip, ruler and timeline stay empty until something is captured or a file is opened. The demo device's data, too, appears only after **Capture**.
- The window is at least 640 px high. As the window narrows, the top bar wraps and the timeline scrolls horizontally (rows are at least 860 px wide); the page itself never shows a horizontal scrollbar.
- At widths ≤ 640 px, the label column shrinks to 132 px and hides the group meta information, the channel numbers, the data stream button and the "Add decoder" text.

## 3. Design tokens

Tokens are CSS custom properties in `src/styles/tokens.css`, mapped to utility classes in the Tailwind theme (`bg-canvas`, `text-ink-2`, `border-line`…); `.theme-dark` and `.density-compact` on the root element switch theme and density. Components use only tokens and never hard-code colours, sizes or durations.

### 3.1 Fills and text

The palette is restrained: fills and text are near-neutral greys with a hint of cool; colour is kept for buses and channels, with lowered chroma and matched lightness, so that no single colour jumps out.

| Token                      | Use                                                            | Light                 | Dark                  |
| -------------------------- | -------------------------------------------------------------- | --------------------- | --------------------- |
| `--canvas`                 | Trace area fill                                                | `#FCFDFE`             | `#0F1112`             |
| `--chrome`                 | Top bar, label column and status bar fill                      | `#F3F5F6`             | `#161718`             |
| `--raise`                  | Dialog fill                                                    | `#FFFFFF`             | `#1E1F21`             |
| `--menu`                   | Menu and popover fill, a shade apart from what lies behind     | `#E9EBED`             | `#252729`             |
| `--control`                | Button and field fill                                          | ink 6%                | ink 7%                |
| `--control-hover`          | Button and field fill under the pointer                        | ink 10%               | ink 12%               |
| `--control-on`             | Selected option of a segmented control, enabled channel        | `#FFFFFF`             | `#484A4E`             |
| `--line`                   | Dividers                                                       | `#E0E1E3`             | `#242527`             |
| `--line-2`                 | Dividers inside menus, rules between values                    | `#C8CBCD`             | `#37393C`             |
| `--ink`                    | Primary text, selected-state fill                              | `#191B1D`             | `#EAEBEC`             |
| `--ink-2`                  | Secondary text                                                 | `#4D5054`             | `#B9BBBD`             |
| `--ink-3`                  | Caption text                                                   | `#63666B`             | `#96989B`             |
| `--trace`                  | Default trace line                                             | `#3B3D40`             | `#BCBEC0`             |
| `--trace-2`                | Narrow-row trace line, empty trigger slot                      | `#95999C`             | `#67696C`             |
| `--grid`                   | Grid lines                                                     | ink 7%                | ink 6%                |
| `--veil`                   | Segment band                                                   | ink 5%                | ink 6%                |
| `--scrim`                  | Behind dialogs and the settings page, over a 12 px blur        | ink 12%               | black 45%             |
| `--hover`                  | Hover fill                                                     | ink 6%                | ink 8%                |
| `--sel`                    | Selected content in the data stream                            | ink 12%               | ink 18%               |
| `--find`                   | Rows containing search matches                                 | `#FBF4DA`             | `#332E19`             |
| `--find-strong`            | Current match; on the timeline, drawn as a band at 45% opacity | `#EBD696`             | `#5E5225`             |
| `--alert` / `--alert-wash` | Error and warning text / fill                                  | `#AC312C` / `#FEF0EE` | `#F28881` / `#381E1C` |

### 3.2 Bus colours

Main colour / decode block fill / darker fill. Within a theme the four buses share lightness and chroma and differ only in hue:

| Bus  | Light                             | Dark                              |
| ---- | --------------------------------- | --------------------------------- |
| I²C  | `#45658D` / `#EFF5FD` / `#DCE7F6` | `#A0C0EB` / `#1C232D` / `#283649` |
| UART | `#277165` / `#ECF7F5` / `#D6ECE7` | `#8BCDBF` / `#172623` / `#1E3C36` |
| SPI  | `#665A8A` / `#F5F3FC` / `#E7E3F5` | `#C0B5E7` / `#23212C` / `#373247` |
| CAN  | `#845169` / `#FBF1F5` / `#F4E0E8` | `#E1ACC4` / `#2B1F24` / `#452D38` |

Other decoders use a neutral generic bus colour: main colour `--ink-2`, fill `--hover`.

### 3.3 Channel colours

Channel colours cycle through the probe wire colours: black, brown, red, orange, yellow, green, blue, violet; the default index is the channel number mod 8.

| Index | Light colour | Wash opacity | Dark colour | Wash opacity |
| ----- | ------------ | ------------ | ----------- | ------------ |
| 0     | `#1F2224`    | 7%           | `#C8CBCE`   | 10.9%        |
| 1     | `#674832`    | 8.8%         | `#A88467`   | 16.6%        |
| 2     | `#A34949`    | 9.8%         | `#DF7F7C`   | 14.8%        |
| 3     | `#B76935`    | 11.6%        | `#E4A067`   | 12.9%        |
| 4     | `#A79139`    | 13.9%        | `#DDCC79`   | 11%          |
| 5     | `#4B8A67`    | 11.5%        | `#7FC39B`   | 12.7%        |
| 6     | `#467BB0`    | 11.1%        | `#73A9E1`   | 13.8%        |
| 7     | `#79508A`    | 9.8%         | `#B085C3`   | 15.1%        |

- Channel colours keep the hues of the probe wires at lowered chroma; in Light, each has a contrast of at least 3:1 against the canvas.
- The opacities differ so that every colour's high-level wash, laid over the canvas, has the same lightness (OKLab lightness difference: Light 0.046, Dark 0.088); a byte block's opacity is `--bk` times the high-level wash's (Light 2.6, Dark 2).
- Red and orange among the channel colours identify channels; they are not status colours.

### 3.4 Fonts

- Interface text uses IBM Plex Sans; values and decoded content use IBM Plex Mono. Both ship with the app. Chinese fallback: `"PingFang SC", "Hiragino Sans GB", "Microsoft YaHei UI", "Microsoft YaHei", "Noto Sans SC", "Noto Sans CJK SC"`; monospace fallback: `"SF Mono", Menlo, Consolas, "Noto Sans Mono CJK SC"`.
- Base size 13 px, secondary information 12 px, captions and tick labels 11 px, line height 1.35. All numbers use tabular figures (`tabular-nums`).

### 3.5 Sizes

| Item                                                                 | Comfortable | Compact |
| -------------------------------------------------------------------- | ----------- | ------- |
| Label column width `--lab`                                           | 204 px      | 204 px  |
| Decode lane height `--h-lane`                                        | 26 px       | 22 px   |
| Trace row height `--h-tr` (I²C and UART raw traces)                  | 30 px       | 24 px   |
| Narrow trace row height `--h-trc` (SPI lines, CAN, "Other channels") | 18 px       | 14 px   |
| Vertical padding inside a group `--gap`                              | 8 px        | 4 px    |

Other sizes: top bar controls are 32 px high with a 6 px radius; the overview strip is 58 px high; the ruler 38 px; the status bar 28 px; decode blocks are inset 2 px at top and bottom, with a 3 px radius.

### 3.6 Motion

| Token                             | Value                                      | Use                                              |
| --------------------------------- | ------------------------------------------ | ------------------------------------------------ |
| `--motion-enter` / `--ease-enter` | 200 ms, `cubic-bezier(0.16, 1, 0.3, 1)`    | Enter: starts fast, ends gently                  |
| `--motion-exit` / `--ease-exit`   | 120 ms, `cubic-bezier(0.4, 0, 1, 1)`       | Exit: quicker than enter, speeding up            |
| `--motion-slide` / `--ease-slide` | 260 ms, `cubic-bezier(0.34, 1.3, 0.64, 1)` | Slide: overshoots by about 3%, then settles back |

When the system asks for reduced motion, all three durations are 0. Tailwind's built-in easing curves are removed; `transition-*` utilities default to the enter duration and curve.

## 4. Top bar

| Control              | Appearance                                                                                | Behaviour                                                                                                                                                                                                                                  |
| -------------------- | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Device               | Button showing the current device name, with a drop-down arrow on the right               | Lists the devices (including Demo, and unsupported devices greyed out) and "Scan again"; below them, "Open file…" (an `.hsync` or `.dsl` capture, or a VCD, FST or GHW waveform, which goes to the import dialog, §11) and "Load session…" |
| Acquisition settings | `sample rate │ duration │ N ch`, separated by 1×14 px vertical rules                      | Popover: sample rate, duration, enabled channels and the device's other options                                                                                                                                                            |
| Trigger              | Icon + "Trigger" + summary                                                                | Without conditions it reads "Not set"; with conditions, "SDA ⟨glyph⟩ + SCL ⟨glyph⟩", up to three; with more, the first two plus "and N channels". A click opens the trigger panel (§7.2)                                                   |
| Capture              | Primary button (`--ink` fill, `--canvas` text) + 28 px drop-down on the right             | Capture / Stop; drop-down: Single, Repeat, Loop, plus Instant                                                                                                                                                                              |
| Search box           | 220–340 px; placeholder "Search values or text, e.g. 4F, HOST"; `/` key hint on the right | See §8                                                                                                                                                                                                                                     |
| Export               | 32×32 icon button                                                                         | "Save capture…", "Export…", "Save session…"                                                                                                                                                                                                |
| Settings             | 32×32 icon button; when an update is available, a 6 px `--ink` dot at the top right       | Opens the settings page (§11); ⌘/Ctrl+, opens and closes it as well; when an update is available, the button is named "Settings (version x available)"                                                                                     |

**Acquisition settings**: durations step down the 1-2-5 sequence from the longest available value until fewer than 1024 samples remain; durations beyond the device memory are marked "RLE", and choosing one turns on RLE compression. On opening, both lists scroll to the current values. When the chosen capture would take more than half of this machine's memory, the popover states below the sample count, in the alert colour, how much it needs; the capture is not blocked. The device's other options (operation mode, channel mode, threshold, filter, RLE, clock and so on) form a column generated from what the device provides (switches, drop-downs, numbers): to the right of the lists in windows at least 1024 px wide (the popover grows to 640 px), below them under a rule in narrower ones, where the popover scrolls. During a capture everything is read-only, with an explanation at the top.

**Capture modes**:

- **Repeat**: interval 0.1–10 s in steps of 0.1 s, default 1 s, set in the drop-down menu; while waiting for the next capture, the status bar reads "Capturing again in N s". Every later capture runs in the background: the screen keeps the previous complete result, the status bar shows progress, and the new result replaces it whole when done; stopping midway discards that capture unless "Show the latest data when a repeat stops" is on.
- **Loop**: selectable only when the device supports it; otherwise greyed out, with the reason given.
- The next capture on the same device keeps the zoom and position; opening a file or Demo zooms to fit.

## 5. Overview strip

- 58 px high, showing the whole capture; each decoder has a 5 px band marking the stretches where it has data, in its bus colour at opacity 0.62.
- The current viewport is drawn as a 1.5 px `--ink` frame with a 4 px radius.
- Ticks are in milliseconds or an adaptive unit; the origin is labelled "Trigger", and the last tick carries the unit ("5 ms").
- A click moves the viewport's centre there; dragging the viewport frame pans; the wheel zooms.
- Without a capture the strip shows only its label: no duration, viewport frame or ticks.
- A hovered data stream item is marked with a 14% `--ink` fill between 1 px `--ink` lines on both sides (at least 2 px wide); the current search match is marked in `--find-strong`.

## 6. Timeline

### 6.1 Ruler and groups

- **Ruler**: sticky at the top, with the unit in the bottom-left corner ("ms", relative to the trigger point). Major ticks carry numbers, minor ticks do not; steps follow the 1-2-5 sequence. The label column stays still during horizontal scrolling. Without a capture it shows no unit or ticks, and "Add decoder" is disabled.
- **Groups**: each group has one group header row (`--h-lane` high), then its decode lanes, then its trace rows. The group header holds an expand arrow, an 8 px square in the bus colour, the group name (bold), the "Decoder settings" and "Show all decoded results in the data stream" buttons that appear only on hover, and meta information on the right ("100 kHz", "115200 8N1", with an explanatory `title`). Groups are separated by a 1 px divider plus `--gap` padding. The "Other channels" group uses a hollow square with a 1 px outline and shows the channel count on the right.
- **Channel rows**: the label column holds, in order, a 7 px dot in the channel colour, the name, the channel number (`D0`, 11 px monospace, `--ink-3`) and the trigger slot. Narrow rows are for SPI lines, CAN and "Other channels". Channels in a decoder group show their role (SDA, RX…); a channel with a name of its own shows the name, and the role moves before the channel number (`SCL D1`).
- **Channel settings**: clicking the dot, double-clicking the name or right-clicking opens them; they change the name and colour, and the chosen colour sits on a `--sel` disc. The name is saved when the settings close (Enter or a click elsewhere), and Esc discards it; a colour takes effect as soon as it is clicked.

### 6.2 Traces

- Lines are 1 CSS pixel wide, with positions rounded to device pixels so they stay sharp.
- Below a high level lies a wash in the channel colour (§3.3), from the top edge of the high line to the bottom edge of the low line.
- Where transitions are too dense to tell apart (such as a zoomed-out SPI clock), the stretch is drawn as a byte block: the high and low level lines and both ends are drawn, and the inside is filled with the block colour; edges less than 3 CSS pixels apart join into one block. SPI data lines reuse the blocks of the clock line in the same group and draw a block only where they change within a byte.
- A line whose blocks in the viewport add up to 8 CSS pixels or more counts as "busy": it draws only lines and blocks, without the high-level wash, so that a bus that idles high does not blur into one band of wash when zoomed out.

### 6.3 Decode blocks

- Absolutely positioned within the lane, inset 2 px at top and bottom, 3 px radius, padding 0 8 px, 12 px text; bus fill with a border in the darker fill, which turns `--ink-3` on hover.
- Blocks narrower than 3.2% of the viewport carry no text; a side cut off by the viewport edge loses its rounded corners and border.
- A selected block is inverted (`--ink` fill, `--canvas` text). Clicking a block makes it the segment and switches the data stream to its decoder; a second click cancels.
- Decode blocks show only what the bus carried and never guess what it meant: no register reads or writes, no bytes taken for text, no folded runs of a repeated word. For protocol-level meaning, stack the matching decoder (such as the 24xx EEPROM on I²C).
- **One block per event, whatever the protocol**: a decoder puts out a stream of events, each of a kind with named fields (an I²C `address` with its address, direction and acknowledge; a UART `frame` with its value; a CAN `id`…). A block carries the kind's name (11 px, bus colour) and the event's main value (monospace, 500): the first number or byte array that is not a counter, else its first text; numbers show in the decoder's number format (hex by default), byte arrays as bytes. Its tip lists every field. Events too narrow to tell apart at the scale (under two pixels, closer than that) pack into one dense block without text, whose tip says how many it holds.
- A decoder's first stream sits on the group's title row; a decoder stacked on it (the EEPROM on I²C) gets a lane of its own below, named after it in the label column.
- Blocks fill in as the capture is taken: decoding follows the samples, so a running capture already shows what has been decoded.

### 6.4 Hover readout

- When the pointer rests on a trace row, a 13% `--ink` layer covers the pulse under it, and the status bar reads it out (§10): the channel's colour dot and name, then the level and width of the pulse ("High 9.8 µs"). It adds the period, frequency and duty cycle the pulse makes with the next one, except on decoder data lines (SDA, MOSI, MISO, CS, CAN RX), which show the pulse alone, and UART lines, which add "≈ N bits" instead. When the viewport cuts the pulse off, the readout shows "≥" and notes "Extends beyond view".
- Nothing but that layer is drawn over the rows, so the readout never hides another channel's waveform.

### 6.5 Edge measurement

- Clicking an edge on a trace row starts a measurement: it takes the nearest edge within 10 px of the pointer, exact to the sample. The end point follows the pointer and snaps to an edge within 10 px on the pointer's row (which may be another channel).
- Clicking another edge completes the measurement, and the result stays on the chart; clicking a further edge starts a new measurement, and clicking empty space or pressing Esc clears it. While measuring, clicking empty space, clicking the start point again or pressing Esc cancels it.
- Each end has a 10 px dot (`--ink` fill, 1.5 px `--canvas` outline), and a 1 px `--ink` horizontal line joins them along the centre line of the end row; when the ends are on different rows, a dashed line drops from the start point to the end row. An end point that has not snapped is drawn as a vertical line the height of the row.
- The status bar reads out the measurement (§10): "Δt 9.80 µs" plus "245 samples · 1/Δt 102.0 kHz", with the time exact to one sample (at most 3 decimals). Until the end point lands on another edge, it prompts "Click another edge to finish, Esc to cancel".
- When not measuring, an 11 px hollow ring appears as the pointer nears an edge, showing that a measurement can start there; while measuring, the cursor is a crosshair and the hover readout pauses.
- Dragging, the wheel and the keyboard pan and zoom as usual, and the measurement stays in place; a drag of more than 3 px is not a click. A new capture clears the measurement.

### 6.6 Segment

- Drag on the ruler to mark a segment: its ends carry two 14 px flags, A and B (`--ink` fill), with an arrowed horizontal line between them labelled with the duration; `--veil` covers the whole segment on every row.
- The flags sit outside the segment and turn inwards near the viewport edges; when the segment is narrower than 9% of the viewport, the duration sits beside B (beside A when B is near the right edge); when one end of the segment lies outside the viewport, the duration reads "Extends beyond view".
- A drag shorter than 2 µs counts as a click; a click on the ruler cancels the segment.
- While a segment exists, the status bar shows its duration; clicking it lists each channel's rising edges, falling edges and edge count within the segment.

## 7. Trigger

### 7.1 Per channel

- At the far right of the label column is the "Trig" column (the ruler row reads "Trig", with a tooltip). Each channel row has a 20 × 16 px trigger slot (`min(16px, --h-trc − 2px)` high on narrow rows); group header rows have none.
- Without a condition, the slot is a short dash (`--trace-2`); with one, it is a small `--ink` tile holding the condition glyph in `--canvas`. The hover fill is `--hover`, and an empty slot keeps it while its menu is open.
- Clicking a trigger slot opens a 176 px wide menu to the right of the label column, level with the row. Its title is "Trigger on XX" (with the channel-colour dot); its six items are, in order, None, Rising edge, Falling edge, Any edge, High level and Low level, with a check mark on the current one; at the bottom is the note "With conditions on several channels, all must hold at once". For rows near the bottom, the menu moves up until it fits.
- The menu closes when an item is chosen, on Esc, on a press outside the menu, when the timeline scrolls, or when the same trigger slot is clicked again. When it is opened from the keyboard, focus lands on the current item and the up and down arrow keys move it; on closing, focus returns to the trigger slot.
- Disabled channels cannot have a trigger; their trigger slots are greyed out.
- Condition glyphs are drawn in a 16 × 16 box, with 1.5 px lines and round caps; the high side gets a flat 26% fill:

| Condition    | Line                                | Fill                 |
| ------------ | ----------------------------------- | -------------------- |
| None         | `M6 8h4`                            | none                 |
| Rising edge  | `M2.5 11.5H8v-7h5.5`                | `M8 4.5h5.5v7H8z`    |
| Falling edge | `M2.5 4.5H8v7h5.5`                  | `M2.5 4.5H8v7H2.5z`  |
| Any edge     | `M2.5 4.5h3l5 7h3M2.5 11.5h3l5-7h3` | none                 |
| High level   | `M2.5 4.5h11`                       | `M2.5 4.5h11v7h-11z` |
| Low level    | `M2.5 11.5h11`                      | none                 |

### 7.2 Trigger panel

The top bar's "Trigger" button opens a 480 px wide panel:

- **Type**: Simple / Stages / Serial. Stages and Serial are selectable only when the device supports them; otherwise they are greyed out, and a `title` gives the reason.
- **Position**: a slider from 1% to the maximum the device allows; a maximum below 90% is noted, and when only 1% is possible the slider is disabled with an explanation.
- **Simple**: the note "Set conditions in each channel row; all must hold at once", the list of conditions set, and "Clear all conditions".
- **Stages**: a stage count of 1–16 and stage tabs; each stage has two patterns, A and B, each a row of channel cells plus "Invert"; the combination "A and B / A or B"; a count (≥ 1) and "In a row".
- **Serial**: three rows of cells for Start, Stop and Clock; the data channel; a bit count of 1–16 (default 8); the data value (high bit on the left, 0 / 1 / X) with its hex conversion.
- **Channel cells**: one cell per channel, with the channel number above and the condition glyph from §7.1 inside; a click steps through the conditions in menu order, a right-click steps backwards, typing X 0 1 R F C sets one directly, and the left and right arrow keys move between cells.
- While a stage or serial trigger is in effect, the summary on the button changes to "N stages" or "serial · SDA · 8 bits", the trigger slots on the timeline drop to 40% opacity, and their `title` explains that the row conditions do not apply for now.

## 8. Search

- Top bar search box: `/` focuses it (except from inside a text field); Esc clears it and removes focus.
- **Decoded data**: a number (decimal, or hexadecimal with `0x`: `79`, `0x4F`) finds the events with a field of that value; two or more numbers in a row (`0x4F 0x32`) find the events whose byte array holds them one after the other, such as the bytes of a frame; any other word (`frame`, `address`, `stop`) finds the events of that kind, or with an enumerated field of that name. Every word must match. Field names never match.
- **Level pattern**: when every word has the form `channel=code` (such as `D3=R D5=1`), the search matches levels. A channel is written as `D3`, `3`, the channel name or the role name (`SDA`); the codes are X 0 1 R F C. A channel that does not exist is flagged inside the box in `--alert`.
- The search starts 150 ms after input, and the right end of the box shows "N found" ("10000+ found" when there are too many). Enter jumps forwards from the start of the viewport, and Shift+Enter backwards from its end, to the first match; after that, they cycle through the matches, showing "3 of 17".
- On each jump: a match outside the viewport is panned to the centre, and one wider than 80% of the viewport is zoomed to leave 10% on each side; a `--find-strong` band covers it on the timeline, and the overview strip marks its position; the data stream switches to its decoder (without opening automatically) and, with the range at "All", scrolls to its row; rows containing matches get a `--find` fill, and the current row a `--find-strong` fill.
- A level pattern finds one match at a time: the timeline draws a 2 px `--ink` vertical line at that sample and the box shows its time, or "No more" when there is none. With only level conditions, a continuous stretch that satisfies them counts as one match.

## 9. Data stream panel

- **Opening and closing**: the panel sits below the timeline and is closed by default; it slides in from, and out to, the bottom edge of the workspace. It opens from the "Data stream" button at the left end of the status bar, and from the button that appears on hover in a group header (which opens that decoder; a second click closes it). A segment does not open it automatically. Its top edge can be dragged to change its height (minimum 80 px), and the timeline keeps at least 180 px.
- **Header** (33 px): on the left, one tab per stream of every decoder — named after the decoder, and after the stream too ("I²C › 24xx EEPROM") when the decoder has several; the current tab has `--ink` text and a 2 px `--ink` line beneath, which slides under the new tab on switching; then a one-line summary ("47 events", "47 events, 12 in the segment"). On the right, in order: "All / Segment", Copy, Export (CSV / text) and Close. Segmented controls are 22 px high; the selected option has a `--control-on` fill.
- **List**: 12 px monospace, 20 px rows, fixed header; one row per event: the time (ms, four decimals, relative to the trigger point), the event's kind, then its fields as "label: value" pairs in the contract's order, values in the decoder's number format. The list grows as a running capture decodes.
- **With a segment**: with the range at "All", items lying wholly inside the segment get a `--sel` fill, and the list scrolls to the first of them (leaving one row above it; not while dragging, only on release). With the range at "Segment", only those items are listed, and a transfer partly inside the segment shows "…" on its missing side; without a segment this option is unavailable, with the hint "Drag along the ruler to select a segment first". Clicking a decode block on the timeline switches the list to its decoder and marks its entry.
- **With the traces**: hovering a list item marks its position on the overview strip; when it lies in the viewport, an 8% `--ink` band (1 px `--ink-3` on both sides) covers it on the timeline, and its decode block gets a 2 px `--ink` frame. Clicking a list item pans the traces there and selects it.

## 10. Status bar and navigation

- From the left: the "Data stream" toggle (`--hover` fill when pressed), the status ("Capture complete", "Waiting for trigger", "Captured 37%"…), save, export or import progress (cancellable; reading a waveform file for the import dialog is not), an error or "Saved to …", the trigger time, the sample count ("131,072 samples") and the segment duration (§6.6), then the edge measurement (§6.5) and the pulse under the pointer (§6.4). The two readouts take the room left before the right-hand items and end in an ellipsis rather than wrap the bar, so the timeline keeps its height; their values are `--ink` at medium weight and the details `--ink-3`.
- On the right: the warning about a too-slow port, and "Switch / Dismiss" for a newly plugged-in device; Zoom out, the current time base ("50 µs/div", at least 64 px wide, centred), Zoom in and Zoom to fit.
- **Pointer**: Ctrl/⌘ + wheel zooms around the pointer; Shift + wheel or a horizontal trackpad swipe pans; dragging in the trace area pans, and clicking an edge starts an edge measurement (§6.5).
- **Keyboard**: `+` / `-` zoom, `←` / `→` pan, `Home` zooms to fit, `/` searches, ⌘/Ctrl+O opens, ⌘/Ctrl+S saves, ⌘/Ctrl+, opens the settings, and `Esc` closes a menu, cancels an edge measurement and cancels the segment, in that order.

## 11. Dialogs and the settings page

- Shared style: the interface behind is blurred by `--veil-blur` (12 px) and dimmed by `--scrim`; over it a panel is centred (`--raise` fill, 6 px radius) without an outline, set apart from the interface by shade alone, with a 40 px title bar and a close button on the right. Focus moves into the panel when it opens; Esc or a click on the backdrop closes it.
- **Add / edit decoder**: a searchable list of the protocols that read samples (name and one-line summary, buses before signal lines), a "Stack on it" select of the decoders that read the chosen protocol's output (the 24xx EEPROM on I²C, SPI NOR on SPI…), an optional name, a select per channel the protocol reads (channel lists are typed as numbers, least significant first), the options of each layer as the decoder declares them (numbers with their bounds, choices, switches, text), and the number base under Display; when editing, the protocol is fixed and the decoder can be removed. Decoders can be added and changed while a capture runs.
- **Export**: format (HSync capture, DSView capture, CSV table, VCD waveform, FST waveform, sigrok session, Raw binary, each with a one-line description; CSV can write one row per sample), range (Whole capture / Segment) and channels (All / None); the save location is chosen after confirming.
- **Import waveform**: opens once a VCD, FST or GHW file chosen with "Open file…" has been read (the status bar shows "Reading N%" meanwhile); the title bar names the file. The body starts with the format and "All" / "None", then a search box, then the file's scopes and signals as a tree in a `--control` box: 24 px rows indented 16 px per level, scopes with a fold chevron and a checkbox that reads all, some or none of their signals, logic signals with a checkbox, their declared range (`[7:0]`) and width ("8 bits"), and reals, strings and events in `--ink-3` marked "no logic level". Searching lists the matching signals with the scopes they lie in. Below the tree, a "Sample rate" field holds the rate the timestamps give (the coarsest grid they all fall on), which the user may replace with the clock the file was sampled at; "From the timestamps" beside it goes back to the given rate, and a value that is no frequency is refused in the alert colour. One line then gives the sample rate, sample count, channel count (one per bit) and memory the import makes at that rate; when timestamps fall between samples, it says that changes closer together than one sample fall together, and an import needing more than half of this machine's memory warns in the alert colour. Every signal is chosen from the start when the file holds no more than 32 bits, otherwise none. The footer holds Cancel and Import; while importing, it shows "Importing N%" with a progress bar, and Cancel stops the import, leaving the dialog and the open capture as they were. The capture that opens counts as the waveform file: the window title and the device button name it, and saving suggests its name with `.hsync`.
- **Settings page**: 760 × 560 px, shrinking in small windows. A 176 px side bar on the left (`--chrome` fill) lists Appearance, Behaviour and Diagnostics, with About set apart at its foot, each with an icon before its name; the selected section has a `--control-on` fill like a segmented control's, which slides to the next section. When an update is available, a `--ink` dot sits at the right of About, and its tip and screen reader text read "Version x available". The current section fills the right, labels on the left and controls on the right; each label and each checkbox's text has an `--ink-3` icon before it, and the buttons carry an icon before their text:
  - **Appearance**: Theme (System / Light / Dark, each with an icon), Density (Comfortable / Compact), Graphics acceleration (Auto / Off, with the renderer in use named below) and Language (System / `简体中文` / `English`), all segmented controls.
  - **Behaviour**: "Centre the view on the trigger" (on), "Follow the latest data while capturing" (on), "Keep scrolling after a quick drag" (on: when a drag is released faster than 0.5 px/ms, panning continues with exponential decay until a press or the wheel stops it), "Show the latest data when a repeat stops" (off), "Check for updates at startup" (on: checks once in the background after startup, and a failure stays silent). The first three have no effect while zoomed to fit.
  - **Diagnostics**: log level, "Write a log file", "Show in folder".
  - **About**: version, license and component credits. Below the version number is the update area: when no update is known, it offers "Check for updates", with "Checking for updates…" or "HSync is up to date" beside it; with an update, it reads "Version x is available" and offers "Download and restart"; while downloading, it shows a percentage ("Downloading…" when the size is unknown), and once installed the app restarts by itself. On failure, the reason appears below, and the user can try again.
  - Focus moves into the panel without a focus ring on a section; when the keyboard moves onto the sections, the ring is drawn inside them. On closing, focus returns to the button that opened the page; opened again, the page shows the section last viewed.

## 12. Accessibility

- Every icon button has an `aria-label`; the group header's expand button has `aria-expanded`; trigger slots have `aria-haspopup="menu"`; menu items have `role="menuitemradio"` plus `aria-checked`; data stream tabs have `role="tab"`; the settings page's side bar is a vertical `role="tablist"`, where ↑ / ↓ / Home / End move between sections; segmented controls use `aria-pressed`.
- Focus ring: 2 px solid `--ink`, offset 2 px (offset −2 px inside tabs).
- With reduced motion on, motion durations are 0, and the start and end states of transitions do not apply either, so every change completes at once (§13).

## 13. Motion

When the interface changes state, brief motion shows where things come from and where they go, timed by the tokens in §3.6. Motion changes only opacity, translation and scale (except panel height), so it does not compete with waveform rendering for the main thread.

| Situation                                                                                                                                      | Motion                                                                                                                                              |
| ---------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Menus and popovers                                                                                                                             | Grow from the corner of the control that opened them: opacity 0→1, scale 0.96→1; reversed on close                                                  |
| Dialogs and the settings page                                                                                                                  | The backdrop fades in; the panel rises 8 px and scales up from 98%                                                                                  |
| Data stream panel                                                                                                                              | Slides in from, and out to, the bottom edge of the workspace; while moving it covers the timeline, which resizes only once, at the start or the end |
| Segmented controls, tabs, the settings page's side bar                                                                                         | The selected option's fill (the underline for data stream tabs) slides to the new option with a slight bounce                                       |
| Panel content changes (trigger type, stage of a stage trigger, export's CSV option, data stream decoder / mode / range, settings page section) | New content rises 4 px and fades in, and old content disappears at once; the frame's height eases to the new content's height                       |
| List additions and removals (devices, stage tabs of a stage trigger, channels with conditions)                                                 | Items scale and fade in or out in place; the other items slide over to close the gap                                                                |
| Appearing notices (status bar messages, warnings, search match count, trigger summary)                                                         | Rise and fade in                                                                                                                                    |
| Group header expand arrow                                                                                                                      | Rotates with the slide timing                                                                                                                       |

- Elements that are leaving no longer respond to the pointer.
- No motion for: anything that follows the pointer (hover readout, edge measurement, segment dragging, panning and zooming); viewport jumps; theme, density and language switches (the canvas redraws in a single frame, so interface colours that transition slowly would fall out of step with it); expanding and collapsing channel groups (the rows are drawn on the canvas).

## 14. Copy and numbers

- The interface supports Simplified Chinese (default) and English, following the system or chosen in Settings. Chinese copy puts clarity first; English copy means the same as the Chinese and has a similar length, so it does not break the layout.
- Sentences are translated whole, as messages with parameters; strings are never concatenated. Decoders' own English annotations are shown as they are.
- Numbers and dates are formatted for the current language: Chinese groups sample counts with narrow spaces (`131 072`), English with commas (`131,072`). Time and frequency units (ns, µs, ms, s, Hz, kHz, MHz) are the same in both languages, with one space between value and unit (`5.24 ms`).
