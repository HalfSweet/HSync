# HSync interface design

This document specifies how the HSync interface looks and behaves: layout, design tokens, the appearance and behaviour of each area, accessibility and motion. The interface follows this document; the `.hsync/` mockups are only a reference. Update this document whenever the interface changes.

## 1. Principles

- **Grouped by decoder**: a channel sits under the decoder it is bound to; unbound channels go into "Other channels", which collapses as a whole.
- **One job per colour**: a decode block's colour says what the event is (its role, the same in every protocol), traces use the channel colour; red and amber are only for errors and warnings.
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
| `--float`                  | Box under the readouts floating over the waveforms             | `--menu` 70%          | `--menu` 70%          |
| `--find`                   | Rows containing search matches                                 | `#FBF4DA`             | `#332E19`             |
| `--find-strong`            | Current match; on the timeline, drawn as a band at 45% opacity | `#EBD696`             | `#5E5225`             |
| `--alert` / `--alert-wash` | Error and warning text / fill                                  | `#AC312C` / `#FEF0EE` | `#F28881` / `#381E1C` |

### 3.2 Event roles

Every kind of event has a role, and a decode block takes the colour of its event's role, whatever the protocol: an I²C address and a CAN identifier look alike, and a transaction's structure shows even where no text fits. Which decoder a block belongs to is told by its lane, under the decoder's group header.

| Role        | What it is                                                       | Examples                                                                   |
| ----------- | ---------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `framing`   | Where a transaction, frame or scan begins or ends                | START, STOP, repeated START, start and end of a CAN frame, dummy bytes     |
| `address`   | Whom the traffic is for                                          | I²C address, CAN identifier, LIN header, USB token, SPI NOR address        |
| `command`   | What is asked                                                    | SPI NOR command, SWD request, USB SETUP request, JTAG instruction shift    |
| `control`   | What a frame says of itself                                      | CAN control field, USB PD header                                           |
| `data`      | What the traffic carries                                         | Data bytes, UART characters, JTAG data shift, status register values       |
| `check`     | A check over the traffic                                         | CRC, checksum                                                              |
| `handshake` | The other side's answer                                          | CAN acknowledge, SWD acknowledge, USB handshake, SD response               |
| `line`      | A condition of the line itself                                   | Line reset, break, JTAG reset, suspend and resume                          |
| `error`     | Something the decoder reports as wrong, or a field value that is | Errors, unknown commands, a wrong parity bit or CRC, an unanswered address |

Each role has a mark colour (`--role-<role>`: blocks without room for text, packed runs, the overview strip), a fill (`--role-<role>-fill`: blocks with text) and a deeper fill (`--role-<role>-deep`: the fill's border and emphasised blocks):

| Role        | Light: mark / fill / deep         | Dark: mark / fill / deep          |
| ----------- | --------------------------------- | --------------------------------- |
| `framing`   | `#7E8082` / `--hover` / `--sel`   | `#BCBEC0` / `--hover` / `--sel`   |
| `address`   | `#356BAE` / `#C8E0FF` / `#A9CDFB` | `#4C7FC0` / `#1E3149` / `#254369` |
| `command`   | `#937ECF` / `#E7E1FE` / `#D5C9FE` | `#A08DDA` / `#38314D` / `#4B3F6C` |
| `control`   | `#247D46` / `#C6E7CD` / `#A6D9B2` | `#41905B` / `#1B3723` / `#1E4D2D` |
| `data`      | `#0CA397` / `#C1F1E9` / `#98E2D8` | `#26B2A5` / `#103E39` / `#05544D` |
| `check`     | `#8F4E8C` / `#F1D2EE` / `#E6B9E2` | `#A2639E` / `#3E263C` / `#573255` |
| `handshake` | `#929430` / `#E7E9C3` / `#D4D79D` | `#A0A147` / `#373818` / `#4B4B14` |
| `line`      | ink 20% / `--hover` / `--sel`     | ink 20% / `--hover` / `--sel`     |
| `error`     | `--alert` / `--alert-wash`        | `--alert` / `--alert-wash`        |

- The six hues (OKLCH 255°, 295°, 152°, 185°, 329°, 110°) share their chroma and two lightness steps, and keep clear of red, amber and the search yellow. Neighbouring roles — those that follow one another on a real bus, such as CAN's identifier, control field, data, CRC and acknowledge — differ by an OKLab ΔE of at least 12, and at least 7 for protanopia and deuteranopia; the gaps between blocks and their text do the rest.
- `line` takes no hue: its blocks carry diagonal stripes (ink 62% on marks, 24% on fills, 1.5 px every 4 px at 135°), which read in every kind of colour vision.
- Errors are red and also differ in shape (§6.3), since red and green can look alike.
- Text on a fill is `--ink` (values) and `--ink-2` (kind names), above 5.9:1 in both themes; an error's text is `--alert`.
- Decoder group squares and the data stream's tabs are neutral (`--ink-2`), so that no colour means a decoder.

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

| Token                             | Value                                      | Use                                                         |
| --------------------------------- | ------------------------------------------ | ----------------------------------------------------------- |
| `--motion-enter` / `--ease-enter` | 200 ms, `cubic-bezier(0.16, 1, 0.3, 1)`    | Enter: starts fast, ends gently                             |
| `--motion-exit` / `--ease-exit`   | 120 ms, `cubic-bezier(0.4, 0, 1, 1)`       | Exit: quicker than enter, speeding up                       |
| `--motion-slide` / `--ease-slide` | 260 ms, `cubic-bezier(0.34, 1.3, 0.64, 1)` | Slide: overshoots by about 3%, then settles back            |
| `--motion-view`                   | 100 ms, quadratic ease-out, `1 − (1 − t)²` | Glide: the timeline's view moving to a new zoom or position |

When the system asks for reduced motion, all four durations are 0. Tailwind's built-in easing curves are removed; `transition-*` utilities default to the enter duration and curve.

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

The SLogic16U3 uses the same capability-driven controls: Stream mode and a 0–6 V threshold (1.7 V initially). Its sample-rate list follows the highest enabled channel: D0–D3, D0–D7 or D0–D15, with respective USB 3.0 limits of 800, 400 or 200 MHz (80, 40 or 20 MHz on USB 2.0). Changing the channel selection updates the available rates. It supports Loop and simple software triggers; unsupported trigger types remain disabled.

**Capture modes**:

- **Repeat**: interval 0.1–10 s in steps of 0.1 s, default 1 s, set in the drop-down menu; while waiting for the next capture, the status bar reads "Capturing again in N s". Every later capture runs in the background: the screen keeps the previous complete result, the status bar shows progress, and the new result replaces it whole when done; stopping midway discards that capture unless "Show the latest data when a repeat stops" is on.
- **Loop**: selectable only when the device supports it; otherwise greyed out, with the reason given.
- The next capture on the same device keeps the zoom and position; opening a file or Demo zooms to fit.

## 5. Overview strip

- 58 px high, showing the whole capture; each decoder has a 5 px band marking the stretches where it has data, in columns of one strip pixel each in the mark colour of the role most of its events have (§3.2), at opacity 0.62. Errors are drawn over the band at full opacity, at least 2 px wide, so that they can be found over the whole capture.
- The current viewport is drawn as a 1.5 px `--ink` frame with a 4 px radius.
- Ticks are in milliseconds or an adaptive unit; the origin is labelled "Trigger", and the last tick carries the unit ("5 ms").
- A click moves the viewport's centre there; dragging the viewport frame pans; the wheel zooms.
- Without a capture the strip shows only its label: no duration, viewport frame or ticks.
- A hovered data stream item is marked with a 14% `--ink` fill between 1 px `--ink` lines on both sides (at least 2 px wide); the current search match is marked in `--find-strong`.

## 6. Timeline

### 6.1 Ruler and groups

- **Ruler**: sticky at the top, with the unit in the bottom-left corner ("ms", relative to the trigger point). Major ticks carry numbers, minor ticks do not; steps follow the 1-2-5 sequence. The label column stays still during horizontal scrolling. Without a capture it shows no unit or ticks, and "Add decoder" is disabled.
- **Groups**: each group has one group header row (`--h-lane` high), then its decode lanes, then its trace rows. The group header holds an expand arrow, an 8 px `--ink-2` square, the group name (bold), the "Decoder settings" and "Show all decoded results in the data stream" buttons that appear only on hover, and meta information on the right ("100 kHz", "115200 8N1", with an explanatory `title`). Groups are separated by a 1 px divider plus `--gap` padding. The "Other channels" group uses a hollow square with a 1 px outline and shows the channel count on the right.
- **Channel rows**: the label column holds, in order, a 7 px dot in the channel colour, the name, the channel number (`D0`, 11 px monospace, `--ink-3`) and the trigger slot. Narrow rows are for SPI lines, CAN and "Other channels". Channels in a decoder group show their role (SDA, RX…); a channel with a name of its own shows the name, and the role moves before the channel number (`SCL D1`).
- **Row height**: dragging a channel row's bottom edge in the label column (a 5 px band across the boundary with the row below, `row-resize` cursor, a 1 px `--ink-3` line while hovered or dragged) sets that channel's height in whole pixels, from `--h-trc` to 200 px; a double-click on the edge gives the row back its default height. Heights are kept with the channel colours: in sessions, in `.hsync` captures and for each device between runs. Group headers and decode lanes keep their heights.
- **Fitting the window**: with "Fit waveform heights to the window" on (§11), the channel rows grow and shrink with the timeline's height, all scaled by one factor so that they keep their proportions, dragged heights included: they fill the room below the ruler exactly, in whole pixels, growing to at most three times their heights (and 200 px) and shrinking until a default wide row is as low as a narrow one (`--h-trc`); beyond either bound the room is left below, or the rows scroll. Group headers, decode lanes and gaps keep their heights. While the rows fit, a dragged row follows the pointer and the others make room, and the height it ends at is kept as its share of the scale.
- **Channel settings**: clicking the dot, double-clicking the name or right-clicking opens them; they change the name and colour, and the chosen colour sits on a `--sel` disc. The name is saved when the settings close (Enter or a click elsewhere), and Esc discards it; a colour takes effect as soon as it is clicked.

### 6.2 Traces

- Lines are 1 CSS pixel wide, with positions rounded to device pixels so they stay sharp.
- Above the high level and below the low one, a row leaves 6.5/30 of its height (4.5/18 on narrow rows), at most 6.5 px (4.5 px): a row made taller gives the extra height to the swing between the levels.
- Below a high level lies a wash in the channel colour (§3.3), from the top edge of the high line to the bottom edge of the low line.
- Where transitions are too dense to tell apart (such as a zoomed-out SPI clock), the stretch is drawn as a byte block: the high and low level lines and both ends are drawn, and the inside is filled with the block colour; edges less than 3 CSS pixels apart join into one block. SPI data lines reuse the blocks of the clock line in the same group and draw a block only where they change within a byte.
- A line whose blocks in the viewport add up to 8 CSS pixels or more counts as "busy": it draws only lines and blocks, without the high-level wash, so that a bus that idles high does not blur into one band of wash when zoomed out.

### 6.3 Decode blocks

- Absolutely positioned within the lane, inset 2 px at top and bottom, 3 px radius, padding 0 8 px, 12 px text; the fill of the event's role (§3.2) with a border in its deeper fill, which turns `--ink-3` on hover.
- A block shows its text wherever it fits, however small a share of the viewport it is. A block without room for any text takes its role's mark colour edge to edge, without padding or border, so that narrow blocks stay distinct when zoomed out; a side cut off by the viewport edge loses its rounded corners and border.
- A selected block is inverted (`--ink` fill, `--canvas` text). Clicking a block makes it the segment and switches the data stream to its decoder; a second click cancels.
- Decode blocks show only what the bus carried and never guess what it meant: no register reads or writes, no bytes taken for text, no folded runs of a repeated word. For protocol-level meaning, stack the matching decoder (such as the 24xx EEPROM on I²C).
- **Event blocks, whatever the protocol**: a decoder puts out a stream of events, each of a kind with named fields (an I²C `address` with its address, direction and acknowledge; a UART `frame` with its value; a CAN `id`…). A block carries the kind's name (11 px, `--ink-2`) and up to two main values (monospace, 500, `--ink`): bus numbers and byte arrays first, measurements if there are no bus values, then its first text, then a counter. Scan identifiers, offsets and bit counts must not displace actual data, such as JTAG's TDI and TDO. Numbers show in the decoder's number format (hex by default), quantities in decimal, byte arrays as bytes. Its tip and accessible name list every field. Events with exactly the same span share a block whose tip lists each event, so coincident state changes and scan boundaries do not cover one another. Individual markers stay at least two pixels wide, including events at a single sample or with no duration. Events too narrow to tell apart at the scale (under two pixels, closer than that) pack into one dense block without text, whose tip says how many it holds; zooming in separates them. A dense block is painted in columns of two pixels, each in the mark colour of the role covering most of it.
- **Errors**: an event whose kind is an error, or whose field values are ones its contract defines as wrong (a wrong parity bit, a CRC that does not match, an address no device acknowledged), shows in the `error` role. Errors pack only with errors, never into a run of other events; one too narrow for text spans the lane's full height and stays at least 3 px wide, at every scale. Values a normal exchange can have are never errors: the NACK that ends an I²C read is not one.
- A decoder's first stream sits on the group's title row; a decoder stacked on it (the EEPROM on I²C) gets a lane of its own below, named after it in the label column.
- **JTAG states**: every stream with the JTAG event schema, including cJTAG and selected chain outputs, has a separate **TAP states** lane immediately below its data lane. States take the role of their column of the TAP state diagram: Test-Logic-Reset and Run-Test/Idle `framing`, the instruction register's states `command`, the data register's `data`; Shift-IR and Shift-DR take the deeper fill. On the data lane, a scan's begin and end markers and its shifts take the role of the register it shifts, so that even a thin marker tells an instruction scan from a data scan. A state occupies the time from entering it until the next state change, capped at a gap or the decoder's known progress; it remains visible when the viewport begins midway through it. The data lane keeps shift data and scan boundary markers, so data and states never cover one another. States use localized names (English uses the standard TAP names, such as `Shift-IR`; Chinese names the operation and register), without a redundant event-kind prefix. State blocks use 4 px horizontal padding; narrow names ellipsize from 40 px onward, and smaller intervals are markers. The tip gives the complete name, both endpoints and duration. Clicking selects the whole interval. The data stream continues to list the original instantaneous state-change events.
- Blocks fill in as the capture is taken: decoding follows the samples, so a running capture already shows what has been decoded.

### 6.4 Hover readout

- When the pointer rests on a trace row, a 13% `--ink` layer covers the pulse under it, and the status bar reads it out (§10): the channel's colour dot and name, then the level and width of the pulse ("High 9.8 µs"). It adds the period, frequency and duty cycle the pulse makes with the next one, except on decoder data lines (SDA, MOSI, MISO, CS, CAN RX), which show the pulse alone, and UART lines, which add "≈ N bits" instead. When the viewport cuts the pulse off, the readout shows "≥" and notes "Extends beyond view".
- With **Readout position** set to Floating (§11), the readout floats over the rows instead of going to the status bar: one line against the top edge of the row, 12 px right of the pointer (left of it near the right edge), in a box with a 4 px radius and a `--float` fill, set apart from the waveforms while they still show through it, the level and width `--ink` at medium weight and the details `--ink-2`. It goes under the row when the ruler leaves no room above it, or when the edge measurement's readout sits over the same row; it stays inside the visible tracks and never takes the pointer.
- Otherwise nothing but that layer is drawn over the rows, so the readout never hides another channel's waveform.

### 6.5 Edge measurement

- Clicking an edge on a trace row starts a measurement: it takes the nearest edge within 10 px of the pointer, exact to the sample. The end point follows the pointer and snaps to an edge within 10 px on the pointer's row (which may be another channel).
- Clicking another edge completes the measurement, and the result stays on the chart; clicking a further edge starts a new measurement, and clicking empty space or pressing Esc clears it. While measuring, clicking empty space, clicking the start point again or pressing Esc cancels it.
- Each end has a 10 px dot (`--ink` fill, 1.5 px `--canvas` outline), and a 1 px `--ink` horizontal line joins them along the centre line of the end row; when the ends are on different rows, a dashed line drops from the start point to the end row. An end point that has not snapped is drawn as a vertical line the height of the row.
- The status bar reads out the measurement (§10): "Δt 9.80 µs" plus "245 samples · 1/Δt 102.0 kHz", with the time exact to one sample (at most 3 decimals). Until the end point lands on another edge, it prompts "Click another edge to finish, Esc to cancel". Floating (§6.4), the readout is centred on the line against the top edge of the end row, on the same `--float` fill, and stays inside the visible tracks.
- When not measuring, an 11 px hollow ring appears as the pointer nears an edge, showing that a measurement can start there; while measuring, the cursor is a crosshair and the hover readout pauses.
- Dragging, the wheel and the keyboard pan and zoom as usual, and the measurement stays in place; a drag of more than 3 px is not a click. A new capture clears the measurement.

### 6.6 Segment

- Drag on the ruler to mark a segment: its ends carry two 14 px flags, A and B (`--ink` fill), with an arrowed horizontal line between them labelled with the duration; `--veil` covers the whole segment on every row. The duration, here and in the status bar, reads in whole nanoseconds below a microsecond ("20 ns"), in microseconds with one decimal below a millisecond ("540.0 µs") and in milliseconds with four decimals above ("1.0460 ms").
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
- SLogic16U3 software triggers allow up to 90%, with at most 64 MiB of pre-trigger history; the capture's trigger marker always uses the actual retained history.
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

- From the left: the "Data stream" toggle (`--hover` fill when pressed), the status ("Capture complete", "Waiting for trigger", "Captured 37%"…), save, export or import progress (cancellable; reading a waveform file for the import dialog is not), an error or "Saved to …", the trigger time, the sample count ("131,072 samples") and the segment duration (§6.6), then the edge measurement (§6.5) and the pulse under the pointer (§6.4), unless they float over the rows (§11). The two readouts take the room left before the right-hand items and end in an ellipsis rather than wrap the bar, so the timeline keeps its height; their values are `--ink` at medium weight and the details `--ink-3`.
- On the right: the warning about a too-slow port, and "Switch / Dismiss" for a newly plugged-in device; Zoom out, the current time base ("50 µs/div", at least 64 px wide, centred), Zoom in and Zoom to fit.
- **Pointer**: Ctrl/⌘ + wheel or a trackpad pinch zooms around the pointer, and so does the wheel alone with "Zoom the waveform with the wheel alone" on (§11), except over the label column, where it still scrolls the rows; Shift + wheel or a horizontal trackpad swipe pans; dragging in the trace area pans, and clicking an edge starts an edge measurement (§6.5). A wheel that turns in notches zooms by √2 a notch (two notches make the 2× of the zoom keys) and the view glides there (§13); a trackpad, a pinch or a wheel that turns smoothly zooms by how far it moves and follows it at once.
- **Keyboard**: by default `+` / `-` zoom, `←` / `→` pan a tenth of the view (half with Shift), `Home` zooms to fit, `/` searches, ⌘/Ctrl+O opens, ⌘/Ctrl+S saves, ⌘/Ctrl+, opens the settings, and `Esc` closes a menu, cancels an edge measurement and cancels the segment, in that order. All of them but `Esc` closing menus and dialogs can be changed in Settings › Shortcuts (§11), which also offers Start or stop capturing, Show or hide the data stream, Export and Open the trigger settings, unbound at first. A shortcut without ⌘/Ctrl leaves text fields alone, and all but Open, Save and the settings leave the keys to an open menu or dialog. `=` and `_`, on the same keys as `+` and `-`, zoom too unless an action has them. Menu items and button tips name the shortcut of their action ("Zoom in (+)"), and none when it has none.

## 11. Dialogs and the settings page

- Shared style: the interface behind is blurred by `--veil-blur` (12 px) and dimmed by `--scrim`; over it a panel is centred (`--raise` fill, 6 px radius) without an outline, set apart from the interface by shade alone, with a 40 px title bar and a close button on the right. Focus moves into the panel when it opens; Esc or a click on the backdrop closes it.
- **Add / edit decoder**: a searchable list of the protocols that read samples (name and one-line summary, buses before signal lines), a "Stack on it" select of the decoders that read the chosen protocol's output (the 24xx EEPROM on I²C, SPI NOR on SPI…), an optional name, a select per channel the protocol reads (channel lists are typed as numbers, least significant first), the options of each layer as the decoder declares them (numbers with their bounds, choices, switches, text), and the number base under Display; when editing, the protocol is fixed and the decoder can be removed. Decoders can be added and changed while a capture runs. A decoder from a loaded module (below) is listed like the others, under the copy its module carries — in the interface's language where it has it, else the closest it declares — then the interface's own copy for a decoder of that name, then its type name, with "Loaded from `file`" as the summary of a module without one; its options, event kinds and event fields are named the same way wherever they show.
- **Export**: format (HSync capture, DSView capture, CSV table, VCD waveform, FST waveform, sigrok session, Raw binary, each with a one-line description; CSV can write one row per sample), range (Whole capture / Segment) and channels (All / None); the save location is chosen after confirming.
- **Decoder layers**: the editor adds compatible layers repeatedly above the selected sample-reading protocol. Each layer keeps its own options, channels and stable instance identity; compatible declared ports determine the connection. Removing a layer removes its dependent upper layers. Existing chains retain their node IDs, exposed streams and unedited fields. A graph with branches, unsupported nodes or connections the layer form cannot represent keeps its topology intact; the editor explains this and permits supported option and display edits without flattening the graph.
- **Imported decoder notes**: when an imported DSView configuration contains unsupported decoders, layers or options, a compact expandable warning appears above the overview. It shows one affected decoder and one note at a time, with previous/next controls and a count. The original configuration stays with the capture through edits and saving/reopening; unsupported decoder entries do not execute. An explicit remove action deletes the selected retained entry. Supported portions continue decoding, and notes explain which portions remain unsupported.
- **Transport catalogue**: dual/quad SPI, native SD SDR and STM8 SWIM are sample-reading base protocols; Arm JTAG-DP stacks on JTAG. QSPI uses fixed lane groups, SD uses native SDR, SWIM exposes a target-clock option, and JTAG-DP supports a single four-bit Arm debug TAP. Multi-layer configuration includes paths such as 1-Wire link, 1-Wire network and DS18B20. Compact JTAG OScan0/1 supplies the same JTAG event schema to the existing chain and debug layers; SWD can feed DP/AP interpretation. MIDI semantic fields, DMX timing and LIN version profiles appear under their compatible inputs.
- **Import waveform**: opens once a VCD, FST or GHW file chosen with "Open file…" has been read (the status bar shows "Reading N%" meanwhile); the title bar names the file. The body starts with the format and "All" / "None", then a search box, then the file's scopes and signals as a tree in a `--control` box: 24 px rows indented 16 px per level, scopes with a fold chevron and a checkbox that reads all, some or none of their signals, logic signals with a checkbox, their declared range (`[7:0]`) and width ("8 bits"), and reals, strings and events in `--ink-3` marked "no logic level". Searching lists the matching signals with the scopes they lie in. Below the tree, a "Sample rate" field holds the rate the timestamps give (the coarsest grid they all fall on), which the user may replace with the clock the file was sampled at; "From the timestamps" beside it goes back to the given rate, and a value that is no frequency is refused in the alert colour. One line then gives the sample rate, sample count, channel count (one per bit) and memory the import makes at that rate; when timestamps fall between samples, it says that changes closer together than one sample fall together, and an import needing more than half of this machine's memory warns in the alert colour. Every signal is chosen from the start when the file holds no more than 32 bits, otherwise none. The footer holds Cancel and Import; while importing, it shows "Importing N%" with a progress bar, and Cancel stops the import, leaving the dialog and the open capture as they were. The capture that opens counts as the waveform file: the window title and the device button name it, and saving suggests its name with `.hsync`.
- **Settings page**: 760 × 560 px, shrinking in small windows. A 176 px side bar on the left (`--chrome` fill) lists Appearance, Behaviour, Shortcuts, Decoders and Diagnostics, with About set apart at its foot, each with an icon before its name; the selected section has a `--control-on` fill like a segmented control's, which slides to the next section. When an update is available, a `--ink` dot sits at the right of About, and its tip and screen reader text read "Version x available". The current section fills the right, labels on the left and controls on the right; each label and each checkbox's text has an `--ink-3` icon before it, and the buttons carry an icon before their text:
  - **Appearance**: Theme (System / Light / Dark, each with an icon), Density (Comfortable / Compact), Readout position (Status bar / Floating, Status bar by default: where the hover readout and the edge measurement read out, §6.4, §6.5; "Where pulse widths and edge measurements show" below), Graphics acceleration (Auto / Off, with the renderer in use named below) and Language (System / `简体中文` / `English`), all segmented controls.
  - **Behaviour**: "Centre the view on the trigger" (on), "Follow the latest data while capturing" (on), "Keep scrolling after a quick drag" (on: when a drag is released faster than 0.5 px/ms, panning continues with exponential decay until a press or the wheel stops it), "Zoom the waveform with the wheel alone" (off: §10), "Fit waveform heights to the window" (on: §6.1), "Show the latest data when a repeat stops" (off), "Check for updates at startup" (on: checks once in the background after startup, and a failure stays silent). The first three have no effect while zoomed to fit.
  - **Shortcuts**: a line on how recording works, with "Restore all defaults" (disabled while nothing differs) on its right, then the actions under File, View and Capture headings (`--ink-3`, caption size). Each action is a row: its name in a 192 px `--ink-2` column, then its shortcut in one 176 × 26 px `--control` box (`⌘S`, `⇧←` on Apple systems, `Ctrl+Shift+S` elsewhere, "Space" in the interface language; "None" in `--ink-3`), with a × at its right end that clears it; a row whose shortcut differs from its default ends in a "Restore the default shortcut" icon button. Clicking the box records the keys pressed together next: it turns `--ink` with `--canvas` text and reads "Press a shortcut…", then shows the modifiers as they go down (`⇧⌘…`), and the first other key completes the shortcut, at most four keys in all. `Esc` or a click elsewhere cancels, and Tab moves on without being recorded. A shortcut another action had moves to this one, and a line under the row says so ("⌘S was taken from “Save the capture”"); restoring an action's default takes it back the same way. No shortcut runs while one is being recorded. Shortcuts are kept in the WebView's local storage.
  - **Decoders**: the decoder modules loaded — decoders written in HSync's DSL, loaded from the module file the compiler writes — after a line saying so. Each module is a `--control` row with a puzzle icon, the decoder's name (its type name when it has no copy), its file path in `--ink-3` (truncated, the whole path as its tip), "Stands in for the built-in … decoder" when a built-in decoder has its name, the reason in the alert colour when it did not load, and a Remove button; "Load module…" (with a folder icon) opens a file dialog for the module's JSON file. Loaded modules are remembered and loaded again at startup, in order; the open capture decodes again with a module as it is loaded or removed. After them come the modules the open capture file brought (a saved capture carries the modules its decoders use), each with the capture file as its path and "Came with the capture file …"; they are removed the same way and are not remembered. Without modules, the list reads "No decoder module is loaded."
  - **Diagnostics**: log level, "Write a log file", "Show in folder".
  - **About**: version, license and component credits. Below the version number is the update area: when no update is known, it offers "Check for updates", with "Checking for updates…" or "HSync is up to date" beside it; with an update, it reads "Version x is available" and offers "Download and restart"; while downloading, it shows a percentage ("Downloading…" when the size is unknown), and once installed the app restarts by itself. On failure, the reason appears below, and the user can try again.
  - Focus moves into the panel without a focus ring on a section; when the keyboard moves onto the sections, the ring is drawn inside them. On closing, focus returns to the button that opened the page; opened again, the page shows the section last viewed.

## 12. Accessibility

- Every icon button has an `aria-label`; the group header's expand button has `aria-expanded`; trigger slots have `aria-haspopup="menu"`; menu items have `role="menuitemradio"` plus `aria-checked`; data stream tabs have `role="tab"`; the settings page's side bar is a vertical `role="tablist"`, where ↑ / ↓ / Home / End move between sections; segmented controls use `aria-pressed`; a shortcut box is named after its action and shortcut, and "Press the new shortcut, Esc to cancel" while it records.
- Focus ring: 2 px solid `--ink`, offset 2 px (offset −2 px inside tabs).
- With reduced motion on, motion durations are 0, and the start and end states of transitions do not apply either, so every change completes at once (§13).

## 13. Motion

When the interface changes state, brief motion shows where things come from and where they go, timed by the tokens in §3.6. Motion changes only opacity, translation and scale (except panel height), so it does not compete with waveform rendering for the main thread.

| Situation                                                                                                                                      | Motion                                                                                                                                                                                                           |
| ---------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Menus and popovers                                                                                                                             | Grow from the corner of the control that opened them: opacity 0→1, scale 0.96→1; reversed on close                                                                                                               |
| Dialogs and the settings page                                                                                                                  | The backdrop fades in; the panel rises 8 px and scales up from 98%                                                                                                                                               |
| Data stream panel                                                                                                                              | Slides in from, and out to, the bottom edge of the workspace; while moving it covers the timeline, which resizes only once, at the start or the end                                                              |
| Segmented controls, tabs, the settings page's side bar                                                                                         | The selected option's fill (the underline for data stream tabs) slides to the new option with a slight bounce                                                                                                    |
| Panel content changes (trigger type, stage of a stage trigger, export's CSV option, data stream decoder / mode / range, settings page section) | New content rises 4 px and fades in, and old content disappears at once; the frame's height eases to the new content's height                                                                                    |
| List additions and removals (devices, stage tabs of a stage trigger, channels with conditions)                                                 | Items scale and fade in or out in place; the other items slide over to close the gap                                                                                                                             |
| Appearing notices (status bar messages, warnings, search match count, trigger summary)                                                         | Rise and fade in                                                                                                                                                                                                 |
| Group header expand arrow                                                                                                                      | Rotates with the slide timing                                                                                                                                                                                    |
| The timeline's view, zoomed by a notch of the wheel, the zoom keys or buttons, or panned by the pan keys                                       | Glides there with the glide timing, the sample under the pointer (for the wheel) or in the middle staying in place; a notch or key during a glide adds to where it is going and glides on from where the view is |

- Elements that are leaving no longer respond to the pointer.
- The waveforms, decode blocks, ruler and overlays move together in every frame of a glide, a drag or a trackpad gesture: the canvas is drawn in the frame that moved the view, once the rows have been updated.
- No motion for: anything that follows the pointer (hover readout, edge measurement, segment dragging, dragging, a trackpad's panning and zooming); viewport jumps (zoom to fit, search hits, a click or drag on the overview strip, data stream rows); theme, density and language switches (the canvas redraws in a single frame, so interface colours that transition slowly would fall out of step with it); expanding and collapsing channel groups (the rows are drawn on the canvas).

## 14. Copy and numbers

- The interface supports Simplified Chinese (default) and English, following the system or chosen in Settings. Chinese copy puts clarity first; English copy means the same as the Chinese and has a similar length, so it does not break the layout.
- Sentences are translated whole, as messages with parameters; strings are never concatenated. Decoders' own English annotations are shown as they are.
- Numbers and dates are formatted for the current language: Chinese groups sample counts with narrow spaces (`131 072`), English with commas (`131,072`). Time and frequency units (ns, µs, ms, s, Hz, kHz, MHz) are the same in both languages, with one space between value and unit (`5.24 ms`).
