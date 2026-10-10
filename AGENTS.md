# AGENTS.md

本文件写给在本仓库工作的 AI agent 与工程师：项目要做什么、硬性要求、怎么构建与验证、怎么提交。动手前先读完本文件，再按需查阅 [DESIGN.md](./DESIGN.md)。

## 项目是什么

HSync 用现代技术栈重写 [DSView](https://github.com/DreamSourceLab/DSView)，目标是一个通用的逻辑分析仪上位机。**初版（v1）的功能与 DSView 的逻辑分析仪功能保持对齐**：支持全部 DSLogic 型号与 Demo 设备、Buffer/Stream 采集、简单/高级/串行触发、协议解码、测量、搜索、`.dsl` 文件读写与导出。完整的对齐清单见 DESIGN.md §2，示波器（DSO）与数据采集（DAQ）模式不在 v1 范围内。

## 硬性要求

这些要求来自项目负责人，任何改动都不能违背：

1. **技术栈**：桌面壳用 **Tauri 2**；前端用 **Vue 3 + TypeScript + Tailwind CSS v4**；波形渲染以 Canvas2D 为基线，**在可用时启用 WebGPU 加速**，并始终保留回退路径。
2. **UI/UX 以 DESIGN.md §9 为准，`.hsync/` 设计稿只作参考**：`.hsync/handoff.md` 与 `.hsync/逻辑分析仪界面改版.html`（本地文件，不入库）是起点，不是必须遵守的规范；界面可以按实际使用体验改进，改动同步写进 DESIGN.md §9。
3. **国际化**：首版支持简体中文（默认）与英文。界面上任何用户可见文字都必须走 vue-i18n，禁止硬编码；Rust 侧不输出自然语言，只输出 i18n key 与参数。
4. **与真实硬件的一切交互都用 Rust 实现**，USB 访问只用 **nusb**。禁止引入 libusb、rusb 或任何 C/C++ 驱动代码的 FFI；DSView 的 C 代码只作为移植参考。
5. **代码风格**：前端遵循 **ESLint + Prettier**；Rust 遵循 rustfmt + clippy。由 **lefthook** 在提交时保证代码已格式化，提交信息由 commitlint 校验。
6. **提交**：使用**细粒度的约定式提交**（Conventional Commits），见下文「提交规范」。

## 仓库地图

| 路径             | 内容                                                                    |
| ---------------- | ----------------------------------------------------------------------- |
| `DESIGN.md`      | 总体设计：范围、架构、硬件协议、数据模型、渲染、解码、文件格式、UI 规格 |
| `docs/`          | 用户文档（`user-guide.md`），界面或操作变化时同步更新                   |
| `src/`           | Vue 前端（组件、Pinia store、渲染器、IPC 封装、i18n）                   |
| `src-tauri/`     | Tauri 应用 crate（命令、事件、资源：固件与 Demo 数据）                  |
| `crates/hsync-*` | Rust 库：core、capture、driver、dslogic、demo、decode、fileio、session  |
| `scripts/`       | 布置内置 CPython（DESIGN.md §7.3），按发行方式构建本机安装包            |
| `.github/`       | CI 检查、安装包构建与 release-please 发布（DESIGN.md §11.5）            |
| `.hsync/`        | 设计稿（本地参考，已在 `.gitignore` 中，不要提交）                      |
| `../DSView`      | 参考实现（DSView 源码），只读                                           |

Rust crate 之间的依赖方向见 DESIGN.md §3.1：`src-tauri → hsync-session → {driver, capture, decode, fileio} → hsync-core`。库 crate 不得依赖 Tauri。

## 常用命令

```sh
pnpm install              # 安装依赖并安装 lefthook 钩子
pnpm dev                  # 只启动前端（Vite，http://localhost:1420）
pnpm tauri dev            # 启动桌面应用（前端 + Rust）
pnpm build                # 类型检查并构建前端
pnpm tauri build          # 打包桌面应用
pnpm bundle               # 按发行方式打包本机安装包（内置 CPython）

pnpm lint                 # ESLint
pnpm lint:fix             # ESLint 自动修复
pnpm format               # Prettier 写入
pnpm format:check         # Prettier 检查
pnpm typecheck            # vue-tsc
pnpm test                 # Vitest（单次运行）

cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

完成一项改动前，至少运行与改动相关的：`pnpm lint`、`pnpm typecheck`、`pnpm test`、`cargo clippy`、`cargo test`。

编译 `crates/hsync-srd`（Python 解码器宿主，经 pyo3 嵌入 CPython）需要 PATH 中有带共享库的 Python 3（发行构建与 CI 改用内置的 CPython，见 DESIGN.md §7.3）；`src-tauri/resources/decoders/` 是从 DSView 复制的解码器，不要改动其内容。

## 编码规范

### 前端

- 组件用 `<script setup lang="ts">`；文件名 PascalCase；composable 以 `use` 开头；Pinia store 命名 `useXxxStore`。
- 跨端类型从 `src/ipc/bindings/`（ts-rs 生成，勿手改）导入；调用后端只经过 `src/ipc/` 中的封装，组件里不直接 `invoke`。
- 样式只使用设计令牌：Tailwind 主题类（`bg-canvas`、`text-ink-2`、`border-line` …）或 `var(--token)`。**禁止**写死色值、渐变、投影、发光；红色与琥珀色只用于错误与警告。
- 数值一律等宽数字（`tabular-nums`）；时间、频率、样本数的格式化统一走 `src/utils/units.ts`。
- 大列表（数据流、解码结果）必须虚拟化；波形绘制不得逐样本循环，只消费后端返回的 bins/边沿数据。
- 可访问性：图标按钮必须有 `aria-label`；菜单、标签页、分段控件遵循 DESIGN.md §9.12。

### Rust

- edition 2024，stable 工具链；`clippy -D warnings` 必须通过；公共 API 写 rustdoc。
- 错误用 `thiserror` 定义的枚举，跨 IPC 的错误转换成 `{ code, params }`，`code` 是 i18n key。
- 硬件、存储、解码 crate 原则上不写 `unsafe`；确需时附 `// SAFETY:` 说明。
- 长任务（采集、解码、保存、导出）必须可取消，并通过事件或 `Channel` 上报进度。

### 硬件层（`hsync-dslogic`、`hsync-driver`）

- 行为与 DSView **逐字节一致**：命令、寄存器、`DSL_setting` 布局、触发编码、传输规模、停止流程都以 `libsigrok4DSL/hardware/DSL/` 为准（DESIGN.md §4 有完整规格与源码位置）。偏离时在代码注释中写明原因。
- 控制传输一律经已认领的 `nusb::Interface` 发出（Windows 不支持 `Device::control_*`）。
- DSView 中没有上限的轮询，移植时一律加超时。
- 每个协议细节都要有单元测试：结构体字节布局用 golden 数据断言；命令序列用 `UsbTransport` 模拟器回放。
- 不要修改 `src-tauri/resources/firmware/` 下的固件与位流（MIT 许可，来自 `DSView/res`，保留其许可文件）。

### 设计实现

- 尺寸、颜色、字号、间距、交互规则与文案以 DESIGN.md §9 为准；设计稿只作参考，好用比像设计稿更重要。设计稿标注为「示意」的部分（写死的数据、只有外观的按钮）要接上真实功能。
- 设计稿里被否决过的方案默认不做：右侧详情面板、底部栏小波形、高电平渐变阴影、并排塞进一页的数据流、顶栏里的全局触发设置；确有需要时先问负责人。
- 遇到 DESIGN.md §15「待确认事项」中的问题，按其中写明的默认做法实现，并在代码里留 `TODO(product):` 注释指向对应条目；不要自行另做决定。

### 国际化

- 文案放在 `src/i18n/locales/zh-CN.json` 与 `en-US.json`，两份文件的 key 必须完全一致（有测试校验）。
- key 用分层命名 `<区域>.<元素>[.<状态>]`；句子用带参数的消息，不在代码里拼接字符串。
- 设计稿中的中文文案可作参考，不清楚的可以改；英文需与中文同义且长度相近。

## 提交规范

格式：

```
<type>(<scope>): <subject>

<body：为什么改、改了什么、有什么影响>
```

- **type**：`feat`、`fix`、`perf`、`refactor`、`style`、`test`、`docs`、`build`、`ci`、`chore`、`revert`。
- **scope**（可省略，填写时必须在白名单内，见 `commitlint.config.js`）：`core`、`capture`、`driver`、`dslogic`、`demo`、`decode`、`fileio`、`session`、`app`、`ui`、`timeline`、`stream`、`trigger`、`render`、`i18n`、`theme`、`ipc`、`store`、`deps`、`tooling`、`repo`。
- **subject**：英文祈使句，小写开头，不加句号，≤ 72 字符。
- **细粒度**：一个提交只做一件可以独立回滚、独立审查的事。配置、脚手架、功能、测试、文档分开提交；格式化改动与逻辑改动不要混在一起；新增依赖与使用它的代码可以放在同一个提交里。
- 破坏性变更在 type 后加 `!`，并在正文写 `BREAKING CHANGE:`。
- 版本号与 `CHANGELOG.md` 由 release-please 从提交信息算出（DESIGN.md §11.5），`feat`、`fix`、`perf` 的 subject 会进入发布说明。不要手改版本号与 `CHANGELOG.md`；发布由负责人合并 release-please 的发布 PR 完成。
- 不要用 `--no-verify` 跳过钩子；钩子失败就修好再提交。不要提交 `.hsync/`、构建产物与本地配置。

示例：

```
feat(dslogic): encode simple trigger into fpga setting stages
fix(timeline): keep trigger menu inside the viewport near the bottom edge
build(tooling): run prettier on staged files in lefthook pre-commit
docs: describe the .dsl block layout in DESIGN.md
```

## 测试要求

- 新功能附带测试；修 bug 先写能复现的测试。
- Rust：存储与查询用 proptest 对照朴素实现；`.dsl` 读写做往返测试，并用 DSView 自带的 demo 文件做兼容性测试。
- 前端：纯逻辑（单位格式化、触发汇总、数据流行生成、视口换算）用 Vitest 单测；交互组件用 Vue Test Utils。
- 涉及界面外观的改动，用 Demo 设备打开 `protocol.demo`，在 1440×900 下检查纸面/夜间 × 舒适/紧凑四种组合，可以参照设计稿画板。

## 许可

本项目为 GPL-3.0-or-later（移植自 GPL 的 libsigrok4DSL 与 libsigrokdecode）。从 DSView 移植的文件在文件头注明来源；固件与 FPGA 位流为 MIT 许可，字体为 SIL OFL 1.1。

## 工作方式

- 先读 DESIGN.md 的相关章节，再动手；架构或协议上的改动同步更新 DESIGN.md。
- 产品层面拿不准的问题（交互、文案、范围）向项目负责人提问，不要猜。
- 不要修改 `../DSView`；它只是参考实现。
