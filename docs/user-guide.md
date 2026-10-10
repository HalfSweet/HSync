# HSync user guide

HSync is a logic analyzer app for DreamSourceLab DSLogic devices and a built-in demo device, covering the logic analyzer features of DSView. This guide walks through the interface in the order you use it. Installing HSync is described in the [README](../README.md#download).

## Interface

From top to bottom, the window has five parts:

- **Top bar**: the device, the acquisition settings (sample rate │ duration │ channels), the trigger and the capture button; on the right, the search box, export and settings.
- **Overview strip**: a thumbnail of the whole capture. A thin band for each decoder marks where it has data, and the outlined box is the current view.
- **Timeline**: the ruler, and the channel and decode rows grouped by decoder. Channels not assigned to a decoder go into the **Other channels** group.
- **Data stream** (optional): a list of all the decoded results of one decoder.
- **Status bar**: the data stream toggle, capture status, trigger time, sample count, segment measurement and zoom buttons.

## Devices

- HSync starts with an empty timeline; waveforms appear once you capture or open a file.
- Click the device button at the left end of the top bar to choose a device. Without hardware, HSync opens the **Demo device**; click **Capture** and it plays back the bundled protocol demo data (I²C, UART, SPI, CAN-FD) with its decoders already set up.
- The first time you open a DSLogic, HSync loads the firmware and FPGA bitstream onto it automatically; this takes a few seconds.
- If you plug in a DSLogic while the demo device is open, the status bar shows "… connected"; click **Switch** to use it. If a loose USB cable makes the device drop out briefly and it reappears within two seconds, HSync reopens it automatically and restores its settings.
- If a USB 3.0 model is plugged into a USB 2.0 port, the status bar reminds you to move it to a USB 3.0 port; otherwise Stream mode's speed is limited.
- **Device options…** at the bottom of the device menu covers the **Operation mode** (**Buffer mode** or **Stream mode**), **Channel mode**, **Threshold voltage** (**Threshold level** on models with fixed thresholds), the input filter (**Filter one-sample glitches**), **RLE compression**, the external clock and its edge (**Use an external clock**, **Sample on the falling clock edge**) and, for Buffer mode, what happens **When stopped**. HSync remembers the settings of each kind of device and restores them the next time you open it.

## Acquisition settings

Click the sample rate │ duration │ channels button in the top bar:

- **Sample rate** and **Duration**: durations are listed in a 1-2-5 sequence. When you change the sample rate, the duration stays as close as possible to what it was.
- In Buffer mode, durations longer than the device memory can hold are marked **RLE**. Choosing one turns on RLE compression automatically (RLE must be allowed in **Device options…**).
- If the selected capture needs more than half of this computer's memory, the pop-up warns you to shorten the duration or use fewer channels.
- **Channels**: click a channel number to enable or disable it.

## Capturing

- Click **Capture** to start, and click it again (it now reads **Stop**) to end. The status bar shows the progress, such as "Waiting for trigger" and the percentage captured.
- Use the arrow to the right of **Capture** to choose how to capture:
  - **Single**: capture once.
  - **Repeat**: capture again a set time after each capture completes. With **Repeat** selected, set the interval under **Repeat every** in the same menu (0.1–10 s, default 1 s). From the second capture on, the screen keeps the previous complete result while the new one is captured in the background, and the new one replaces it in one go once complete. By default, stopping midway discards the new capture and keeps the previous one; to show the latest data instead, turn on **Show the latest data when a repeat stops** in the settings.
  - **Loop**: capture continuously, keeping only the latest stretch (as long as the selected duration); when you stop, that last stretch is shown. Available only on devices that can loop (a DSLogic in Stream mode, the demo device).
  - **Instant**: ignore the trigger conditions and start at once.
- Successive captures with the same device keep the current zoom and position, so you can keep your eye on the same detail across repeated captures.
- When the computer cannot take in the data fast enough, the status bar reports that data was lost ("USB bandwidth exceeded; some data was lost").

## Triggers

- **Simple trigger**: each channel row has a trigger slot at the right end of its label. Click it to choose **None**, **Rising edge**, **Falling edge**, **Any edge**, **High level** or **Low level**. When several channels have conditions, the trigger fires only when all of them hold at once. The **Trigger** button in the top bar sums up the current conditions.
- Click **Trigger** in the top bar to open the trigger settings:
  - **Position**: the share of the whole capture that comes before the trigger point (fixed at 1% in Stream mode).
  - **Stages** (Buffer mode): up to 16 stages. Each stage has two patterns, **Pattern A** and **Pattern B**; you can **Invert** each one and **Combine** them as **A and B** or **A or B**. Once a stage has held the set number of times (**Count**; tick **In a row** to require consecutive matches), the trigger moves on to the next stage.
  - **Serial** (Buffer mode): **Start**, **Stop** and **Clock** conditions, the **Data** channel, and a data **Value** of up to 16 bits (each bit 0, 1 or X).

## Viewing waveforms

- **Zoom**: Ctrl (⌘ on macOS) + wheel zooms around the pointer. You can also use the −, + and **Zoom to fit** buttons in the status bar, or the `+`, `-` and `Home` keys.
- **Pan**: drag in the waveform area; use Shift + wheel or swipe sideways on a trackpad; or press `←` and `→` (with Shift, half a screen at a time). If you let go after a quick drag, the waveform keeps gliding for a moment (you can turn this off in the settings).
- **Overview strip**: click to centre the view there, drag to move the view, and use the wheel to zoom.
- **Hover readout**: rest the pointer on a waveform to see the width of the pulse under it. Periodic signals such as clocks and PWM also show the period, frequency and duty cycle, and UART lines show roughly how many bits the pulse spans.
- **Edge measurement**: click an edge on a waveform to start measuring. When the pointer comes near another edge, on the same channel or another, it snaps to it, and a bubble shows the time between the two edges, the number of samples and 1/Δt as you move. Click again to finish; the result stays on the waveform. Click another edge to start a new measurement, or click an empty spot or press `Esc` to clear it. While measuring you can still drag, pan with the wheel and zoom, so if the end point is off screen, move there first and then click.
- **Segment**: drag along the ruler to set points A and B, or click a decode block to make its start and end the segment. The status bar shows the segment's duration; click it to see each channel's rising edges, falling edges and edge count within the segment. Press `Esc` to clear the segment.
- **Channels**: click a channel's colour dot, double-click its name or right-click it to rename it or change its colour.
- The arrow at the start of a group header expands or collapses the group.

## Protocol decoding

- Click **Add decoder** at the top left of the timeline and search for a protocol: the native I²C, UART, SPI, CAN and CAN-FD decoders, or one of the 150 Python decoders that come with DSView. Protocols that need a lower-level decoder are stacked on it automatically; EEPROM24xx, for example, stacks on I²C. Choose a channel for each role and set each layer's options.
- **Decode range**: choose **Whole capture** or **Segment**. With **Segment**, only the samples between A and B are decoded; this corresponds to DSView's decode start/end cursors.
- Click the settings button on a group header (**Decoder settings**) to change or remove its decoder. Under **Display** you can choose the number base (**Numbers**: **Hex**, **Dec**, **Oct**, **Bin** or **ASCII**) and which **Annotation rows** to show.
- **Data stream**: click **Data stream** at the left end of the status bar, or the data stream button on a group header (**Show all decoded results in the data stream**). It lists one decoder at a time; you can switch between **All** and **Segment**, and for UART between **Text** and **Hex**. Click a row to jump to that place on the waveform. You can **Copy** the list or **Export** it as CSV or text.

## Search

Press `/` to focus the search box in the top bar:

- **Decoded data**: type hex (`4F`, `0x4F`, `4F 32`) or text (`HOST`) to search all decoders. Press Enter for the next match, Shift + Enter for the previous one, and `Esc` to clear the box.
- **Level pattern**: type conditions such as `D3=R D5=1` (0 low, 1 high, R rising, F falling, C either edge, X any), then press Enter to find the next place where they all hold.

## Files

- **Open**: **Open file…** in the device menu, or ⌘O / Ctrl+O, reads DSView `.dsl` files (all versions).
- **Save**: **Save capture…** in the export menu (the download icon in the top bar), or ⌘S / Ctrl+S, saves a `.dsl` file that DSView can open, including channel names and decoders.
- **Export**: **Export…** in the export menu writes `.dsl`, CSV, VCD, sigrok `.sr` or raw binary. You can export just the segment, or only some of the channels. You can cancel a save or an export while it runs.
- **Sessions**: **Save session…** in the export menu stores the device settings, channels, triggers, decoders and capture mode as `.hsync-session.json`; **Load session…** in the device menu also reads DSView `.dsc` sessions.

## Settings

The **Settings** button at the right end of the top bar (or ⌘, / Ctrl+,) opens the settings page; pick a section on the left:

- **Appearance**: **Theme** (**System**, **Light** or **Dark**), **Density** (**Comfortable** or **Compact**), **Language** (**System**, **简体中文** or **English**) and **Graphics acceleration**: with **Auto (WebGPU)**, waveforms are drawn with WebGPU, falling back to Canvas2D automatically where WebGPU is unavailable; the page shows which one is in use.
- **Behaviour**: **Centre the view on the trigger**, **Follow the latest data while capturing**, **Keep scrolling after a quick drag**, **Show the latest data when a repeat stops** and **Check for updates at startup**. The first three have no effect while the view is zoomed to fit.
- **Diagnostics**: the **Log level**, whether to **Write a log file**, and **Show in folder**, which opens the folder that holds the log.
- **About** (at the foot of the side bar): the version and license, and updates. Click **Check for updates**; when a new version is available, click **Download and restart**, and once the download finishes the app restarts into the new version. With **Check for updates at startup** on, a small dot appears on the settings button and on **About** when a new version is found.

Opening menus and panels, switching options, and adding or removing list items all come with short animations. If you turn on reduced motion in your system's accessibility settings, the interface stops playing them.

## Keyboard shortcuts

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
| Shift + wheel                 | Pan                                                      |
