# Squid desktop

Squid has two native desktop builds, macOS and Windows, built with the same
technology stack as `nex_shift`:

- Tauri 2 shell and updater handoff
- React 19 + TypeScript UI
- Vite 6 development and asset pipeline
- The existing `squid-core`, `squid-engine`, `squid-platform` and `squid-stats`
  Rust workspace

The desktop application is a second frontend over the same engine used by the
CLI and TUI. It does not maintain a separate cleanup implementation.

The shared Nex colour and component standard lives in
[`DESIGN.md`](DESIGN.md). Squid uses the same primary palette as `nex_shift`.

## Run locally

Prerequisites:

- Node.js 20+ and pnpm
- Rust toolchain and Cargo
- macOS: Xcode Command Line Tools
- Windows: MSVC build tools and WebView2

```bash
pnpm install
pnpm desktop:dev
```

The Vite-only development page cannot call Tauri commands. Use `desktop:dev`
when exercising scans, deletes, storage analysis, system data, or updates.

Useful checks:

```bash
pnpm check
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

## Screens

| Screen | What it does |
| --- | --- |
| Computer overview | Security status strip, cleanup scan entry, disk health, cleanup result categories and live CPU/memory/storage summaries |
| Daily cleanup | Runs the normal cleanup rule set, groups results by category, supports per-item and per-category selection, Trash or permanent deletion, then offers undo through History |
| Uninstall apps | Reads installed apps, builds an app-specific removal plan, shows leftover paths and warnings, then uninstalls selected items |
| Deep Clean | Searches chosen project roots for regenerable build artefacts with Git and file-age protections |
| Security | XProtect / Defender status, scan coverage, findings by confidence, quarantine and restore, recent security activity |
| Startup items | LaunchAgents, LaunchDaemons, login items, registry Run keys, startup folders, scheduled tasks and services; disable and re-enable the ones the current user owns |
| Storage | Measures a chosen folder, exposes the largest children, drills down, and finds the largest files |
| Real-time monitor | Two tabs. **Performance**: live CPU, memory, network, disk, battery, volume and process data. **Processes**: the whole process table with signature and startup-item columns, and process termination after confirmation |
| History | Lists cleanup sessions, shows the ledger, restores files that are still in Trash/Recycle Bin, and opens the bin |
| Settings | Chinese/English language selection that defaults to the system language, launch at login, the security section (scan schedule, notifications, auto-quarantine, rule-pack updates, scan exclusions), deletion mode, history retention, protected paths, local file locations, and software updates |

`⌘1`–`⌘0` on macOS and `Ctrl+1`–`Ctrl+0` on Windows switch screens, in sidebar
order: `1` overview, `2` daily cleanup, `3` uninstall apps, `4` deep clean,
`5` storage, `6` real-time monitor, `7` security, `8` startup items,
`9` history, `0` settings.

In 0.1.6 `⌘7`/`⌘8` moved from History/Settings to Security/Startup items;
History and Settings are now `⌘9` and `⌘0`.

The sidebar groups these into **Maintenance** (overview, daily cleanup,
uninstall apps, deep clean), **Protection** (security, startup items) and
**Insights** (storage, real-time monitor), with History and Settings pinned at
the bottom.

The two new screens reuse the interface vocabulary the older screens already
had rather than introducing a second one: `.panel` for a card, `.status-chip` for
a status, `.search-field`, `.icon-button`, `.text-button`, `.metric-icon` with a
`tone-*` class, `.settings-row` for a switch, `.empty-inline` for an empty list.
Findings, startup entries and quarantined files share one row shape
(`.item-row` / `.item-main` / `.item-copy` / `.item-actions`), so a change to how
a row looks is one edit that reaches all three. Copy stays to one sentence per
screen and per control: a count sits on the filter that produces it, not also in
a summary line above it.

## Security page

Squid scans when asked. There is no daemon, no kernel extension and no
privileged helper, so XProtect and Defender remain the real-time protection and
this page only reads what they report.

- **Scan modes.** Quick covers application bundles, login-item targets, browser
  extensions and the folders a browser writes to. Full adds the user's Library
  and document folders and must be started deliberately. Custom scans the paths
  chosen. All three are cancellable.
- **Confidence.** `confirmed` means a published hash or signature rule matched
  and is the only level that may ever be moved automatically. `suspicious` needs
  two corroborating signals and is report-only. `advisory` is a single weak
  signal, such as an unsigned file that does not start at login.
- **Coverage.** A scan that could not read a folder says so. The page never
  renders "clean" for a range it did not look at, and unreadable folders are
  listed rather than omitted.
- **Nothing is uploaded.** No file name, hash, finding or quarantined file
  leaves the machine. Rule packs are downloaded; nothing is sent back.
- **Quarantine is reversible.** Findings are moved by backend-issued id, never
  by a path the interface supplies. The file is re-verified against the hash the
  scan recorded, stored without the execute bit, and restoring returns the
  original bytes, mode and timestamps.

## Tray

Squid keeps running after its main window is closed. The tray menu can reopen
the window, start a malware scan, and jump directly to Security, Startup items,
Daily cleanup, Uninstall apps or Deep Clean. One disabled row states the current
protection status — the count of unhandled confirmed findings, "not scanned yet"
before the first scan, and "scanning…" while one is running.
The tray follows the active interface language. Use **Quit Squid** in the same
menu to stop the background process.

**Settings → Startup** registers the app for the current user only. macOS
uses a user LaunchAgent, Windows uses the current-user Run key, and Linux uses
an XDG autostart desktop entry. Enabling or disabling launch at login does not
require administrator access.

## Safety boundary

The UI never submits a path for deletion. A scan returns an array of items and a
process-local scan ID. Delete commands accept only indices into that cached scan
or removal plan. The Rust command layer then:

1. Rejects out-of-range indices.
2. Forces locked items to remain unselected.
3. Reconstructs the same `CleanupItem` set the scanner produced.
4. Calls `squid-engine::delete`, which re-validates ownership, safety anchors,
   protected locations and the user whitelist before removing anything.

Locked items remain visible but cannot be selected. A permanent delete always
requires the confirm sheet; the default mode comes from the same `Settings`
file used by the CLI and TUI.

## Storage permissions on macOS

Squid measures file metadata only. The normal cleanup scan does not cover
`~/Downloads`, and disk capacity is read with `statvfs` rather than through
CoreFoundation's CacheDelete-aware volume API, so neither path asks for access
to downloads or the media library.

Full Disk Access is optional. On first launch Squid shows one setup card that
reports whether the app can read TCC-protected locations such as Safari, Mail and
Messages data. It does not force a permission prompt and does not block the app:
without access those locations are skipped. The same status and a link to
**System Settings › Privacy & Security › Full Disk Access** remain in
**Settings › System permissions**. The check runs again when the window regains
focus, and the card can be reopened by setting `desktop_setup_reviewed = false`
in `config.toml`.

Folder-dependent tools remain explicit. Storage analysis starts only after you
choose a folder. macOS may still show a folder permission prompt when a folder
you selected contains protected locations. Deny any access you do not want
Squid to have; the rest of the application still works. Deep Clean's default
project roots can include Desktop and Documents, so edit that list before
scanning if you do not want those prompts.

## Updates

The update implementation mirrors `nex_shift`:

- A build derives its feed from the git `origin` remote or `SQUID_UPDATE_URL`.
- The feed is an orphan `releases` branch containing installers and
  `squid-update.json`.
- The manifest always provides exact byte size and SHA256; both are checked
  before an installer can run.
- A build made with `SQUID_UPDATE_PUBKEY` additionally requires and verifies
  `squid-update.sig`, a detached Ed25519 signature over the manifest's exact
  bytes.
- The application checks at most once a day when automatic checks are enabled;
  the last-check day lives in `updates.json`.
- A verified download can be installed and restarted, or discarded with the
  neutral outlined action shared with `nex_shift`.
- macOS mounts the DMG after exit and swaps `Squid.app` with rollback on failure.
- Windows waits for `squid-desktop.exe` to exit, runs the same NSIS installer
  silently, then relaunches it from `%LOCALAPPDATA%\Programs\Squid`.

See [`RELEASE.md`](RELEASE.md) for the publishing commands, artifact names, and
signing setup.

## Native CLI remains separate

The desktop binary is `squid-desktop`. The existing `squid` binary continues to
contain the CLI/TUI and is not replaced by the desktop package. Both use the same
configuration and history directories, so a cleanup made in either entry point
appears in the other one's history.
