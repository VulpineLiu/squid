# squid

> clean · uninstall · purge · watch — a system utility that never touches what it does not own

**squid** 用一套 Rust 代码实现了四款经典付费工具的常用能力：

| 参考软件 | squid 对应能力 | 入口 |
| --- | --- | --- |
| CleanMyMac | 缓存 / 日志 / 浏览器垃圾 / Xcode DerivedData 等可再生文件清理 | `squid clean` |
| AppCleaner | 卸载 App 并连带删除散落的残留 | `squid uninstall` |
| （深度清理）| node_modules、构建产物、磁盘镜像、陈旧压缩包 | `squid purge` |
| DaisyDisk | 磁盘占用树，谁吃掉了我的硬盘 | `squid analyze` / `state` 存储面板 |
| iStat Menus | CPU / 内存 / 网络 / 磁盘 / 温度 / 电池 / 进程实时监控 | `squid status` / `state` 系统面板 |

界面（终端里）参考 CleanMyMac：暗色卡片、分组列表、勾选后一键清理。配色按 24-bit 设计，
遇到不支持真彩色的终端（macOS 自带 Terminal.app 就不支持）会自动退到最接近的 256 色，
观感基本一致；`SQUID_TRUECOLOR=always|never` 可以强制。

**状态不依赖颜色**：光标行在降级终端里改用「反白 + 加粗」表达（反白由终端自己的前景/背景色
完成，任何调色板、包括纯灰度终端都能显示），勾选框用加粗 `●` 与细体 `○` 区分，容量条用
`█/░` 字符而非色块。所以在 Terminal.app、`TERM=vt100`、甚至远程 ssh 里，选中/勾选/进度
依然看得出来，不需要真彩色。

- 语言：Rust（`1.80+`，workspace 拆分为 7 个 crate）
- 平台：macOS 已完整支持；Linux / Windows 通过 `squid-platform` 的 `Platform` trait 预留，见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- 形态：命令行 + 内置 TUI（`squid state`），无后台常驻进程

---

## 安装 / 构建

```bash
# 从源码安装到 ~/.cargo/bin
cargo install --path crates/squid-cli

# 或者本地构建调试版本
cargo build -p squid-cli
./target/debug/squid --help
```

squid 是**一条普通命令**，不是 .app：装完之后在任意目录敲 `squid` 就行，
不需要进到软件目录里执行，也不会往 shell 配置里写东西、不留后台进程。

### 发布安装包（不是 dmg）

```bash
./scripts/release.sh
# -> dist/squid-<版本>-universal-apple-darwin.tar.gz   (arm64 + x86_64 通用二进制)
# -> dist/squid-<版本>-universal-apple-darwin.tar.gz.sha256
```

脚本做的事：`cargo build --release` 两种 Apple target → `lipo -create` 合成通用二进制
→ ad-hoc `codesign` → 打 tar.gz + sha256，最后打印要填进
[`packaging/homebrew/squid.rb`](packaging/homebrew/squid.rb) 的校验和与
`gh release create` 命令。只出当前芯片的包用 `ARCHES="aarch64-apple-darwin" ./scripts/release.sh`。

**为什么是 tar.gz 而不是 dmg**：dmg 的意义是把 `.app` bundle 拖进 Applications，
而 squid 没有 bundle、没有图标、不参与 LaunchServices，套一层 dmg 只会多一次
「已损坏，无法打开」的隔离属性摩擦。tar.gz 也是 `cargo-binstall` / Homebrew 期望的形状。
真要给别人装，除了 ad-hoc 签名还需要 Apple Developer ID 签名 + `notarytool` 公证：

```bash
codesign --force --options runtime --sign "Developer ID Application: ..." dist/squid
xcrun notarytool submit dist/squid-<版本>-universal-apple-darwin.tar.gz --keychain-profile squid --wait
xcrun stapler staple dist/squid-<版本>-universal-apple-darwin.tar.gz
```

### 装到 PATH

```bash
./scripts/install.sh                      # 用 target/release/squid
./scripts/install.sh ./dist/squid         # 装下载来的二进制
PREFIX=~/.local/bin ./scripts/install.sh  # 免 sudo，装进自己的目录
```

`install.sh` 按 `/opt/homebrew/bin` → `/usr/local/bin` → `~/.local/bin` 找第一个可写目录
（`PREFIX=` 可覆盖），装完顺手清 `com.apple.quarantine` 并补一次 ad-hoc 签名。前两个目录
本来就在 PATH 上，所以装完直接就是 `squid ...`；只有落到 `~/.local/bin` 且它不在 PATH 时，
脚本才会打印一行需要追加到 `~/.zshrc` 的 `export PATH=`。

### Homebrew

`packaging/homebrew/squid.rb` 是可直接放进自己 tap 的 formula 模板（`brew install yourname/tools/squid`），
把 release.sh 输出的 sha256 填进去即可。

## 快速上手

先看，再删——所有会动文件的命令都支持 `--dry-run`：

```bash
squid status                      # 一次性系统快照（CPU / 内存 / 磁盘 / 进程）
squid doctor                      # 权限、配置、历史记录自检

squid clean --dry-run             # 看看能清多少，什么都不删
squid clean                       # 扫描 → 逐项确认 → 移入废纸篓
squid clean --categories caches,browsers --min-age 7

squid uninstall --list            # 已安装的 App 及体积
squid uninstall Xmind --dry-run   # 预览 App 本体 + 残留清单
squid uninstall Xmind             # 卸载并删除残留

squid purge --dry-run             # 项目构建产物（node_modules 等）
squid purge ~/code ~/work         # 指定目录

squid analyze ~/Library/Caches    # 磁盘占用树（DaisyDisk 式）
squid history                     # squid 删过什么
squid undo                        # 把最近一次进废纸篓的会话还原回去
squid state                       # 进入 TUI
```

## 命令一览

全局参数：`--json`（机器可读，日志走 stderr）、`--quiet`、`--no-color`。

| 命令 | 作用 | 常用参数 |
| --- | --- | --- |
| `clean` | 按规则清理缓存、日志、浏览器与开发者垃圾、崩溃报告、废纸篓等 39 条内置规则 | `--dry-run` `-y` `--categories <a,b>` `--orphans` `--min-age <天>` `--user-only` `--trash` `--permanent` |
| `uninstall [NAME]` | 按名称或 bundle id 定位 App，生成并执行「本体 + 残留」计划 | `--list` `--dry-run` `-y` `--trash` `--permanent` `--json` |
| `purge [PATHS…]` | 深度清理：项目构建目录、磁盘镜像、安装包、陈旧下载 | `--dry-run` `-y` `--all` `--min-age <天>` `--json` |
| `state` | 打开 TUI：清理 / 卸载 / 深度清理 / 存储 / 系统 / 历史 六个面板 | — |
| `status` | 系统快照，适合脚本与 cron | `--processes <n>` |
| `analyze [PATH]` | 单路径体积树 + 最大文件（默认主目录） | `--depth <层>` `--top <n>` `--files` |
| `history` | 历史会话（时间、命令、释放空间、条目数） | `--limit <n>` |
| `undo [ID]` | 还原某次废纸篓会话（默认最近一次） | — |
| `whitelist add\|list\|remove` | 保护路径，任何扫描都不会再选中它 | — |
| `rules` | 列出当前生效的规则 | `--json` |
| `doctor` | 权限探测、配置与历史健康检查 | `--json` |

## TUI（`squid state`）

六个面板，CleanMyMac 式卡片布局：`1` 清理 · `2` 卸载 · `3` 深度清理 · `4` 存储 · `5` 系统 · `6` 历史。

启动后焦点默认落在左侧的功能选择区：`↑`/`↓` 直接选功能，`enter`（或 `esc`）把方向键交还给右侧的数据列表，
再按一次 `esc` 又回到菜单；`1`–`6` 与 `Tab` 任何时候都能直达。

| 按键 | 功能 |
| --- | --- |
| `Tab` / `1`–`6` | 切换面板 |
| `esc` | 逐级返回，一次只退一级：确认框 → 取消；卸载计划 → 应用列表；最大文件 → 原来的目录；目录 → 上一级；列表 → 功能菜单。帮助浮层、筛选输入框同样吃 `esc`（`n`、`backspace` 也能取消确认框） |
| `⌥↑` / `⌥↓`（或 `Ctrl+↑/↓`） | 在任意面板直接切换六个功能 |
| `j` `k` `↑` `↓`、`PgUp/PgDn`、`Home/End` | 移动 |
| `space` | 勾选项目或整个分组 |
| `enter` | 扫描 / 查看详情 / 进入目录 |
| `s` | 扫描当前面板 |
| `a` | 全选 / 全不选 |
| `d` | 清理已勾选内容 |
| `m` | 废纸篓 ⇄ 永久删除 |
| `u` | 还原上一次废纸篓会话 |
| `f` | 卸载面板：筛选应用列表（输入即筛选，`esc` 清空筛选、`enter` 保留）；存储面板：列出当前目录里最大的文件 |
| `h` `l` `←` `→` | 折叠 / 进入 |
| `r` | 刷新当前面板（存储面板重新扫描磁盘占用） |
| `c` | 取消正在运行的任务（`esc` 只负责返回，不取消任务） |
| `?` / `q` | 帮助 / 退出 |

界面语言自动跟随 `$LANG`（中文环境下为中文），也可在配置里写 `language = "en"` 强制英文。

任务进行中按下的按键不会石沉大海：squid 会弹出提示并说清楚是哪个任务占着通道
（「删除还在进行，请稍候，或按 `c` 取消」），确认框在忙时按回车只会保留你的回答、等任务结束再按一次即可。

存储面板的磁盘扫描是唯一允许跑在后台的任务：它只读、可能要走几十秒，因此它只把自己挂在标题上的
「后台扫描」徽章，不会让卸载、清理等其它功能报「上一个任务还在进行」；删除完成后的磁盘占用刷新也走这条后台通道，
进度条结束后一定消失，不留着一条跑完的旧进度条装成卡住的样子。

颜色在启动的那一刻就探测完，日志第一行会写出这次拿到的是哪一档，而不是等第一张卡片画错了再猜：

1. **24-bit 真彩色**：只有终端**明确说得出**自己支持才给（`COLORTERM=truecolor`/`24bit`、iTerm、WezTerm、
   Ghostty、VS Code 这类白名单、或 `TERM` 里带 `truecolor`）。按设计的 RGB 原样绘制，选中行是设计稿里的那条强调色底条。
2. **256 色**（Apple 的 Terminal.app、`TERM=xterm`、ssh 到老机器、什么都没说的终端）：每个 RGB 就近换成 xterm
   调色板里的条目。这一档是默认落点——调色板小一点的代价只是颜色近似，而看不懂 `48;2;r;g;b` 的终端会直接把那串数字打在屏幕上。
   强调色在小调色板里靠不住，所以选中行换成**灰底高亮条**，右侧数据列表用相邻两级灰度条纹交替——
   行与行的分别由灰度承担，一行可以从左读到右，光标也不会是 flat 灰板上唯一亮着的一格。
   两个相邻的灰度若被量化成同一个索引，会自动在灰阶上再走一级，条纹不会消失。
3. **没有调色板**（设置了 `NO_COLOR`、或 `TERM=dumb`）：完全不发颜色，选中行用反白 + 加粗，
   勾选态和菜单里的 `[n]` 徽章另带符号，不依赖任何颜色。

`SQUID_TRUECOLOR=never|always` 可以手动降级 / 升级，`NO_COLOR` 与全局的 `squid --no-color` 一样强制走第三档。

### 数字是怎么显示的

账本里的数字最容易骗人，所以它有明确的规矩：

- **历史/账本一律两位小数**（`24.31 GB`、`753.66 MB`），列表和侧栏为了紧凑仍是一位（`13.0 GB`）。
- **计数一定带单位并分组**：中文 `1,634 项`、英文 `1,634`。`1634·24.3 GB` 这种写法已经消失了——
  中间那个点正好落在小数点该在的位置，两个数会被读成一个坏掉的数。
- **「可删除」说的是还没删的东西**：清理面板的标题是 `清理 · 62 项 · 可删除 1.79 GB / 25.35 GB`，
  即「已勾选 / 全部发现」，不会再用「已删除」把待办写成结果。
- **进废纸篓不算已释放**：`d` 默认只是 `rename` 到 `~/.Trash`，同一卷上的改名一个字节都没离开磁盘。
  标题会写 `27.21 GB 移入废纸篓 · 清空废纸篓后才真正释放`，`u` 还原过的会话在总计里算 `0`，
  那一行自己标 `已还原`（记录本身仍写它当时删了多少）。
- **删除成功后自动重扫**：一次真实删除会让面板上每个数字都过时（缓存会重建、规则命中的文件也换了），
  所以删除完成即自动跑一遍 `s`，侧栏「分组」的容量随之重算；存储面板的磁盘占用走后台刷新，
  若你正停在某个下钻目录里，则只提示「按 `r` 重新扫描」，不会把你正在读的东西抽走。
- 中文界面里连分组名都是中文：开发文件 / 系统垃圾 / 应用缓存 / 浏览器缓存 / 日志与崩溃报告 /
  构建产物 / 废纸篓 / 多余语言包 / 卸载残留。

## 安全模型

squid 的前提是：**它可以清理一个 245 GB 的系统盘，但不能毁掉其中的任何一个用户文件。**

1. **默认进废纸篓**。`DeleteMode::Trash` 是默认值，`--permanent` 才真正 `unlink`。卸载计划里的 App
   本体（`/Applications/Foo.app`）是真正被删除的一项，不是只删残留；`u` 或 `squid undo` 原样放回。
2. **`--dry-run`  everywhere**。任何会删除的命令都能只看不动。
3. **非交互环境需要显式确认**。stdin 不是 TTY 时，没有 `-y/--yes` 就直接拒绝执行删除。
4. **删除时二次校验**。扫描结果可能已经过时，`squid_engine::delete::execute` 会对每一条重新做安全检查、白名单检查和锚点校验，嵌套子项在父项之前处理，避免重复计数与 ENOENT。
5. **硬性禁区**。`squid_core::safety::SafetyPolicy` 内置 `HARD_DENY`（`/`、`/usr`、`/System`、`/Library`、`/Applications`、`/private/var/folders`…）与 `DENY_EXACT`（`~/Library`、`~/Documents`、`~/.ssh`、`~/.aws`… 只允许删子项、不允许整目录递归删除），递归删除还要求在允许的锚点之下至少 `min_depth = 1` 层，并且永不跟随符号链接。
6. **规则自带刹车**：`protect` 白名单、`min_age_days`、`needs_admin`、`skip_if_running`（例如 Docker Desktop 正在运行时不会提出删它的 1 TB 虚拟机磁盘）、`risky`（默认不勾选并提示复核）。
7. **删除账本 + `undo`**。每次真实删除写一个 JSON 会话（含每条的废纸篓目标路径），`squid undo` 把仍留在废纸篓里的文件原样放回。
8. **白名单**。`squid whitelist add '~/Library/Caches/com.apple.Safari'` 之后，任何规则、任何面板都不会再选中它。

## 配置

配置文件：`~/Library/Application Support/squid/config.toml`（`squid doctor` 会打印实际路径）。

```toml
delete_mode = "trash"          # trash | permanent
history_keep = 60              # 保留多少条历史会话
purge_min_age_days = 7         # 7 天内动过的构建产物不算垃圾
purge_max_entries = 200000     # 单个项目探测的文件数上限
show_allocated = true          # 显示实际占盘大小而非逻辑大小
language = "zh"                # "en" | "zh"，留空跟随 $LANG
whitelist = ["~/Library/Caches/ImportantApp"]
disabled_rules = ["user-trash"]
purge_roots = ["~/code", "~/Projects", "~/Desktop"]
```

环境变量：

| 变量 | 作用 |
| --- | --- |
| `SQUID_CONFIG_DIR` | 覆盖配置目录（可移植安装、CI、测试） |
| `SQUID_STATE_DIR` | 覆盖状态目录（历史账本所在地） |
| `SQUID_TRUECOLOR` | `always` / `never` 强制指定配色能力，跳过自动探测 |
| `SQUID_LANG` | `zh` / `en` 强制界面语言，等价于 `squid --lang zh` |

界面语言判定顺序：配置文件 `language` → `SQUID_LANG` → `LC_ALL` → `LC_MESSAGES` → `LANG`
→ macOS 的 `AppleLocale`/`AppleLanguages`（`defaults read -g`）→ 英文。`C` / `POSIX` /
空值视为「没有信息」，继续往下走，所以在 `ssh`、`cron`、CI 里不会被误判成英文——
macOS 上仍然能从系统偏好里读到中文。`squid doctor` 会打印 language 和 colour 两行，
标明最终选了什么、是被哪一条决定的；TUI 里按 `?` 也有同样的诊断行。

自定义规则放到 `<config_dir>/rules/*.toml`，与内置规则合并（同 `id` 覆盖内置）：

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

`pattern` 的最长字面量前缀成为该条目的 **锚点（anchor）**，删除时只允许在锚点之下操作——这就是「一个规则写错也不会把 `~/.cargo` 整个删掉」的原因。

## 项目结构

```
crates/
├── squid-core      模型、字节格式化、路径、安全策略、设置、删除账本、进度/取消
├── squid-platform  Platform trait + macOS 实现（App 清单、残留匹配、卷）Linux/Windows 占位
├── squid-rules     TOML 规则引擎：模式展开、锚点、保护列表、年龄与风险标记
├── squid-stats     sysinfo 采样器、趋势、电池、uptime（iStat 面板的数据源）
├── squid-engine    四大引擎：clean / uninstall / purge / analyze + 唯一的删除通道 delete
├── squid-tui       ratatui 界面（六个面板、任务线程、主题、中英双语文案）
└── squid-cli       clap 命令行入口
```

依赖方向单向：`core → platform → rules → stats → engine → tui → cli`。
细节见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)，威胁模型与 Responsible disclosure 见 [docs/SECURITY.md](docs/SECURITY.md)。

## 开发

```bash
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
```

TUI 的键盘与渲染取舍用真实 pty 冒烟测试守着（需要 `pyte`）：

```bash
python3 -m pip install --user pyte
cargo build --release
python3 scripts/tui_smoke.py          # menu / menu-mono / uninstall
python3 scripts/tui_smoke.py uninstall   # 只跑连续卸载（自己造临时 app，用完删除）
```

测试不会删除真实用户数据：删除相关用例写在 `~/Library/Caches/squid-delete-test-*` 的临时目录里，并通过 `SQUID_STATE_DIR` 把账本重定向到临时目录。

## 跨平台计划

| 平台 | 现状 |
| --- | --- |
| macOS | 完整实现（规则、App 残留、卷、Launch Services 卸载钩子） |
| Linux | `squid-platform/src/linux.rs` 已实现 `.desktop` / Flatpak / Snap 应用清单与 XDG 残留目录；缺 `rules/linux.toml` 与真机验证 |
| Windows | `squid-platform/src/windows.rs` 为占位实现，需注册表卸载表、`%LOCALAPPDATA%` 规则、回收站 API 与 UAC 提权 |

## 关于 Mole 与许可证

squid 的 Rust 实现是**净室（clean-room）实现**：仓库中仅作参考存放的 [Mole](https://github.com) 源码采用 **GPL v3**，本项目**没有复制、翻译或衍生其任何代码**，仅将其作为“清理哪些路径、有哪些坑”的行为参考。`/Mole` 已在 `.gitignore` 中排除。

squid 自身采用 **MIT OR Apache-2.0** 双许可证（见 `LICENSE`、`LICENSE-MIT`、`LICENSE-APACHE`）。

如果你把 squid 用于商业分发，请自行确认不与 GPL 代码静态链接。
