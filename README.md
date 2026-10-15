# squid

macOS 磁盘清理与系统监控工具：清理缓存与系统垃圾、卸载 App 并连带清掉残留、深度清理构建产物、
分析磁盘占用、实时监控资源。命令行与内置 TUI 共用同一套引擎，产物是一个静态二进制，
不装后台进程、不写 shell 配置、不需要 root。

## 特性

| 能力 | 说明 | 入口 |
| --- | --- | --- |
| 清理 | 39 条内置规则：应用缓存、浏览器缓存、日志与崩溃报告、Xcode DerivedData、废纸篓等 | `squid clean` |
| 卸载 | 按名称或 bundle id 定位 App，本体与散落的偏好、缓存、容器一起进入删除计划 | `squid uninstall` |
| 深度清理 | `node_modules` 等项目构建产物、磁盘镜像、陈旧安装包与下载 | `squid purge` |
| 存储分析 | 体积树与最大文件，可逐层下钻 | `squid analyze`、`state` 存储面板 |
| 系统监控 | CPU / 内存 / 网络 / 磁盘 / 电池 / 温度 / 进程 | `squid status`、`state` 系统面板 |
| 历史与撤销 | 每次真实删除写一条 JSON 账本，可将仍在废纸篓里的文件原样放回 | `squid history`、`squid undo` |

- 语言：Rust 1.80+，workspace 由 7 个 crate 组成
- 界面：中英双语文案，跟随系统语言；真彩色 / 256 色 / 无色三档自动适配
- 结构：命令行与 TUI 共用同一套扫描与删除通道，行为、确认、账本完全一致
- 平台：macOS 完整支持，Linux / Windows 见[路线图](#路线图)

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

发布产物是通用二进制（arm64 + x86_64）的 tar.gz，附带 sha256：

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
| `uninstall [NAME]` | 按名称或 bundle id 定位 App，生成并执行「本体 + 残留」计划 | `--list` `--dry-run` `-y` `--trash` `--permanent` `--json` |
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

**语言**：中英双语，判定顺序为配置文件 `language` → `SQUID_LANG` → `LC_ALL` → `LC_MESSAGES` →
`LANG` → macOS 系统偏好（`AppleLocale`/`AppleLanguages`）→ 英文；`C`/`POSIX`/空值视为无信息继续下探。
`squid doctor` 与 TUI 的 `?` 面板都会打印最终生效的语言与配色档位及其来源。

## 安全模型

前提：**可以清理一个 245 GB 的系统盘，但不能毁掉其中的任何用户文件。**

1. **默认进废纸篓**：`DeleteMode::Trash` 为默认，`--permanent` 才执行 `unlink`；卸载计划包含 App 本体；`squid undo` 原样放回。
2. **`--dry-run` everywhere**：任何删除命令都能只看不动。
3. **非交互环境需显式确认**：stdin 不是 TTY 时，缺少 `-y/--yes` 直接拒绝执行删除。
4. **删除时二次校验**：执行前对每条重新做安全检查、白名单检查与锚点校验；嵌套子项先于父项处理，避免重复计数与 `ENOENT`。
5. **硬性禁区**：`squid_core::safety::SafetyPolicy` 内置 `HARD_DENY`（`/`、`/usr`、`/System`、`/Library`、`/Applications`、`/private/var/folders` …）与 `DENY_EXACT`（`~/Library`、`~/Documents`、`~/.ssh`、`~/.aws` … 只允许删子项）；递归删除要求位于允许锚点之下至少 `min_depth = 1` 层，且永不跟随符号链接。
6. **规则自带刹车**：`protect` 白名单、`min_age_days`、`needs_admin`、`skip_if_running`（进程运行时不提出删除其虚拟磁盘）、`risky`（默认不勾选并提示复核）。
7. **删除账本与 `undo`**：每次真实删除写一个 JSON 会话，记录每条的废纸篓目标路径。
8. **白名单**：`squid whitelist add '~/Library/Caches/com.apple.Safari'` 之后，任何规则与面板都不会再选中该路径。

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
`~/.config/squid`、`~/.local/state/squid`）。`squid doctor` 会打印实际生效的配置与账本路径。

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
├── squid-platform  Platform trait + macOS 实现（App 清单、残留匹配、卷）；Linux/Windows 占位
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
| Windows | 平台层已实现：注册表卸载表（HKLM 64/32 位 + HKCU）、AppData 残留目录、卷枚举、回收站（`IFileOperation`）；CI 每次提交做 MSVC 目标交叉编译检查 | 真机验证、`rules/windows.toml`、UAC 提权、winget 发布 |

其他：规则贡献流程与规则覆盖度基准、TUI 主题自定义、Linux/Windows 冒烟测试。

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
