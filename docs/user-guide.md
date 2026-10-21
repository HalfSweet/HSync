# HSync user guide

HSync is a general-purpose logic analyzer app. It drives DreamSourceLab DSLogic devices, ALIENTEK ATK-Logic devices (DL16 and DL16 Plus), PXLogic PX-Logic devices (PX-Logic 32, 16 Pro, 16 Plus and 16 Base) and a built-in demo device, with experimental support for Sipeed's SLogic16U3. This guide walks through the interface in the order you use it. Installing HSync is described in the [README](../README.md#download).

## Interface

From top to bottom, the window has five parts:

- **Top bar**: the device, the acquisition settings (sample rate │ duration │ channels), the trigger and the capture button; on the right, the search box, export and settings.
- **Overview strip**: a thumbnail of the whole capture. A thin band for each decoder marks where it has data, coloured like its decode blocks, with its errors in red; the outlined box is the current view.
- **Timeline**: the ruler, and the channel and decode rows grouped by decoder. Channels not assigned to a decoder go into the **Other channels** group.
- **Data stream** (optional): a list of all the decoded results of one decoder.
- **Status bar**: the data stream toggle, capture status, trigger time, sample count, segment measurement and zoom buttons.

## Devices

- HSync starts with an empty timeline; waveforms appear once you capture or open a file.
- Click the device button at the left end of the top bar to choose a device. Without hardware, HSync opens the **Demo device**; click **Capture** and it plays back the bundled protocol demo data (I²C, UART, SPI, CAN-FD), with its decoders already set up the first time.
- The first time you open a DSLogic, HSync loads the firmware and FPGA bitstream onto it automatically; this takes a few seconds.
- An ATK-Logic is listed as **ATK-Logic** until it is opened, then by its model, such as **DL16 Plus**. HSync does not update its firmware: if it reports that the firmware needs an update, update it with ALIENTEK's ATK-Logic app, then plug the device in again.
- A PX-Logic is likewise listed as **PXLogic** until it is opened (except the first 32-channel devices, named at once), then by its model, such as **PX-Logic 16 Plus**. HSync loads its FPGA bitstreams each time it opens it. If the device runs another firmware version than the one HSync brings (PXView 1.6.5's), HSync first writes its own into the device, which restarts and is opened again; this takes a few seconds more.
- A Sipeed **SLogic16U3** uses the firmware already installed on the device. HSync does not update its firmware. This port is experimental; see [SLogic16U3](#sipeed-slogic16u3) below for connection details and limits.
- If you plug in a logic analyzer while the demo device is open, the status bar shows "… connected"; click **Switch** to use it. If a loose USB cable makes the device drop out briefly and it reappears within two seconds, HSync reopens it automatically and restores its settings.
- If a USB 3.0 model is plugged into a USB 2.0 port, the status bar reminds you to move it to a USB 3.0 port; otherwise Stream mode's speed is limited.
- The device's options sit in the acquisition settings (below): the **Operation mode** (**Buffer mode** or **Stream mode**), **Channel mode**, **Threshold voltage** (**Threshold level** on models with fixed thresholds), the input filter (**Filter one-sample glitches**), **RLE compression**, the external clock and its edge (**Use an external clock**, **Sample on the falling clock edge**), the **External trigger input** and, for Buffer mode, what happens **When stopped**. Each device lists only the options it has: an ATK-Logic has the operation mode and a **Threshold voltage** from −5 to 5 V; a PX-Logic has the operation and channel modes, a **Threshold voltage** from 0 to 6 V, the filter, the clock edge and the **External trigger input**. An SLogic16U3 has Stream mode and a **Threshold voltage** from 0 to 6 V, initially 1.7 V; its sample-rate choices follow the enabled channels automatically.
- HSync remembers the settings of each kind of device and restores them the next time you open it: the device options, enabled channels, triggers, channel colours and capture mode at once, the channel names and decoders with the first capture after it opens. Showing a file or another device's capture leaves the names and decoders remembered for this device alone; removing every decoder is remembered too.

### Sipeed SLogic16U3

Connect the analyzer with a USB 3.0 data cable, close any other application using it, then select **SLogic16U3** from the device menu. On Windows it needs WinUSB; on Linux install the SLogic16U3 udev rule in the [README](../README.md#first-launch). HSync recognizes the normal device with USB ID `359f:3031`; bootloader mode and other SLogic models are not supported.

For a first capture, connect the analyzer's ground and one channel to a known low-frequency digital signal, select 5 MHz and a short duration, leave the trigger unset and press **Capture**. The port uses the installed firmware and does not flash the device. Alongside mock-based protocol and capture tests, a real SLogic16U3 passed USB 3.0 discovery and opening, three consecutive captures at 5 MHz in each of the 4-, 8- and 16-channel modes, and short captures configured at their 800, 400 and 200 MHz limits. Every capture delivered exactly 65,536 samples. The captured inputs were static, so channel mapping and signal timing remain unverified; sustained high-rate throughput and USB 2.0 also need hardware validation.

SLogic16U3 captures in Stream mode, including Single, Repeat, Loop and Instant. The highest enabled channel determines the sampling limit, even if only one channel is enabled:

| Enabled channels | Maximum with USB 3.0 | Maximum with USB 2.0 |
| ---------------- | -------------------- | -------------------- |
| Only D0–D3       | 800 MHz              | 80 MHz               |
| Only D0–D7       | 400 MHz              | 40 MHz               |
| Any of D8–D15    | 200 MHz              | 20 MHz               |

These are the device’s advertised limits; sustained host throughput has not yet been measured. Choose from the discrete rates shown in the acquisition settings. For example, enabling only D15 still has the 16-channel limit. The threshold is shared by all channels. Simple edge and level triggers are evaluated in software; stages, serial triggers, Buffer mode, RLE, external clocks and external trigger inputs are unavailable. The pre-trigger history uses at most 64 MiB (33,554,432 samples), so long captures can contain less history than the requested trigger position; the trigger marker shows the actual position.

## Acquisition settings

Click the sample rate │ duration │ channels button in the top bar:

- **Sample rate** and **Duration**: durations are listed in a 1-2-5 sequence. When you change the sample rate, the duration stays as close as possible to what it was.
- In Buffer mode, durations longer than the device memory can hold are marked **RLE**. Choosing one turns on RLE compression automatically (on a DSLogic, **RLE compression** must be allowed in the device options beside the lists). A compressed capture fits only while the signals change slowly enough; when they change too often, the capture stops and the status bar says so. Devices without RLE compression, such as a PX-Logic, list only the durations their memory holds.
- If the selected capture needs more than half of this computer's memory, the pop-up warns you to shorten the duration or use fewer channels.
- **Channels**: click a channel number to enable or disable it.
- Beside the lists (below them in a narrow window) are the device's other options, listed under [Devices](#devices). While a capture runs they cannot be changed.

## Capturing

- Click **Capture** to start, and click it again (it now reads **Stop**) to end. The status bar shows the progress, such as "Waiting for trigger" and the percentage captured.
- Use the arrow to the right of **Capture** to choose how to capture:
  - **Single**: capture once.
  - **Repeat**: capture again a set time after each capture completes. With **Repeat** selected, set the interval under **Repeat every** in the same menu (0.1–10 s, default 1 s). From the second capture on, the screen keeps the previous complete result while the new one is captured in the background, and the new one replaces it in one go once complete. By default, stopping midway discards the new capture and keeps the previous one; to show the latest data instead, turn on **Show the latest data when a repeat stops** in the settings.
  - **Loop**: capture continuously, keeping only the latest stretch (as long as the selected duration); when you stop, that last stretch is shown. Available only on devices that can loop (a DSLogic, an ATK-Logic or a PX-Logic in Stream mode, an SLogic16U3, or the demo device).
  - **Instant**: ignore the trigger conditions and start at once.
- Successive captures with the same device keep the current zoom and position, so you can keep your eye on the same detail across repeated captures.
- When the computer cannot take in the data fast enough, the status bar reports that data was lost ("USB bandwidth exceeded; some data was lost").

## Triggers

- **Simple trigger**: each channel row has a trigger slot at the right end of its label. Click it to choose **None**, **Rising edge**, **Falling edge**, **Any edge**, **High level** or **Low level**. When several channels have conditions, the trigger fires only when all of them hold at once. The **Trigger** button in the top bar sums up the current conditions.
- Click **Trigger** in the top bar to open the trigger settings:
  - **Position**: the share of the whole capture that comes before the trigger point (fixed at 1% in Stream mode on DSLogic, ATK-Logic and PX-Logic; adjustable up to 90% on SLogic16U3, subject to its pre-trigger history limit).
  - **Stages** (a DSLogic in Buffer mode): up to 16 stages. Each stage has two patterns, **Pattern A** and **Pattern B**; you can **Invert** each one and **Combine** them as **A and B** or **A or B**. Once a stage has held the set number of times (**Count**; tick **In a row** to require consecutive matches), the trigger moves on to the next stage.
  - **Serial** (a DSLogic in Buffer mode): **Start**, **Stop** and **Clock** conditions, the **Data** channel, and a data **Value** of up to 16 bits (each bit 0, 1 or X).
- **External trigger input** (a PX-Logic): in the device options, choose the edge or level on the device's trigger input that fires the trigger (**Rising edge**, **High level**, **Falling edge**, **Low level** or **Any edge**), or **Off** to ignore the input. An **Instant** capture ignores it too.

## Viewing waveforms

- **Zoom**: Ctrl (⌘ on macOS) + wheel, or pinching on a trackpad, zooms around the pointer; to zoom with the wheel alone, turn on **Zoom the waveform with the wheel alone** in the settings (over the channel labels on the left, the wheel then still scrolls the channels). You can also use the −, + and **Zoom to fit** buttons in the status bar, or the `+`, `-` and `Home` keys. Each notch of a mouse wheel zooms by the same step and the view glides there smoothly, as do the zoom buttons and keys and the `←` / `→` keys; turning several notches quickly adds them up. A trackpad follows your fingers directly. With reduced motion turned on in the system, the view moves at once.
- **Pan**: drag in the waveform area; use Shift + wheel or swipe sideways on a trackpad; or press `←` and `→` (with Shift, half a screen at a time). If you let go after a quick drag, the waveform keeps gliding for a moment (you can turn this off in the settings).
- **Overview strip**: click to centre the view there, drag to move the view, and use the wheel to zoom.
- **Hover readout**: rest the pointer on a waveform and the pulse under it is shaded, while the status bar shows its channel and width, plus the period, frequency and duty cycle it makes with the next pulse. Data lines a decoder uses, such as SDA or MOSI, show only the width; UART lines show roughly how many bits the pulse spans instead. To read it beside the pointer instead, set **Readout position** to **Floating** in the settings (Appearance): the readout then floats just above the row in a semi-transparent box, so the waveforms behind it stay visible; the edge measurement's result floats above its line the same way.
- **Edge measurement**: click an edge on a waveform to start measuring. When the pointer comes near another edge, on the same channel or another, it snaps to it, and the status bar shows the time between the two edges, the number of samples and 1/Δt as you move. Click again to finish; the measurement stays on the waveform and its result in the status bar. Click another edge to start a new measurement, or click an empty spot or press `Esc` to clear it. While measuring you can still drag, pan with the wheel and zoom, so if the end point is off screen, move there first and then click.
- **Segment**: drag along the ruler to set points A and B, or click a decode block to make its start and end the segment. The status bar shows the segment's duration; click it to see each channel's rising edges, falling edges and edge count within the segment. Press `Esc` to clear the segment.
- **Channels**: click a channel's colour dot, double-click its name or right-click it to rename it or change its colour.
- **Waveform height**: drag the bottom edge of a channel's label to make its waveform taller or shorter; double-click the edge to give it back its default height. Like colours, heights are saved in sessions and `.hsync` files.
- **Fitting the window**: the waveforms grow and shrink with the window, keeping their proportions, so that they fill the timeline: up to three times their height in a tall window, and in a short one down to the height of the narrow rows before the channels scroll. Dragging an edge then makes that waveform taller while the others make room. To keep fixed heights instead, turn off **Fit waveform heights to the window** in the settings (Behaviour).
- The arrow at the start of a group header expands or collapses the group.

## Protocol decoding

- Click **Add decoder** at the top left of the timeline and search for a protocol. HSync decodes with its own engine, uiharu: buses (I²C, UART, SPI, CAN, USB, I²S, JTAG, SWD, MDIO, Microwire, 1-Wire, PS/2, Wiegand…) and signal lines (PWM, timing, parallel, WS281x, AM230x, infrared…). Protocols that read another one's output — the 24xx EEPROM on I²C, SPI NOR flash and SD cards on SPI, Modbus RTU and LIN on UART — are chosen under **Stack on it**. Choose a channel for each line the protocol reads, set the options and, if you like, give the decoder a name.
- Decoding follows the capture: a running capture already shows what has been decoded, and decoders can be added or changed while it runs.
- Every event a decoder finds becomes a block on the timeline, named after its kind (address, data, frame…) with its main value; hovering shows every field. Click the settings button on a group header (**Decoder settings**) to change or remove a decoder; under **Display** you can choose the number base (**Hex**, **Dec**, **Oct**, **Bin** or **ASCII**).
- A block's colour says what the event is, the same in every protocol: blue for an address (an I²C address, a CAN ID), violet for a command, green for a frame's control field, teal for data, magenta for a CRC or checksum, olive for the other side's answer (an acknowledge, a handshake), grey for where a transaction begins or ends, grey stripes for a condition of the line itself (a line reset, a break), and red for an error — including a wrong parity bit, a CRC that does not match or an address no device answered.
- Blocks prioritize the data carried on the bus, such as JTAG's TDI/TDO, over scan numbers, offsets and bit counts. Single-sample events such as scan boundaries appear as thin markers; a block shows its text whenever it fits, a narrower one keeps its colour, and nearby events merge into a dense block coloured by what most of them are. Errors never merge with other events and stay visible however far you zoom out. Zoom in or use the **Data stream** to inspect them. Events with exactly the same span share a marker whose hover details list each event.
- **JTAG**: the **TAP states** lane separates states from shift data, coloured by the register they belong to: violet for the instruction register, teal for the data register, grey for reset and idle, with the shift states darker; on the data lane, each scan's begin and end markers and its shifts take the colour of the register it shifts. Each state extends until the next state change, stopping at any data gap or the current decoding progress. `Shift-IR` means shifting the instruction register; `Shift-DR` means shifting the data register. State names follow the interface language. Hover for the complete name and duration, or click to select its whole interval. The data stream still records the exact instant each state was entered.
- **Data stream**: click **Data stream** at the left end of the status bar, or the data stream button on a group header (**Show all decoded results in the data stream**). It lists one stream at a time, one row per event with its kind and fields; you can switch between **All** and **Segment**. Click a row to jump to that place on the waveform. You can **Copy** the list or **Export** it as CSV or text.
- **Your own decoders**: a decoder written in HSync's decoder language (see [uiharu/docs/dsl.md](../uiharu/docs/dsl.md)) runs in the app without a rebuild. Compile it with `pnpm dsl:compile`, then open **Settings → Decoders** and click **Load module…** to choose the `.json` file it wrote: the decoder appears under **Add decoder** like the built-in ones, and a module named like a built-in decoder (`spi`, `uart`…) stands in for it while it is loaded, so you can try a change to a built-in decoder on a capture at once. Loaded modules are remembered and loaded again at startup; **Remove** lets one go. A module the app cannot load stays listed with the reason. A module shows under the names its author gave it in the DSL (`locales`, in Chinese and English or whatever languages it declares), falling back to the app's own copy for a decoder of that name and then to the identifiers. A saved `.hsync` capture carries the modules its decoders use: opened on another machine, it brings them along, listed under **Settings → Decoders** with the capture file as their origin until they are removed or the app is restarted.

## Search

Press `/` to focus the search box in the top bar:

- **Decoded data**: type a number (`79` or `0x4F`) to find the events with a field of that value, several numbers (`0x4F 0x32`) to find the events whose bytes hold them in a row, or a word (`address`, `frame`, `stop`) to find events of that kind or with that enumerated value; every word must match. Press Enter for the next match, Shift + Enter for the previous one, and `Esc` to clear the box.
- **Level pattern**: type conditions such as `D3=R D5=1` (0 low, 1 high, R rising, F falling, C either edge, X any), then press Enter to find the next place where they all hold.

## Files

- **Open**: **Open file…** in the device menu, or ⌘O / Ctrl+O, reads HSync's own `.hsync` captures, which come back with the decoders (and the decoder modules they use), channel names, colours, heights and view they were saved with, and DSView `.dsl` files (all versions). Choosing a VCD, FST or GHW waveform instead, as simulators and FPGA logic analyzers (Vivado's ILA, Quartus' Signal Tap, …) export them, opens the **Import waveform** dialog.
- **Import**: the dialog lists the file's scopes and signals as a tree; tick the signals to import (search them by name), and click **Import**. Every bit becomes a channel, so a vector `data[7:0]` gives `data[0]` to `data[7]`; signals without a logic level (reals, strings) cannot be imported, and unknown (`x`) or high-impedance (`z`) values read as low. The **Sample rate** starts as the file's timestamps give it: the coarsest grid every timestamp falls on, which gives a logic analyzer export its sampling clock back when enough signals change; type the clock the file was sampled at if you know it, such as `25 MHz`. A line under the tree shows the sample rate, the number of samples and channels, and the memory the import needs. An imported capture has no decoders; add them as usual. **Save capture…** writes it as an `.hsync` file.
- **Save**: **Save capture…** in the export menu (the download icon in the top bar), or ⌘S / Ctrl+S, saves an `.hsync` file: HSync's own format, holding the samples (compressed), the channel names, the decoders and the loaded decoder modules they use, the channel colours and heights and the view, which all come back when the file is opened; the device settings of the moment are stored in it too. To give a capture to DSView, export it as a DSView capture: the `.dsl` file opens in DSView with its channel names and keeps HSync's decoders for HSync, and the decoders a DSView file lists are taken over when both know the protocol (I²C with a 24xx EEPROM, UART, SPI, CAN).
- **Export**: **Export…** in the export menu writes an HSync `.hsync` or DSView `.dsl` capture, CSV, VCD, FST, sigrok `.sr` or raw binary. You can export just the segment, or only some of the channels. You can cancel a save or an export while it runs.
- **Sessions**: **Save session…** in the export menu stores the device settings, channels, triggers, decoders and capture mode as `.hsync-session.json`; **Load session…** in the device menu also reads DSView `.dsc` sessions. A session loaded before anything was captured keeps its channel names and decoders for the first capture.

## Settings

The **Settings** button at the right end of the top bar (or ⌘, / Ctrl+,) opens the settings page; pick a section on the left:

- **Appearance**: **Theme** (**System**, **Light** or **Dark**), **Density** (**Comfortable** or **Compact**), **Language** (**System**, **简体中文** or **English**) and **Graphics acceleration**: with **Auto (WebGPU)**, waveforms are drawn with WebGPU, falling back to Canvas2D automatically where WebGPU is unavailable; the page shows which one is in use.
- **Behaviour**: **Centre the view on the trigger**, **Follow the latest data while capturing**, **Keep scrolling after a quick drag**, **Zoom the waveform with the wheel alone**, **Show the latest data when a repeat stops** and **Check for updates at startup**. The first three have no effect while the view is zoomed to fit.
- **Shortcuts**: every keyboard shortcut, grouped as **File**, **View** and **Capture**. Click an action's box and press the keys of the new shortcut together, such as ⌘⇧S or Ctrl+Shift+S (up to four keys); `Esc` cancels. **×** clears a shortcut, the arrow at the right of a row restores that action's default, and **Restore all defaults** restores them all. Giving an action a shortcut that another one had moves it, and the row says from where. **Start or stop capturing**, **Show or hide the data stream**, **Export** and **Open the trigger settings** have no shortcut until you give them one.
- **Diagnostics**: the **Log level**, whether to **Write a log file**, and **Show in folder**, which opens the folder that holds the log.
- **About** (at the foot of the side bar): the version and license, and updates. Click **Check for updates**; when a new version is available, click **Download and restart**, and once the download finishes the app restarts into the new version. With **Check for updates at startup** on, a small dot appears on the settings button and on **About** when a new version is found.

Opening menus and panels, switching options, and adding or removing list items all come with short animations. If you turn on reduced motion in your system's accessibility settings, the interface stops playing them.

## Keyboard shortcuts

These are the defaults; change them in **Settings › Shortcuts**. Shortcuts without ⌘ or Ctrl do nothing while you type in a text field.

| Key                           | Action                                                   |
| ----------------------------- | -------------------------------------------------------- |
| `/`                           | Focus the search box                                     |
| Enter / Shift + Enter         | Next / previous search match                             |
| `+` / `-`                     | Zoom in / out                                            |
| `←` / `→` (faster with Shift) | Pan                                                      |
| `Home`                        | Zoom to fit                                              |
| `Esc`                         | Close a menu or dialog; clear the measurement or segment |
| ⌘O / Ctrl+O                   | Open a file                                              |
| ⌘S / Ctrl+S                   | Save the capture                                         |
| ⌘, / Ctrl+,                   | Open or close the settings                               |
| Ctrl / ⌘ + wheel              | Zoom around the pointer                                  |
| Wheel                         | Scroll the channels, or zoom with the wheel setting on   |
| Shift + wheel                 | Pan                                                      |
