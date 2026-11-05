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
| Computer overview | Smart scan entry, disk health, cleanup result categories and live CPU/memory/storage summaries |
| Daily cleanup | Runs the normal cleanup rule set, groups results by category, supports per-item and per-category selection, Trash or permanent deletion, then offers undo through History |
| Uninstall apps | Reads installed apps, builds an app-specific removal plan, shows leftover paths and warnings, then uninstalls selected items |
| Deep Clean | Searches chosen project roots for regenerable build artefacts with Git and file-age protections |
| Storage | Measures a chosen folder, exposes the largest children, drills down, and finds the largest files |
| System | Live CPU, memory, network, disk, battery, volume and process data; process termination asks for confirmation |
| History | Lists cleanup sessions, shows the ledger, restores files that are still in Trash/Recycle Bin, and opens the bin |
| Settings | Language, deletion mode, history retention, protected paths, local file locations, and software updates |

`⌘1`–`⌘8` on macOS and `Ctrl+1`–`Ctrl+8` on Windows switch screens.

## Tray

Squid keeps running after its main window is closed. The tray menu can reopen
the window and jump directly to Daily cleanup, Uninstall apps or Deep Clean.
Use **Quit Squid** in the same menu to stop the background process.

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
