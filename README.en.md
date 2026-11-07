# Squid

[简体中文](README.zh-CN.md) · [Bilingual home](README.md)

**Give your computer a bath.**

Squid is a local system-care app for macOS and Windows. It cleans rebuildable caches and system junk, uninstalls apps with their leftovers, removes project build artefacts, maps disk usage, shows live system health, and scans for malware with reversible quarantine.

The desktop app, CLI, and TUI share one Rust scan-and-delete engine. Cleaning rules, safety boundaries, the deletion ledger, and settings live in one place. Squid requires no cloud service and no root access.

## Product surfaces

| Surface | Stack | Binary |
| --- | --- | --- |
| Desktop app | Tauri 2 + React 19 + TypeScript + Vite 6 | `squid-desktop` |
| CLI | Rust + clap | `squid` |
| TUI | Rust + ratatui + crossterm | `squid state` |

The desktop app supports macOS 12+ and Windows 10/11 x64. The CLI and TUI share the same Rust workspace.

## Features

| Feature | Description |
| --- | --- |
| Computer overview | Disk health, common actions, and CPU, memory, and storage summaries |
| Daily cleanup | 38 built-in rules on macOS and 30 on Windows, covering app and browser caches, logs, crash reports, developer caches, Trash, and more |
| Uninstall apps | Finds an app by name and builds one plan containing the app bundle plus preferences, caches, containers, or AppData leftovers |
| Deep Clean | Finds rebuildable build output such as `node_modules`, disk images, installers, and stale downloads while protecting recently used projects |
| Storage | Measures folders level by level, with the largest children and files |
| Real-time monitor | Samples CPU, memory, network, disk, battery, volumes, and high-usage processes every two seconds. The Processes tab lists every process with its signature and startup entry, and ends them after confirmation |
| Security | Reports XProtect or Defender status, scans for malware by hash, YARA rule and heuristic, grades findings confirmed / suspicious / advisory, and quarantines them reversibly |
| Startup items | LaunchAgents, LaunchDaemons, login items, registry Run keys, startup folders, scheduled tasks and services; the ones the current user owns can be disabled and re-enabled |
| History and undo | Every real deletion is recorded in a local ledger; items still in Trash or the Recycle Bin can be restored |
| System updates | Checks, verifies, downloads, and installs new versions from the release channel |

Language settings offer only Simplified Chinese and English, with the current system language selected by default. A manual choice is saved as the interface language for this computer.

## Desktop app

### Sidebar

- **Maintenance**: Computer overview, Daily cleanup, Uninstall apps, Deep Clean
- **Protection**: Security, Startup items
- **Insights**: Storage, Real-time monitor
- **Bottom**: History, Settings

Use `⌘1`–`⌘0` on macOS or `Ctrl+1`–`Ctrl+0` on Windows to switch pages, in the
sidebar order above. Since 0.1.6, `7` and `8` open Security and Startup items;
History and Settings moved to `9` and `0`.

### Menu bar / tray

Closing the main window only hides it; Squid keeps running. The tray menu offers:

- Open Squid
- Protection status (a read-only status line)
- Scan for malware
- Manage startup items
- Daily cleanup
- Uninstall apps
- Deep Clean
- Quit Squid

The tray follows the active interface language. Reopening the app, double-clicking the app icon, or triggering the single-instance handler restores and focuses the main window. Only **Quit Squid** ends the process.

### Software updates

- The system information panel always shows the current OS version, architecture, and Squid version.
- Version changes, release notes, and actions appear only when a newer version exists; there is no duplicate "up to date" message.
- After a verified download, install and restart, or use the neutral outlined **Discard** button that matches Shift to remove the downloaded file.
- Automatic checks are enabled by default and run at most once per day.
- Installers must match the manifest's size and SHA256. Builds with a release public key also verify the manifest's Ed25519 signature.

### Permissions and privacy

- On first launch, Squid shows a security-readiness card once: system protection, scan
  access, notifications, launch at login and rule updates. Every item is optional, no
  permission is requested that you did not ask for, and closing the card changes nothing.
- Full Disk Access stays optional. Without it, protected folders are skipped and the
  Security page reports the coverage as limited rather than clean.
- **Settings → System permissions** provides a status check and a shortcut to **System Settings → Privacy & Security → Full Disk Access**.
- Without Full Disk Access, TCC-protected locations are skipped and every other feature remains available.
- Storage analysis and Deep Clean read only user-selected or configured folders; file contents are never uploaded.
- The app makes no network requests other than update checks, installer downloads and
  rule-pack downloads. **Nothing is ever uploaded**: no file name, hash, finding or
  quarantined file leaves the machine.
- Security scanning does not install a daemon, kernel extension or privileged helper.
  XProtect and Defender stay the real-time protection; Squid reads their state, never
  turns them off, and never calls `xprotect update` on your behalf.

### Launch at login

**Settings → Startup** includes a launch-at-login switch. When enabled, Squid starts automatically after the current user signs in and keeps its tray entry available. The setting is user-level and requires no administrator access.

See [`docs/DESKTOP.md`](docs/DESKTOP.md) for details.

## Quick start

### Desktop development

Requirements:

- Node.js 20+ and pnpm
- Stable Rust
- macOS: Xcode Command Line Tools
- Windows: MSVC Build Tools and WebView2 Runtime

```bash
pnpm install
pnpm desktop:dev
```

A plain Vite page cannot call Tauri commands. Use `pnpm desktop:dev` when testing scans, deletion, system data, or updates.

### CLI / TUI

```bash
cargo install --path crates/squid-cli
# or
cargo build --release -p squid-cli

squid state                       # TUI
squid status                      # system snapshot
squid clean --dry-run             # preview cleanable items
squid uninstall --list            # installed apps
squid purge ~/code                # project build artefacts
squid analyze ~/Library/Caches    # storage analysis
squid history                     # deletion ledger
squid undo                        # restore the latest Trash session
```

Common commands:

| Command | Purpose |
| --- | --- |
| `clean` | Clean caches, logs, browser and developer junk, crash reports, and Trash by rule |
| `uninstall [NAME]` | Locate an app and build an app-plus-leftovers uninstall plan |
| `purge [PATHS…]` | Remove project build folders, disk images, installers, and stale downloads |
| `state` | Open the TUI with cleanup, uninstall, deep clean, storage, system, and history panels |
| `status` | Print a CPU, memory, network, disk, battery, and process snapshot |
| `analyze [PATH]` | Print a directory-size tree and the largest files |
| `history` / `undo [ID]` | Review history or restore a Trash session |
| `whitelist add\|list\|remove` | Maintain protected paths that are never selected |
| `rules` | List active rules |
| `doctor` | Check permissions, configuration, history, and effective paths |
| `guard status` | Report XProtect or Defender state, rule-pack version, startup counts and quarantine size |
| `guard scan [PATHS…]` | Scan for malware. No paths uses the quick range; `--full`, `--json`, `-y` to auto-quarantine confirmed |
| `guard startup` | List login/startup entries; `--all` includes the operating system's own, `--risky` filters |
| `guard quarantine <ID>` / `restore <ID>` | Act on a finding id a scan printed, or put a quarantined file back |
| `guard held` | List what the quarantine holds, and why |

Every delete command supports `--dry-run`; non-interactive environments require `-y/--yes`. Run `squid <command> --help` for complete options.

## Architecture

```text
crates/
├── squid-core       models, paths, safety policy, settings, deletion ledger, progress, cancellation
├── squid-platform   macOS / Linux / Windows adapters: apps, leftovers, volumes, Trash, processes
├── squid-rules      TOML rule engine: expansion, anchors, protection, age, and risk flags
├── squid-stats      sysinfo sampling, trends, battery, network, and disk activity
├── squid-guard      malware scanning, YARA and hash rules, startup trust, reversible quarantine
├── squid-engine     clean / uninstall / purge / analyze and the single deletion path
├── squid-tui        ratatui terminal interface
└── squid-cli        clap command-line entry point

src/                 React desktop interface
src-tauri/           Tauri shell, commands, tray, permission checks, and updater
scripts/             development, verification, and dual-platform release scripts
docs/                architecture, design, desktop, and release specifications
```

`src-tauri` is separate from the root Cargo workspace, but depends directly on `squid-core`, `squid-platform`, `squid-guard`, `squid-stats`, and `squid-engine`. The desktop app does not maintain a second cleanup implementation.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for architecture details and [`docs/DESIGN.md`](docs/DESIGN.md) for the shared Nex color, typography, component, and language rules.

## Safety model

1. **Index-based deletion**: the desktop UI can only submit indexes from a scan, never arbitrary paths.
2. **Revalidation before execution**: the Rust layer rebuilds each item and rechecks ownership, safety anchors, protected locations, the whitelist, and lock state.
3. **Trash by default**: deletion mode defaults to `trash`; files are removed directly only when permanent deletion is explicitly selected.
4. **Hard boundaries**: system roots, `/usr`, `/System`, `/Applications`, and similar locations cannot be recursively deleted. Symlinks are never followed.
5. **Rule brakes**: rules support `protect`, `min_age_days`, `skip_if_running`, and `risky`.
6. **Ledger and undo**: real deletions are recorded in a local JSON ledger; files still in Trash can be restored.
7. **Process protection**: the process is sampled again before termination; Squid itself, PID 1, and critical system processes are refused.

The GUI and CLI use the same `squid-engine::delete` path, so behavior, locks, and audit results stay consistent.

## Data and configuration

| Purpose | macOS | Windows |
| --- | --- | --- |
| Configuration and rules | `~/Library/Application Support/squid/` | `%APPDATA%\squid\` |
| History ledger | `~/Library/Application Support/squid/history/` | `%APPDATA%\squid\history\` |
| Delete destination | `~/.Trash` or permanent deletion | Recycle Bin or permanent deletion |

Linux uses XDG directories. The desktop app and CLI/TUI share configuration and history, so a cleanup from either surface is visible in the other.

Common configuration:

```toml
delete_mode = "trash"
history_keep = 60
purge_min_age_days = 7
purge_max_entries = 200000
show_allocated = true
# Empty or omitted means "follow system"; set "en" or "zh" to override
language = ""
whitelist = ["~/Library/Caches/ImportantApp"]
disabled_rules = ["user-trash"]
purge_roots = ["~/code", "~/Projects", "~/Desktop"]
```

Main environment variables:

| Variable | Purpose |
| --- | --- |
| `SQUID_CONFIG_DIR` | Override the configuration directory |
| `SQUID_STATE_DIR` | Override the state and history directory |
| `SQUID_LANG` | Force the CLI/TUI language to `zh` or `en` |
| `SQUID_UPDATE_URL` | Override the desktop update manifest URL for staging |
| `SQUID_TRUECOLOR` / `NO_COLOR` | Control TUI color |
| `SQUID_SYMBOL_CELLS` / `SQUID_CONSOLE_TRACE` | Diagnose Windows console character width and layout |

`SQUID_LANG` is a CLI override. The desktop app's default language comes from the system and can be changed in the app.

## Development and verification

Frontend:

```bash
pnpm check
pnpm build
```

Rust workspace:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
```

Tauri:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

The TUI uses a real-pty smoke test; Windows consoles also have a capture, replay, and frame-check test:

```bash
python3 -m pip install --user pyte
python3 scripts/tui_smoke.py
python3 scripts/tui-frame-check.py --selftest
```

CI runs workspace tests, an MSVC target cross-check, the desktop frontend build, and the Tauri Rust check.

## Build and release

Keep the version identical in all four places:

- `Cargo.toml` workspace version
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`
- `package.json`

Build installers:

```bash
scripts/build-macos-release.sh
scripts/build-windows-release.sh
```

Artifacts:

```text
artifacts/Squid-<version>-macOS-universal.dmg
artifacts/Squid-Setup-<version>-win-x64.exe
artifacts/SHA256SUMS.txt
```

Publish to the update channel:

```bash
scripts/publish-release.sh --notes /path/to/release-notes.md
```

The publish script rebuilds the `releases` branch with `squid-update.json`, both installers, and an optional `squid-update.sig`. The manifest records exact size and SHA256, which the client verifies again after download.

See [`docs/RELEASE.md`](docs/RELEASE.md) for the full release, signing, and manifest process.

## Platform status

| Platform | Status |
| --- | --- |
| macOS | Full desktop and CLI/TUI support; universal DMG |
| Windows x64 | Desktop and CLI/TUI support; NSIS installer |
| Linux | Platform layer and basic CLI/TUI support; built-in rules and desktop installers are not yet provided |

## License

Squid is licensed under **GNU General Public License v3.0 only** (`GPL-3.0-only`). See [`LICENSE`](LICENSE).
