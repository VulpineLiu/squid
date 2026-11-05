# Squid

[English](README.en.md) · [返回双语首页](README.md)

**给你的电脑洗个澡。**

Squid 是面向 macOS 与 Windows 的本地系统维护工具：清理可重建的缓存与系统垃圾、卸载应用并处理残留、清理项目构建产物、分析磁盘占用，以及实时查看系统状态。

桌面端、命令行与 TUI 共用同一套 Rust 扫描和删除引擎。清理规则、安全边界、删除账本与配置只维护一份，不依赖云端服务，也不需要 root。

## 产品组成

| 入口 | 技术栈 | 二进制 |
| --- | --- | --- |
| 桌面应用 | Tauri 2 + React 19 + TypeScript + Vite 6 | `squid-desktop` |
| 命令行 | Rust + clap | `squid` |
| 终端界面 | Rust + ratatui + crossterm | `squid state` |

桌面端支持 macOS 12+ 与 Windows 10/11 x64；CLI/TUI 使用同一个 Rust workspace。

## 功能

| 功能 | 说明 |
| --- | --- |
| 电脑概览 | 磁盘健康、常用操作入口与 CPU、内存、存储摘要 |
| 日常清理 | macOS 38 条 / Windows 30 条内置规则，覆盖应用与浏览器缓存、日志、崩溃报告、开发缓存与废纸篓等 |
| 应用卸载 | 按名称定位应用，把应用本体与偏好、缓存、容器或 AppData 残留组成同一个卸载计划 |
| 深度清理 | 查找 `node_modules` 等可重建构建产物、磁盘镜像、安装包和陈旧下载，并保护近期仍在使用的项目 |
| 存储空间 | 逐层测量目录、查看最大子项与最大文件 |
| 实时监控 | 每两秒采样 CPU、内存、网络、磁盘、电池、卷和高占用进程，可确认后结束进程 |
| 历史与撤销 | 每次真实删除写入本地账本；仍在废纸篓或回收站中的内容可以还原 |
| 系统更新 | 从发布分支检查、校验、下载并安装新版本 |

语言设置只提供简体中文与 English 两项，首次使用时默认选中当前系统语言。手动选择会保存为该电脑上的界面语言。

## 桌面应用

### 左侧菜单

- **维护**：电脑概览、日常清理、应用卸载、深度清理
- **洞察**：存储空间、实时监控
- **底部**：历史记录、设置

`⌘1`–`⌘8`（macOS）或 `Ctrl+1`–`Ctrl+8`（Windows）可以直接切换页面。

### 常驻托盘

关闭主窗口只会隐藏窗口，Squid 进程继续运行。托盘菜单提供：

- 打开 Squid
- 日常清理
- 应用卸载
- 深度清理
- 退出 Squid

托盘菜单跟随当前界面语言。再次打开应用、双击应用图标或通过单实例唤起时，会恢复并聚焦主窗口。只有选择“退出 Squid”才会结束进程。

### 软件更新

- 设置中的系统信息面板始终显示当前系统版本、架构与 Squid 版本。
- 只有检测到更高版本时，才显示版本变化、更新说明和操作按钮；已经是最新版本时不显示额外提示。
- 下载完成后可安装并重启，或用与 Shift 一致的中性描边“放弃”按钮清除已下载文件。
- 自动检查默认开启，每天最多检查一次。
- 安装包必须通过 SHA256 与文件大小校验；带发布公钥的构建还会校验清单的 Ed25519 签名。

### 权限与隐私

- 首次启动会显示一次可选的全盘访问检查，不会强制弹出 macOS 权限窗口，也不会阻止应用启动。
- “设置 → 系统权限”提供权限状态检查和打开“系统设置 → 隐私与安全性 → 全盘访问”的入口。
- 没有全盘访问时，受 TCC 保护的位置会被跳过，其余功能仍然可用。
- 存储分析与深度清理只在用户选择或配置目录后读取；文件内容不会上传。
- 除更新检查和安装包下载外，应用不发起网络请求。

### 开机自启

“设置 → 启动”提供开机自启开关。启用后 Squid 会在当前用户登录时自动启动并保留托盘入口；该设置只写入当前用户级启动项，不需要管理员权限。

完整说明见 [`docs/DESKTOP.md`](docs/DESKTOP.md)。

## 快速开始

### 桌面端开发

环境要求：

- Node.js 20+ 与 pnpm
- Rust stable
- macOS：Xcode Command Line Tools
- Windows：MSVC Build Tools 与 WebView2 Runtime

```bash
pnpm install
pnpm desktop:dev
```

仅运行 Vite 页面时无法调用 Tauri 命令；需要验证扫描、删除、系统数据或更新流程时请使用 `pnpm desktop:dev`。

### CLI / TUI

```bash
cargo install --path crates/squid-cli
# 或
cargo build --release -p squid-cli

squid state                       # TUI
squid status                      # 系统快照
squid clean --dry-run             # 预览可清理内容
squid uninstall --list            # 已安装应用
squid purge ~/code                # 深度清理项目构建产物
squid analyze ~/Library/Caches    # 存储分析
squid history                     # 历史账本
squid undo                        # 还原最近一次废纸篓会话
```

常用命令：

| 命令 | 作用 |
| --- | --- |
| `clean` | 按规则清理缓存、日志、浏览器与开发者垃圾、崩溃报告和废纸篓 |
| `uninstall [NAME]` | 定位应用并生成“本体 + 残留”卸载计划 |
| `purge [PATHS…]` | 清理项目构建目录、磁盘镜像、安装包与陈旧下载 |
| `state` | 打开包含清理、卸载、深度清理、存储、系统和历史面板的 TUI |
| `status` | 输出 CPU、内存、网络、磁盘、电池与进程快照 |
| `analyze [PATH]` | 输出目录体积树与最大文件 |
| `history` / `undo [ID]` | 查看历史或还原废纸篓会话 |
| `whitelist add\|list\|remove` | 维护永不选中的保护路径 |
| `rules` | 列出当前生效的规则 |
| `doctor` | 检查权限、配置、历史文件和生效路径 |

所有删除命令都支持 `--dry-run`；非交互环境必须使用 `-y/--yes`。完整参数请运行 `squid <command> --help`。

## 架构

```text
crates/
├── squid-core       模型、路径、安全策略、设置、删除账本、进度与取消
├── squid-platform   macOS / Linux / Windows 平台适配：应用、残留、卷、回收站、进程
├── squid-rules      TOML 规则引擎：模式展开、锚点、保护、年龄与风险标记
├── squid-stats      sysinfo 采样、趋势、电池、网络与磁盘活动
├── squid-engine     clean / uninstall / purge / analyze 与唯一删除通道
├── squid-tui        ratatui 终端界面
└── squid-cli        clap 命令行入口

src/                 React 桌面界面
src-tauri/           Tauri 壳、命令适配、托盘、权限检查与更新器
scripts/             开发、验证与双平台发布脚本
docs/                架构、设计、桌面与发布规范
```

`src-tauri` 独立于根 Cargo workspace，但直接依赖 `squid-core`、`squid-platform`、`squid-stats` 与 `squid-engine`，桌面端不维护第二套清理实现。

更多设计见 [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)；Nex 系列统一颜色、字体、组件和语言规范见 [`docs/DESIGN.md`](docs/DESIGN.md)。

## 安全模型

1. **索引式删除**：桌面界面只能提交扫描结果中的索引，不能提交任意待删除路径。
2. **执行前复验**：Rust 层重新构造扫描条目，并检查路径所有权、安全锚点、保护列表、白名单与锁定状态。
3. **默认进废纸篓**：删除模式默认是 `trash`；只有显式选择永久删除才直接移除。
4. **硬性禁区**：系统根目录、`/usr`、`/System`、`/Applications` 等位置不能被递归删除；符号链接不会被跟随。
5. **规则刹车**：规则支持 `protect`、`min_age_days`、`skip_if_running` 与 `risky` 标记。
6. **账本与撤销**：真实删除记录到本地 JSON 账本，仍在废纸篓中的文件可以还原。
7. **进程保护**：结束进程前重新采样并核对名称，拒绝 Squid 自身、系统首进程和关键系统进程。

GUI 和 CLI 使用同一个 `squid-engine::delete` 通道，因此行为、锁定和审计结果一致。

## 数据与配置

| 用途 | macOS | Windows |
| --- | --- | --- |
| 配置与规则 | `~/Library/Application Support/squid/` | `%APPDATA%\squid\` |
| 历史账本 | `~/Library/Application Support/squid/history/` | `%APPDATA%\squid\history\` |
| 删除去向 | `~/.Trash` 或永久删除 | 回收站或永久删除 |

Linux 使用 XDG 目录。桌面端与 CLI/TUI 共用配置和历史，因此任一端产生的清理记录都能在另一端查看。

常用配置：

```toml
delete_mode = "trash"
history_keep = 60
purge_min_age_days = 7
purge_max_entries = 200000
show_allocated = true
# language 留空或删除表示跟随系统；也可手动设为 "en" 或 "zh"
language = ""
whitelist = ["~/Library/Caches/ImportantApp"]
disabled_rules = ["user-trash"]
purge_roots = ["~/code", "~/Projects", "~/Desktop"]
```

主要环境变量：

| 变量 | 作用 |
| --- | --- |
| `SQUID_CONFIG_DIR` | 覆盖配置目录 |
| `SQUID_STATE_DIR` | 覆盖状态与历史目录 |
| `SQUID_LANG` | 强制 CLI/TUI 使用 `zh` 或 `en` |
| `SQUID_UPDATE_URL` | 覆盖桌面端更新清单地址，适合测试发布通道 |
| `SQUID_TRUECOLOR` / `NO_COLOR` | 控制 TUI 颜色 |
| `SQUID_SYMBOL_CELLS` / `SQUID_CONSOLE_TRACE` | 排查 Windows 控制台字符宽度与排版 |

`SQUID_LANG` 是命令行覆盖项；桌面界面的默认语言来自系统设置，并可在应用内修改。

## 开发与验证

前端：

```bash
pnpm check
pnpm build
```

Rust workspace：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
```

Tauri：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

TUI 使用真实 pty 冒烟测试；Windows 控制台另有“抓帧、复放、量框”验证：

```bash
python3 -m pip install --user pyte
python3 scripts/tui_smoke.py
python3 scripts/tui-frame-check.py --selftest
```

CI 同时执行 workspace 测试、MSVC 目标交叉检查、桌面前端构建和 Tauri Rust 检查。

## 打包与发布

版本必须同时写入以下四处：

- `Cargo.toml` workspace 版本
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`
- `package.json`

构建安装包：

```bash
scripts/build-macos-release.sh
scripts/build-windows-release.sh
```

产物：

```text
artifacts/Squid-<version>-macOS-universal.dmg
artifacts/Squid-Setup-<version>-win-x64.exe
artifacts/SHA256SUMS.txt
```

发布到更新通道：

```bash
scripts/publish-release.sh --notes /path/to/release-notes.md
```

发布脚本会重建 `releases` 分支，写入 `squid-update.json`、两个安装包和可选的 `squid-update.sig`。清单包含精确大小与 SHA256，客户端下载后再次校验。

完整发布流程、签名和清单格式见 [`docs/RELEASE.md`](docs/RELEASE.md)。

## 平台状态

| 平台 | 状态 |
| --- | --- |
| macOS | 桌面端与 CLI/TUI 完整支持，发布通用 DMG |
| Windows x64 | 桌面端与 CLI/TUI 支持，发布 NSIS 安装包 |
| Linux | 平台层与 CLI/TUI 基础实现；尚未提供内置 Linux 规则和桌面安装包 |

## 许可证

Squid 使用 **GNU General Public License v3.0 only**（`GPL-3.0-only`）。完整条款见 [`LICENSE`](LICENSE)。
