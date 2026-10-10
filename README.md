<div align="center">

<img src="assets/brand/app-icon.svg" width="112" height="112" alt="HSync logo">

# HSync

**A modern logic analyzer app for DreamSourceLab DSLogic and ALIENTEK ATK-Logic**

Capture, trigger, decode and measure, and keep working with your DSView files.

[![Release](https://img.shields.io/github/v/release/HalfSweet/HSync?style=flat-square&label=release&color=45658D)](https://github.com/HalfSweet/HSync/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/HalfSweet/HSync/total?style=flat-square&color=277165)](https://github.com/HalfSweet/HSync/releases)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-665A8A?style=flat-square)](#download)
[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-845169?style=flat-square)](LICENSE)

**English** · [简体中文](README.zh-CN.md)

[Download](#download) · [Getting started](#getting-started) · [User guide](docs/user-guide.md) · [Report a problem](https://github.com/HalfSweet/HSync/issues/new)

</div>

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/screenshots/en-US/hero-dark.webp">
  <img src="assets/screenshots/en-US/hero-light.webp" alt="HSync showing a capture of I²C, UART, SPI and CAN-FD buses, with decoded transfers above their waveforms and the I²C data stream below">
</picture>

HSync is a desktop app for DreamSourceLab's DSLogic and ALIENTEK's ATK-Logic logic analyzers. Plug one in and press **Capture**: decoded I²C, UART, SPI and CAN traffic appears right above the waveforms it came from, and the 150 protocol decoders that come with DSView are a search away. HSync opens and saves DSView's `.dsl` files, so your captures come along, and everything it needs is in the package: the device firmware, the decoders and the Python runtime they use.

No hardware at hand? HSync opens its built-in **Demo device**, which plays back a recording of four buses, so you can try everything straight away.

## Features

### Capture

- **Every DSLogic model**, from the original DSLogic to the U3Pro32 ([full list](#supported-devices)). Firmware and FPGA bitstreams are built in and load by themselves.
- **ALIENTEK ATK-Logic DL16 and DL16 Plus**, up to 1 GHz in Buffer mode, with the threshold anywhere from −5 to 5 V.
- **Buffer and Stream modes**, with RLE compression for captures longer than the device's memory, and a warning when a capture would need more than half of your computer's memory.
- **Single, Repeat, Loop and Instant capture.** Repeat keeps the last complete capture on screen while the next one records in the background; Loop keeps capturing and holds on to the most recent stretch.
- **Device options** for the threshold voltage, glitch filter, external clock and more, remembered for next time.
- **Hot-plug aware.** Plug in a logic analyzer while the demo is open and HSync offers to switch to it; if the USB connection drops for a moment, HSync reopens the device with its settings.

### Trigger

- **Triggers on the channel rows.** Click the trigger slot to the right of a channel's name and pick a rising, falling or any edge, or a high or low level.
- **Stage triggers** with up to 16 stages, each combining two patterns that must hold a set number of times, and **serial triggers** on a data value of up to 16 bits (on a DSLogic in Buffer mode).
- **Adjustable trigger position**, with the ruler counting time from the trigger.

![The trigger panel set to a three-stage trigger, with conditions on channels D0, D1 and D5](assets/screenshots/en-US/trigger.webp)

### Decode

- **Native I²C, UART, SPI, CAN and CAN-FD decoders**, drawn in line with the waveforms. They show what the bus carried, byte by byte, without guessing what it meant.
- **150 more protocols** from DSView's decoder library, from 1-Wire and JTAG to USB Power Delivery and WS281x LEDs. Decoders that build on others stack automatically.
- **Decode the whole capture or just a segment**, and show values in hex, decimal, octal, binary or ASCII.
- **The data stream** lists everything a decoder found. Click a row to jump to it on the waveform; copy the list or export it as CSV or text.

![A search for HOST: its bytes, 48 4F 53 54, highlighted on the SPI MISO lane and in the data stream](assets/screenshots/en-US/search.webp)

### Measure and search

- **An overview of the whole capture**, with each decoder's activity, to see where things happen and get there in one click.
- **Hover a pulse** to read its width, plus the period, frequency and duty cycle it makes with the next pulse.
- **Click two edges** to measure the time between them, on one channel or across channels; the pointer snaps to edges.
- **Mark a segment** on the ruler to see its duration and every channel's rising and falling edges inside it.
- **Search** decoded data for values or text (`4F 32`, `HOST`), or find a level pattern such as `D3=R D5=1`.

![An edge measurement of one SCL period, 10.04 µs, and the edge counts of every channel in a marked segment, in the Dark theme](assets/screenshots/en-US/measure.webp)

### Files

- **Opens DSView `.dsl` captures** of every version, and saves `.dsl` files that DSView opens, with channel names and decoders.
- **Exports CSV, VCD, sigrok `.sr` and raw binary**: the whole capture or a segment, all channels or a few.
- **Sessions** keep your device settings, channels, triggers and decoders; DSView `.dsc` sessions load too.

### Interface

- **Light and Dark themes**, Comfortable and Compact density, and English or Simplified Chinese.
- **Waveforms drawn with WebGPU** where it is available, with Canvas2D as the fallback.
- **Keyboard shortcuts** for zooming, panning, searching, opening and saving.
- **Updates inside the app**: HSync checks at startup and installs a new version in one click.

## Download

Get the latest version from the [Releases page](https://github.com/HalfSweet/HSync/releases/latest).

| Platform | Package                             | Requirements                     |
| -------- | ----------------------------------- | -------------------------------- |
| macOS    | `.dmg` (Apple silicon and Intel)    | macOS 13.3 or later              |
| Windows  | `-setup.exe` (x64)                  | Windows 10 or 11                 |
| Linux    | `.AppImage`, `.deb` or `.rpm` (x64) | Ubuntu 22.04, Debian 12 or newer |

The firmware, the protocol decoders and the Python they run on are all included; you don't need DSView or Python. On another platform, you can [build HSync from source](#building-from-source).

### First launch

- **macOS**: HSync isn't notarized by Apple yet, so the first time you open it, macOS says it can't verify the app. Close the message, open **System Settings → Privacy & Security** and click **Open Anyway** next to the note about HSync. You only need to do this once.
- **Windows**: the installer isn't code-signed yet, so SmartScreen may stop it: click **More info → Run anyway**. HSync reaches DSLogic and ATK-Logic devices through the WinUSB driver; if it says no usable USB driver is available, install WinUSB for the device with [Zadig](https://zadig.akeo.ie/) (**Options → List All Devices**, select the device, choose **WinUSB** and click **Install Driver** or **Replace Driver**).
- **Linux**: to use a DSLogic or an ATK-Logic without root, add udev rules for them, then plug the device in again. DSView installs the DreamSourceLab rule, so you can skip that one if you have it.

  ```sh
  sudo tee /etc/udev/rules.d/60-dreamsourcelab.rules > /dev/null <<'EOF'
  SUBSYSTEM=="usb", ATTRS{idVendor}=="2a0e", MODE="0666"
  EOF
  sudo tee /etc/udev/rules.d/60-atk-logic.rules > /dev/null <<'EOF'
  SUBSYSTEM=="usb", ATTRS{idVendor}=="1a86", ATTRS{idProduct}=="ffcc", MODE="0666"
  EOF
  sudo udevadm control --reload-rules
  sudo udevadm trigger
  ```

### Updates

HSync looks for a new version each time it starts (turn this off under **Settings → Behaviour**). When one is out, a dot appears on the settings button: open **Settings → About** and click **Download and restart**.

## Getting started

1. **Open a device.** Connect your logic analyzer and choose it from the device menu at the top left. The first time you open a DSLogic, HSync loads its firmware, which takes a few seconds. Without hardware, you're on the **Demo device**.
2. **Set up the capture.** The button next to the device menu shows the sample rate, duration and channel count; click it to change them and the device's options: the operation mode (Buffer or Stream), the threshold and, on a DSLogic, the filter and the clock.
3. **Set a trigger** (optional). Click the trigger slot to the right of a channel's name and pick an edge or a level. **Trigger** in the top bar has the trigger position, stage triggers and serial triggers.
4. **Capture.** Press **Capture**. The arrow beside it switches between **Single**, **Repeat** and **Loop**, or starts an **Instant** capture that ignores the trigger.
5. **Decode.** Click **Add decoder**, search for a protocol and choose its channels. Open the **Data stream** from the status bar to list everything it decoded.
6. **Look closer.** Ctrl/⌘ + wheel zooms, dragging pans and `Home` shows the whole capture. Hover a pulse to read it, click two edges to measure between them and press `/` to search.
7. **Save.** ⌘S / Ctrl+S saves a `.dsl` file that DSView opens too; **Export…** in the export menu writes CSV, VCD, sigrok and binary files.

The [user guide](docs/user-guide.md) covers every feature and keyboard shortcut.

## Supported devices

| Device              | Channels | USB      |
| ------------------- | -------- | -------- |
| DSLogic             | 16       | 2.0      |
| DSLogic Pro         | 16       | 2.0      |
| DSLogic Plus        | 16       | 2.0      |
| DSLogic Basic       | 16       | 2.0      |
| DSLogic U2Basic     | 16       | 2.0      |
| DSLogic U2Pro16     | 16       | 2.0      |
| DSLogic U3Pro16     | 16       | 3.0      |
| DSLogic U3Pro32     | 32       | 3.0      |
| ATK-Logic DL16      | 16       | 2.0      |
| ATK-Logic DL16 Plus | 16       | 2.0      |
| Demo device         | 16       | built in |

HSync talks to DSLogic devices the way DSView does, and to ATK-Logic devices the way ALIENTEK's ATK-Logic app does, byte for byte. It has been tested on a DSLogic U2Pro16; if you use another model, please [let us know how it went](https://github.com/HalfSweet/HSync/issues/new). DSCope oscilloscopes, DSView's oscilloscope and data acquisition modes, and the PWM outputs of ATK-Logic devices aren't supported yet. To update the firmware of an ATK-Logic device, use ALIENTEK's ATK-Logic app.

## Questions

**Do I need DSView?**
No. HSync brings its own firmware, decoders and Python runtime. You can keep DSView installed alongside it, and HSync opens DSView's captures and sessions.

**Can I take a capture to other tools?**
Yes: export it as a sigrok session for PulseView, as VCD for GTKWave and other waveform viewers, or as CSV for spreadsheets and scripts.

**Where are the logs?**
Open **Settings**, and under **Diagnostics** click **Show in folder**. Attach the latest log when you [report a problem](https://github.com/HalfSweet/HSync/issues/new).

## Building from source

You need:

- [Node.js](https://nodejs.org/) 22.12 or later and [pnpm](https://pnpm.io/) 12
- [Rust](https://rustup.rs/) (stable)
- [Tauri's system dependencies](https://tauri.app/start/prerequisites/) for your platform
- Python 3 with its shared library, for `pnpm tauri dev` (the protocol decoders run on it): install `python3-dev` on Debian and Ubuntu; the python.org installers and Homebrew's Python already include it

```sh
git clone https://github.com/HalfSweet/HSync.git
cd HSync
pnpm install
pnpm tauri dev    # run HSync from source
pnpm bundle       # build the installers for this machine
```

`pnpm bundle` builds the way releases are built: it downloads the Python runtime that ships with HSync and puts the installers in `target/release/bundle/`. On macOS it makes one build for Apple silicon and Intel, in `target/universal-apple-darwin/release/bundle/`; first run `rustup target add aarch64-apple-darwin x86_64-apple-darwin`. To contribute, start with [AGENTS.md](AGENTS.md), which covers the project's requirements, commands and commit conventions.

## Acknowledgements

- [DSView](https://github.com/DreamSourceLab/DSView) by DreamSourceLab: the device support and protocol decoders are ported from its libsigrok4DSL and libsigrokdecode4DSL, which grew out of the [sigrok](https://sigrok.org/) project.
- The DSLogic firmware and FPGA bitstreams come from DreamSourceLab under the MIT license.
- [ATK-Logic](https://github.com/alientek-openedv/atk-logic) by ALIENTEK: the support for ATK-Logic devices is ported from it.
- [IBM Plex](https://github.com/IBM/plex) fonts, under the SIL Open Font License 1.1.
- [Lucide](https://lucide.dev/) icons, under the ISC license.
- Built with [Tauri](https://tauri.app/), [Vue](https://vuejs.org/) and [Rust](https://www.rust-lang.org/).

## License

HSync is free software, released under the [GNU General Public License v3.0 or later](LICENSE).
