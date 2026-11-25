# Squid Usage Guide

For Squid 0.1.14. See [`README.en.md`](../README.en.md) for the project overview and [`CHANGELOG.md`](../CHANGELOG.md) for version history.

## 1. Download and install

Desktop installers are published on the origin repository's `releases` branch and attached to the GitHub release. The GitHub repository does not carry that branch.

- Release directory: <https://git.unissense.tech/mula/nex_squid/src/branch/releases>
- Update manifest: <https://git.unissense.tech/mula/nex_squid/raw/branch/releases/squid-update.json>

Current version 0.1.14:

| Platform | Installer |
| --- | --- |
| macOS 12+ | [`Squid-0.1.14-macOS-universal.dmg`](https://git.unissense.tech/mula/nex_squid/raw/branch/releases/Squid-0.1.14-macOS-universal.dmg) |
| Windows 10/11 x64 | [`Squid-Setup-0.1.14-win-x64.exe`](https://git.unissense.tech/mula/nex_squid/raw/branch/releases/Squid-Setup-0.1.14-win-x64.exe) |

Both installers are unsigned builds. Verify their exact size and SHA256 against `SHA256SUMS.txt` in the release directory.

### macOS

1. Open the DMG and drag `Squid.app` into Applications.
2. On first launch, right-click `Squid` in Finder and choose **Open**.
3. If macOS reports that the app is damaged, remove the download quarantine flag and open it again:

   ```bash
   xattr -dr com.apple.quarantine /Applications/Squid.app
   open /Applications/Squid.app
   ```

4. Squid needs no root access and installs no background service, kernel extension, or privileged helper.

### Windows

1. Run `Squid-Setup-<version>-win-x64.exe`.
2. If SmartScreen appears, choose **More info** > **Run anyway**.
3. The default install location is `%LOCALAPPDATA%\Programs\Squid`; no administrator rights are required.
4. If the installer cannot write to `%TEMP%` (some 360 / PC Manager environments), use `一键安装.bat` from the release directory. It copies the installer to `%TEMP%` on drive C: and starts it there.
5. If Squid opens a blank window, install the Microsoft Edge WebView2 Evergreen Runtime and relaunch:
   <https://developer.microsoft.com/microsoft-edge/webview2/>

## 2. First launch

- The interface follows the system language by default. Switch between Simplified Chinese and English in **Settings → General**.
- The first launch shows an optional permission-readiness card once. It does not force a system permission prompt and does not block startup.
- Closing the main window only hides it; Squid keeps running in the tray. Only **Quit Squid** in the tray menu ends the process.
- **Settings → Startup** enables launch at login. It writes a user-level startup item and needs no administrator access.

## 3. Desktop app

### Sidebar

- **Maintenance**: Overview, Daily cleanup, Uninstall apps, Deep Clean, Permissions (Windows only)
- **Protection**: Security, Startup items
- **Insights**: Storage, Real-time monitor
- **Bottom**: History, Settings

Use `⌘1`–`⌘8` on macOS or `Ctrl+1`–`Ctrl+9` on Windows to switch pages in sidebar order. macOS hides the Windows-only Permissions screen, so later pages shift up.

### Daily cleanup

1. Open **Daily cleanup** and choose a scan scope.
2. Review results by category; each category can be unchecked independently.
3. Choose the deletion mode: Trash / Recycle Bin by default, or explicit permanent deletion.
4. Start cleanup and confirm the dialog.

Rules cover app and browser caches, logs, crash reports, developer caches, upgrade installers, and Trash / Recycle Bin. macOS ships 38 built-in rules; Windows ships 30. Every CLI delete command supports `--dry-run`; the desktop app scans before you select anything.

### Uninstall apps

1. Open **Uninstall apps** and search for an app.
2. Squid builds one plan containing the app bundle plus preferences, caches, containers, or AppData leftovers.
3. Review every item, then confirm.

Leftover matching excludes common document extensions so user documents are not treated as app leftovers.

### Deep Clean

Deep Clean finds rebuildable build output such as `node_modules`, disk images, installers, and stale downloads. Edit `purge_roots` to change the default project roots; `purge_min_age_days` protects projects that are still in use.

### Storage

Choose a folder and Squid measures it level by level, listing the largest children and files. Storage analysis reads only folders you choose or configure; file contents are never uploaded.

### Real-time monitor and processes

- Samples CPU, memory, network, disk, battery, volumes, and high-usage processes every two seconds.
- The **Processes** tab lists every process with its signature status and startup entry.
- Before ending a process, Squid samples it again and checks the name. Squid itself, PID 1, and critical system processes are refused.
- The monitor can be pinned as a desktop widget; all widget settings open from the gear at its top.

### Security

- Reads XProtect / Defender state; it never turns protection off or changes it for you.
- Scans by hash, YARA rule, and heuristic, grading findings as confirmed / suspicious / advisory.
- Confirmed findings can be quarantined and restored.
- The Security page can trust normal software by publisher, app folder, or advisory class. Confirmed threats are never silently allowlisted.
- File names, hashes, findings, and quarantined files are never uploaded.

### Startup items and Permissions

- **Startup items** lists LaunchAgents, LaunchDaemons, login items, registry Run/RunOnce keys, startup folders, scheduled tasks, and services. Entries owned by the current user can be disabled and re-enabled.
- Windows **Permissions** groups startup entries by app and adds shell context-menu extensions, popup-window rules, and a background watcher.

### History and undo

Every real deletion is recorded in a local ledger. Items still in Trash / Recycle Bin can be restored from **History**. On the CLI, use `squid history` and `squid undo`.

### Software updates

- Automatic checks run at most once a day; check manually in **Settings → Updates**.
- The manifest comes from the origin `releases` branch, not from GitHub.
- Installers must match the manifest's exact size and SHA256. Builds with a release public key also verify the manifest's Ed25519 signature.
- After a verified download, install and restart, or discard the downloaded file.

## 4. CLI and TUI

Install:

```bash
cargo install --path crates/squid-cli --locked
```

Or build from source:

```bash
cargo build --release -p squid-cli
```

Common commands:

| Command | Purpose |
| --- | --- |
| `squid state` | Open the TUI |
| `squid status` | Print a system snapshot |
| `squid clean --dry-run` | Preview cleanable items |
| `squid clean -y` | Run cleanup by rule |
| `squid uninstall --list` | List installed apps |
| `squid uninstall <NAME>` | Build an app-plus-leftovers uninstall plan |
| `squid purge <PATH>` | Remove project build artefacts |
| `squid analyze <PATH>` | Print a directory-size tree and largest files |
| `squid history` | Review the deletion ledger |
| `squid undo [ID]` | Restore the latest or a named Trash session |
| `squid whitelist add\|list\|remove` | Maintain protected paths |
| `squid rules` | List active rules |
| `squid doctor` | Check permissions, configuration, history, and effective paths |
| `squid guard status` | Report XProtect / Defender state, rule-pack version, startup counts, and quarantine size |
| `squid guard scan [PATHS…]` | Scan for malware; `--full`, `--json`, `-y` to auto-quarantine confirmed findings |
| `squid guard startup` | List login/startup entries; `--all`, `--risky` |
| `squid guard quarantine <ID>` / `restore <ID>` | Quarantine or restore a finding |
| `squid guard held` | List quarantine contents and reasons |

Every delete command supports `--dry-run`. Non-interactive environments must pass `-y/--yes` or Squid refuses the deletion. Run `squid <command> --help` for complete options.

## 5. Configuration and data

| Purpose | macOS | Windows |
| --- | --- | --- |
| Configuration and rules | `~/Library/Application Support/squid/` | `%APPDATA%\squid\` |
| History ledger | `~/Library/Application Support/squid/history/` | `%APPDATA%\squid\history\` |
| Delete destination | `~/.Trash` or permanent deletion | Recycle Bin or permanent deletion |

Linux uses XDG directories. The desktop app and CLI/TUI share configuration and history.

Common configuration (`config.toml`):

```toml
delete_mode = "trash"
history_keep = 60
purge_min_age_days = 7
purge_max_entries = 200000
show_allocated = true
# Empty means "follow system"; set "en" or "zh" to override
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

## 6. Safety model

1. The desktop UI can only submit indexes from a scan, never arbitrary paths.
2. Before execution, Squid rebuilds each item and rechecks ownership, safety anchors, protected locations, the whitelist, and lock state.
3. Deletion goes to Trash / Recycle Bin by default; permanent deletion must be explicit.
4. System roots, `/usr`, `/System`, `/Applications`, and similar locations cannot be recursively deleted. Symlinks are never followed.
5. Every real deletion is recorded in a local ledger; items still in Trash can be restored.
6. Before ending a process, Squid samples it again and refuses critical system processes.

## 7. Uninstall

### macOS

1. Quit Squid.
2. Move `/Applications/Squid.app` to the Trash.
3. To remove configuration and history as well:

   ```bash
   rm -rf "$HOME/Library/Application Support/squid"
   ```

### Windows

1. Uninstall Squid from **Settings → Apps**.
2. To remove configuration and history as well, delete `%APPDATA%\squid`.

### CLI

```bash
cargo uninstall squid-cli
```

## 8. Troubleshooting

### macOS says the app is damaged

This is Gatekeeper quarantine on an unsigned build, not a damaged file. If right-click **Open** is not enough:

```bash
xattr -dr com.apple.quarantine /Applications/Squid.app
```

### Windows opens a blank window

Install the WebView2 Evergreen Runtime and relaunch Squid.

### Windows installer cannot write temporary files

When the installer runs from a non-system drive or a folder restricted by endpoint protection, NSIS may fail to unpack into `%TEMP%`. Copy it to `%TEMP%` on drive C: and run it there, or use `一键安装.bat` from the release directory.

### Cleanup finds less than expected

Check Full Disk Access on macOS, plus `whitelist`, `disabled_rules`, and the Security page's trusted list. Without Full Disk Access, TCC-protected locations are skipped and the Security page reports coverage as limited.

### Update check fails

Confirm the origin release channel is reachable:

<https://git.unissense.tech/mula/nex_squid/raw/branch/releases/squid-update.json>

In a proxy or offline environment, set `SQUID_UPDATE_URL` to a reachable mirror of the manifest.

### TUI layout is wrong

Try `SQUID_TRUECOLOR=0` or `NO_COLOR=1`. On Windows consoles, use `SQUID_SYMBOL_CELLS` / `SQUID_CONSOLE_TRACE` to diagnose character width and layout.

## 9. More documentation

- [Project overview](../README.en.md)
- [Changelog](../CHANGELOG.md)
- [Architecture](ARCHITECTURE.md)
- [Desktop app](DESKTOP.md)
- [Release and updates](RELEASE.md)
- [Security](SECURITY.md)
