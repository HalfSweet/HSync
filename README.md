# HSync

HSync 是一个现代化的逻辑分析仪上位机，用 Tauri 2、Vue 3 与 Rust 重写 [DSView](https://github.com/DreamSourceLab/DSView)。初版的目标是与 DSView 的逻辑分析仪功能保持对齐：支持 DreamSourceLab 的全部 DSLogic 型号与 Demo 设备、Buffer/Stream 采集、触发、协议解码、测量、搜索，以及 DSView `.dsl` 文件的读写。

> 项目处于开发阶段：v1 的逻辑分析仪功能已与 DSView 基本对齐，并在 DSLogic U2Pro16 上实测过采集、触发与循环采集；安装包由 GitHub Actions 构建（macOS Apple 芯片、Windows x64、Linux x64，内置 Python 解码器宿主），随版本发布在 GitHub Releases，应用内可以检查并安装更新。

## 文档

- [用户指南](./docs/user-guide.md)：界面与操作。
- [DESIGN.md](./DESIGN.md)：界面设计规范——布局、设计令牌、各区域的外观与交互、可访问性与动效。
- [AGENTS.md](./AGENTS.md)：项目的硬性要求、构建命令、编码与提交规范。

## 开发

需要 Node.js ≥ 22.12、pnpm 12、Rust stable、PATH 中带共享库的 Python 3（Python 协议解码器宿主经 pyo3 嵌入 CPython），以及 [Tauri 的系统依赖](https://tauri.app/start/prerequisites/)。

```sh
pnpm install        # 安装依赖，同时安装 lefthook 钩子
pnpm tauri dev      # 启动桌面应用
pnpm dev            # 只启动前端（http://localhost:1420）
```

检查与测试：

```sh
pnpm lint && pnpm typecheck && pnpm test
cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

按发行方式构建本机的安装包（下载并内置 CPython，参数传给 `tauri build`）：

```sh
pnpm bundle
```

提交信息遵循约定式提交，由 commitlint 校验，发布说明据此生成（发布方式见 [AGENTS.md](./AGENTS.md)）；暂存的文件在提交前由 Prettier、ESLint 与 rustfmt 处理。GitHub Actions 在每个 PR 与 `main` 的推送上运行同样的检查。

## 许可

GPL-3.0-or-later，见 [LICENSE](./LICENSE)。DSLogic 固件与 FPGA 位流来自 DreamSourceLab，以 MIT 许可分发；IBM Plex 字体以 SIL OFL 1.1 许可分发。
