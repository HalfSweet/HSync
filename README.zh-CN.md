<div align="center">

<img src="assets/brand/app-icon.svg" width="112" height="112" alt="HSync 图标">

# HSync

**现代的通用逻辑分析仪上位机**

采集、触发、解码、测量：多家厂商的逻辑分析仪，用同一个上位机。

[![Release](https://img.shields.io/github/v/release/HalfSweet/HSync?style=flat-square&label=release&color=45658D)](https://github.com/HalfSweet/HSync/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/HalfSweet/HSync/total?style=flat-square&color=277165)](https://github.com/HalfSweet/HSync/releases)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-665A8A?style=flat-square)](#下载)
[![License](https://img.shields.io/badge/license-Apache--2.0%20OR%20GPL--3.0-845169?style=flat-square)](#许可)

[English](README.md) · **简体中文**

[下载](#下载) · [快速上手](#快速上手) · [用户指南（英文）](docs/user-guide.md) · [反馈问题](https://github.com/HalfSweet/HSync/issues/new)

</div>

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/screenshots/zh-CN/hero-dark.webp">
  <img src="assets/screenshots/zh-CN/hero-light.webp" alt="HSync 显示一段包含 I²C、UART、SPI 与 CAN-FD 总线的采集：解码结果位于对应波形上方，右侧是 I²C 的数据流">
</picture>

HSync 是不绑定任何厂商的逻辑分析仪桌面上位机。它支持 DreamSourceLab 的 DSLogic、正点原子（ALIENTEK）的 ATK-Logic 与 PXLogic 的 PX-Logic 系列，并实验性支持矽速（Sipeed）的 SLogic16U3（[完整列表](#支持的设备)），每台设备都能用上同样的功能。接上设备，按下「采集」：I²C、UART、SPI 与 CAN 的解码结果就显示在对应波形的正上方，采集还没结束就能看到，另有 30 种协议解码器搜索即用。HSync 以自己的 `.hsync` 格式保存采集，能打开、导出 DSView 的 `.dsl` 文件，也能导入 VCD、FST、GHW 波形，以前的采集照常可用；运行所需的一切都已打包在内：设备固件与解码器。

手边没有硬件？HSync 会打开内置的「演示设备」，回放一段包含四种总线的采集数据，所有功能都可以马上试用。

## 功能

### 采集

- **支持多家厂商的逻辑分析仪**：全部 DreamSourceLab DSLogic 型号、正点原子 ATK-Logic DL16 与 DL16 Plus、PXLogic 的 PX-Logic 32、16 Pro、16 Plus 与 16 Base，以及实验性支持的矽速 SLogic16U3（[完整列表](#支持的设备)）。HSync 按各厂商自家软件的方式驱动设备，所需的固件与 FPGA 位流自动加载。
- **Buffer 与 Stream 两种模式**：设备支持时，可启用 RLE 压缩让时长超出设备内存；所需内存超过本机一半时会提前提醒。
- **单次、重复、循环、即时四种采集方式。** 重复采集时，屏幕保留上一次的完整结果，新的一次在后台采集；循环采集持续进行，只保留最新的一段。
- **设备选项**：阈值电压、毛刺滤波、外部时钟、外部触发输入等，设置会被记住。
- **支持热插拔。** 打开演示设备时插入逻辑分析仪，HSync 会提示切换过去；USB 短暂断开后，HSync 会重新打开设备并恢复设置。

### 触发

- **触发条件直接设在通道行上。** 点通道名右侧的触发位，选择上升沿、下降沿、任意沿、高电平或低电平。
- **多级触发**最多 16 级，每级由两组条件组合而成，满足指定次数后进入下一级；**串行触发**可匹配最多 16 位的数据值（DSLogic 的 Buffer 模式）。
- **触发位置可调**，标尺以触发点为零点计时。

![触发面板设置为三级触发，条件设在通道 D0、D1 与 D5 上](assets/screenshots/zh-CN/trigger.webp)

### 解码

- **I²C、UART、SPI 与 CAN 解码器**，解码块与波形对齐，逐事件显示总线上实际传输的内容，不猜测其含义。
- **另有 30 种协议**来自 HSync 自己的解码引擎 [uiharu](uiharu/)：从 1-Wire、JTAG、USB 到 USB PD、WS281x 灯珠；读取其他协议输出的解码器（24xx EEPROM、SPI NOR 闪存、Modbus RTU、LIN）叠加在下层协议上。
- **解码跟着采集走**：设备还在填充采集时结果就陆续出现，期间也能添加或修改解码器。
- **自己编写解码器**：用 HSync 的 TypeScript 解码器语言编写，可自带多语言名称，在设置中加载即可使用，无需重新构建应用，用到它的采集文件会随文件携带；与内置解码器同名的模块会替代它。
- **数值可用十六进制、十进制、八进制、二进制或 ASCII 显示**，每个解码器各自选择。
- 波形右侧的**数据流**列出一个解码器的全部结果：点一行跳到波形上的对应位置并查看它的全部字段，也可以复制列表或导出为 CSV、文本。

![搜索 0x4F：含有它的 I²C 数据字节在时间线与数据流中都被高亮](assets/screenshots/zh-CN/search.webp)

### 测量与搜索

- **全程概览**显示整段采集与各解码器的活动区间，哪里有数据一目了然，点一下就能跳过去。
- **悬停在脉冲上**读出宽度，以及它与下一个脉冲构成的周期、频率与占空比。
- **单击两个边沿**测量间隔，同一通道或跨通道均可，指针会自动吸附到边沿。
- **在标尺上拖出选段**，查看时长以及每个通道在段内的上升沿、下降沿数量。
- **搜索**解码结果中的数值、字节序列或事件种类（`0x4F 0x32`、`address`），或按电平条件查找（如 `D3=R D5=1`）。

![暗色主题下测量一个 SCL 周期（10.04 µs），并列出选段内各通道的边沿数](assets/screenshots/zh-CN/measure.webp)

### 文件

- **以 `.hsync` 文件保存采集**，解码器、通道名、颜色与视图一并保存；**打开各版本的 DSView `.dsl` 采集文件**，双方都支持的解码器会一并接手；导出的 `.dsl` 文件 DSView 也能打开，通道名一并保存。
- **导入 VCD、FST、GHW 波形**：仿真器与 FPGA 片上逻辑分析仪（Vivado ILA、Signal Tap 等）导出的波形，勾选信号后每一位成为一个通道，采样率就是文件当初的采样时钟。
- **导出 DSView `.dsl`、CSV、VCD、FST、sigrok `.sr` 与原始二进制**：可以导出全程或选段、全部通道或部分通道。
- **会话**保存设备设置、通道、触发与解码器；也能加载 DSView 的 `.dsc` 会话。

### 界面

- **「亮色」与「暗色」两种主题**，「舒适」与「紧凑」两种密度，简体中文与英文界面。
- **支持时用 WebGPU 绘制波形**，不支持时回退到 Canvas2D。
- **快捷键**覆盖缩放、平移、搜索、打开、保存等操作，均可自定义。
- **应用内更新**：启动时检查新版本，一键安装。

## 下载

在 [Releases 页面](https://github.com/HalfSweet/HSync/releases/latest)下载最新版本。

| 平台    | 安装包                               | 系统要求                           |
| ------- | ------------------------------------ | ---------------------------------- |
| macOS   | `.dmg`（Apple 芯片与 Intel）         | macOS 13.3 或更高版本              |
| Windows | `-setup.exe`（x64）                  | Windows 10 或 11                   |
| Linux   | `.AppImage`、`.deb` 或 `.rpm`（x64） | Ubuntu 22.04、Debian 12 或更新版本 |

固件与协议解码器都已包含在内，无需另装厂商软件。其他平台可以[从源码构建](#从源码构建)。

### 首次运行

- **macOS**：HSync 尚未经过 Apple 公证，第一次打开时 macOS 会提示无法验证此 App。关闭提示后，打开「系统设置 → 隐私与安全性」，在关于 HSync 的提示旁点击「仍要打开」。只需操作一次。
- **Windows**：安装程序尚未签名，SmartScreen 可能会拦截：点击「更多信息 → 仍要运行」。HSync 通过 WinUSB 驱动访问逻辑分析仪；如果提示没有可用的 USB 驱动，用 [Zadig](https://zadig.akeo.ie/) 为设备安装 WinUSB（「Options → List All Devices」，选中该设备，选择「WinUSB」后点击「Install Driver」或「Replace Driver」）。
- **Linux**：要以普通用户身份使用逻辑分析仪，为它的厂商添加 udev 规则，然后重新插拔设备。DSView 与 PXView 会安装各自的规则，已覆盖的可以跳过。

  ```sh
  # DreamSourceLab DSLogic
  sudo tee /etc/udev/rules.d/60-dreamsourcelab.rules > /dev/null <<'EOF'
  SUBSYSTEM=="usb", ATTRS{idVendor}=="2a0e", MODE="0666"
  EOF
  # 正点原子 ATK-Logic
  sudo tee /etc/udev/rules.d/60-atk-logic.rules > /dev/null <<'EOF'
  SUBSYSTEM=="usb", ATTRS{idVendor}=="1a86", ATTRS{idProduct}=="ffcc", MODE="0666"
  EOF
  # PXLogic PX-Logic
  sudo tee /etc/udev/rules.d/60-pxlogic.rules > /dev/null <<'EOF'
  SUBSYSTEM=="usb", ATTRS{idVendor}=="1a86", ATTRS{idProduct}=="5237", MODE="0666"
  SUBSYSTEM=="usb", ATTRS{idVendor}=="16c0", ATTRS{idProduct}=="05dc", ATTRS{manufacturer}=="PX*", MODE="0666"
  EOF
  # 矽速 SLogic16U3
  sudo tee /etc/udev/rules.d/60-sipeed-slogic.rules > /dev/null <<'EOF'
  SUBSYSTEM=="usb", ATTRS{idVendor}=="359f", ATTRS{idProduct}=="3031", MODE="0666"
  EOF
  sudo udevadm control --reload-rules
  sudo udevadm trigger
  ```

### 更新

HSync 每次启动时检查新版本（可在「设置 → 行为」里关闭）。有新版本时，设置按钮上会出现一个小圆点：打开「设置 → 关于」，点击「下载并重启」。

## 快速上手

1. **打开设备。** 连接逻辑分析仪，在左上角的设备菜单里选中它。有些设备第一次打开时，HSync 要先加载固件，需要几秒钟。没有硬件时，当前设备就是「演示设备」。
2. **设置采集。** 设备菜单右边的按钮显示采样率、时长与通道数，点它即可修改这些以及设备选项：运行模式（Buffer / Stream）、阈值，以及设备提供的其他设置，如毛刺滤波或外部时钟。
3. **设置触发**（可选）。点通道名右侧的触发位，选择一种边沿或电平。顶栏的「触发」里有触发位置、多级触发与串行触发。
4. **采集。** 点「采集」。旁边的下拉箭头可以在「单次」「重复」「循环」之间切换，或发起忽略触发条件的「即时」采集。
5. **解码。** 点「添加协议解码」，搜索协议并为它选择通道。点顶栏右侧的「数据流」按钮查看全部解码结果。
6. **细看。** Ctrl/⌘ + 滚轮缩放，拖动平移，`Home` 缩放到全程。指针悬停在脉冲上即可读数，单击两个边沿测量间隔，按 `/` 搜索。
7. **保存。** ⌘S / Ctrl+S 保存为 `.hsync` 文件，解码器、颜色与视图一并保存；导出菜单里的「导出…」可以导出 DSView `.dsl`、CSV、VCD、FST、sigrok 与二进制文件。

全部功能与快捷键见[用户指南](docs/user-guide.md)（英文）。

## 支持的设备

| 厂商           | 设备                 | 通道数 | USB  |
| -------------- | -------------------- | ------ | ---- |
| DreamSourceLab | DSLogic              | 16     | 2.0  |
| DreamSourceLab | DSLogic Pro          | 16     | 2.0  |
| DreamSourceLab | DSLogic Plus         | 16     | 2.0  |
| DreamSourceLab | DSLogic Basic        | 16     | 2.0  |
| DreamSourceLab | DSLogic U2Basic      | 16     | 2.0  |
| DreamSourceLab | DSLogic U2Pro16      | 16     | 2.0  |
| DreamSourceLab | DSLogic U3Pro16      | 16     | 3.0  |
| DreamSourceLab | DSLogic U3Pro32      | 32     | 3.0  |
| 正点原子       | ATK-Logic DL16       | 16     | 2.0  |
| 正点原子       | ATK-Logic DL16 Plus  | 16     | 2.0  |
| PXLogic        | PX-Logic 32          | 32     | 3.0  |
| PXLogic        | PX-Logic 16 Pro      | 16     | 3.0  |
| PXLogic        | PX-Logic 16 Plus     | 16     | 3.0  |
| PXLogic        | PX-Logic 16 Base     | 16     | 3.0  |
| 矽速           | SLogic16U3（实验性） | 16     | 3.0  |
| —              | 演示设备             | 16     | 内置 |

HSync 与每台设备的通信方式都与其厂商的软件逐字节一致：DSLogic 同 DSView，ATK-Logic 同正点原子的 ATK-Logic 软件，PX-Logic 同 PXView。已在 DSLogic U2Pro16 上实测；如果你在用其他型号，欢迎[反馈使用情况](https://github.com/HalfSweet/HSync/issues/new)。DSCope 示波器、DSView 的示波器与数据采集模式、ATK-Logic 设备的 PWM 输出，以及 PX-Logic 设备的触发输出与 PWM 输出暂不支持。ATK-Logic 设备的固件请用正点原子的 ATK-Logic 软件更新。HSync 打开固件版本不同的 PX-Logic 设备时，会先把 PXView 1.6.5 的固件写入设备，与 PXView 对待自家固件的做法相同。

SLogic16U3 驱动使用 Rust 与 nusb，参照[矽速官方的 libsigrok 驱动](https://github.com/sipeed/libsigrok/tree/slogic-dev/src/hardware/sipeed-slogic-analyzer)移植。支持 Stream 采集、简单软件触发、循环采集和 0–6 V 阈值。USB 3.0 下，设备标称 16 通道最高 200 MHz，仅使用 D0–D7 时最高 400 MHz，仅使用 D0–D3 时最高 800 MHz；USB 2.0 下分别限为 20、40 和 80 MHz。驱动使用设备已安装的固件，不会刷写固件。目前属于实验性支持，已通过模拟测试和真机 USB 3.0 基础测试：4、8、16 通道模式下各以 5 MHz 连续采集三次，每次 65,536 个样本；另分别按 800、400、200 MHz 上限完成短采集。通道映射、信号时序、高采样率持续吞吐量和 USB 2.0 仍待真机验证。其他 SLogic 型号及引导加载模式暂不支持。详见[连接与采集说明（英文）](docs/user-guide.md#sipeed-slogic16u3)。

## 常见问题

**需要安装逻辑分析仪厂商的软件吗？**
不需要。HSync 自带固件与解码器。DSView 等厂商软件可以同时安装，HSync 也能打开 DSView 的采集文件与会话。

**能把采集数据带到其他软件里吗？**
可以：导出为 DSView 采集文件后用 DSView 打开，导出为 sigrok 会话后用 PulseView 打开，导出为 VCD 或 FST 后用 GTKWave、Surfer 等波形查看器打开，或导出为 CSV 用表格与脚本处理。反过来，「打开文件…」也能导入这些工具和 FPGA 片上逻辑分析仪写出的 VCD、FST、GHW 波形。

**日志在哪里？**
打开「设置」，在「诊断」下点击「在文件夹中显示」。[反馈问题](https://github.com/HalfSweet/HSync/issues/new)时附上最新的日志会很有帮助。

## 从源码构建

需要：

- [Node.js](https://nodejs.org/) 22.12 或更高版本，以及 [pnpm](https://pnpm.io/) 12
- [Rust](https://rustup.rs/)（stable）
- 所在平台的 [Tauri 系统依赖](https://tauri.app/start/prerequisites/)
