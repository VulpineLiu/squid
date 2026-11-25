# Desktop release and update channel

## 代码仓上传规则（用户指令「上传代码仓」即指本节）

- 地址：`https://git.unissense.tech/mula/nex_squid.git`（origin 已内置账号，
  凭据 lijunjie，勿写入其他文件）。
- 分支：**dev**。提交信息用中文，经 `git commit --file -`（或 `-F 文件`）传入，
  避免 pwsh 引号问题；发布二进制需 `git add -f`（release 目录不在 .gitignore
  白名单内）。
- 打包版上传时同步刷新 `release/<ver>/`（安装包/便携包/SHA256SUMS/
  squid-update.json/RELEASE_NOTES），并在 `release/CHANGELOG-SINCE-<上次上传版本>.md`
  补每个版本的更新说明，注明本版学习/借鉴的 GitHub 开源项目及链接。

Squid uses the same release model as `nex_shift`: a git branch is the update
server. No object storage or release API is required.

## Artifact names

Both installers must carry the exact version from `src-tauri/tauri.conf.json`:

```text
artifacts/Squid-<version>-macOS-universal.dmg
artifacts/Squid-Setup-<version>-win-x64.exe
```

Additional readmes and checksum files may travel with them, but
`scripts/publish-release.sh` rejects any other installer-like file. This prevents
an old DMG or stale EXE from silently entering a new release.

## macOS

```bash
scripts/build-macos-release.sh
```

The script installs both Apple Rust targets, builds a universal app and DMG,
verifies the code signature and image, then copies the result to the versioned
artifact name. Use `--skip-build` to re-verify and repackage an existing build.

For distribution outside local testing, replace the ad-hoc identity in
`src-tauri/tauri.conf.json` with a Developer ID identity and notarize the app.
The CI configuration does not hold Apple credentials.

## Windows

On macOS:

```bash
pnpm add -g cargo-xwin
scripts/build-windows-release.sh
```

The script builds the React bundle for the WebView2 target, cross-compiles
`x86_64-pc-windows-msvc`, checks the PE machine and embedded frontend, then uses
NSIS to produce `Squid-Setup-<version>-win-x64.exe`.

On Windows, run the same script from Git Bash or MSYS2. It uses the native MSVC
toolchain instead of `cargo-xwin`. The installer is per-user and installs to:

```text
%LOCALAPPDATA%\Programs\Squid
```

The executable inside is `squid-desktop.exe`, matching both the NSIS installer
and the update handoff script.

### 360/电脑管家 environments: ship 一键安装.bat (mandatory)

On the developer machine (and other boxes running 腾讯电脑管家/360), the
endpoint protector blocks **unsigned installers started from a non-system
drive** from unpacking into `%TEMP%`: double-clicking
`release/<ver>/Squid-Setup-*.exe` on D: dies with
`NSIS Error - Error writing temporary file` — or worse, a silent `/S` run
exits 0 without installing anything. The same installer run from `C:\Users\<u>\
AppData\Local\Temp` works, verified live.

Therefore every release directory must contain `一键安装.bat` next to the
setup exe: it copies the installer into `%TEMP%` (C:) and launches it from
there. The README's 提示 section documents this as remedy #1. When assembling
a new release, create the batch (ASCII encoding, version-pinned filename) and
test it once — the wizard window must appear with no NSIS dialog.


### Over-the-top install (custom NSIS template)

`bundle.windows.nsis.template` points at `src-tauri/nsis/installer.nsi`, a copy
of the `@tauri-apps/cli` default template with one change: the "detected an
older version — uninstall or reinstall?" page calls `Abort` unless a legacy WiX
install is found, so upgrades overwrite the previous install in place (inheriting
`$INSTDIR` from the registry) instead of prompting. User settings live under
`%APPDATA%\squid` and are never touched by install or uninstall, so they carry
across upgrades automatically.

Because it is a full template copy, re-sync it whenever `@tauri-apps/cli` (and
thus the bundled `installer.nsi`) is upgraded: re-copy the new template and
re-apply the `Abort` block in `Function PageReinstall`.

## Signing key

Create the Ed25519 key once, outside the repository:

```bash
python3 -m pip install cryptography
python3 scripts/release-key.py keygen ~/.config/squid-release/release.ed25519
```

Build with the public key so the binary refuses a manifest that is missing or
incorrectly signed:

```bash
export SQUID_RELEASE_KEY=~/.config/squid-release/release.ed25519
scripts/build-macos-release.sh
scripts/build-windows-release.sh
```

Both build scripts derive `SQUID_UPDATE_PUBKEY` from
`${SQUID_RELEASE_KEY%.ed25519}.pub`. Keep the private key out of git.

## Publish

Build both installers first. The publish script rebuilds the `releases` branch
as one fresh root commit, so old installers never accumulate in history:

```bash
scripts/publish-release.sh \
  --notes RELEASE_NOTES.md \
  --key ~/.config/squid-release/release.ed25519
```

Useful checks:

```bash
scripts/publish-release.sh --dry-run --key ~/.config/squid-release/release.ed25519
scripts/publish-release.sh --no-push --key ~/.config/squid-release/release.ed25519
```

The branch contains:

```text
squid-update.json
squid-update.sig
Squid-<version>-macOS-universal.dmg
Squid-Setup-<version>-win-x64.exe
```

## Feed URL

`src-tauri/build.rs` derives the manifest URL from `origin`:

| Remote | Feed |
| --- | --- |
| Gitea / Forgejo | `https://host/owner/repo/raw/branch/releases/squid-update.json` |
| GitHub | `https://github.com/owner/repo/raw/releases/squid-update.json` |
| GitLab | `https://gitlab.example/owner/repo/-/raw/releases/squid-update.json` |

`SQUID_UPDATE_URL` overrides it at build time. The same variable at runtime
overrides the baked value, which is useful for staging feeds:

```bash
SQUID_UPDATE_URL=https://host/owner/repo/raw/branch/staging/squid-update.json \
  open Squid.app
```

## Manifest

Schema version 1:

```json
{
  "schema": 1,
  "version": "0.2.0",
  "date": "2026-11-03",
  "notes": "Release notes shown in Settings.",
  "artifacts": {
    "macos": {
      "file": "Squid-0.2.0-macOS-universal.dmg",
      "sha256": "<64 lowercase hex characters>",
      "size": 123456789
    },
    "windows": {
      "file": "Squid-Setup-0.2.0-win-x64.exe",
      "sha256": "<64 lowercase hex characters>",
      "size": 45678901
    }
  }
}
```

Windows ARM64 first looks for `windows-arm64`, then falls back to `windows`.
Unknown artifact keys are ignored. A larger manifest schema is refused.

## Update flow and trust

1. The app fetches the manifest from the release branch.
2. If the build has a public key, `squid-update.sig` must verify over the
   manifest's exact bytes.
3. Version comparison follows semver, including prerelease precedence.
4. The platform installer is downloaded to a temporary directory.
5. Exact byte size and SHA256 are checked. Any mismatch deletes the download.
6. Only a verified file can reach **Install and restart**.
7. The helper waits for Squid to exit, applies the update, and reopens the app.

The signature proves the manifest came from the release key. SHA256 and size
prove the downloaded installer is the file that manifest names. HTTPS protects
the transport; it is not treated as a substitute for either check.

## Version bump

Keep these four values equal:

- `Cargo.toml` workspace version
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`
- `package.json`

Then run frontend checks, Rust tests (including `src-tauri`), build both
installers, and publish.
