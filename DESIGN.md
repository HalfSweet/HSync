# HSync 设计文档

> 用现代技术栈重写 DSView：一个通用的逻辑分析仪上位机。
> 本文是项目的总体设计，覆盖产品范围、架构、硬件协议、数据模型、渲染、协议解码、文件格式、UI/UX 规范、国际化、工程规范与里程碑。
> 需求与协作约定见 [AGENTS.md](./AGENTS.md)。

| 项目     | 内容                                                                                     |
| -------- | ---------------------------------------------------------------------------------------- |
| 状态     | 草案 v0.1（2026-10-10）                                                                  |
| 参考实现 | DSView v1.3.x（`DreamSourceLab/DSView`，本地路径 `../DSView`）                           |
| 设计输入 | `.hsync/handoff.md` 与 `.hsync/逻辑分析仪界面改版.html`（本地设计稿，已在 `.gitignore`） |
| 许可证   | GPL-3.0-or-later（见 §1.4）                                                              |

---

## 目录

1. [背景与目标](#1-背景与目标)
2. [功能范围：与 DSView 对齐](#2-功能范围与-dsview-对齐)
3. [总体架构](#3-总体架构)
4. [硬件层：Rust + nusb](#4-硬件层rust--nusb)
5. [数据模型与存储](#5-数据模型与存储)
6. [波形渲染](#6-波形渲染)
7. [协议解码](#7-协议解码)
8. [文件格式与导入导出](#8-文件格式与导入导出)
9. [UI/UX 规范](#9-uiux-规范)
10. [国际化](#10-国际化)
11. [工程规范](#11-工程规范)
12. [测试策略](#12-测试策略)
13. [里程碑](#13-里程碑)
14. [风险与对策](#14-风险与对策)
15. [待确认事项](#15-待确认事项)
16. [附录：DSView 源码索引](#16-附录dsview-源码索引)

---

## 1. 背景与目标

### 1.1 DSView 的现状

DSView 是 DreamSourceLab 为 DSLogic（逻辑分析仪）与 DSCope（示波器）提供的上位机，基于 sigrok 分支：

| 组成                  | 技术                                       | 规模          |
| --------------------- | ------------------------------------------ | ------------- |
| `DSView/pv`           | Qt5/C++ GUI                                | 约 5.7 万行   |
| `libsigrok4DSL`       | C + glib + libusb，设备驱动、会话、IO      | 约 2.5 万行   |
| `libsigrokdecode4DSL` | C 运行时嵌入 CPython，151 个 Python 解码器 | 约 350 个文件 |

工程上的主要问题：

- **全局可变状态**：触发配置是全局单例（`trigger.c` 的 `struct ds_trigger *trigger`），设备、会话靠全局上下文串联，难以测试，也无法同时操作两台设备。
- **线程模型脆弱**：采集线程、libusb 事件轮询、热插拔线程、解码线程之间靠全局标志位与 `sleep` 协调。
- **驱动与 UI 耦合**：配置项通过 `GVariant` + 整数 key 传递，取值列表以裸指针形式塞进 `uint64`（如 `SR_CONF_CHANNEL_MODE` 返回 `&channel_mode_list`），中英文文案写死在驱动里。
- **渲染**：Qt `QPainter` 逐帧重画，样式与语义脱节（详见设计稿「原界面的七处问题」，§9.1）。

### 1.2 目标

1. **初版（v1）与 DSView 的逻辑分析仪功能对齐**：支持全部 DSLogic 型号、Demo 设备、Buffer/Stream 采集、简单/高级/串行触发、协议解码、测量、搜索、`.dsl` 文件读写与导出。对齐清单见 §2。
2. **与真实硬件的交互全部用 Rust 重写**，USB 访问使用 [`nusb`](https://crates.io/crates/nusb)（纯 Rust，无 libusb 依赖）。
3. **新的 UI/UX**：按 `.hsync` 设计稿实现单窗口界面，纸面/夜间两套主题、舒适/紧凑两种密度。
4. **国际化**：首版支持简体中文（默认）与英文。
5. **性能**：在 32 通道 × 10 亿采样点的数据上平移、缩放保持 60 fps；Stream 模式下 USB 3.0 满带宽写入不丢数据。
6. **可扩展**：驱动层是通用抽象，后续可接入其他逻辑分析仪（如 fx2lafw 类设备）而不改动上层。

### 1.3 非目标（v1）

| 不做                                          | 说明                                                                          |
| --------------------------------------------- | ----------------------------------------------------------------------------- |
| 示波器（DSO）与数据采集（DAQ）模式            | DSCope 系列及 DSLogic 的模拟模式；驱动抽象为其预留 `Analog` 通道类型，v2 再做 |
| DSCope 校准、FFT、Math、Lissajous             | 仅示波器相关                                                                  |
| DSLogic 的「内部测试 / 外部测试 / DRAM 回环」 | 工厂测试功能，不对普通用户开放（驱动保留实现以便诊断，见 §4.7）               |
| 固件/FPGA 位流的修改                          | 直接复用 DSView 发布的二进制                                                  |

### 1.4 许可证

- `libsigrok4DSL`、`libsigrokdecode4DSL` 为 GPL-3.0-or-later / GPL-2.0-or-later，DSView 整体按 GPL-3.0-or-later 发布。本项目移植其驱动逻辑、复用其解码器与 Demo 数据，因此 **整体采用 GPL-3.0-or-later**。
- `DSView/res` 下的固件（`*.fw`）与 FPGA 位流（`*.bin`）由 DreamSourceLab 以 **MIT** 许可发布（`DSView/res/license.txt`），可随应用分发，需保留其版权声明。
- 字体 IBM Plex Sans / Mono 为 SIL OFL 1.1，随应用打包。

---

## 2. 功能范围：与 DSView 对齐

下表列出 DSView 逻辑分析模式下的全部用户可见功能，以及在新界面中的归宿。标记：**v1** 首版实现；**v1.x** 首版之后的小版本；**—** 不做。

### 2.1 设备与采集

| DSView 功能                                         | HSync                                          | 版本 |
| --------------------------------------------------- | ---------------------------------------------- | ---- |
| 设备列表、热插拔、自动选中新设备                    | 顶栏「设备选择」下拉；插入新设备时提示是否切换 | v1   |
| Demo 设备（随机 / 协议演示数据）                    | 内置 Demo 设备，回放随包的演示采集             | v1   |
| 固件自动下载（DSLogic、DSLogic Pro）、FPGA 位流加载 | 驱动自动完成，打开设备时显示进度               | v1   |
| 采样率、采样深度（按时长选择，含 RLE 提示）         | 顶栏「采集设置」弹出层：采样率 / 时长 / 通道数 | v1   |
| 单次 / 重复（间隔）/ 循环 采集                      | 「采集」按钮旁的下拉：单次、重复、循环、即时   | v1   |
| 即时采集（Instant，忽略触发）                       | 同上                                           | v1   |
| Buffer / Stream 运行模式                            | 设备选项对话框                                 | v1   |
| 通道模式（如 16ch@20MHz、12ch@25MHz…）              | 设备选项对话框                                 | v1   |
| 阈值电压（VTH 连续可调 / 3.3V·5V 两档）             | 设备选项对话框                                 | v1   |
| 输入滤波（1 个采样周期）、RLE 压缩                  | 设备选项对话框                                 | v1   |
| 外部时钟、时钟边沿                                  | 设备选项对话框                                 | v1   |
| Buffer 模式停止策略（立即停止 / 上传已采集数据）    | 设备选项对话框                                 | v1   |
| 通道启用/禁用、改名、颜色                           | 通道行的右键菜单与「采集设置」弹出层           | v1   |
| 采集进度（等待触发、已采集百分比）                  | 状态栏                                         | v1   |
| USB 2.0 口降速告警、带宽不足（溢出）告警            | 状态栏消息 + 一次性提示                        | v1   |
| 设备默认会话（`*.def.dsc`）                         | 读取后作为设备初始配置                         | v1   |

### 2.2 触发

| DSView 功能                                                 | HSync                                                              | 版本 |
| ----------------------------------------------------------- | ------------------------------------------------------------------ | ---- |
| 简单触发：每通道 X/0/1/上升/下降/任意沿                     | **通道行上的触发位**（设计稿 §9.6），多个通道同时满足（AND）才触发 | v1   |
| 触发位置（预触发比例 1–90%，Stream 模式固定 1%）            | 顶栏「触发」下拉面板（§9.7.1，设计待补）                           | v1   |
| 高级触发：最多 16 级、每级两组条件、与/或、取反、计数、连续 | 顶栏「触发」下拉面板中的「多级触发」（设计待补）                   | v1   |
| 串行触发：起始/停止/时钟/数据通道/数据值/位宽               | 同上（设计待补）                                                   | v1   |
| 多通道触发提示框                                            | 由触发菜单底部说明「同时满足才触发」替代                           | v1   |

### 2.3 查看与测量

| DSView 功能                                    | HSync                                                                         | 版本 |
| ---------------------------------------------- | ----------------------------------------------------------------------------- | ---- |
| 时间轴缩放、平移、缩放到全程                   | 状态栏缩放按钮 + 指针/键盘手势（§9.10）                                       | v1   |
| 全程概览                                       | 「全程概览」条带（设计稿新增）                                                | v1   |
| 光标（多个）、光标间时间差                     | 选段 A/B 两旗（设计稿）+ 可添加的命名光标（v1.x）                             | v1   |
| 悬停测量：脉宽、周期、频率、占空比             | **悬停读数**（设计稿）                                                        | v1   |
| 光标区间测量：上升沿/下降沿/边沿数             | 选段后在状态栏显示区间测量                                                    | v1   |
| 通道分组、组展开收起                           | **按解码器分组**，未绑定通道归入「未命名通道」组（设计稿）                    | v1   |
| 模式搜索（每通道 X/0/1/R/F/C，前后跳转）       | 顶栏搜索框：既搜解码结果（设计稿），也支持 `D3=R D5=1` 形式的电平模式（§9.8） | v1   |
| 主题（亮/暗）、字号、标题栏显示配置文件        | 设置面板：纸面/夜间/跟随系统、舒适/紧凑                                       | v1   |
| 触发点居中显示、拖动滚动波形、自动滚到最新数据 | 设置面板                                                                      | v1   |
| 重复模式停止时保留最新数据                     | 设置面板                                                                      | v1   |

### 2.4 协议解码

| DSView 功能                                                 | HSync                                                    | 版本 |
| ----------------------------------------------------------- | -------------------------------------------------------- | ---- |
| 151 个解码器（含叠加解码器，如 I²C → EEPROM24xx）           | 原生 Rust 解码器 + sigrok 兼容的 Python 解码器宿主（§7） | v1   |
| 解码器选项、通道映射、解码起止光标                          | 「添加协议解码」对话框；起止范围用选段                   | v1   |
| 注解行显隐、数值进制（十六进制/十进制/八进制/二进制/ASCII） | 解码器组标题行菜单                                       | v1   |
| 解码结果表（Protocol dock）、搜索、跳转                     | **数据流面板**（设计稿），点击一项跳到波形               | v1   |
| 解码结果导出（CSV/TXT）                                     | 数据流面板「导出」                                       | v1   |

### 2.5 文件

| DSView 功能                                | HSync                                          | 版本 |
| ------------------------------------------ | ---------------------------------------------- | ---- |
| 打开/保存 `.dsl`（v1/v2/v3 读取，v3 写入） | 完全兼容（§8.1）                               | v1   |
| 保存选段内的数据                           | 保存对话框中的「仅选段」                       | v1   |
| 导出 CSV、VCD、srzip(sigrok)、Gnuplot 等   | 导出对话框：CSV、VCD、sigrok `.sr`、原始二进制 | v1   |
| 会话（`.dsc`）读写、加载默认会话           | 兼容读取；新格式见 §8.3                        | v1   |
| 日志选项（级别、写文件）                   | 设置面板「诊断」                               | v1   |

---

## 3. 总体架构

### 3.1 进程与分层

```
┌──────────────────────────── WebView（Vue 3） ────────────────────────────┐
│  components/   顶栏 · 全程概览 · 时间轴 · 数据流面板 · 状态栏 · 对话框      │
│  stores/       Pinia：device · capture · view · channels · decoders · prefs │
│  render/       WaveformRenderer（Canvas2D 基线 / WebGPU 加速）              │
│  ipc/          类型安全的 invoke / 事件 / 二进制通道封装（类型由 ts-rs 生成） │
└───────────────▲──────────────────────────────────────────────┬───────────┘
                │ Tauri IPC：command（JSON）· event · Response（二进制）       │
┌───────────────┴──────────────────── Rust（Tauri 进程） ───────┴───────────┐
│ src-tauri        应用壳：命令、事件、状态、设置持久化                       │
│ hsync-session    会话：设备 ↔ 采集 ↔ 解码 的编排，后台任务与取消             │
│ hsync-driver     设备抽象（Driver/Device trait）、设备管理、nusb 热插拔      │
│ hsync-dslogic    DSLogic 驱动（移植自 libsigrok4DSL，nusb）                 │
│ hsync-demo       Demo 设备                                                  │
│ hsync-capture    逻辑数据存储：按通道位压缩 + 多级摘要（LOD）、查询、测量   │
│ hsync-decode     解码框架、注解模型、原生解码器、sigrok 兼容宿主（pyo3）    │
│ hsync-fileio     .dsl 读写、导出（CSV/VCD/sr/bin）、会话文件                │
│ hsync-core       公共类型：采样率、时间、通道、触发模型、错误、配置项       │
└────────────────────────────────────────────────────────────────────────────┘
```

依赖方向严格自上而下：`src-tauri → hsync-session → {driver, capture, decode, fileio} → hsync-core`；`hsync-dslogic`、`hsync-demo` 实现 `hsync-driver` 的 trait。任何 crate 都不依赖 Tauri，便于单测与命令行工具复用。

### 3.2 仓库结构

```
HSync/
├── AGENTS.md / DESIGN.md / README.md / LICENSE
├── package.json · pnpm-lock.yaml · pnpm-workspace.yaml
├── lefthook.yml · commitlint.config.js · eslint.config.js · prettier.config.js
├── tsconfig*.json · vite.config.ts · index.html
├── Cargo.toml（workspace）· rustfmt.toml · clippy.toml · rust-toolchain.toml
├── src/                     # Vue 前端
│   ├── main.ts · App.vue
│   ├── styles/              # Tailwind 入口、设计令牌（tokens.css）
│   ├── components/          # 按区域分目录：header/ overview/ timeline/ stream/ status/ dialogs/ common/
│   ├── composables/         # 视口换算、指针手势、键盘、可访问性
│   ├── stores/              # Pinia
│   ├── render/              # 波形渲染器
│   ├── ipc/                 # invoke 封装与生成的绑定（bindings/ 由 ts-rs 生成，勿手改）
│   ├── i18n/                # vue-i18n 初始化与 locales/zh-CN.json、en-US.json
│   └── utils/               # 单位格式化、数值显示
├── src-tauri/               # Tauri 应用 crate
│   ├── tauri.conf.json · capabilities/ · icons/
│   ├── resources/firmware/  # DSLogic 固件与 FPGA 位流（MIT）
│   ├── resources/demo/      # Demo 采集（来自 DSView/demo）
│   └── src/
├── crates/
│   ├── hsync-core/ hsync-capture/ hsync-driver/ hsync-dslogic/
│   ├── hsync-demo/ hsync-decode/ hsync-fileio/ hsync-session/
└── tests/fixtures/          # 跨 crate 共享的测试数据（小型 .dsl、golden 字节流）
```

### 3.3 线程与异步模型

- Tauri 命令运行在 tokio 运行时上；**USB 传输**使用 nusb 的异步 `Endpoint::next_complete()`，每台设备一个采集任务（tokio task），不再需要 libusb 那样的事件轮询线程。
- **数据写入**：采集任务把收到的 USB 缓冲区通过有界通道（`crossbeam-channel` 或 `tokio::mpsc`）交给存储任务，存储任务负责解交织与摘要更新；缓冲区用完归还给采集任务复用（零分配稳态）。
- **解码**：每个解码器栈一个工作线程（CPU 密集，`std::thread` 或 `rayon`），按块增量消费采样数据，支持取消；Python 解码器在同一线程内持有 GIL。
- **前端通知**：状态变化走 Tauri event；高频数据（采集进度、新增样本数）合并后每 ≥33 ms 发一次，避免淹没 WebView。
- **共享状态**：`Arc<RwLock<…>>`（`parking_lot`）只读多写少；采集中的存储使用「追加者独占 + 读者看已提交长度」的无锁快照（原子 `committed_len`）。

### 3.4 IPC 设计

| 种类    | 用途                                         | 形式                                                           |
| ------- | -------------------------------------------- | -------------------------------------------------------------- |
| command | 请求/响应：设备列表、设置选项、开始/停止采集 | `invoke('device_set_option', { … })`，参数/返回值为 serde 类型 |
| event   | 后端推送：设备增减、采集状态、进度、解码进度 | `listen('capture://state', …)`                                 |
| 二进制  | 视口波形、解码块、导出流                     | 命令返回 `tauri::ipc::Response`（`ArrayBuffer`），格式见 §5.4  |
| Channel | 长任务的流式结果（保存/导出进度、解码增量）  | `tauri::ipc::Channel<T>`                                       |

- 所有跨端类型在 Rust 端定义并用 **ts-rs** 生成 TypeScript（`src/ipc/bindings/`），前端不得手写重复定义。
- 错误统一为 `{ code: string, params: Record<string, unknown> }`，`code` 是 i18n key（如 `error.device.firmwareMissing`），前端负责翻译（§10.4）。

---

## 4. 硬件层：Rust + nusb

本章是 `libsigrok4DSL/hardware/DSL/{dsl.c,dslogic.c,dsl.h,command.c}`、`trigger.c`、`hardware/common/ezusb.c` 与 `lib_main.c` 设备管理部分的移植规格。**行为必须与 DSView 逐字节一致**，任何偏离都要在代码注释里写明原因。

### 4.1 驱动抽象

```rust
pub trait Driver: Send + Sync {
    fn id(&self) -> &'static str;                       // "dslogic" / "demo"
    fn matches(&self, info: &UsbDeviceInfo) -> Option<ProbeResult>; // 识别 VID/PID/速度
    fn open(&self, candidate: Candidate) -> BoxFuture<Result<Box<dyn Device>>>;
}

pub trait Device: Send {
    fn info(&self) -> &DeviceInfo;                      // 型号、序列、能力
    fn options(&self) -> Vec<OptionDescriptor>;         // 可设置项及其取值范围（含 i18n key）
    fn get(&self, key: OptionKey) -> Result<OptionValue>;
    fn set(&mut self, key: OptionKey, value: OptionValue) -> Result<OptionEffects>;
    fn channels(&self) -> &[ChannelInfo];
    fn start(&mut self, req: AcquisitionRequest, sink: SampleSink) -> BoxFuture<Result<Acquisition>>;
}
```

- `OptionKey` 是枚举（`SampleRate`、`SampleLimit`、`OperationMode`、`ChannelMode`、`Threshold`、`Vth`、`Filter`、`Rle`、`ClockType`、`ClockEdge`、`BufferOption`、`TriggerPos` …），取代 DSView 的整数 key + `GVariant`。
- `OptionDescriptor` 携带取值列表与依赖关系（如通道模式决定可选采样率），UI 据此动态生成表单；文案由前端按 key 翻译，驱动不含任何自然语言。
- `SampleSink` 接收统一的数据包：`Header`、`Trigger { sample }`、`Logic(LogicChunk)`、`Overflow`、`Progress { triggered, captured }`、`End { status }`。

### 4.2 支持的设备

VID 统一为 `0x2A0E`（DreamSourceLab）。同一 PID 按 USB 速度区分配置（USB 2.0 = High，USB 3.0 = Super）。

| PID      | 型号            | 速度  | 固件（需下载时） | FPGA 位流（3.3V / 5V）            | 通道 | 硬件深度（样本·通道） | 特性                           |
| -------- | --------------- | ----- | ---------------- | --------------------------------- | ---- | --------------------- | ------------------------------ |
| `0x0001` | DSLogic         | HS    | `DSLogic.fw`     | `DSLogic33.bin` / `DSLogic50.bin` | 16   | 256 Mi                | SEEP, BUF                      |
| `0x0003` | DSLogic Pro     | HS    | `DSLogicPro.fw`  | `DSLogicPro.bin`                  | 16   | 256 Mi                | SEEP, VTH, BUF                 |
| `0x0020` | DSLogic Plus    | HS    | —                | `DSLogicPlus.bin`                 | 16   | 256 Mi                | VTH, BUF                       |
| `0x0021` | DSLogic Basic   | HS    | —                | `DSLogicBasic.bin`                | 16   | 256 Ki                | VTH                            |
| `0x0029` | DSLogic U2Basic | HS    | —                | `DSLogicU2Basic.bin`              | 16   | 64 Mi                 | VTH, BUF                       |
| `0x002A` | DSLogic U3Pro16 | HS/SS | —                | `DSLogicU3Pro16.bin`              | 16   | 2 Gi                  | VTH, BUF, USB30, ADF4360       |
| `0x002C` | DSLogic U3Pro32 | HS/SS | —                | `DSLogicU3Pro32.bin`              | 32   | 2 Gi                  | VTH, BUF, USB30, ADF4360, CH32 |
| `0x002D` | DSLogic U2Pro16 | HS    | —                | `DSLogicU2Pro16.bin`              | 16   | 4 Gi                  | VTH, BUF, ADF4360, SECURITY    |
| `0x0030` | DSLogic Plus    | HS    | —                | `DSLogicPlus-pgl12.bin`           | 16   | 256 Mi                | VTH, BUF, MAX25_VTH, SECURITY  |
| `0x0031` | DSLogic U2Basic | HS    | —                | `DSLogicU2Basic-pgl12.bin`        | 16   | 64 Mi                 | VTH, BUF, MAX25_VTH, SECURITY  |
| `0x0034` | DSLogic Plus    | HS    | —                | `DSLogicPlus-pgl12-2.bin`         | 16   | 256 Mi                | VTH, BUF, MAX25_VTH, SECURITY  |
| `0x0035` | DSLogic U2Basic | HS    | —                | `DSLogicU2Basic-pgl12-2.bin`      | 16   | 64 Mi                 | VTH, BUF, MAX25_VTH, SECURITY  |

- 硬件深度是板载存储的「样本 × 通道」总量，单通道可用深度 = `(硬件深度 / 启用通道数) & !1023`；无 `BUF` 能力的型号（Basic）只能用 Stream 模式。
- 只有带 `SEEP`（小启动 EEPROM）的老型号需要主机下载 FX2 固件；其余型号从板载存储启动。仓库中没有的 `*.fw` 文件名照抄 DSView，缺失时报 `error.device.firmwareMissing`。
- DSCope 的 PID（`0x0002`、`0x0004`、`0x0022`–`0x0028`、`0x002B`）在 v1 中识别为「暂不支持的设备」，在设备列表中灰显，不再静默跳过。
- 速度低于 High（< 480 Mbps）时提示「USB 端口速度过低」（DSView 的 `DS_EV_DEVICE_SPEED_NOT_MATCH`）。

**能力位**（`CAPS_FEATURE_*`）与 **通道模式表**（`channel_modes[]`，30 项，含 Stream/Buffer、最大采样率、预分频 `pre_div`）原样移植为 `const` 表，见 `dsl.h:49-441`。

### 4.3 枚举、固件下载与热插拔

1. 使用 `nusb::list_devices()` 枚举，`nusb::watch_devices()` 监听热插拔（替代 libusb hotplug + 轮询线程）。
2. 对 VID 匹配的设备读取字符串描述符：厂商 `"DreamSourceLab"` 且产品 `"USB-based DSL Instrument v2"` 视为固件已就绪；否则执行 **EZ-USB 固件下载**：
   - `set_configuration(1)`；
   - 厂商 OUT 请求 `0xA0`，`wValue=0xE600`，数据 `[0x01]`：CPU 复位；
   - 按 4096 字节分块，厂商 OUT 请求 `0xA0`，`wValue=偏移`，`wIndex=0`，写入 `*.fw` 原始镜像；
   - `wValue=0xE600` 写 `[0x00]`：释放复位，设备重枚举。
3. 设备在下载固件或掉线后会以新地址重新出现。管理器维持「等待重连」窗口（DSView 为 500 ms，`CHECK_DEV_RECONNECT_TIMES`）：窗口内出现同型号设备视为同一设备，自动重新打开并恢复配置；超时才发出「设备移除」。
4. 正在采集的设备被拔出：立即结束采集任务，`capture://live` 事件以 `end: "detached"` 结束。
5. **macOS 上的未配置设备**：没有驱动认领的厂商类设备在 macOS 上保持「未配置」，IORegistry 中没有接口，认领接口 0 会报「未找到」。设备固件对 GET_CONFIGURATION 仍回答 1，nusb 据此推断的当前配置不可信。做法与 libusb 相同：认领失败且错误为「未找到」时 `set_configuration(1)` 后重试（DSLogic U2Pro16 实测）。

### 4.4 控制协议

设备只用接口 0（配置 1），**所有控制传输经已认领的 `Interface` 发出**（nusb 在 Windows 上不支持 `Device::control_*`）。

| 请求         | 方向 | `bRequest` | 载荷                                               |
| ------------ | ---- | ---------- | -------------------------------------------------- |
| `CTL_WR`     | OUT  | `0xB0`     | `ctl_header` + 数据（≤60 字节）                    |
| `CTL_RD_PRE` | OUT  | `0xB1`     | `ctl_header`（4 字节）                             |
| `CTL_RD`     | IN   | `0xB2`     | 读取 `size` 字节；须在 `CTL_RD_PRE` 之后等待 10 ms |

`ctl_header` 为紧凑小端结构：`dest: u8, offset: u16, size: u8`。超时统一 3000 ms。

`dest` 取值（`command.h`）：

| 值  | 名称               | 用途                                  |
| --- | ------------------ | ------------------------------------- |
| 0   | `FW_VERSION`       | 读 2 字节 major/minor，要求 major = 2 |
| 2   | `HW_STATUS`        | 读 1 字节状态，见下                   |
| 3   | `PROG_B`           | FPGA 编程引脚                         |
| 5   | `LED`              | 指示灯                                |
| 6   | `INTRDY`           | 数据就绪握手                          |
| 7   | `WORDWIDE`         | GPIF 16 位宽                          |
| 8   | `START` / 9 `STOP` | 开始 / 停止 GPIF 采集（size = 0）     |
| 10  | `BULK_WR`          | 预告随后批量写入的长度（3 字节小端）  |
| 12  | `NVM`              | EEPROM 读写，`offset` 为地址          |
| 14  | `I2C_REG`          | 写 FPGA 寄存器，`offset` 为寄存器地址 |
| 15  | `I2C_STATUS`       | 读 FPGA 状态寄存器                    |
| 24  | `I2C_EXT`          | 扩展 I²C（POGOPIN 型号）              |

`HW_STATUS` 位：`7 GPIF_DONE`、`6 FPGA_DONE`、`5 FPGA_INIT_B`、`4 SYS_OVERFLOW`、`3 SYS_CLR`、`2 SYS_EN`、`1 LED_RED`、`0 LED_GREEN`。

FPGA 寄存器：`CTR0 = 0x70`（`bmFORCE_RDY = 1<<1`、`bmFORCE_STOP = 1<<2`），`VTH = 0x78`，`SEC_CTRL = 0x73`，`SEC_DATA = 0x75`，`ADCC = 0x48`，`HW_STATUS_ADDR = 0x05`，`HDL_VERSION_ADDR = 0x04`。

### 4.5 打开设备

1. `open()` → 读 `FW_VERSION`；major ≠ 2 时标记为「固件不兼容」（`error.device.firmwareIncompatible`）。
2. 认领接口 0；失败按原因区分「设备被占用」「驱动未安装（Windows 需 WinUSB）」。
3. 读 `HW_STATUS`：
   - `FPGA_DONE` 未置位 → 按阈值档位选择位流并**配置 FPGA**（§4.6）；
   - 已置位 → 写 `CTR0 = 0`，读 `I2C_STATUS` 前 5 字节，第 4 字节为 HDL 版本，须等于 `0x0E`。
4. 写 `CTR0 = 0`，从 `NVM` 地址 `0x3C00` 读 16 字节（8 个 u16）；具备 `SECURITY` 能力时执行安全校验握手（`SECU_START = 0x0513`、`SECU_CHECK = 0x0219`、8 步）。
5. 写 `VTH` 寄存器：`MAX25_VTH` 型号为 `vth / 3.3 × (1.0/2.0) × 255`，其余为 `vth / 3.3 × (1.5/2.5) × 255`（截断为 u8）。
6. 具备 `ADF4360` 的型号写入时钟芯片初始化序列 `adc_clk_init_500m`。

### 4.6 FPGA 配置

```
PROG_B ← !bmWR_PROG_B          （拉低）
LED    ← 全灭
PROG_B ← bmWR_PROG_B           （拉高）
轮询 HW_STATUS 直到 FPGA_INIT_B
INTRDY ← !bmWR_INTRDY
BULK_WR ← 位流长度（3 字节小端）
EP2 OUT 批量写入整个位流（超时 1000 ms）
INTRDY ← bmWR_INTRDY
轮询 HW_STATUS 直到 GPIF_DONE
INTRDY ← !bmWR_INTRDY
轮询 HW_STATUS 直到 FPGA_DONE，然后 LED ← 绿
WORDWIDE ← bmWR_WORDWIDE
```

DSView 的轮询没有上限；移植时每个轮询加 2 s 超时，超时报 `error.device.fpgaConfigTimeout`。切换 3.3V/5V 阈值档位（仅原版 DSLogic 两个位流不同）时需要重新配置 FPGA。

### 4.7 采集配置（arm）

每次采集前通过 EP2 OUT 下发 `DSL_setting`（372 字节，小端，无填充），32 通道型号再追加 `DSL_setting_ext32`（204 字节）。

**`DSL_setting` 布局**

| 偏移 | 字段                                         | 值                                                      |
| ---- | -------------------------------------------- | ------------------------------------------------------- |
| 0    | `sync: u32`                                  | `0xF5A5F5A5`                                            |
| 4    | `mode_header`, `mode`                        | `0x0001`，模式位见下                                    |
| 8    | `divider_header`, `div_l`, `div_h`           | `0x0102`，分频                                          |
| 14   | `count_header`, `cnt_l`, `cnt_h`             | `0x0302`，`actual_samples >> 4`                         |
| 20   | `trig_pos_header`, `tpos_l`, `tpos_h`        | `0x0502`，触发位置                                      |
| 26   | `trig_glb_header`, `trig_glb`                | `0x0701`，`(启用通道数 & 0x1F) << 8 \| (stages & 0xFF)` |
| 30   | `dso_count_header`, `dso_cnt_l`, `dso_cnt_h` | `0x0802`，逻辑模式下为 `actual_samples`                 |
| 36   | `ch_en_header`, `ch_en_l`, `ch_en_h`         | `0x0A02`，通道启用位图                                  |
| 42   | `fgain_header`, `fgain`                      | `0x0C01`，逻辑模式为 0                                  |
| 46   | `trig_header`                                | `0x40A0`                                                |
| 48   | `trig_mask0[16]` … `trig_logic1[16]`         | 8 组 × 16 级 × u16                                      |
| 304  | `trig_count[16]: u32`                        | 每级计数                                                |
| 368  | `end_sync: u32`                              | `0xFA5AFA5A`                                            |

`*_header` 的高字节是寄存器地址、低字节是随后的 u16 个数。发送顺序：`STOP` → （非 SuperSpeed 配置时）`WORDWIDE` → `BULK_WR = 186`（`372/2`，即使随后还有 ext32 也只报主结构）→ 轮询 `HW_STATUS` 直到 `SYS_CLR` → EP2 写 `DSL_setting` → （CH32）EP2 写 `DSL_setting_ext32` → `INTRDY` 高 → 读 `HW_STATUS`，`GPIF_DONE` 必须置位。

**`mode` 位**

| 位  | 含义                                 | 位  | 含义                            |
| --- | ------------------------------------ | --- | ------------------------------- |
| 0   | 触发使能                             | 8   | 输入滤波（1T）                  |
| 1   | 外部时钟                             | 9   | 即时采集                        |
| 2   | 时钟边沿                             | 10  | 慢速采集（每毫秒字节数 < 1024） |
| 3   | RLE                                  | 11  | 串行触发                        |
| 4   | DSO                                  | 12  | Stream 模式                     |
| 5   | 半速（= `half_samplerate`）          | 13  | DRAM 回环测试                   |
| 6   | 四分之一速（= `quarter_samplerate`） | 14  | 外部测试                        |
| 7   | 模拟 / 循环采集                      | 15  | 内部测试                        |

**分频**：`t = ceil(hw_max_samplerate / samplerate)`；`div_h = (t >= pre_div ? pre_div - 1 : t - 1) << 8`；`t = ceil(t / pre_div)`；`div_l = t & 0xFFFF`；`div_h += t >> 16`。

**触发位置**：`p = max(trigger_pos% × limit_samples, 64)`，Buffer 模式上限为 `通道深度 × 90%`，Stream 模式上限为 `通道深度 × 10%`；`tpos_l = p & 0xFFC0`，`tpos_h = p >> 16`。其中通道深度 = `(hw_depth / 启用通道数) & !1023`。

**触发编码**（`trigger.c`）：每个通道在每级有两组条件字符 `X 0 1 R F C`（不关心、低、高、上升、下降、任意沿）。对通道 i：

- `mask` 位 = 字符 ∈ {`X`, `C`}；`value` 位 = 字符 ∈ {`1`, `R`}；`edge` 位 = 字符 ∈ {`R`, `F`, `C`}；
- 通道 0–15 写入 `DSL_setting`，16–31 写入 `DSL_setting_ext32`；
- 半速/四分之一速时把低 8/4 位复制到高位（无 `ADF4360` 的型号：四分之一速用 ¼ 复制、半速用 ½ 复制；有 `ADF4360` 的型号：四分之一速采样率时用 ½ 复制）；
- `trig_logic = (logic << 1) | inv`，`logic` 的位 0 为「与(1)/或(0)」、位 1 为「连续」。

**简单触发**（每通道一个条件，即设计稿的触发位）写入第 0 级，`logic = 1`（与，即「同时满足」），其余 15 级填 `mask = 0xFFFF, value = edge = 0, logic = 2, count = 0`。任一通道设了条件则 `mode` 位 0 置位。高级/串行触发按 DSView 触发面板的规则填满各级（`triggerdock.cpp:306-411`）。

### 4.8 数据通路

- **端点**：EP6 IN（`0x86`）。第一个 IN 传输读「触发头」：长度为 512 字节（无 `USB30` 能力）或 1024 字节（有 `USB30` 能力，与当前速度无关）。

  ```
  struct ds_trigger_pos { check_id: u32 = 0x55555555, real_pos: u32, ram_saddr: u32,
                          remain_cnt_l: u32, remain_cnt_h: u32, status: u32 }
  ```

  `status & 1` 表示已触发，`real_pos` 为触发点的样本序号（即触发前的样本数），DSView 直接把它当作触发位置。`tpos` 只是预触发样本的**上限**：条件在采集一开始就满足时（例如输入悬空时的低电平触发）`real_pos = 0`，FPGA 不会先填满预触发缓存；即时采集（未启用触发）同样返回 `status = 1, real_pos = 0`。Buffer 模式下若 `remain_cnt < limit_samples`（被提前结束），实际样本数改为 `(limit - remain) & !1023` 并按 64 样本对齐重算字节数——正常结束时 `remain = 0`，样本数因此被向下取整到 1024 的倍数（DSView 同样如此）。

- **数据格式（cross）**：每 64 个样本为一组，组内依次是每个**启用通道**（按通道号升序）的一个 u64（位 j = 该通道第 j 个样本，小端）。启用 n 个通道时一组 8n 字节。这正是按通道位压缩存储所需的形状，存储端只需按通道分拣 u64（§5.2）。**USB 包边界是任意的**（传输长度是 512/1024 的倍数而不是 8n 的倍数，例如 12 通道 @ 25 MHz 时每包 750 080 字节），分拣器必须跨包保留「当前通道 + 半个 word」的余数。
- **传输规模**：Stream 模式每个传输承载 20 ms（HS）/ 10 ms（SS）的数据，按 512/1024 对齐；同时在途的传输覆盖 100 ms（HS）/ 40 ms（SS），最多 64 个。Buffer 模式每个传输 1 MiB；DSView 在 Linux/macOS 上只挂 1 个、Windows 上挂 4 个（HS）/ 16 个（SS），HSync 在所有平台统一挂 4/16 个以提高上传速度。每毫秒字节数 = `ceil(samplerate / 1000 × 启用通道数 / 8)`。传输不设超时（与 DSView 一致），由停止流程负责取消。
- **采集线程不做重活**：传输完成后立即把缓冲区交给存储任务并重新提交一个池化缓冲区。DSView 在 libusb 回调里同步写入并计算摘要，处理慢时会拖慢重提交，Stream 模式下容易溢出——这是要修掉的问题。
- **结束条件**：非循环模式下累计字节数 ≥ `actual_bytes`（`align_up(limit, 1024) / 64 × n × 8`）即结束，最后一个包截断到剩余长度。
- **进度**：等待触发期间（连续约 16 个轮询周期无数据）读 `I2C_STATUS` 前 4 字节：`trig_hit, cnt3, cnt2, cnt1`；`triggered = trig_hit & 1`，`captured = (trig_hit >> 2) << 32 | cnt3 << 24 | cnt2 << 16 | cnt1 << 8`；进度 = 已触发 ? `(limit − captured) / limit` : `captured / limit`。
- **溢出**：Stream 模式下同样条件读 `HW_STATUS`，`SYS_OVERFLOW` 置位即上报 `Overflow`，UI 提示「USB 带宽不足，请降低采样率」。
- **停止**：用户第一次停止写 `CTR0 = bmFORCE_RDY`，设备会把已采集数据收尾上传；所有传输结束后发送 `STOP`。Buffer 模式且停止策略为「上传已采集的数据」时，若仍在等待触发，写 `CTR0 = bmFORCE_STOP`。取消在途传输用 `Endpoint::cancel_all()`。

### 4.9 设备选项

| 选项                      | 取值                                                        | 说明                                        |
| ------------------------- | ----------------------------------------------------------- | ------------------------------------------- |
| `OperationMode`           | Buffer / Stream（/ 内部测试，仅诊断模式可见）               | 切换后重选首个匹配的通道模式                |
| `ChannelMode`             | 当前模式下 `profile.channels` 允许的通道模式                | 决定通道数与采样率范围                      |
| `SampleRate`              | 型号采样率表中落在通道模式 `[min, max]` 内的值              | 10 Hz … 1 GHz                               |
| `SampleLimit`             | 按时长挑选，见 §9.4                                         | Buffer 模式受硬件深度约束，开启 RLE 可超出  |
| `Threshold`               | 1.8/2.5/3.3V、5.0V                                          | 无 `VTH` 能力的型号；切换需重配 FPGA        |
| `Vth`                     | 0.0–5.0 V（`MAX25_VTH` 型号 0.0–2.5 V？见 §15），步进 0.1 V | 有 `VTH` 能力的型号                         |
| `Filter`                  | 无 / 1 个采样周期                                           |                                             |
| `Rle`                     | 开 / 关                                                     | 测试模式下不可用                            |
| `ClockType` / `ClockEdge` | 内部/外部；上升/下降                                        |                                             |
| `BufferOption`            | 立即停止 / 上传已采集的数据                                 | 仅 Buffer 模式                              |
| `MaxHeight`               | 1X–5X                                                       | DSView 的波形高度倍数；HSync 由密度设置替代 |

默认值照抄 `DSLogic_dev_new()`：Stream 模式、默认采样率 1 MHz、默认深度 1 M 样本、阈值 3.3V 档、VTH 1.0 V、无滤波、触发边沿 8 余量。

### 4.10 Demo 设备

- 默认回放随包的演示采集（DSView 的 `demo/logic/protocol.demo`：25 MHz、131 072 个样本、16 通道，含 I²C、UART、SPI、CAN-FD，正是设计稿所用的数据），按选择的采样率与时长循环拼接，节奏与实时一致。
- 另提供「随机」与「计数器」两种合成图案，边沿可落在任意样本上（DSView 的随机图案只在 8 样本边界翻转，会掩盖边沿查找的 bug），并可注入毛刺，用于测试。
- Demo 设备走与真实设备完全相同的 `Device` trait 与数据包，包括触发头与进度，便于端到端测试。

### 4.11 可测试性

- 控制/批量传输抽象为 `trait UsbTransport`，生产实现包装 nusb，测试实现（`hsync-driver` 的 `mock` 特性）记录调用序列并由可编程的模拟设备应答；`hsync-dslogic/tests/device.rs` 用它覆盖打开、FPGA 配置、选项规则、带触发的 Stream 采集、Buffer 停止上传与 Stream 停止。
- **Golden 测试**：`crates/hsync-dslogic/tools/golden` 把 DSView 原封不动的 `dsl.c`、`command.c`、`trigger.c` 与一个拦截 `libusb_bulk_transfer` 的桩程序一起编译，对 41 组组合（全部型号族 × 通道模式 × 采样率含半速/四分之一速 × 简单/高级/串行触发 × 32 通道）记录 `DSL_setting` / `DSL_setting_ext32` 的字节，存入 `tests/fixtures/settings.txt`；Rust 实现逐字节一致。
- **真机冒烟**：`cargo run -p hsync-dslogic --example dslogic_capture -- [--buffer] [--mode N] [--rate HZ] [--samples N] [--trigger CH:COND] [--stop-after S]` 打开第一台 DSLogic，采集并打印吞吐、触发与各通道边沿数。
- 真机回归清单（人工）：每个型号的打开、阈值切换、Buffer/Stream 各采一次、简单触发、高级触发、拔插恢复。

**真机记录**（DSLogic U2Pro16，固件 2.2，USB 2.0，macOS）：

| 场景                                 | 结果                                                                      |
| ------------------------------------ | ------------------------------------------------------------------------- |
| 首次打开（FPGA 空白）                | 下发 `DSLogicU2Pro16.bin` 465 304 字节，安全校验通过，约 0.6 s            |
| 再次打开                             | `FPGA_DONE` 已置位，HDL 版本 `0x0E`                                       |
| Stream 1 MHz × 16 通道 × 1 M 样本    | 完成，1 000 448 样本                                                      |
| Stream 100 MHz × 3 通道 × 50 M 样本  | 完成，持续 37.5 MB/s 无溢出                                               |
| Buffer 500 MHz × 16 通道 × 20 M 样本 | 完成，上传 40 MB，45 MB/s（4 个 1 MiB 传输在途）                          |
| Buffer 1 MHz 采 1 亿样本，1 s 后停止 | 进度每约 0.3 s 上报；`FORCE_STOP` 后上传 1 019 904 样本，以「已停止」结束 |
| Stream 1 s 后停止                    | `FORCE_RDY` 后以「已停止」结束，保留 990 976 样本                         |

---

## 5. 数据模型与存储

### 5.1 时间基准

- 采集的基本坐标是**样本序号** `u64`；时间 = `(sample − trigger_sample) / samplerate`。
- 界面上所有时间都**相对触发点**（设计稿：标尺、数据流列表的「时间 ms」）；未触发的采集以第 0 个样本为零点，标「起点」而不是「触发」。
- 外部时钟采集没有时间，只显示样本序号（单位「样本」）。

### 5.2 逻辑数据存储（`hsync-capture`）

```
LogicCapture
├── meta: samplerate, sample_count（已提交）, trigger_sample, channels[]
└── channels[i]: ChannelData
    ├── chunks: Vec<Chunk>        // 每块 2^24 个样本（= .dsl 的 block，2 MiB）
    │     Chunk::Constant(bool)   // 整块恒为 0/1，不占数据内存
    │     Chunk::Bits(Box<[u64]>) // 位压缩，样本 j 在 word j/64 的 bit j%64
    └── summary: [Level; 3]       // 多级摘要（64:1）
          level k 覆盖 64^(k+1) 个样本的块：
            uniform: BitVec   // 块内是否全相同（含与前一块边界无跳变无关）
            value:   BitVec   // uniform 时的电平
```

- **追加**：cross 格式的每个 u64 直接写入对应通道的当前 chunk，同时更新 level 0 摘要（`w == 0 || w == !0`），满 64 个再向上汇总。稳态下写入是纯拷贝 + 位运算，单核可达数 GB/s，满足 USB 3.0 Stream。
- **常量块折叠**：一个 chunk 写满时若全为 0/全为 1，转成 `Constant`，空闲通道几乎不占内存。
- **容量**：软件深度上限沿用 DSView（64 位系统 16 Gi 样本 · 通道合计）；超过内存预算时（默认物理内存的 50%）提示降低深度，v1.x 支持把已满的 chunk 换出到内存映射临时文件（`memmap2`）。
- **循环（Loop）采集**：chunk 组成环形队列，丢弃最旧的块，`first_sample` 前移（DSView 的 `_loop_offset`）。

  实现：`ChannelData::drop_front` 以整 word 为单位从头部丢弃样本——先记一个偏移（`head`，所有查询按它换算），偏移攒满一个 chunk 时连同摘要各层对应的块一起释放，内存上限是采样深度加一个 chunk。采集期间 ingest 线程每收到一包就把超出采样深度的部分丢掉，触发点随之前移、移出窗口后清除；`LiveStatus.dropped` 记累计丢弃的样本数，界面据此在长度不变时也重新取波形。停止后 `compact()` 把剩下的样本搬回对齐的 chunk（`.dsl` 写入直接按 chunk 读），再跑解码。只有 Stream 模式能循环（`Device::can_loop`，DSView 在 Buffer 模式下退回单次）：DSLogic 置 FPGA 的循环位（即 DSView 的 `ANALOG_MODE_BIT`）并一直转发数据到停止；Demo 设备总能循环。

### 5.3 查询原语

| 原语                                     | 用途                         | 实现要点                                                 |
| ---------------------------------------- | ---------------------------- | -------------------------------------------------------- |
| `sample(ch, s) -> bool`                  | 悬停读数、测量               | 直接取位                                                 |
| `next_edge(ch, s, dir) -> Option<u64>`   | 悬停脉宽、跳到下一边沿、搜索 | 先用摘要跳过 uniform 块，再在 word 内用 `trailing_zeros` |
| `bins(ch, range, n) -> [u8]`             | 视口波形（§5.4）             | 每格取首/末电平与是否有跳变，按格宽选择摘要层            |
| `count_edges(ch, range) -> (rise, fall)` | 选段测量                     | `t = w ^ (w << 1 \| carry)`，`popcount(t & w)` 等        |
| `find_pattern(range, pattern, dir)`      | 电平模式搜索（X/0/1/R/F/C）  | 先对含边沿条件的通道求边沿候选，再校验其余通道           |
| `iter_words(chs, range)`                 | 解码器输入                   | 零拷贝借出 u64 切片                                      |

### 5.4 视口查询协议（二进制）

前端按「每个像素列一格」请求：`capture_query_bins { capture_id, channels, start_sample, end_sample, bins }`，`bins = min(画布像素宽, 样本数)`。返回 `ArrayBuffer`：

```
u32 version = 1 · u32 channel_count · u32 bins · u64 start · u64 end
per channel: u32 channel_index · u8[bins] cells
cell 位：bit0 = 格内首样本电平，bit1 = 末样本电平，bit2 = 格内有跳变，bit3 = 格内无数据
```

第 i 格覆盖样本 `[start + ⌊i·N/bins⌋, start + ⌊(i+1)·N/bins⌋)`。32 通道 × 4000 列每帧 128 KiB，平移时前端只请求新露出的列并拼接缓存（列缓存以 `samples_per_bin` 为键，缩放时失效）。

---

## 6. 波形渲染

### 6.1 渲染结构

- 时间轴区域分两层：**DOM 层**（标签列、组标题、解码块、触发位、菜单、选段旗、悬停读数）与 **画布层**（网格、波形、SPI 字节块、选段底色）。
- 画布层是**一个**贴在滚动容器视口上的 `<canvas>`（`position: sticky`），按「行布局模型」（由分组、展开状态与密度令牌计算出的每行 y 与高度）绘制可见行。DOM 与画布共用同一份行布局，避免读取 DOM 尺寸。
- 设备像素比（DPR）适配：画布尺寸 = CSS 尺寸 × DPR，线条按 0.5 像素对齐（与设计稿 `crispEdges` 观感一致）。

### 6.2 `WaveformRenderer`

```ts
interface WaveformRenderer {
  resize(cssWidth: number, cssHeight: number, dpr: number): void
  setTheme(tokens: RenderTokens): void // 从 CSS 变量读取的颜色与不透明度
  draw(frame: FrameDescriptor): void // 行布局 + 每行 bins 数据 + 叠加层
  dispose(): void
}
```

- **Canvas2D（基线，所有平台）**：每行把 cells 转成一条折线（高电平段填底色），相邻同值格合并成一段，绘制复杂度与「跳变段数」成正比而不是与像素数成正比。
- **WebGPU（加速，可用时启用）**：在 macOS WKWebView（WebGPU 可用的版本）、Windows WebView2 上启用，Linux WebKitGTK 与不支持的平台自动回退 Canvas2D。设置面板提供「图形加速」（自动 / 关闭），并显示当前用的渲染器。

  实现（`src/render/webgpu.ts`、`rects.ts`）：波形层的每一块都是轴对齐矩形——网格线、高电平底色、线段（水平段两端各多出半个线宽，拼出斜接的效果）与竖直跳变——所以 CPU 端把 `traceShape` 的结果展开成矩形实例（设备像素坐标 + 预乘 RGBA，32 字节/个），GPU 端用一个 4 顶点三角带的实例化绘制一次画完，行区域用 scissor 裁在标尺之下。与 Canvas2D 共用 cells 与形状算法，两者输出一致。设备丢失时换一块新 canvas 回退 Canvas2D（一块 canvas 只能有一种上下文）。

- 渲染预算：单帧 ≤ 4 ms（32 行 × 4000 列）。数据未返回时沿用上一帧的缓存并缩放过渡，不出现空白闪烁。

### 6.3 绘制规则（来自设计稿）

- 线宽 1.25 px（窄行 1 px），高电平与基线之间填通道线色的平涂底色，不透明度用每种线色各自的值（§9.3）。
- 格内有跳变且相邻格密集时画竖线；当一个像素内跳变过多（如 SPI 时钟在当前缩放下），**同一条线上画成字节块**而不逐沿绘制：由解码器提供的字节区间决定块的起止，块的底色不透明度是高电平底色的 3.4 倍（夜间 2.4 倍）。
- 被视口边缘切断的解码块去掉那一侧的圆角和边线。

---

## 7. 协议解码

### 7.1 注解模型

与 DSView 的 `srd_proto_data_annotation` 对齐并扩展：

```rust
pub struct Annotation {
    pub start: u64, pub end: u64,     // 样本区间
    pub class: u16,                   // 解码器内的注解类序号
    pub row: u16,                     // 注解行（annotation_rows）
    pub kind: u32,                    // DSView 的三元组注解类型码（颜色/语义提示）
    pub texts: SmallVec<[Arc<str>; 3]>, // 由长到短的多个版本，供不同宽度显示
    pub value: Option<NumericValue>,  // DSView 扩展：数值，用于按进制显示（{$} 占位）
}
```

- 注解按 `(decoder, row)` 分桶，桶内按 `start` 有序存放在分块数组中，配合与 §5.2 相同思路的稀疏摘要，视口查询 O(log n + k)。
- 每个总线（I²C、UART、SPI、CAN/CAN-FD）另有**呈现层（presenter）**：把底层注解组织成设计稿所需的「含义行」（如「从 0x04 读出 4F」）和数据流面板的表格（§9.9）。presenter 在 Rust 中实现，输出与语言无关的结构（模板 key + 参数），由前端翻译。

### 7.2 两种解码器

| 类型                      | 范围                                                                         | 说明                                                                                            |
| ------------------------- | ---------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| 原生 Rust 解码器          | I²C、UART、SPI、CAN、CAN-FD（v1）；1-Wire、I²S、JTAG、SWD、USB PD 等（v1.x） | 性能关键、与设计稿深度集成；输出与对应 sigrok 解码器的注解类/行保持一致，便于两者互换与对照测试 |
| sigrok 兼容 Python 解码器 | DSView 自带的其余 151 个解码器（含叠加）                                     | 通过 **pyo3** 嵌入 CPython，Rust 实现 `sigrokdecode` 模块                                       |

**解码范围**（DSView 的解码起止光标）：解码器对话框里选「全程」或「选段」；选段时把首末样本以 HSync 自己的键 `"decode range": [first, last]` 存进 DSView 格式的解码器条目（DSView 读取时忽略这个键，改动它会重新解码）。原生解码器与 Python 宿主都从 `first` 开始：第一次 `wait()` 从这里开始，这一点的「前一个电平」取它自己（libsigrokdecode 的初始引脚 `SAME_AS_SAMPLE0`），所以这里不会匹配到边沿，`skip` 也从这里数起；`end()` 时 `last_samplenum` 是范围的末端。UART、CAN 把范围起点处正忙的线路当作采集开头就在帧中间来处理。

### 7.3 sigrok 兼容宿主（`hsync-decode::srd`）

- **API 面**：`Decoder` 基类的 `wait()`、`put()`、`register()`、`has_channel()`、`printlog()`；类属性 `api_version = 3`、`id`、`name`、`longname`、`desc`、`license`、`inputs`、`outputs`、`tags`、`channels`、`optional_channels`、`options`、`annotations`、`annotation_rows`、`binary`；常量 `OUTPUT_ANN / OUTPUT_PYTHON / OUTPUT_BINARY / OUTPUT_META`、`SRD_CONF_SAMPLERATE`；生命周期 `metadata()`、`start()`、`reset()`、`decode()`；实例属性 `samplenum`、`matched`。
- **`wait()` 语义**：参数为条件字典或字典列表；字典内各项为「与」，列表内各字典为「或」；条件为 `'l' 'h' 'r' 'f' 'e' 's'` 与 `{'skip': n}`。Rust 实现直接在位压缩数据上扫描：对 `'l'/'h'` 用 word 级 `!w`/`w` 找首个满足位，对边沿用 `w ^ (w << 1 | carry)`，比 C 版逐样本快一个数量级。
- **DSView 扩展**（必须兼容，否则部分解码器无法运行）：
  1. 解码器输入是**每通道独立的位缓冲**（DSView 的 `srd_session_send(..., const uint8_t **inbuf, const uint8_t *inbuf_const, ...)`），与我们的存储同构；
  2. `annotations` 可为三元组 `('类型码', 'id', '描述')`，类型码作为 `kind`；
  3. `put()` 的文本列表可含一个整数（数值）或以 `@` 开头的十六进制串（如 `'@%02X' % v`），文本里的 `{$}` 由前端按用户选择的进制替换；
  4. 解码器 id 可带版本前缀（`0:i2c`、`1:i2c`），`.dsl` 中按此 id 保存。
- **叠加**：下层 `OUTPUT_PYTHON` 的 `put` 直接调用上层实例的 `decode(ss, es, data)`，与 libsigrokdecode 一致。
- **分发**：随应用打包 python-build-standalone 的精简 CPython（只含解码器用到的标准库：`collections`、`re`、`struct`、`math`、`functools`、`enum`、`json`、`zlib`、`decimal`、`calendar`、`string`、`operator`），解码器源码作为资源。Python 解码器以 GPL-2.0-or-later/GPL-3.0-or-later 发布，与本项目许可证兼容。
- 宿主不可用（初始化失败）时，原生解码器照常工作，Python 解码器在列表中标「不可用」。

**实现（`crates/hsync-srd`，对照 libsigrokdecode4DSL 逐条核对）：**

- **加载**：pyo3（0.29.2）嵌入 CPython；`sigrokdecode` 模块放进 `sys.modules`，解码器目录（随应用打包的 `resources/decoders`，复制自 DSView）放在 `sys.path` 最前；按目录名 `importlib.import_module("1-i2c")` 导入，读取 `Decoder` 类的元数据。导入时关闭字节码写入、屏蔽旧解码器的转义序列警告。目录清单启动时在后台读一次（约 0.5 s，150 个），`decode_catalog` 提供给添加解码对话框。
- **注解类型码**：三元组的首项是类型码；二元组从 7 起递增，跟在上一个三元组之后则沿用其码再递增（`decoder.c` `get_annotations`）。
- **注解行**：没有 `annotation_rows` 时用一行（以解码器名命名）收下全部类；有行时一个类归入最后一个列出它的行，任何行都没列出的类不显示（`decoderstack.cpp`）。时间轴上每行一条通道，名称 `解码器 id/行 id`，标签为行的描述；没有原生呈现的解码器第一行放在组标题行上。
- **`wait()`**：条件字母只看首字符：`h l r f e n`（`n` 为无边沿，没有 `s`），未知字母永不满足；字典内与、列表内或；`None`、`[]`、`{}` 为无条件等待（第一次停在样本 0，之后每次下一个样本）。命中后下一次从命中样本 +1 开始；`{'skip': n}` 在上次命中后第 n 个样本命中（DSView 的计数器在命中后从 1 起，`create_term_list`），第一次等待时为样本 n；`skip: 0` 立即返回原样本。未连接的可选通道按常量低电平参与匹配、在返回元组中为 255；超出通道数的索引抛 `IndexError`。`self.matched` 是**整数位掩码**（位 j 对应条件 j），DSView 的 35 个解码器都按位测试。扫描在 Rust 里按 64 样本一字进行，期间释放 GIL。样本用完或任务被取消时抛出继承自 `BaseException` 的内部异常结束 `decode()`，不算错误；`decode()` 正常返回则算错误。
- **`put()`**：`OUTPUT_ANN` 的数据为 `[类, [文本…]]`；格式不对的注解丢弃并记日志，不抛异常。文本列表中的整数作为数值（取最后一个），`'@'` 加十六进制的文本也作为数值（优先于整数）且不单独成行；显示时文本里的 `{$}` 换成数值（目前固定十六进制），没有文本则只显示数值。`OUTPUT_PYTHON` 依次调用上层的 `decode(ss, es, data)`；上层出错只记日志并在状态里报告，下层继续。`OUTPUT_BINARY`、`OUTPUT_META` 与 DSView 一样不处理。同一输出重复 `register()` 返回同一 id；`printlog()` 写日志。
- **生命周期**：自上而下实例化（`__init__` 之后设置 `options`，值按默认值的类型转换），自下而上 `metadata(SRD_CONF_SAMPLERATE, 采样率)` 与 `start()`，`start()` 之后 `samplenum = 0`、`matched = 0`；完整解码后调用 `end()`：底层先得到 `last_samplenum`，再依次调用上层的 `end()`（DSView 扩展）。
- **会话中的位置**：没有叠加的原生总线照旧用原生解码器；原生总线上叠加了解码器时，原生解码器画总线，整个栈在 Python 里跑一遍得到上层注解，作为附加通道放在原生通道之下；其余解码器整栈用 Python。添加解码对话框按 DSView 的规则补全下层（选 `eeprom24xx` 得到 `1:i2c → eeprom24xx`，跳过 `0:` 轻量版本），存为 DSView 的 `"stacked decoders"`。
- **构建要求**：编译 `hsync-srd` 需要带共享库的 Python 3（开发机用系统或 Homebrew 的 Python 即可）；发行包内置精简 CPython 留待 M5。

### 7.4 执行模型

- 每个解码器栈一个工作线程，从 `LogicCapture` 借出 word 切片，按 1 Mi 样本一段推进，段间检查取消标志并上报进度。
- 采集进行中即可解码（Stream 模式），按已提交长度增量推进；采集结束时调用 `decode` 的收尾（等价 `srd_session_end`）。
- 修改选项或通道映射：取消并从头重跑；注解存储按「代」(generation) 隔离，旧结果在新结果到达前继续显示。

---

## 8. 文件格式与导入导出

### 8.1 `.dsl`（DSView 捕获文件）

ZIP 容器（deflate）：

| 条目           | 内容                                                                                                                                        |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `header`       | INI 文本，见下                                                                                                                              |
| `decoders`     | JSON 数组：解码器栈、选项、通道映射、注解行显隐                                                                                             |
| `session`      | JSON：设备、采样参数、通道（名称、颜色、启用、显示顺序）、解码器、触发                                                                      |
| `L-<ch>/<blk>` | 逻辑数据：通道 `ch` 的第 `blk` 块，位压缩，LSB 先（样本 8k+b 在字节 k 的位 b）；每块 2^24 样本（2 MiB），末块可更短；全 0/全 1 的块也要写出 |

`header` 示例（v3，取自 DSView 自带 `demo/logic/protocol.demo`）：

```ini
[version]
version = 3
[header]
driver = virtual-demo
device mode = 0
capturefile = data
total samples = 131072
total probes = 16
total blocks = 1
samplerate = 25 MHz
trigger time = -28800000
trigger pos = 0
probe0 = SDA
probe1 = SCL
…
```

- 只保存启用且有数据的通道，`probe<index>` 用原始通道号。`trigger time` 是触发时刻的 Unix 毫秒时间戳。
- **v1**：单个 `data` 条目，每样本 2 字节交织（位 i = 通道 i）。只读，读取时转成按通道存储。
- **v2/v3**：按通道分块，如上。写出一律 v3。
- **保存**（`Workspace::save` → `write_dsl_file`）：先写到同目录的 `<文件名>.part`，写完 `fsync` 后改名覆盖目标，失败时原文件不变。`session` 用打开时读到的那份（新采集则新建，含 `Version: 3`、`DeviceMode: 0`）并更新 `Device`、`Sample rate`、`Sample count`、`decoder`；`channel` 必须列出所有保存的通道（`type: 10000`、`enabled: true`），因为 DSView 打开文件时会把 `session.channel` 里没有的通道设为禁用（`mainwindow.cpp` `load_config_from_json`）。保存后采集的来源变为该文件；采集进行中不能保存。界面：导出按钮菜单「保存采集…」与 ⌘/Ctrl+S，设备菜单「打开文件…」与 ⌘/Ctrl+O；新采集默认文件名 `HSync-YYYYMMDD-HHMMSS.dsl`。
- `decoders` 中每项：`id`、`label`、`channel`（`[{"sda": 0}, …]`）、`options`、`show`（注解行显隐）、`stacked decoders`、`version: 2`、`view_index`。
- HSync 特有的状态（分组展开、主题等界面状态）不写入 `.dsl`，以保证 DSView 可以打开我们保存的文件；如需扩展，写入额外条目 `hsync.json`（DSView 会忽略未知条目）。

### 8.2 导出

| 格式         | 说明                                                                              |
| ------------ | --------------------------------------------------------------------------------- |
| CSV          | 首列时间（秒，相对触发）或样本序号，后接各通道 0/1；可选「仅变化行」              |
| VCD          | 标准 Value Change Dump，`$timescale` 取能整除采样周期的最大单位                   |
| sigrok `.sr` | sigrok 会话格式（zip：`version`、`metadata`、`logic-1-<n>`），便于 PulseView 打开 |
| 原始二进制   | 每样本 ⌈n/8⌉ 字节交织，小端                                                       |
| 解码结果     | 数据流面板导出：CSV（时间、解码器、行、文本）与纯文本（与面板显示一致）           |

导出范围：全程或选段；通道：全部或勾选。

实现（`hsync_fileio::export`、`ExportSource::export`）：

- 先在工作区锁内取出采集与名称（`ExportSource`），随后在锁外写文件，时间轴照常响应；选段或部分通道先 `LogicCapture::slice` 出一份再写。所有格式都先写 `<名称>.part`，完成后改名覆盖，取消或失败不留文件。
- **CSV** 照 DSView `output/csv.c`：`; CSV, generated by …`、`; Channels (n/m)`、`; Sample rate: 25 MHz`、`; Sample count: N` 四行注释，表头 `Time(s), SDA, SCL`（名称前有空格，同 DSView），时间按 `%0.15g` 从采集首样本起算；默认只在任一通道变化处写一行，可选每个采样点一行。
- **VCD**：`$timescale` 取能整除采样周期的最大单位（25 MHz → 10 ns，400 MHz → 100 ps；不能整除 10¹⁵ fs 的采样率用 1 ps），只写变化，最后一行时间戳标出长度（同 DSView）。
- **sigrok `.sr`**：`version`（`2`）、`metadata`（`[global] sigrok version`、`[device 1]` 的 `capturefile=logic-1`、`total probes`、`samplerate`、`total analog=0`、`probeN`、`unitsize`）与每 4 Mi 样本一个的 `logic-1-N`；导出的第 k 个通道在第 k 位（DSView 按通道号写 `probeN` 而按顺序放位，通道不连续时对不上，这里统一按顺序）。
- **原始二进制**：每样本 ⌈n/8⌉ 字节，第 k 个导出通道在第 k 位，小端；交织用 8×8 位矩阵转置，不逐样本循环。
- `.dsl` 导出（含「保存选段」）只保留所用通道全部被导出的解码器。
- 保存与导出约每 1% 通过 Tauri `Channel` 报告进度，`file_cancel` 取消；状态栏显示「正在导出 45%」与「取消」，完成后显示文件名。

### 8.3 会话文件

- **读取**：兼容 DSView `.dsc`（JSON，`Version`、`Device`、`DeviceMode`、`Sample rate`、`Sample count`、`channel[]`、`decoder[]`、触发相关字段）与随设备的 `*.def.dsc` 默认会话。

  设备选项按 libsigrok 的选项名（`hwdriver.c` 的 `sr_config_info`）对应：`Operation Mode`、`Stop Options`、`Channel Mode`、`Threshold Levels`、`Threshold Level`（版本 3 起是阈值电压，之前是阈值档位）、`Filter Targets`、`Enable RLE Compress`、`Using External Clock`、`Using Clock Negedge`；列表型的值在版本 3 中是代码，之前是英文或中文文本（`dslogic.c` 的 `lang_text_map`），通道模式只认代码。`CollectMode`（0 单次、1 重复、2 循环）对应采集方式。选项只在同一驱动上生效，按「先模式、后其他」的顺序设置；设备不接受的值跳过。`Max Height`、`Language`、示波器的触发选项等与 HSync 无关的键忽略。

- **写入**：HSync 自己的会话格式 `*.hsync-session.json`（带格式名与版本号），内容是设备选项、通道（启用、名称、线色、触发条件）、触发设置、解码器与采集方式。

---

## 9. UI/UX 规范

本章把 `.hsync` 设计稿整理成实现规格。**以设计稿最终状态为准**；设计稿里被否决的方案（右侧详情面板、底部栏小波形、高电平渐变阴影、把所有总线并排塞进一页的数据流、顶栏里的全局触发设置）不实现。示意稿中的模板运行时（`support.js`、`<x-dc>`、`{{…}}`、`<sc-for>`、`<sc-if>`、`preset` 等）不移植，要拿走的是外观、结构、交互规则与文案。

### 9.1 设计原则

原界面（DSView 1.3.2）的七处问题与对应原则：

| 原问题               | 原则                                                                             |
| -------------------- | -------------------------------------------------------------------------------- |
| 显著性不等于重要性   | **通道按解码器分组**，未绑定解码器的通道归入可整组收起的「未命名通道」           |
| 颜色没有承担语义     | **颜色各管一件事**：解码块用总线色，波形用通道线色；红色与琥珀色只留给错误与警告 |
| 给的是原始数据       | 解码块与时间轴对齐；整段内容放进**数据流面板**，一次只看一个解码器               |
| 不知道自己在哪       | **全程概览** + 以触发点为零点的整齐刻度                                          |
| 一个状态占了五个位置 | **触发按通道设**：每通道一个触发位，条件画在自己的行上                           |
| 操作要先切模式       | **单窗口**，没有侧边面板，没有需要先切换的模式                                   |
| 版式缺少纪律         | **扁平**：无渐变、投影、发光；层次只靠底色深浅与 1 px 线                         |

### 9.2 布局

自上而下：**顶栏 → 全程概览 → 时间轴（标尺 + 分组行，可纵向滚动）→ 数据流面板（可开合）→ 状态栏**。

```
┌ 顶栏 ──────────────────────────────────────────────────────────────────────┐
│ ⎍ [设备 ▾] [25 MHz | 5.24 ms | 16 通道 ▾] [⎍ 触发 未设置 ▾] [▶ 采集|▾] … [🔍 搜索 /] [⤓] [⚙] │
├ 全程概览（58px）─────────────────────────────────────────────────────────────┤
├ 标签列 204px ─┬ 标尺（38px，sticky）─────────────────────────────────────────┤
│ I²C 组标题     │ 解码块（含义行）                                              │
│   从机 0x62    │ 解码块（字节行）                                              │
│   SDA  D0  ▢  │ 波形                                                          │
│ …              │ …                                                             │
├ 数据流面板（默认关闭，可拖高，时间轴至少保留 180px）─────────────────────────┤
├ 状态栏（28px）：[≡ 数据流] 采集完成 · 触发时间 · 采样点数 …… [− 50 µs/格 + ↔] ┤
└──────────────────────────────────────────────────────────────────────────────┘
```

- 窗口最小 640 px 高；窗口变窄时顶栏换行、时间轴横向滚动（行最小宽度 860 px），页面本身不出现横向滚动条。
- 宽度 ≤ 640 px：标签列 132 px，隐藏组的元信息、通道号、数据流按钮与「添加协议解码」的文字。

### 9.3 设计令牌

全部令牌定义为 CSS 自定义属性（`src/styles/tokens.css`），主题切换只改根元素上的类名（`.theme-night`）与密度类名（`.density-compact`），并在 Tailwind v4 的 `@theme` 中映射成工具类（如 `bg-canvas`、`text-ink-2`、`border-line`）。

**底色与文字**

| 令牌            | 用途                                                        | 纸面      | 夜间      |
| --------------- | ----------------------------------------------------------- | --------- | --------- |
| `--canvas`      | 波形区底色                                                  | `#FCFCFA` | `#0F1317` |
| `--chrome`      | 顶栏、标签列、状态栏底色                                    | `#F1F2EE` | `#161B20` |
| `--raise`       | 按钮、菜单底色                                              | `#FFFFFF` | `#1E242A` |
| `--line`        | 分隔线                                                      | `#DFE2DB` | `#262D34` |
| `--line-2`      | 控件边框                                                    | `#C5C9C1` | `#39424B` |
| `--ink`         | 主要文字、选中态底色                                        | `#171B1F` | `#E9EDF0` |
| `--ink-2`       | 次要文字                                                    | `#4A525A` | `#AEB7BF` |
| `--ink-3`       | 说明文字                                                    | `#5F6871` | `#8C959E` |
| `--trace`       | 默认波形线                                                  | `#414A52` | `#B9C2CA` |
| `--trace-2`     | 窄行波形线、空触发位                                        | `#8A929A` | `#6F7982` |
| `--grid`        | 网格线                                                      | ink 7%    | ink 6%    |
| `--veil`        | 选段底色带                                                  | ink 5%    | ink 6%    |
| `--hover`       | 悬停底色                                                    | ink 6%    | ink 8%    |
| `--sel`         | 数据流中被选中的内容                                        | ink 12%   | ink 18%   |
| `--find`        | 数据流中含搜索命中的行                                      | `#FBF0C4` | `#3A3215` |
| `--find-strong` | 当前命中：数据流行、概览标记；时间轴上以 45% 不透明度画带子 | `#F3D36A` | `#6E5B16` |

**总线色**（主色 / 解码块底色 / 较深底色）

| 总线 | 纸面                              | 夜间                              |
| ---- | --------------------------------- | --------------------------------- |
| I²C  | `#1F55C5` / `#E6EDFC` / `#CBDAF9` | `#8FB5FF` / `#182844` / `#223B69` |
| UART | `#0C6B58` / `#DFF1EB` / `#C2E4D9` | `#62D3B6` / `#10302A` / `#184A40` |
| SPI  | `#5B38B8` / `#ECE7FB` / `#D9CFF7` | `#B9A3FF` / `#261F45` / `#382D69` |
| CAN  | `#A3256E`                         | `#F29BCB`                         |

其余解码器（sigrok 兼容的 140 余个）使用中性的「通用总线色」：主色 `--ink-2`，底色 `--hover`（与设计稿中 SPI「传输中」的 `la-b-quiet` 一致），避免引入新的高饱和色。

**波形线色**（按探头线色环循环：黑、棕、红、橙、黄、绿、蓝、紫；线色序号默认 = 通道号 mod 8）

| 序号 | 纸面线色  | 底色不透明度 | 夜间线色  | 底色不透明度 |
| ---- | --------- | ------------ | --------- | ------------ |
| 0    | `#171B1F` | 7.5%         | `#BCC5D1` | 12.6%        |
| 1    | `#5C2600` | 8.3%         | `#9A6B2A` | 23.3%        |
| 2    | `#9E1334` | 8.7%         | `#DF636C` | 19%          |
| 3    | `#C3540B` | 11.6%        | `#FA8F00` | 15.5%        |
| 4    | `#A99100` | 14.9%        | `#E0CE00` | 12.7%        |
| 5    | `#259263` | 12.3%        | `#65C777` | 14.6%        |
| 6    | `#308EEC` | 13.6%        | `#3D9DFF` | 16.9%        |
| 7    | `#803197` | 9.7%         | `#A05FC4` | 21.1%        |

不透明度各不相同是有意的：使每种颜色的高电平底色叠在画布上亮度一致。字节块不透明度 = 高电平底色 × 3.4（夜间 × 2.4）。

> 注意：波形线色中的红（序号 2）与橙（序号 3）属于**通道标识色**，不是状态色；「红色与琥珀色只留给错误与警告」约束的是界面的状态表达（文字、图标、徽标、底色），不包括与物理探头线对应的通道线色。

**字体与字号**

- 界面文字 IBM Plex Sans，数值与解码内容 IBM Plex Mono；中文回退：`"PingFang SC", "Hiragino Sans GB", "Microsoft YaHei UI", "Microsoft YaHei", "Noto Sans SC", "Noto Sans CJK SC"`；等宽回退：`"SF Mono", Menlo, Consolas, "Noto Sans Mono CJK SC"`。字体随应用本地打包，不依赖网络。
- 基准 13 px，次要信息 12 px，说明与刻度 11 px；行高 1.35。数值一律等宽数字（`font-variant-numeric: tabular-nums`）。

**尺寸**

| 项目                                              | 舒适   | 紧凑   |
| ------------------------------------------------- | ------ | ------ |
| 标签列宽 `--lab`                                  | 204 px | 204 px |
| 解码行高 `--h-lane`                               | 26 px  | 22 px  |
| 波形行高 `--h-tr`（I²C、UART 原始波形）           | 30 px  | 24 px  |
| 窄波形行高 `--h-trc`（SPI 各线、CAN、未命名通道） | 18 px  | 14 px  |
| 组内上下留白 `--gap`                              | 8 px   | 4 px   |

其他：顶栏控件高 32 px、圆角 6 px；全程概览高 58 px；标尺高 38 px；状态栏高 28 px；解码块在行内上下各留 2 px、圆角 3 px。

### 9.4 顶栏

| 控件     | 外观                                                             | 行为（v1）                                                                                                                                                        |
| -------- | ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 标志     | 20 px 波形图标                                                   | 无                                                                                                                                                                |
| 设备选择 | `la-btn`，文字为当前设备名，右侧下拉箭头                         | 下拉列出设备（含 Demo、已打开的文件、灰显的不支持设备）；底部「设备选项…」打开设备选项对话框                                                                      |
| 采集设置 | `采样率 │ 时长 │ N 通道`，竖分隔线 1×14 px                       | 弹出层：采样率列表、时长列表（DSView 的 1-2-5 序列，超出硬件深度时标「RLE」）、通道启用与通道模式                                                                 |
| 触发汇总 | 图标 + 「触发」+ 汇总                                            | 无条件写「未设置」；有条件写「SDA ⟨图形⟩ + SCL ⟨图形⟩」，最多列三个，超过三个列前两个加「等 N 个通道」。点击打开触发设置面板（触发位置、多级/串行触发，设计待补） |
| 采集     | 主按钮（`--ink` 底，`--canvas` 字）+ 右侧 28 px 下拉             | 采集/停止；下拉：单次、重复（间隔）、循环、即时                                                                                                                   |
| 搜索框   | 220–340 px，占位「搜索数值或文本，如 4F、HOST」，右侧 `/` 键提示 | `/` 聚焦；在当前数据流解码器（或全部解码器）中查找，结果在列表中高亮并可前后跳转（§9.8）                                                                          |
| 导出     | 32×32 图标按钮                                                   | 导出对话框（§8.2）                                                                                                                                                |
| 设置     | 32×32 图标按钮                                                   | 设置面板：主题、密度、语言、图形加速、行为选项、诊断                                                                                                              |

时长列表按 DSView 的算法生成（`samplingbar.cpp:472-618`）：从最大可用时长开始按 1-2-5 序列递减，直到样本数 < 1024。Buffer 模式下超出设备内存（按启用通道数分到每通道的深度，`DeviceState.hardwareDepth`）的时长后面标「RLE」，选中它们时驱动自动打开 RLE 压缩（需在设备选项里允许 RLE），与 DSView 相同。所选采集按位压缩后（样本数 × 通道数 / 8 字节）超过本机物理内存一半时，弹出层底部用告警色说明需要多少内存并建议缩短时长或减少通道，但不阻止采集。

**采集方式**（实现）：

- **重复**：间隔 1 s（DSView 默认）。第一次之后的每一次都在后台采集——DSView 的后备缓冲：屏幕保持上一次的完整结果，状态栏显示后台这次的进度，完成后整体替换；中途停止时丢弃这一次、继续显示上一次，除非设置里打开「重复模式下停止后显示最新数据」（DSView 的 `swapBackBufferAlways`，默认关）。会话层用 `CaptureRequest.hold` 实现：被保持的采集与后台采集都能按 id 查询，后台采集结束时由 ingest 线程判定是否替换（判定先于结束事件），被丢弃的不再解码。
- **循环**：只在设备能循环时可选（Stream 模式的 DSLogic、Demo 设备，§5.2），否则菜单项灰显并说明原因；设备换到 Buffer 模式时退回单次，与 DSView 相同。
- 同一设备的下一次采集保持缩放与位置（按时间换算，采样率变了也对得上），不再每次缩放到全程；打开文件或 Demo 时仍缩放到全程。

**设置面板**（实现）：主题、密度、图形加速、语言（分段控件）；「行为」四个开关对应 DSView 的显示选项并沿用其默认值：触发位置自动居中显示（开；采集找到触发点时把它移到视口中央）、采集时自动滚动到最新数据（开；数据流入时最新样本保持在右缘）、快速拖动后波形继续滚动（开；松手速度超过 0.5 px/ms 时按指数衰减继续平移，按下或滚轮即停）、重复模式下停止后显示最新数据（关）。这几项在视口缩放到全程时不起作用。「诊断」：日志级别（错误/警告/信息/调试，只作用于 HSync 自己的 crate，其他 crate 固定为警告及以上）、写入日志文件（默认开；文件 4 MiB 轮转、保留两份）与「在文件夹中显示」。设置存在 WebView 的本地存储里，日志设置在启动时交给后端。

### 9.5 全程概览

- 58 px 高的带子画出整段采集；每个解码器一条 5 px 的细带标出有数据的区间，颜色为总线色，不透明度 0.62。
- 当前视口用 `--ink` 色、1.5 px 框、4 px 圆角的矩形表示（`la-vp`）。
- 刻度按毫秒（或自适应单位），起点标「触发」，末刻度带单位（如「5 ms」）。
- 交互（设计稿未定义，建议，见 §15）：在概览上点击使视口中心移到该处；拖动视口框平移；滚轮缩放。
- 数据流列表悬停某项时，在概览上用 14% `--ink` 底色 + 1 px 边线标出其位置（`la-ovh`，最小宽 2 px）。

### 9.6 时间轴

**标尺**：sticky 置顶，单位标在左下角（「ms」，相对触发点）。主刻度带数字，小刻度不带；步长按缩放取 1-2-5 序列（设计稿在 50 µs/格时主刻度 50 µs、小刻度 10 µs）。标签列在横向滚动时固定。

**分组**：每组一行组标题（`--h-lane` 高），下面是解码行，再下面是原始波形。组标题包含：展开/收起箭头、总线色 8 px 方块、组名（加粗）、悬停才出现的「在数据流里看」按钮、右侧元信息（如「100 kHz」「115200 8N1」「1.25 MHz」，带解释性 `title`）。组与组之间 1 px 分隔线和 `--gap` 留白。未命名通道组的方块是 1 px 描边的空心方块，右侧显示通道数。

**通道行**：标签列内依次为：线色圆点（7 px）、通道名、通道号（`D0`，等宽 11 px，`--ink-3`）、触发位。窄行用于 SPI 各线、CAN 与未命名通道。

**解码块**（`la-blk`）：

- 绝对定位在行内，上下各留 2 px，圆角 3 px，内边距 0 8 px，12 px 字；底色为总线底色、边线为总线较深底色；悬停边线 `--ink-3`。
- 宽度不到视口 3.2% 的块不显示文字；被视口边缘切断的一侧去掉圆角与边线。
- 选中的块反白（`--ink` 底、`--canvas` 字）；点击块 = 把它的起止设为选段，并把数据流切到它所属的解码器；再点一次取消。
- **I²C** 两行：上行是整次传输的含义（「从 0x04 读出 4F」）；下行是字节，地址字节前标「写/读」（11 px，总线色），地址字节用较深底色（`la-strong`）。应答位只写一个字母：`A` 普通文字（总线色），`N` 加 1 px 细框；起始/重复起始/停止为 `S`、`Sr`、`P` 小标记（`la-mk`）。
- **UART**：每帧一块，块内字符（13 px 等宽加粗）+ 十六进制（说明色）；空格写「空格」。
- **SPI**：组标题行显示整次传输「传输中 · 偏移 81–160」（安静色块）；MISO 行把可打印字符逐个居中排在各自字节位置上，控制字符画成 `·`（SPI 主色），同一文本行的字节合成一个块；MOSI 行把长串相同字节折叠为「FF × 224 · 从偏移 16 起没有变化」。
- **CAN-FD**：视口内没有帧时，行内显示「视口内没有帧」与「上一帧 / 下一帧」按钮（跳转到前/后一帧）。

**悬停读数**：鼠标停在波形行上，显示脚下这个脉冲的电平与宽度（如「高 9.8 µs」）；时钟类信号再给周期与频率（相邻两个脉冲宽度差 < 34% 时视为时钟）；UART 线再给「约 N 位」。脉冲被视口切断时写「≥」并在第二项写「延伸到视口外」。读数气泡 `--ink` 底、`--canvas` 字、12 px，居中于脉冲、左右夹在 9%–91% 之间，脉冲本身盖一层 13% `--ink` 的底色。v1 对所有波形行开放（设计稿只示意了 SDA、SCL、RX）。

**选段（A/B）**：

- 在标尺上拖动选出一段：两端是 A、B 两个 14 px 小旗（`--ink` 底），中间用带箭头的横线标时长；整段时间在所有行上盖 `--veil` 底色。
- 小旗在段外，靠视口边缘时转向段内；段宽不足视口 9% 时时长标在 B 旁（B 靠右缘时标在 A 旁）。段的一端在视口外时，时长写「延伸到视口外」。
- 拖动不到 2 µs 算单击；在标尺上单击取消选段。
- 选段后在状态栏显示区间测量（时长、各可见通道的上升沿/下降沿数，v1 对齐 DSView 的光标测量）。

### 9.7 触发：按通道设置

- 标签列最右边是「触发」列（标尺行的标签区右侧写「触发」，带说明 `title`）。每个通道行有一个 20 × 16 px 的触发位（窄行时高度为 `min(16px, --h-trc − 2px)`），组标题行没有。
- 没设条件：一道短横，颜色 `--trace-2`；设了条件：`--ink` 底色小块，里面是条件图形（`--canvas` 色）。悬停底色 `--hover`；菜单打开时 1.5 px `--ink` 外框。
- 点触发位，在标签列右侧、与该行平齐的位置弹出 176 px 宽的菜单：标题「XX 的触发条件」（带通道线色圆点），六个选项依次是不触发、上升沿、下降沿、任意沿、高电平、低电平，当前项打勾，底部说明「多个通道都设了条件时，同时满足才触发」。靠近底部的行，菜单上移到能完整显示（菜单高 236 px）。
- 菜单在这些时候关闭：选了一项、按 Esc、在菜单外按下、时间轴滚动、再点一次同一个触发位。用键盘打开时焦点落在当前项上，上下方向键移动，关闭后焦点回到触发位。
- 多个通道各设各的，同时满足（AND）才触发——这与 DSView 简单触发一致（`triggerdock.cpp:976`「Capture will only triggered when all setted channels fullfill at one sample」）。
- 被禁用的通道不能设触发（DSView：「Disabled channels cannot be used for trigger」），触发位置灰。
- 六种图形画在 16 × 16 格子里，线宽 1.5 px、圆头，高电平一侧有 26% 平涂底色：

| 条件   | 线                                  | 底色块               |
| ------ | ----------------------------------- | -------------------- |
| 不触发 | `M6 8h4`                            | 无                   |
| 上升沿 | `M2.5 11.5H8v-7h5.5`                | `M8 4.5h5.5v7H8z`    |
| 下降沿 | `M2.5 4.5H8v7h5.5`                  | `M2.5 4.5H8v7H2.5z`  |
| 任意沿 | `M2.5 4.5h3l5 7h3M2.5 11.5h3l5-7h3` | 无                   |
| 高电平 | `M2.5 4.5h11`                       | `M2.5 4.5h11v7h-11z` |
| 低电平 | `M2.5 11.5h11`                      | 无                   |

条件到硬件字符的映射：上升沿 `R`、下降沿 `F`、任意沿 `C`、高电平 `1`、低电平 `0`、不触发 `X`。

### 9.7.1 触发面板（§15 第 5 项的默认实现）

顶栏「触发」按钮打开 480 px 宽的面板，字段与限制照 DSView 的 Trigger dock（`triggerdock.cpp`）：

- **方式**：简单 / 多级 / 串行。多级与串行只在设备的 `TriggerCaps.advanced` 为真时可选（DSLogic 的 Buffer 模式）；演示设备、Stream 模式与未打开设备时灰显并在 `title` 中说明原因。实际生效的方式为 `advanced ? 所选 : 简单`。
- **触发位置**：滑块 1–`maxPosition`%。DSLogic：Stream 模式固定 1%（滑块禁用）；Buffer 模式 90%，采集长度超过硬件深度时为 `⌊depth×90/limit⌋`（DSView 先做整数除法，`ceil` 不起作用），至少 1。驱动收到的位置也按此夹紧。
- **简单**：说明「在各通道所在的行里设条件，同时满足才触发」，列出已设条件，「清除全部条件」。
- **多级**：级数 1–16 与级标签；每级两个条件（A、B）各一排通道格子与「取反」，组合「A 与 B 同时满足 / A 或 B 满足」，次数（≥1）与「连续」。默认值同 DSView：与、次数 1、不取反、不连续、条件全为 X。
- **串行**：起始、停止、时钟三排格子，数据通道，位数 1–16（默认 8），数据值（16 位，高位在左，0/1/X；少写的高位按 X）与十六进制换算（只对使用的低位）。
- **通道格子**：每通道一格，上方是通道号，格内是 §9.7 的条件图形；单击按菜单顺序切换，右键反向，键入 X 0 1 R F C 直接设定，左右方向键移动。
- 多级或串行生效时，按钮上的汇总改为「多级 · N 级」「串行 · SDA · 8 位」，时间轴触发位降为 40% 不透明度并在 `title` 中说明行上的条件暂不生效。

编码（`TriggerSetup::advanced`）照 `TriggerDock::commit_trigger`：多级写入前 N 级（`last_stage = N−1`，`logic = 连续<<1 | 与`）；串行写入第 0 级（起始/停止）、第 1 级（时钟，次数 1）、第 2 级（数据通道处为 `0`，≥16 的通道写入第二个条件，与 DSView 一致）、第 3 级（数据位 k 写在位置 k，次数为位数−1）。串行页借用的多级页字段（级数、组合、取反、第 1/3 级的第二个条件）取 DSView 的默认值。

### 9.8 搜索

- 顶栏搜索框（`/` 聚焦，Esc 清空并失焦）。输入十六进制（`4F`、`0x4F`）或文本（`HOST`），在解码结果中查找：命中项在数据流面板中高亮，`Enter`/`Shift+Enter` 前后跳转并把波形滚到该处。
- 电平模式搜索（DSView 的 Search dock）：输入 `D3=R D5=1` 形式（或用搜索框下拉中的模式编辑器按通道选 X/0/1/R/F/C），从当前位置向前/向后查找。

实现（v1）：

- **判别**：每个词都是 `通道=代码` 时按电平模式处理，否则搜解码结果。通道可写 `D3`、`3`、通道名或解码角色名（`SDA`、`CS#`），不区分大小写；不存在的通道在框内右侧以 `--alert` 提示「没有通道 D9」。
- **解码结果**（`hsync_decode::search`）：查询同时按十六进制与文本理解——`4F`、`0x4F`、`4F 32` 是字（序列）；单个多于两位的偶数位十六进制（`CAFE`）既当一个字也当逐字节；任何查询也按文本（ASCII 字母不区分大小写，非 ASCII 按 UTF-8 字节）查找。查找范围是「同一条线上前后相继的字」：I²C 一次传输（地址按显示的 7 位或 8 位，加数据字节）、UART 整条线、SPI 一次传输的 MISO 或 MOSI、CAN 一帧的数据；单个十六进制数还匹配 CAN 标识符。命中不重叠，按时间排序，最多返回 10 000 个（框内显示「10000+ 处」）。每个命中带上所在的数据流行号（按文本/十六进制两种列表分别计算）与 SPI 线别。
- **步进**：输入后 150 ms 开始搜索并显示「N 处」；第一次回车从视口起点向后（Shift+回车从视口终点向前）找第一处，之后在命中间循环，框内显示「3/17」。每一步：命中不在视口内时平移到居中（比视口 80% 还宽时缩放到它两侧各留 10%），时间轴盖一条 `--find-strong` 带子，全程概览上标出位置，数据流切到该解码器（不自动打开面板），列表在「全程」范围下滚到该行（上面留一行），含命中的行 `--find` 底色、当前行 `--find-strong` 底色。
- **电平模式**（`LogicCapture::find_pattern`）：按 64 个样本一个字并行判断；边沿条件比较前一样本与本样本，因此样本 0 不会命中边沿。含边沿条件时每个满足的样本都是命中；只有电平条件时只取每段连续满足区间的第一个样本——这与 DSView 不同（DSView 对纯电平模式逐样本前进，向前查找还会停在原处），目的是让「下一处」真正移到下一段。第一次从视口起点（向前找时为终点）开始，之后从上一处的下一个（上一个）样本开始；时间轴在命中样本处画 2 px `--ink` 竖线，框内显示其时间，找不到更多时显示「没有更多」。

### 9.9 数据流面板

- **位置和开合**：时间轴下方，默认关闭。入口：状态栏左端「数据流」按钮；每个解码器组标题行上悬停才出现的按钮（直接打开该解码器，再点一次关闭）。选段不会自动打开它。上边缘可拖动调高度（最小 80 px），时间轴至少保留 180 px。
- **头部**（33 px）：左边是解码器标签页（当前项下面 2 px `--ink` 线），接着一句汇总（如「9 次传输，选段内 4 个字节」）；右边依次是「文本 / 十六进制」（只在 UART 与 SPI 出现）、「全程 / 选段」、「复制」、关闭。分段控件 22 px 高，选中项 `--raise` 底 + 1 px 描边。
- **列表**：等宽 12 px，行高 20 px，表头固定（sticky）。第一列是时间（ms，四位小数，相对触发点）。
  - **I²C**：一行一次传输：「含义」列 + 「总线上的顺序」列，后者依次是 `S`、`写62`、数据字节、`Sr`、`读62`、数据字节、`N`、`P`；应答了的字节不标记，只把 `N` 写出来。
  - **UART**：文本方式按换行分行，一行最多 80 列，`\r` `\n` `\t` 写成转义并用说明色；十六进制方式每行 16 个字节，右边是对应字符。
  - **SPI**：先 MISO 后 MOSI，各带字节偏移（十进制）。文本方式下连续 4 个以上可打印字符当作文本，其余每行 16 个十六进制字节。
  - **CAN-FD**：一行一帧：标识符、数据字节、帧类型和校验结果。
  - 连续 8 个以上相同字节折成一项，如 `FF × 224`（文本方式下只折不可打印的值）。
  - 其他解码器（通用呈现）：一行一个注解，列为时间、注解行名、文本。
- **采集边界**：开头已经在传、缺少起始条件的 I²C 传输不解码，列一行说明；结尾没传完的，列到最后一个完整字节，后面加「…」和说明。UART 文本两端各用一个「…」表示被截断的帧。
- **与选段的关系**：范围为「全程」时，完整落在 A、B 之间的内容加 `--sel` 底色，列表滚到第一处（上面留一行）；拖动过程中不滚，松手后再滚。范围切到「选段」时只列这些内容，一次传输只有部分在选段里时，缺的一侧用「…」表示；没有选段时该选项不可用（提示「先在标尺上拖出一段」）。点时间轴上的解码块，列表切到它所属的解码器并标出它。
- **与波形的关系**：悬停列表项时，全程概览上标出它的位置；它在当前视口内时，时间轴上也盖一条带子（8% `--ink`，两侧 1 px `--ink-3` 线），对应解码块加 2 px `--ink` 框。点击列表项（设计稿「有但示意稿未做」）：波形平移到该处并选中它。
- 列表必须虚拟化（只渲染可见行 ± 缓冲），以承载百万级结果。

### 9.10 状态栏与导航

- 左起：「数据流」开关（按下态 `--hover` 底）、状态文字（「采集完成」「等待触发」「已采集 37%」…）、触发时间（`触发时间 2026-10-06 17:01:46`）、采样点数（`131 072 个采样点`，千分位用窄空格）。
- 右侧：缩小、当前时基（`50 µs/格`，最小宽 64 px 居中）、放大、缩放到全程。
- **指针与键盘**（设计稿未定义，建议，见 §15）：Ctrl/⌘ + 滚轮以指针为中心缩放；Shift + 滚轮或触控板横向滑动平移；在波形区按住拖动平移（DSView「拖动滚动波形」选项默认开启）；`+`/`-` 缩放，`←`/`→` 平移，`Home` 缩放到全程，`/` 搜索，`Esc` 关闭菜单/取消选段。

### 9.11 对话框与次级界面（设计稿未覆盖）

以下 DSView 功能在设计稿中没有对应画面，v1 按同一套令牌与组件做最简实现，待设计补图（§15）：

- 设备选项对话框（§4.9 的全部选项）；
- 添加/编辑协议解码对话框（解码器列表与搜索、通道映射、选项、叠加解码器、注解行显隐、数值进制）；
- 触发设置面板（触发位置、多级触发、串行触发）；
- 导出与保存对话框；设置面板；关于。

### 9.12 可访问性

- 所有图标按钮有 `aria-label`；组标题的展开按钮有 `aria-expanded`；触发位 `aria-haspopup="menu"`；菜单项 `role="menuitemradio"` + `aria-checked`；数据流标签页 `role="tab"`；分段控件 `aria-pressed`。
- 焦点环：2 px `--ink` 实线、2 px 偏移（标签页内偏移 −2 px）。
- 遵守 `prefers-reduced-motion`：仅在允许动效时给展开箭头加 120 ms 旋转过渡。

---

## 10. 国际化

### 10.1 方案

- 前端使用 **vue-i18n**（Composition API 模式，`legacy: false`），语言包为 JSON：`src/i18n/locales/zh-CN.json`（默认、源语言）、`en-US.json`。构建期用 `@intlify/unplugin-vue-i18n` 预编译消息。
- 语言选择：设置面板中「跟随系统 / 简体中文 / English」，默认跟随系统（`navigator.language` 以 `zh` 开头用中文，否则英文），持久化到应用设置。
- 设计稿里的全部中文文案是**定稿文案**，原样进入 `zh-CN.json`；英文由开发者翻译，需与中文同义、同长度级别（避免撑破布局）。

### 10.2 Key 约定

- 分层命名：`<区域>.<元素>[.<状态>]`，如 `header.capture.start`、`timeline.trigger.menuTitle`、`stream.scope.segment`、`status.captured`。
- 句子级拼接一律用带参数的消息，不在代码里拼字符串：`"trigger.summary.more": "等 {count} 个通道"`、`"status.samples": "{count} 个采样点"`。
- 复数用 vue-i18n 的复数语法（英文需要，中文同形）。
- 新增文案必须同时提供中英两份；`pnpm lint` 中的 i18n 检查（`@intlify/eslint-plugin-vue-i18n`）会报告缺失与未使用的 key。

### 10.3 数值、时间与单位

- 数字、日期用 `Intl.NumberFormat` / `Intl.DateTimeFormat`，按当前语言格式化。采样点数在中文环境用窄不换行空格分组（`131 072`），与设计稿一致。
- 时间与频率单位（ns、µs、ms、s、Hz、kHz、MHz）在两种语言中相同；数值与单位之间一个空格（`5.24 ms`）。时间格式化函数集中在 `src/utils/units.ts`。

### 10.4 后端文案

- Rust 侧**不输出自然语言**：错误、选项名、取值名、presenter 的「含义」模板都用稳定的 key + 参数，由前端翻译。例如 I²C 含义行由 presenter 输出 `{ key: "i2c.meaning.readReg", params: { reg: "04", data: "4F" } }`，中文「从 0x{reg} 读出 {data}」、英文「Read {data} from 0x{reg}」。
- sigrok 兼容解码器自带的英文注解文本不翻译（与 DSView 行为一致）；解码器名称与选项名提供可选的翻译表（`src/i18n/decoders/`），没有翻译时显示原文。

---

## 11. 工程规范

### 11.1 技术栈与版本

| 领域   | 选型                                                                                                 |
| ------ | ---------------------------------------------------------------------------------------------------- |
| 桌面壳 | Tauri 2（`tauri` 2.12、`@tauri-apps/cli` 2.12），插件：dialog、fs、store、log、opener                |
| 前端   | Vue 3.5 + TypeScript 6.0（`typescript-eslint` 尚不支持 TS 7）+ Vite 8 + Pinia + vue-i18n 11 + VueUse |
| 样式   | Tailwind CSS v4（`@tailwindcss/vite`，CSS-first 配置）+ 设计令牌 CSS 变量                            |
| 渲染   | Canvas2D；WebGPU（可用时）                                                                           |
| Rust   | edition 2024，stable 工具链；nusb 0.2、tokio、serde、thiserror、zip、pyo3、ts-rs、tracing            |
| 包管理 | pnpm（前端）、cargo workspace（Rust）                                                                |
| 测试   | Vitest + Vue Test Utils（前端）；`cargo test` + proptest（Rust）                                     |

### 11.2 代码风格

- **前端**：ESLint（flat config：`@eslint/js`、`typescript-eslint` 类型感知规则、`eslint-plugin-vue` recommended、`@intlify/eslint-plugin-vue-i18n`、`eslint-config-prettier` 关闭与格式冲突的规则）+ Prettier（`prettier-plugin-tailwindcss` 排序类名）。格式只由 Prettier 决定，ESLint 只管正确性。
- **Rust**：`rustfmt`（仓库根 `rustfmt.toml`）+ `clippy -D warnings`；公共 API 写 rustdoc；`unsafe` 需要 `// SAFETY:` 注释，硬件 crate 原则上不用 `unsafe`。
- **命名**：Vue 组件 PascalCase 文件名；composable 以 `use` 开头；Pinia store 用 `useXxxStore`；Rust 模块 snake_case；IPC 命令 `snake_case`、事件 `domain://kebab-case`。
- **样式**：只用令牌（Tailwind 主题类或 `var(--token)`），禁止在组件里写死色值；禁止渐变、阴影（`box-shadow` 仅用于设计稿规定的内描边与焦点环）。

### 11.3 Git 钩子（lefthook）

| 钩子         | 任务                                                                                                                     |
| ------------ | ------------------------------------------------------------------------------------------------------------------------ |
| `pre-commit` | 对暂存文件：Prettier 格式化并重新暂存；ESLint `--fix`；`.rs` 文件 `cargo fmt`；`Cargo.toml` 变更时跑 `cargo fmt --check` |
| `commit-msg` | commitlint 校验约定式提交                                                                                                |
| `pre-push`   | `pnpm typecheck`、`pnpm test`、`cargo clippy --workspace -- -D warnings`、`cargo test --workspace`                       |

### 11.4 约定式提交

格式：`<type>(<scope>): <subject>`，subject 用英文祈使句、小写开头、不加句号，≤ 72 字符；正文说明动机与影响；破坏性变更用 `!` 与 `BREAKING CHANGE:`。

- **type**：`feat`、`fix`、`perf`、`refactor`、`style`、`test`、`docs`、`build`、`ci`、`chore`、`revert`。
- **scope**（commitlint 白名单）：`core`、`capture`、`driver`、`dslogic`、`demo`、`decode`、`fileio`、`session`、`app`（src-tauri）、`ui`、`timeline`、`stream`、`trigger`、`render`、`i18n`、`theme`、`ipc`、`store`、`deps`、`tooling`、`repo`。
- **细粒度**：一个提交只做一件可独立回滚的事；脚手架、配置、功能、测试、文档分开提交；不要把格式化改动与逻辑改动混在一起。

---

## 12. 测试策略

| 层级         | 内容                                                                                                     |
| ------------ | -------------------------------------------------------------------------------------------------------- |
| 单元（Rust） | 位存储与摘要（proptest 对照朴素实现）、边沿查找、bins、触发编码与 `DSL_setting` golden 字节、`.dsl` 往返 |
| 驱动         | `UsbTransport` 模拟器回放打开/配置/采集的完整命令序列                                                    |
| 解码         | 原生解码器与 sigrok 解码器在同一组采集上的注解逐条对照（DSView 自带 demo 采集 + 自制样本）               |
| 单元（前端） | 单位格式化、触发汇总文案、数据流行生成（I²C/UART/SPI/CAN 规则）、视口换算                                |
| 组件         | 触发菜单键盘交互、数据流面板范围切换、选段逻辑                                                           |
| 视觉回归     | 以 Demo 设备打开 `protocol.demo`，在 1440×900 下截图与设计稿画板对照（两套主题、两种密度）               |
| 端到端       | 启动应用 → Demo 采集 → 添加解码 → 保存 `.dsl` → 用 DSView 打开验证                                       |

验收标准沿用设计稿：两套主题、两种密度下颜色与尺寸与令牌表一致；界面无渐变与投影、状态表达不用红/琥珀；16 个通道各有一个触发位，设两个以上通道时顶栏汇总正确；数据流一次只显示一个解码器并列出整段采集，框选后高亮与「选段」筛出的内容一致；悬停列表项时概览与时间轴上的标记位置正确；窗口变窄时顶栏换行、时间轴横向滚动、页面不出现横向滚动条。

---

## 13. 里程碑

| 里程碑  | 内容                                                                                                | 完成标志                                            | 状态                                                                                                                                                                                                                                                        |
| ------- | --------------------------------------------------------------------------------------------------- | --------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M0 基础 | 仓库、工具链、钩子、文档；Tauri + Vue 壳；令牌、主题、密度、i18n                                    | `pnpm tauri dev` 打开空壳，两套主题与两种语言可切换 | 完成                                                                                                                                                                                                                                                        |
| M1 数据 | `hsync-capture` 存储与查询；`.dsl` 读取；Demo 设备；时间轴渲染（Canvas2D）、缩放平移、概览、标尺    | 打开 `protocol.demo`，波形与设计稿一致              | 完成                                                                                                                                                                                                                                                        |
| M2 硬件 | `hsync-driver` 设备管理与热插拔；`hsync-dslogic` 全部型号的打开、FPGA、Buffer/Stream 采集、简单触发 | 真机采集结果与 DSView 逐位一致                      | 驱动、Demo 设备、设备与采集界面完成，U2Pro16 真机通过；重连窗口（2 s，同型号重开并恢复设置）与按驱动记住设置完成（默认值与 `*.def.dsc` 一致）；循环采集完成（Stream 模式与 Demo 设备），U2Pro16 真机验证：10 MHz 连续 3 s、约 20 MB/s，保留最新 1 Mi 样本   |
| M3 解码 | 注解模型；原生 I²C/UART/SPI/CAN-FD；presenter；解码行；数据流面板；选段；悬停读数                   | 设计稿全部画板可复现                                | 完成：四种原生解码器与呈现、解码行、数据流（含选段与逐字节底色）、添加/修改/移除解码器                                                                                                                                                                      |
| M4 对齐 | sigrok 兼容宿主（pyo3）；高级/串行触发与触发位置；搜索；测量；导出；保存；会话；设置                | §2 中全部 v1 项完成                                 | 已完成：sigrok 兼容宿主（150 个解码器、叠加）、搜索、触发位置与多级/串行触发、选段沿计数、打开/保存/导出（含进度与取消）、数据流导出、通道改名与颜色、会话、数值进制与注解行显隐、设置面板（行为与诊断）、重复采集的后备缓冲、`.dsc` 中的设备选项与采集方式 |
| M5 打磨 | WebGPU 渲染；大容量（换出）；性能与内存基准；三平台打包与签名；用户文档                             | 发布 v1.0                                           | 已完成：WebGPU 渲染（实例化矩形，自动回退）；待做：换出、基准、打包（含内置 CPython）、用户文档                                                                                                                                                             |

---

## 14. 风险与对策

| 风险                                    | 对策                                                                              |
| --------------------------------------- | --------------------------------------------------------------------------------- |
| 无法在所有型号的真机上回归              | golden 字节测试保证与 DSView 一致；征集持有各型号的测试者；驱动日志可导出         |
| Windows 驱动：nusb 需要 WinUSB          | 安装包附带 WinUSB 的 INF（DSView 也需要驱动安装）；打开失败时给出明确指引         |
| macOS 上 nusb 与设备重枚举的时序        | 等待重连窗口可配置，重试打开 3 次                                                 |
| 嵌入 CPython 的体积与签名               | Python 宿主做成可选组件（首次使用时下载或随完整版安装包），原生解码器覆盖常用协议 |
| WebView 差异（WebGPU 可用性、字体渲染） | Canvas2D 为基线；字体本地打包；视觉回归在三平台各跑一次                           |
| 超大采集占满内存                        | 内存预算检查 + 换出到内存映射文件；Stream 模式实时显示已用内存                    |

---

## 15. 待确认事项

实现前请产品负责人确认。括号内是在确认之前采用的默认做法。

**来自设计稿**

1. 多个通道的触发条件按「同时满足才触发」处理（DSView 简单触发即如此，硬件也支持「任一满足」）。是否需要提供「任一满足」？若提供，汇总与菜单说明需要改。（默认：只做「同时满足」。）
2. I²C 的蓝与 SPI 的紫较接近，色觉障碍用户不易区分，是否把 SPI 换成离蓝更远的颜色？（默认：沿用设计稿。）
3. 选段之后是否自动打开数据流面板？（默认：不打开。）
4. 波形平移或缩放时，数据流列表是否跟着滚到视口对应位置？（默认：不跟随。）
5. 触发位置、多级触发、协议触发的界面尚未设计。（默认：顶栏「触发」下拉面板，按 DSView 的字段做最简表单。）

**本文新增**

6. 时间轴的指针手势（滚轮、拖动、概览交互）设计稿未定义，§9.5/§9.10 为建议方案。
7. 设备选项、添加解码、导出、设置等对话框尚无设计稿（§9.11）。
8. 140 余个非 I²C/UART/SPI/CAN 解码器的配色：默认使用中性的通用总线色，是否需要为常见协议（1-Wire、I²S、JTAG、SWD、USB PD…）分配专属总线色？
9. `MAX25_VTH` 型号的 VTH 可调上限：DSView 驱动按 2.5 V 量程换算寄存器值，但界面允许的范围需以硬件规格确认。
10. Python 解码器宿主的分发方式：随完整安装包内置，还是首次使用时下载？
11. 是否需要在 v1 支持 DSView 之外的逻辑分析仪（如 fx2lafw 类设备）？架构已预留，但不在 v1 范围内。

---

## 16. 附录：DSView 源码索引

| 主题                          | 位置                                                                                          |
| ----------------------------- | --------------------------------------------------------------------------------------------- |
| 设备型号表、通道模式、能力位  | `libsigrok4DSL/hardware/DSL/dsl.h`                                                            |
| 控制协议、寄存器地址          | `libsigrok4DSL/hardware/DSL/command.{h,c}`                                                    |
| 打开、FPGA 配置/arm、数据接收 | `libsigrok4DSL/hardware/DSL/dsl.c`                                                            |
| DSLogic 选项、扫描、开始/停止 | `libsigrok4DSL/hardware/DSL/dslogic.c`                                                        |
| 触发编码                      | `libsigrok4DSL/trigger.c`、`DSView/pv/dock/triggerdock.cpp`、`DSView/pv/view/logicsignal.cpp` |
| 固件下载                      | `libsigrok4DSL/hardware/common/ezusb.c`                                                       |
| 设备管理、热插拔、采集线程    | `libsigrok4DSL/lib_main.c`                                                                    |
| `.dsl` 读写                   | `DSView/pv/storesession.cpp`、`libsigrok4DSL/session_driver.c`                                |
| 逻辑数据存储与摘要            | `DSView/pv/data/logicsnapshot.{h,cpp}`                                                        |
| 采集进度与触发位置            | `DSView/pv/sigsession.cpp`（`get_capture_status`、`feed_in_trigger`）                         |
| 采样时长列表                  | `DSView/pv/toolbars/samplingbar.cpp`                                                          |
| 解码器运行时                  | `libsigrokdecode4DSL/{type_decoder.c,decoder.c,instance.c,session.c}`                         |
| 解码器                        | `libsigrokdecode4DSL/decoders/`                                                               |
| 界面文案                      | `lang/{cn,en}/*.json`                                                                         |
| Demo 数据                     | `DSView/demo/{logic,analog,dso}/*.demo`                                                       |
