# squid

macOS / Windows 磁盘清理与系统监控工具：清理缓存与系统垃圾、卸载 App 并连带清掉残留、深度清理构建产物、
分析磁盘占用、实时监控资源。命令行与内置 TUI 共用同一套引擎，产物是一个静态二进制，
不装后台进程、不写 shell 配置、不需要 root。

## 桌面应用

仓库现在同时提供 macOS / Windows 原生桌面程序 **Squid**。桌面端沿用 `nex_shift` 的技术栈
（Tauri 2 + React 19 + TypeScript + Vite）和更新通道，直接复用本仓库现有 Rust 清理引擎：

- 界面：CleanMyMac 风格的概览、清理、应用卸载、深度清理、存储空间、系统状态、历史记录和设置；
- 安全：GUI 只能选择后端扫描缓存中的索引，删除前仍由 Rust 引擎重新校验路径、锚点、白名单和锁定状态；
- 更新：`releases` 分支中的 `squid-update.json` + 安装包，文件大小与 SHA256 始终校验，可选 Ed25519 清单签名；
- 本地化：中英双语，跟随现有 `Settings` 语言偏好。

开发与构建：

```bash
pnpm install
pnpm desktop:dev                 # 开发窗口
pnpm desktop:build               # 当前平台安装包
bash scripts/build-macos-release.sh
bash scripts/build-windows-release.sh
```

完整的桌面架构、权限和发布说明见 [`docs/DESKTOP.md`](docs/DESKTOP.md) 与
[`docs/RELEASE.md`](docs/RELEASE.md)。Nex 系列应用的统一主色与设计常量见
[`docs/DESIGN.md`](docs/DESIGN.md)。下面的 CLI / TUI 文档仍然有效，三个入口共用同一套扫描与删除核心。

## 特性

| 能力 | 说明 | 桌面 / CLI |
| --- | --- | --- |
| 清理 | 内置规则 macOS 39 条 / Windows 30 条：应用与浏览器缓存、日志与崩溃报告、Xcode DerivedData、Windows Update 残留、废纸篓 / 回收站等 | 桌面“清理” / `squid clean` |
| 卸载 | 按名称定位 App（macOS 扫 `/Applications` 与 bundle id，Windows 读注册表卸载表），本体与散落的偏好、缓存、容器 / AppData 残留一起进入删除计划 | 桌面“应用程序” / `squid uninstall` |
| 深度清理 | `node_modules` 等项目构建产物、磁盘镜像、陈旧安装包与下载 | 桌面“深度清理” / `squid purge` |
| 存储分析 | 体积树与最大文件，可逐层下钻 | 桌面“存储空间” / `squid analyze` |
| 系统监控 | CPU / 内存 / 网络 / 磁盘 / 电池 / 温度 / 进程，并可结束选中的进程 | 桌面“系统状态” / `squid status` |
| 历史与撤销 | 每次真实删除写一条 JSON 账本，可将仍在废纸篓 / 回收站里的文件原样放回 | 桌面“历史记录” / `squid history`、`squid undo` |

- 语言：Rust 1.80+，workspace 由 7 个 crate 组成
- 界面：中英双语文案，跟随系统语言；真彩色 / 256 色 / 无色三档自动适配
- 结构：命令行与 TUI 共用同一套扫描与删除通道，行为、确认、账本完全一致
- 平台：macOS 完整支持；Windows 10 / 11 用同一个构建，已在局域网真机跑通（那台机器自报
  `Windows 10 Pro (build 18363)`）：回收站真实删除与还原、按名称卸载并带走 AppData 残留、系统面板结束进程，
  中英文 TUI 在 legacy conhost + cp936 下的排版由判据守着而不是靠肉眼——界面启动时自己量一次
  「一个字形在这里占几格」，被收两格就整屏改用 ASCII 边框，不再叠成花屏；
  UAC 提权、Windows 11 复验与 Linux 见[路线图](#路线图)

---

## 安装

squid 是一条普通命令：装完之后在任意目录敲 `squid` 即可，不需要进入安装目录执行。

### 从源码

```bash
cargo install --path crates/squid-cli     # 装到 ~/.cargo/bin
# 或本地构建
cargo build --release -p squid-cli && ./target/release/squid --help
```

### 脚本安装

```bash
./scripts/install.sh                      # 使用 target/release/squid
./scripts/install.sh ./dist/squid         # 安装已下载的二进制
PREFIX=~/.local/bin ./scripts/install.sh  # 免 sudo，装进指定目录
```

安装目录按 `PREFIX` → `/opt/homebrew/bin` → `/usr/local/bin` → `~/.local/bin` 取第一个可写者；
前两者本身就在 `PATH` 上，落到 `~/.local/bin` 且它不在 `PATH` 时，脚本会打印一行待追加到 `~/.zshrc`
的 `export PATH=`。脚本同时清除 `com.apple.quarantine` 并补一次 ad-hoc 签名。

### 预编译包

发布产物是通用二进制（arm64 + x86_64）的 tar.gz，附带 sha256；发布脚本目前只出 Apple 产物，
Windows 需从源码构建（`cargo build --release --target x86_64-pc-windows-msvc`）：

```bash
curl -LO https://github.com/nexdev/squid/releases/download/v0.1.0/squid-0.1.0-universal-apple-darwin.tar.gz
curl -LO https://github.com/nexdev/squid/releases/download/v0.1.0/squid-0.1.0-universal-apple-darwin.tar.gz.sha256
shasum -a 256 -c squid-0.1.0-universal-apple-darwin.tar.gz.sha256

tar xzf squid-0.1.0-universal-apple-darwin.tar.gz
install -m 0755 squid-0.1.0-universal-apple-darwin/squid /usr/local/bin/squid
xattr -d com.apple.quarantine /usr/local/bin/squid 2>/dev/null || true
```

包内是 `squid` 通用二进制、`LICENSE`、`README.md`；单架构构建的包名为
`squid-<版本>-aarch64-apple-darwin.tar.gz` / `…-x86_64-apple-darwin.tar.gz`。

### Homebrew

[`packaging/homebrew/squid.rb`](packaging/homebrew/squid.rb) 是可直接放进自有 tap 的 formula 模板，
填入发布脚本输出的 sha256 后 `brew install <yourname>/tools/squid`。

### Gatekeeper 与签名

随仓库脚本产出的是 ad-hoc 签名二进制，本机自用与 `install.sh` 安装不会触发拦截。分发给他人时
需要 Apple Developer ID 签名与公证：

```bash
codesign --force --options runtime --sign "Developer ID Application: <Team>" dist/squid
xcrun notarytool submit dist/squid-<版本>-universal-apple-darwin.tar.gz --keychain-profile squid --wait
xcrun stapler staple dist/squid-<版本>-universal-apple-darwin.tar.gz
```

## 快速上手

先看，再删——所有会动文件的命令都支持 `--dry-run`：

```bash
squid status                      # 一次性系统快照（CPU / 内存 / 磁盘 / 进程）
squid doctor                      # 权限、配置、历史自检，并打印生效路径

squid clean --dry-run             # 只看能清多少
squid clean                       # 扫描 → 逐项确认 → 移入废纸篓
squid clean --categories caches,browsers --min-age 7

squid uninstall --list            # 已安装的 App 及体积
squid uninstall Xmind --dry-run   # 预览 App 本体 + 残留清单
squid uninstall Xmind             # 卸载并删除残留

squid purge --dry-run             # 项目构建产物（node_modules 等）
squid purge ~/code ~/work         # 指定目录

squid analyze ~/Library/Caches    # 磁盘占用树
squid history                     # squid 删过什么
squid undo                        # 还原最近一次废纸篓会话
squid state                       # 进入 TUI
```

## 命令一览

全局参数：`--json`（机器可读，日志走 stderr）、`--quiet`、`--no-color`、`--lang <zh|en>`。

| 命令 | 作用 | 常用参数 |
| --- | --- | --- |
| `clean` | 按规则清理缓存、日志、浏览器与开发者垃圾、崩溃报告、废纸篓等 | `--dry-run` `-y` `--categories <a,b>` `--orphans` `--min-age <天>` `--user-only` `--trash` `--permanent` |
| `uninstall [NAME]` | 按名称定位 App（Windows 取注册表卸载表），生成并执行「本体 + 残留」计划 | `--list` `--dry-run` `-y` `--trash` `--permanent` `--json` |
| `purge [PATHS…]` | 深度清理：项目构建目录、磁盘镜像、安装包、陈旧下载 | `--dry-run` `-y` `--all` `--min-age <天>` `--json` |
| `state` | 打开 TUI：清理 / 卸载 / 深度清理 / 存储 / 系统 / 历史 | — |
| `status` | 系统快照，适合脚本与 cron | `--processes <n>` |
| `analyze [PATH]` | 单路径体积树与最大文件（默认主目录） | `--depth <层>` `--top <n>` `--files` |
| `history` | 历史会话（时间、命令、释放空间、条目数） | `--limit <n>` |
| `undo [ID]` | 还原某次废纸篓会话（默认最近一次） | — |
| `whitelist add\|list\|remove` | 保护路径，任何扫描不再选中它 | — |
| `rules` | 列出当前生效的规则 | `--json` |
| `doctor` | 权限探测、配置与历史健康检查 | `--json` |

## TUI（`squid state`）

六个面板：`1` 清理 · `2` 卸载 · `3` 深度清理 · `4` 存储 · `5` 系统 · `6` 历史。

启动后焦点位于左侧功能选择区，`↑`/`↓` 选功能，`enter` 或 `esc` 将方向键交还给右侧数据列表，
再按一次 `esc` 回到菜单。

| 按键 | 功能 |
| --- | --- |
| `Tab` / `1`–`6` | 切换面板 |
| `esc` | 逐级返回，一次退一级：确认框 → 取消；卸载计划 → 应用列表；最大文件 → 原目录；目录 → 上一级；列表 → 功能菜单。帮助浮层与筛选输入框同样响应 `esc`（确认框还可用 `n`、`backspace` 取消） |
| `⌥↑` / `⌥↓`（或 `Ctrl+↑/↓`） | 在任意面板直接切换六个功能 |
| `j` `k` `↑` `↓`、`PgUp`/`PgDn`、`Home`/`End` | 移动 |
| `space` | 勾选条目或整个分组 |
| `enter` | 扫描 / 查看详情 / 进入目录 |
| `s` | 扫描当前面板 |
| `a` | 全选 / 全不选 |
| `d` | 清理已勾选内容 |
| `x` | 系统面板：结束光标所在的进程（先弹确认框，复述 pid / CPU / 内存并说明「它不会有机会保存任何内容」）；其余面板与 `d` 等价 |
| `m` | 废纸篓 ⇄ 永久删除 |
| `u` | 还原上一次废纸篓会话 |
| `f` | 卸载面板：筛选应用列表（`esc` 清空筛选、`enter` 保留）；存储面板：列出当前目录最大的文件 |
| `h` `l` `←` `→` | 折叠 / 进入 |
| `r` | 刷新当前面板 |
| `c` | 取消正在运行的任务 |
| `?` / `q` | 帮助 / 退出 |

删除完成后自动重新扫描当前面板，分组容量随之重算；存储面板的磁盘占用走后台刷新，
停在某个下钻目录时只提示按 `r`。进废纸篓不等于已释放空间，标题会区分「移入废纸篓」与「真正释放」，
被 `u` 还原的会话在总计中记为 `0`。

任务执行期间的按键会给出明确提示（按 `c` 可取消），确认框在忙时保留待确认内容直到任务结束。

**显示适配**：启动时探测终端能力并分三档——终端明确声明支持真彩色（`COLORTERM=truecolor|24bit`、
iTerm、WezTerm、Ghostty、VS Code、`TERM` 含 `truecolor`）时按设计的 RGB 绘制；否则退到 256 色，
每个 RGB 就近映射到 xterm 调色板，选中行使用灰底高亮条、数据列表用相邻两级灰度交替；
`NO_COLOR` 或 `TERM=dumb` 时完全不发颜色。三档下选中、勾选、进度都由反白 / 加粗 / `●○` / `█░`
表达，不依赖颜色。`SQUID_TRUECOLOR=always|never` 强制指定档位。

**Windows 控制台**：legacy conhost 进备用屏时换掉的只是「正在显示的那块缓冲区」，进程手里的窗口比真实
存在的格子更宽；cp936 码页又把 `─│·…↑↓✓` 这批 East Asian Ambiguous 字形收两格，而排版按一格算。
两种偏差最后长成同一种花屏，所以界面每一拍重新量一次尺寸（只有真的变了才擦屏重画），并在启动时
逐字符量一次字形的价格（往屏幕上写一个字符再擦掉），付不起的字符整屏换成 ASCII 边框。
命令行不当这个测量的客人——测量要动它正显示的东西——它按码页取悲观值，代价是被误伤的机器上少一个
`█`、多一个 `#`。`SQUID_SYMBOL_CELLS=1|2` 强制指定价格，`SQUID_CONSOLE_TRACE=<文件>` 把控制台自报的
几组数字写进文件留证。

**语言**：中英双语，判定顺序为配置文件 `language` → `SQUID_LANG` → `LC_ALL` → `LC_MESSAGES` →
`LANG` → macOS 系统偏好（`AppleLocale`/`AppleLanguages`）→ 英文；`C`/`POSIX`/空值视为无信息继续下探。
`squid doctor` 与 TUI 的 `?` 面板都会打印最终生效的语言与配色档位及其来源。

## 安全模型

前提：**可以清理一个 245 GB 的系统盘，但不能毁掉其中的任何用户文件。**

1. **默认进废纸篓 / 回收站**：`DeleteMode::Trash` 为默认，`--permanent` 才执行 `unlink`；卸载计划包含 App 本体；`squid undo` 原样放回。
2. **`--dry-run` everywhere**：任何删除命令都能只看不动。
3. **非交互环境需显式确认**：stdin 不是 TTY 时，缺少 `-y/--yes` 直接拒绝执行删除。
4. **删除时二次校验**：执行前对每条重新做安全检查、白名单检查与锚点校验；嵌套子项先于父项处理，避免重复计数与 `ENOENT`。
5. **硬性禁区**：`squid_core::safety::SafetyPolicy` 内置 `HARD_DENY`（`/`、`/usr`、`/System`、`/Library`、`/Applications`、`/private/var/folders` …）与 `DENY_EXACT`（`~/Library`、`~/Documents`、`~/.ssh`、`~/.aws` … 只允许删子项）；递归删除要求位于允许锚点之下至少 `min_depth = 1` 层，且永不跟随符号链接。
6. **规则自带刹车**：`protect` 白名单、`min_age_days`、`needs_admin`、`skip_if_running`（进程运行时不提出删除其虚拟磁盘）、`risky`（默认不勾选并提示复核）。
7. **删除账本与 `undo`**：每次真实删除写一个 JSON 会话，记录每条的废纸篓目标路径；Windows 的回收站自己保管文件，账本只记「它进了回收站」，还原因此在资源管理器里完成。
8. **白名单**：`squid whitelist add '~/Library/Caches/com.apple.Safari'` 之后，任何规则与面板都不会再选中该路径。
9. **结束进程也有禁区**：不结束 squid 自己，不结束机器的第一个进程（unix 的 pid 0/1、Windows 的 System Idle 与 pid 4），Windows 上另封 `csrss` / `smss` / `wininit` / `services` / `lsass` / `winlogon`——提权后的进程确实能打开它们，代价是一次回车换来一台重启的机器。确认框交给执行方的是采样而不是 pid，动手前重新读一次；名单里故意没有 `svchost`，因为机器活得过它。

## 路径规划

### 数据与配置

| 用途 | 默认位置 | 覆盖方式 |
| --- | --- | --- |
| 配置文件 | `~/Library/Application Support/squid/config.toml` | `SQUID_CONFIG_DIR` |
| 自定义规则 | `~/Library/Application Support/squid/rules/*.toml` | `SQUID_CONFIG_DIR` |
| 删除账本 | `~/Library/Application Support/squid/history/<会话>.json` | `SQUID_STATE_DIR` |
| 删除去向 | `~/.Trash`（`delete_mode = "trash"`）；`permanent` 时直接 `unlink` | 配置 `delete_mode`、`--trash` / `--permanent` |
| 白名单 | 配置文件 `whitelist` 数组 | `squid whitelist` |
| 内置规则 | 编译进二进制的 `crates/squid-rules/rules/*.toml` | `disabled_rules` |

Linux 下配置与状态目录遵循 XDG（`$XDG_CONFIG_HOME/squid`、`$XDG_STATE_HOME/squid`，回落到
`~/.config/squid`、`~/.local/state/squid`）；Windows 下配置、规则与账本都在 `%APPDATA%\squid\`
（`config.toml`、`rules\*.toml`、`history\<会话>.json`），删除去向是回收站——回收站自己保管文件，
账本因此不记物理路径，`squid undo` 在 Windows 上把还原交给资源管理器的回收站。
`squid doctor` 会打印实际生效的配置与账本路径。

squid 只在上述位置写入，扫描过程全程只读。

### 可执行文件

| 安装方式 | 落点 |
| --- | --- |
| `cargo install` | `~/.cargo/bin/squid` |
| `scripts/install.sh` | `$PREFIX` → `/opt/homebrew/bin` → `/usr/local/bin` → `~/.local/bin` |
| `scripts/release.sh` | `dist/squid-<版本>-universal-apple-darwin.tar.gz` 及其 `.sha256` |

### 配置项

```toml
delete_mode = "trash"          # trash | permanent
history_keep = 60              # 保留多少条历史会话
purge_min_age_days = 7         # 7 天内动过的构建产物不算垃圾
purge_max_entries = 200000     # 单个项目探测的文件数上限
show_allocated = true          # 显示实际占盘大小而非逻辑大小
language = "zh"                # "en" | "zh"，留空跟随系统语言
whitelist = ["~/Library/Caches/ImportantApp"]
disabled_rules = ["user-trash"]
purge_roots = ["~/code", "~/Projects", "~/Desktop"]
```

### 环境变量

| 变量 | 作用 |
| --- | --- |
| `SQUID_CONFIG_DIR` | 覆盖配置目录（可移植安装、CI、测试） |
| `SQUID_STATE_DIR` | 覆盖状态目录（历史账本所在地） |
| `SQUID_TRUECOLOR` | `always` / `never` 强制指定配色能力，跳过自动探测 |
| `SQUID_LANG` | `zh` / `en` 强制界面语言，等价于 `squid --lang` |
| `SQUID_SYMBOL_CELLS` | `1` / `2` 强制指定「一个歧义字形占几格」，覆盖对控制台的实测（Windows 排版排查） |
| `SQUID_CONSOLE_TRACE` | 把控制台自报的尺寸与字形价格写进指定文件，留证据用 |
| `NO_COLOR` | 存在即关闭全部颜色输出 |

### 自定义规则

`<config_dir>/rules/*.toml` 与内置规则合并，同 `id` 覆盖内置：

```toml
[[rule]]
id = "my-app-cache"
category = "app-caches"
title = "MyApp caches"
patterns = ["~/Library/Caches/myapp/*"]   # `~` 展开；`*` 不跨越路径分隔符
mode = "path"                             # path = 删条目本身, contents = 只清空目录
os = ["macos"]
protect = ["keep-me"]
min_age_days = 3
select_by_default = true
note = "MyApp 会自动重建这些缓存。"
```

每条 pattern 的最长字面量前缀成为该条目的**锚点（anchor）**，删除只允许发生在锚点之下，
因此一条写错的规则不会波及锚点以外的目录。

## 项目结构

```
crates/
├── squid-core      模型、字节格式化、路径、安全策略、设置、删除账本、进度/取消
├── squid-platform  Platform trait 与 macOS / Linux / Windows 三套实现：App 清单、残留匹配、卷、
│                   回收站（Windows 走 shell 的 `IFileOperation`）、进程终止、控制台尺寸与字形宽度探测
├── squid-rules     TOML 规则引擎：模式展开、锚点、保护列表、年龄与风险标记
├── squid-stats     sysinfo 采样器、趋势、电池、uptime
├── squid-engine    clean / uninstall / purge / analyze 四个引擎 + 唯一的删除通道 delete
├── squid-tui       ratatui 界面（六个面板、任务线程、主题、中英双语文案）
└── squid-cli       clap 命令行入口
```

依赖方向单向：`core → platform → rules → stats → engine → tui → cli`。
架构细节见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)，威胁模型与漏洞披露见 [docs/SECURITY.md](docs/SECURITY.md)。

## 开发与测试

```bash
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
```

TUI 的键盘与渲染行为由真实 pty 冒烟测试守护（需要 `pyte`）：

```bash
python3 -m pip install --user pyte
cargo build --release
python3 scripts/tui_smoke.py              # menu / menu-mono / uninstall
python3 scripts/tui_smoke.py uninstall    # 只跑连续卸载
```

Windows 上没有 pty 可以 fork，界面排版改由「抓帧 → 复放 → 量框」守护：`win-tui-capture.sh` 让真机的
ConPTY 收到什么就原样存什么、抓回本地，`tui-frame-check.py` 用 pyte 把它复放成画面再逐帧量边框——
一幅画面要顶住 512 字节的写入不变才算画稳，半帧不许下判词：

```bash
python3 scripts/tui-frame-check.py --selftest                # 先证明判据本身没坏
python3 scripts/tui-frame-check.py /tmp/squid-win-tui.bin    # 再判那一帧
TUI_REUSE=1 TUI_OUT=/tmp/x.bin ./scripts/win-tui-capture.sh  # 只重算旧帧，不连真机
```

真机侧的构建与测试由 `scripts/win-sync.sh` 同步源码后跑 `scripts/win-check.ps1`
（test / clippy / release build，产物在 `C:\dev\target`）。

删除相关的测试用例只在 `~/Library/Caches/squid-delete-test-*` 临时目录内操作，
并通过 `SQUID_STATE_DIR` 将账本写入临时目录，不会触碰真实用户数据。

## 发布

```bash
./scripts/release.sh                                   # 通用二进制
ARCHES="aarch64-apple-darwin" ./scripts/release.sh      # 只出当前架构
```

脚本执行 `cargo build --release`（各 Apple target）→ `lipo -create` 合成通用二进制 →
ad-hoc `codesign` → 打包 tar.gz 与 sha256，并打印 `gh release create` 命令与需要填进
Homebrew formula 的校验和。

## 路线图

| 平台 | 现状 | 待办 |
| --- | --- | --- |
| macOS | 完整实现（规则、App 残留、卷、卸载钩子）| Developer ID 签名与公证流水线 |
| Linux | `squid-platform/src/linux.rs` 已实现 `.desktop` / Flatpak / Snap 应用清单与 XDG 残留目录 | `rules/linux.toml`、真机验证 |
| Windows | 平台层已实现：注册表卸载表（HKLM 64/32 位 + HKCU）、AppData 残留目录、卷枚举、回收站（`IFileOperation`）、强制结束进程（附关键进程名单）；安全层按 Windows 语义比较路径（大小写不敏感、盘符根/`$Recycle.Bin` 等硬封禁）；内置 30 条清理规则（临时文件、WER 崩溃报告、浏览器/Shell/开发缓存、Windows.old 等）；CI 每次提交做 MSVC 目标交叉编译检查；已在局域网内的真机验证通过：回收站真实删除（按卷核算字节数、可在资源管理器里还原）、`undo` 的措辞与行为、`--json` 输出只含一份 JSON、系统面板结束进程（确认框复述采样、动手前重读 pid）、列表翻页后高亮不再丢失、动态信息不再压住左侧菜单；TUI 的排版由判据守住而不是靠肉眼：`win-tui-capture.sh` 把真机 ConPTY 吐出的原始 ANSI 抓回本地用 pyte 复放，`tui-frame-check.py` 逐帧问「右边框落在第几列」，并以「顶住 512 字节写入不变」为画稳条件，`--selftest` 先证明判据本身没坏 | UAC 提权、长路径 manifest、Windows 11 真机复验、winget 发布 |

其他：规则贡献流程与规则覆盖度基准、TUI 主题自定义、Linux 冒烟测试（Windows 已有抓帧复放判据）。

## 许可证

squid 采用 **GNU General Public License v3.0 only**（`GPL-3.0-only`），全文见 [LICENSE](LICENSE)。

```
squid — disk cleaning and system monitoring utility
Copyright (C) 2026 squid contributors

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, version 3.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/gpl-3.0.html>.
```
