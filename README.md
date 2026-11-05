# Squid

**给你的电脑洗个澡。** · **Give your computer a bath.**

[简体中文](README.zh-CN.md) · [English](README.en.md)

Squid is a local system-care app for macOS and Windows. It cleans rebuildable caches and system junk, uninstalls apps with their leftovers, removes project build artefacts, maps disk usage, and monitors live system health.

Squid 是面向 macOS 与 Windows 的本地系统维护工具。桌面端、CLI 与 TUI 共用同一套 Rust 扫描和删除引擎，不依赖云端服务，也不需要 root。

## Highlights / 功能

- **Daily cleanup / 日常清理** — 38 macOS rules and 30 Windows rules for caches, logs, crash reports, developer data, and Trash.
- **Uninstall apps / 应用卸载** — One plan for the app and its preferences, caches, containers, or AppData leftovers.
- **Deep Clean / 深度清理** — Rebuildable build output, disk images, installers, and stale downloads with project-age protection.
- **Storage & monitor / 存储与监控** — Folder-level disk analysis, live CPU, memory, network, disk, battery, and process views.
- **History & updates / 历史与更新** — Restorable Trash sessions and a size- and SHA256-verified update channel.

The interface follows the system language by default and can be switched to Simplified Chinese or English in Settings.

界面默认跟随系统语言，可在设置中切换简体中文或 English。

## Start / 开始

```bash
pnpm install
pnpm desktop:dev
```

CLI / TUI:

```bash
cargo install --path crates/squid-cli
squid state
squid clean --dry-run
```

## Documentation / 文档

- [中文完整说明](README.zh-CN.md)
- [Full English guide](README.en.md)
- [Architecture / 架构](docs/ARCHITECTURE.md)
- [Nex design standard / 设计规范](docs/DESIGN.md)
- [Desktop / 桌面端](docs/DESKTOP.md)
- [Release and updates / 发布与更新](docs/RELEASE.md)

## License / 许可证

**GNU General Public License v3.0 only** (`GPL-3.0-only`). See [`LICENSE`](LICENSE).
