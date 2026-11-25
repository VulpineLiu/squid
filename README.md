# Squid

**给你的电脑洗个澡。** · **Give your computer a bath.**

[简体中文](README.zh-CN.md) · [English](README.en.md) · [使用帮助](docs/USAGE.zh-CN.md) · [Usage guide](docs/USAGE.md) · [更新日志](CHANGELOG.md)

Squid is a local system-care app for macOS and Windows. It cleans rebuildable caches and system junk, uninstalls apps with their leftovers, removes project build artefacts, maps disk usage, and monitors live system health.

Squid 是面向 macOS 与 Windows 的本地系统维护工具。桌面端、CLI 与 TUI 共用同一套 Rust 扫描和删除引擎，不依赖云端服务，也不需要 root。

## Download / 下载

Latest release: **0.1.11** (2026-11-22).

| Platform | Package |
| --- | --- |
| macOS 12+ | [`Squid-0.1.11-macOS-universal.dmg`](https://git.unissense.tech/mula/nex_squid/raw/branch/releases/Squid-0.1.11-macOS-universal.dmg) |
| Windows 10/11 x64 | [`Squid-Setup-0.1.11-win-x64.exe`](https://git.unissense.tech/mula/nex_squid/raw/branch/releases/Squid-Setup-0.1.11-win-x64.exe) |

Installers live on the origin repository's `releases` branch:
<https://git.unissense.tech/mula/nex_squid/src/branch/releases>

The GitHub repository mirrors source code only. It does not carry the `releases` branch, and the app's update feed still points at the origin channel.

安装包发布在 origin 仓库的 `releases` 分支。GitHub 只镜像源码，不承载发布分支；应用更新仍从 origin 的发布通道检查。

## Highlights / 功能

- **Daily cleanup / 日常清理** — 38 macOS rules and 30 Windows rules for caches, logs, crash reports, developer data, upgrade installers, and Trash.
- **Uninstall apps / 应用卸载** — One plan for the app and its preferences, caches, containers, or AppData leftovers.
- **Deep Clean / 深度清理** — Rebuildable build output, disk images, installers, and stale downloads with project-age protection.
- **Storage & monitor / 存储与监控** — Folder-level disk analysis, live CPU, memory, network, disk, battery, and process views.
- **Security / 安全** — XProtect and Defender status, malware scanning with confirmed / suspicious / advisory findings, reversible quarantine, and a trust allowlist. No daemon, no kernel extension, no upload.
- **Startup items / 启动项** — LaunchAgents, LaunchDaemons, login items, registry Run keys, startup folders, scheduled tasks and services; disable the ones you own.
- **Desktop widget / 桌面小工具** — Pin the Real-time monitor to the desktop as a focus-free board.
- **History & updates / 历史与更新** — Restorable Trash sessions and a size- and SHA256-verified update channel.

The interface follows the system language by default and can be switched to Simplified Chinese or English in Settings.

界面默认跟随系统语言，可在设置中切换简体中文或 English。

## Start / 开始

Desktop development:

```bash
pnpm install
pnpm desktop:dev
```

CLI / TUI:

```bash
cargo install --path crates/squid-cli --locked
squid state
squid clean --dry-run
```

## Documentation / 文档

- [中文完整说明](README.zh-CN.md)
- [Full English guide](README.en.md)
- [中文使用帮助](docs/USAGE.zh-CN.md)
- [English usage guide](docs/USAGE.md)
- [更新日志 / Changelog](CHANGELOG.md)
- [Architecture / 架构](docs/ARCHITECTURE.md)
- [Nex design standard / 设计规范](docs/DESIGN.md)
- [Desktop / 桌面端](docs/DESKTOP.md)
- [Release and updates / 发布与更新](docs/RELEASE.md)
- [Security / 安全](docs/SECURITY.md)

## Project / 项目

- Source mirror: <https://github.com/VulpineLiu/squid>
- Origin and update channel: <https://git.unissense.tech/mula/nex_squid>

## License / 许可证

**GNU General Public License v3.0 only** (`GPL-3.0-only`).

Copyright © 2026 mula.liu. See [`LICENSE`](LICENSE) and [`COPYRIGHT`](COPYRIGHT).
