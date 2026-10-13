# squid 架构

本文描述 squid 的模块划分、数据流、安全不变量与跨平台方案。阅读顺序建议：先读「两条铁律」，再看 crate 图，最后按需查各引擎。

## 设计目标

1. **一台机器可以只有 245 GB，但用户的文件必须是无价的。** 所有设计里优先级最高的一条。
2. **单二进制、零常驻。** 没有 daemon、没有 launch agent、没有后台上传；`squid` 退出后不留痕迹。
3. **CLI 与 TUI 共用同一套引擎。** 终端界面不是玩具前端，`squid clean` 与 TUI 里的「清理」面板调用的是同一个函数。
4. **可脚本化。** 任何命令都能 `--json`、`--dry-run`、`--quiet`，退出码稳定。

非目标：不做实时文件系统监听、不做定时任务、不做 GUI（AppKit/SwiftUI）、不做云同步。

## 两条铁律（不变量）

**铁律一：扫描不删除。**
每个引擎都遵循 `Scan → Approve → Delete` 契约。`scan()` 只产出 `ScanReport`（一组 `CleanupItem`），幂等、无副作用，可以随时中断，可以对着 1 TB 的目录跑十次。

**铁律二：只有 `squid_engine::delete::execute` 能删东西。**
`squid-engine` 内部禁止直接调用 `std::fs::remove_*`。删除必须经过这一个函数，因此「安全校验 + 账本 + 废纸篓可还原」在架构上无法被绕过——新增引擎时想跳过检查，就只能显式地写出一条很难看代码评审的语句。

`execute` 做的事：丢弃嵌套重复项（父目录先删会让子项 ENOENT、体积重复计数）→ 对每个条目重新执行 `Engine::block_reason()`（锁、白名单、`SafetyPolicy`）→ 预先计算废纸篓目标路径 → 删除 → 写 ledger 会话。之所以要"重新校验"，是因为一次扫描可能跑了几分钟，磁盘早就不一样了。

## crate 依赖图

```
                 ┌──────────────┐
                 │  squid-core  │  模型 / 字节 / 路径 / 安全策略 / 设置 / 账本 / 进度
                 └──────┬───────┘
              ┌─────────┼──────────┐
       ┌──────▼─────┐ ┌─▼────────┐ │
       │ squid-     │ │ squid-   │ │  ← 只依赖 core
       │ platform   │ │ rules    │ │
       └──────┬─────┘ └─┬────────┘ │
              │         │          │
         ┌────▼─────────▼───────┐  │   ┌────────────┐
         │     squid-engine     │◄─┼───┤ squid-stats│ ← 只依赖 core
         └────┬────────────┬────┘  │   └─────┬──────┘
      ┌───────▼──┐    ┌────▼───────▼──┐      │
      │ squid-tui│    │   squid-cli   │◄─────┘
      └──────────┘    └───────────────┘
```

依赖方向严格单向，`squid-core` 不知道其它任何 crate 的存在。

### squid-core

| 模块 | 关键类型 | 职责 |
| --- | --- | --- |
| `model` | `CleanupItem`, `Category`, `ItemKind`, `DeleteMode`, `LockReason`, `CleanSummary`, `ScanReport` | 唯一的数据货币。`CleanupItem{path, kind, size, category, label, rule_id, selected, allocated, lock, note, anchors}` |
| `bytes` | `Bytes`, `format_thousands` | 逻辑大小 vs 实际占盘大小（`st_blocks * 512`）两套口径 |
| `paths` | `expand`, `home_dir`, `pretty`, `label_for` | `~` 展开、`~/…` 显示、中文安全的路径缩短 |
| `fs` | `measure`, `measure_all`, `kind_of`, `remove_item_with`, `trash_path`, `trash_target_for`, `empty_trash` | 体积测量（walkdir + rayon 并行，忽略符号链接）、安全删除、跨卷回收站 |
| `safety` | `SafetyPolicy` | 见「安全模型」 |
| `settings` | `Settings`, `locale()` | `config.toml`、白名单、规则开关、`SQUID_CONFIG_DIR`/`SQUID_STATE_DIR` |
| `ledger` | `Session`, `LedgerEntry`, `write_session`, `load_sessions`, `restore` | 删除账本与 `undo` |
| `progress` | `Progress`, `Event`, `CancelToken` | 引擎与 UI 之间唯一的沟通渠道；跨线程通道，取消是协作式的 |

### squid-platform

`Platform` trait 把「这个操作系统上 App 装在哪、残留放在哪、卷怎么枚举、删除前要不要先停服务」抽象出来：

```rust
fn app_sources(&self) -> Vec<AppSource>;
fn list_apps(&self, cancel: &CancelToken, size_apps: bool) -> Result<Vec<AppInfo>>;
fn leftover_roots(&self) -> Vec<LeftoverRoot>;
fn prepare_uninstall(&self, app: &AppInfo) -> LifecycleReport;   // 停 launchd / 注销 Launch Services
fn finish_uninstall(&self, app: &AppInfo, removed: &[PathBuf]) -> LifecycleReport;
fn volumes(&self) -> Result<Vec<Volume>>;
fn protected_prefixes(&self) -> Vec<PathBuf>;
fn running_executables(&self) -> Vec<String>;                    // 供 skip_if_running 使用
```

`LeftoverRoot { dir, naming, deep, category }` 描述一个残留目录以及它的命名方式（`BundleId` / `AppName` / `Either` / `GroupContainer`）。`deep` 默认 false —— `Application Support` 这种目录只匹配直接子项，绝不递归。

`leftovers::match_name` 负责把 `Xmind.app` 与 `com.xmind.win_zen.bundle`、`Xmind-state`、`Xmind Saved Application State` 之类关联起来。这里有两个曾经踩过的坑，回归测试已经锁住：

- 匹配前大小写要归一化（`Xmind` vs `xmind`）；
- `KNOWN_SUFFIXES` 带点而文件名切分后不带点，两侧必须用同一种归一化形式比较（`has_known_suffix`）。另外 `.pdf/.png/.mp3` 这类文档后缀绝不能出现在「按名称匹配」的后缀表里，否则 `Xmind-notes.pdf` 会被当成残留删掉。

### squid-rules

规则是 TOML，内置于 `crates/squid-rules/rules/macos.toml`（39 条），通过 `include_str!` 编译进二进制；用户规则从 `<config_dir>/rules/*.toml` 合并，同 `id` 覆盖内置。

```toml
[[rule]]
id = "xcode-derived-data"
category = "developer"
patterns = ["~/Library/Developer/Xcode/DerivedData/*"]
mode = "path"          # path = 删除条目本身；contents = 只清空目录内容
os = ["macos"]
protect = ["SomeKeep", "SomeKeep/*"]
min_age_days = 3
select_by_default = true
needs_admin = false
skip_if_running = ["Docker Desktop", "com.docker.backend"]
risky = true
note = "Xcode 会重新生成。"
```

核心是 `expand_pattern(pattern) -> Vec<Match{path, anchor}>`：

- `*` **不跨越路径分隔符**（globset 语义），所以 `~/Library/Caches/*` 只会命中一层；
- 每条匹配都携带 **anchor（锚点）** —— 模式的最长字面量前缀。锚点是删除时的权限边界：`SafetyPolicy` 只允许在锚点之下删除，且拒绝锚点自身。这条设计使得「一条规则写错」的最坏后果被限制在该规则自己的目录里；
- 不含通配符的字面模式（如 `~/.cargo/.package-cache`）匹配的是这个文件本身、锚点取其父目录。曾经有个 bug 是把锚点当成了匹配结果，等于提出删除整个 `~/.cargo`——已有回归测试 `wildcard_free_patterns_match_the_file_not_the_parent`。

`risky` 与 `skip_if_running` 是给「体积巨大但语义危险」的规则准备的刹车，典型是 `container-images`：Docker 的 VM 磁盘可能有 1 TB，一旦正在运行还去删就是数据事故，所以它既不默认勾选、又在检测到 Docker/OrbStack 进程时直接锁定。

### squid-stats

`Sampler::{new, prime, refresh}` 基于 `sysinfo`，两次采样之间才有 CPU%/网速，因此 `prime()` 是必须的预热步骤。产出 `SystemSnapshot{cpu, memory, network, disk, uptime_secs, top_processes, volumes, battery, host}` 与 `TrendStore`（滚动历史，给 TUI 的迷你折线/柱状图）。`battery.rs`、`human_uptime()` 提供 iStat 式细节。

### squid-engine

| 引擎 | 做法 |
| --- | --- |
| `clean` | 遍历规则 → `expand_pattern` → `protect`/年龄/白名单/运行中过滤 → `measure_all` 并行测体积 → `sort_for_display` 按分组排序。`--orphans` 会额外调用平台残留匹配，寻找已消失 App 的孤儿目录 |
| `uninstall` | `scan_apps` 建立 App 清单（名称、bundle id、体积、是否运行中、Homebrew cask）→ `find` 模糊定位 → `build_plan` 生成「本体 + 按 bundle id/名称匹配的残留」→ 用户确认 → `remove_plan`（含平台 `prepare_uninstall`/`finish_uninstall` 钩子）。共享给同厂商多个安装的残留会被 `LockReason::SharedWithOtherInstall` 锁住 |
| `purge` | 遍历 `purge_roots`（或用户给定路径，默认深度 6、最多 30 万目录），把 `ALWAYS`（node_modules、target、DerivedData、`.venv`…）识别为可再生；`AMBIGUOUS`（dist、build、out、venv…）只有当同目录存在 `package.json`/`Cargo.toml`/`go.mod` 等 PROJECT_MARKERS、或用户显式 `--all` 时才提出。`DO_NOT_DESCEND` 阻止进入 `.git`、Library、挂载点等。附带 APFS 本地快照（`com.apple.TimeMachine*`）与旧 iOS 备份、磁盘镜像等非目录项 |
| `analyze` | DaisyDisk 式体积树：`explore`/`expand` 逐层展开、`largest_files`、`buckets` 分类聚合。总量在遍历超限时标记为下限而非真值 |

### squid-tui

ratatui + crossterm。六个面板 `Cleanup / Uninstall / Purge / Storage / Stats / History`，`App` 是唯一状态机，键盘事件 → `Action` → 重绘。

- **绝不阻塞 UI**：每个引擎调用都通过 `job::spawn` 跑到独立线程，返回 `JobHandle<T>`；`Progress` 事件与最终结果走两条 crossbeam 通道，事件循环 `poll()` 非阻塞取结果。取消是协作式的 `CancelToken`。
- 每个 job 自己 `Engine::new()`，避免跨线程借用，代价是每次扫描重读一遍小 TOML。
- `theme::Strings::t(en, zh)` 集中管理双语文案，`Palette`/`card()` 复刻 CleanMyMac 的暗色卡片观感（背景 `Rgb(16,18,23)`、强调色 `Rgb(86,182,246)`、圆角卡片）。
  调色板只写 RGB，输出一律经 `Palette::resolve`：`SQUID_TRUECOLOR` → `COLORTERM` → `TERM_PROGRAM` → `TERM`
  依次探测真彩色能力，不支持时映射到 xterm-256（灰阶走 24 级灰条）。Terminal.app 只认 256 色，
  少了这一步它会把 `48;2;…` 当文字打出来，暗色主题直接变成白底黑字。
- **选中状态不能只靠颜色**：`Palette::selected_for(truecolor)` 在真彩色下是强调色条，降级终端里是
  `REVERSED | BOLD`（反白由终端自身的前景色/背景色完成，灰度终端同样成立）；勾选状态另有加粗 `●`
  与细体 `○` 两条冗余通道。`close_gaps` 负责让高亮行成为连续的一整条：列分隔用的是无样式
  `Span::raw(" ")`，反白时会在行里留下一道道暗缝，所以只在「整行非空白 span 样式一致」时把空白也染上；
  路径列只截断不补齐，因此 `item_row` 末尾还会按显示宽度补一段同色空格，让高亮条一直顶到卡片右边框。

### squid-cli

clap derive。约定：

- `--json` 时 stdout 只允许是数据。所有人类可读输出经 `DIAG_STDERR` 改道 stderr，`squid clean --json | jq` 永远干净；
- stdin 不是 TTY 且没有 `-y/--yes` 时，拒绝执行真实删除；
- 输出宽度按 `terminal_width()` 自适应，列宽用显示宽度（CJK 占两列）对齐，`clip` 截断 + `fit` 补空格。

## 安全模型（分层）

| 层 | 位置 | 拦截什么 |
| --- | --- | --- |
| 规则自我约束 | `squid-rules` | `protect`、`min_age_days`、`select_by_default=false`、`risky`、`skip_if_running`、`os` |
| 锚点边界 | `CleanupItem::anchors` + `SafetyPolicy::check` | 只允许在规则声明的锚点之下删除，且永不删除锚点自身 |
| 硬性禁区 | `safety::HARD_DENY` | `/`、`/usr`、`/System`、`/Library`、`/Applications`、`/private/var/folders`、`/Volumes`… 无论什么规则都拒绝 |
| 危险目录 | `safety::DENY_EXACT` | `~/Library`、`~/Documents`、`~/.ssh`、`~/.aws`、`~/Library/Application Support/MobileSync`… 允许删子项、禁止整目录递归删 |
| 深度 / 家目录 / 符号链接 | `SafetyPolicy{min_depth, allow_system_paths, follow_symlinks}` | 非 root 时拒绝家目录之外的路径（`OUTSIDE_HOME_OK` 例外）；递归删除必须有最小深度；永不跟随符号链接 |
| 卷 | `forbidden_mounts` | 由 `platform.volumes()` 注入，不删别的卷 |
| 用户白名单 | `Engine::is_whitelisted` | glob 命中即永不选中 |
| 运行时状态 | `LockReason` | App 正在运行、被其它安装共享、权限不属于当前用户 |
| 兜底 | 默认 Trash + `--dry-run` + 账本 + `undo` | 前面全漏了还能还原 |

`Engine::new()` 会根据是否 root 选择 `SafetyPolicy::system()` 或 `user_only()`，后者拒绝家目录之外的路径，
例外只有 `OUTSIDE_HOME_OK = ["/Applications"]`：管理员组的普通用户在 Finder 里把 App 拖进废纸篓不需要密码，
卸载器必须做同一件事。例外只覆盖 `/Applications` **里的条目**（目录本身仍在 `HARD_DENY`），
且 root 拥有的 App 早在 `build_plan` 就被 `writable_by_us` 锁住了。

### 事故记录：卸载「成功」了，App 还在

一次用户反馈「选中 Chatbox → d → 确认 → 毫无反应」，最后挖出四层独立原因，每一层都足以让事故静默：

1. `pending_plan` 从未被赋值（见下一节）；
2. 任务运行期间所有变更键被 `if self.busy() { return }` 静默吞掉，`confirm_delete()` 还会顺手清空
   `confirm` + `pending`——用户的「是」被吃掉了。现在统一走 `App::busy_refusal()`，明确提示「上一个任务
   还在进行，或按 c 取消」，并且**保留**审批；
3. **App 本体的锚点不存在**：`.app` 条目过去复用 `default_anchors()`，而那份锚点表里根本没有
   `/Applications`——它是「squid 被允许进去打扫的文件夹」清单，应用程序目录不是。于是每个卸载计划都是
   批准 6 项、删除 5 项，`.app` 带着「outside every declared cleanup anchor」被跳过。现在
   `bundle_anchors()` 取 bundle 的**父目录**作为锚点，既最小又不越权；
4. **非 root 走不到 `/Applications`**：`user_only()` 的家目录规则把系统 App 一律拒绝。见上面的
   `OUTSIDE_HOME_OK`。

同一次排查还修了 Plan 视图的一个隐性错位：它用 `index == 0` 认定第一行是「应用本体」，而
`build_plan` 是按体积降序排序的——一旦某个容器比 App 大，就有一项残留被藏进表头、下面每行都错一位。
现在按 `rule_id == "uninstall-bundle"` 识别，行与光标严格一一对应。

教训写进断言里：`the_bundle_itself_passes_the_safety_policy`、
`a_plain_user_can_still_uninstall_into_applications`、`a_busy_app_answers_instead_of_ignoring_you`、
`the_plan_view_shows_the_bundle_and_every_leftover`（后者读回测试缓冲区：三项计划必须画出三个勾选行、
表头文案不得占据其中任何一行、加粗的那一行永远是 `.app`、光标落在哪一行就高亮哪一行）。
删除路径的测试必须把沙盒建在**真实形态**的位置上：早先的卸载测试把假 App 放在 `~/Library/Caches/…` 下，
正好落在锚点里，所以这个 bug 一路绿灯。

## 数据流

```
config.toml ─┐
             ├─► Engine{platform, settings, rules, safety, whitelist}
rules/*.toml ┘                         │
                                       ├── scan() ─► Vec<CleanupItem> ─► CLI 表格 / TUI 面板
                                       │                                       │ 用户勾选
                                       │                                       ▼
                                       └── delete::execute(DeleteJob) ─► SafetyPolicy 复检
                                                  │                       fs::remove_item_with
                                                  │                       ledger::write_session
                                                  ▼
              ~/Library/Application Support/squid/history/<session>.json
                                                  │
                                          squid undo ─► ledger::restore ─► rename 回原路径
```

## 性能

体积测量是这类工具唯一的重活。做法：`walkdir`（`contents_first(false)`、`max_open(64)`、跳过符号链接）单目录内顺序遍历，`measure_all` 在条目之间用 rayon 并行（线程数 `clamp(4,16)`），每完成一个条目就发一次进度事件，所以 UI 的进度条是真实进度而不是假动画。`purge` 用 `max_entries` 限制单项目文件数、`MAX_DIRS` 限制总目录数，超限时明确标注结果是下限。实测 `clean --dry-run` 在 18 万条目 / 30 GB 量级上约 5 秒。

## 跨平台移植指南

新增一个平台 = 实现 `Platform` + 提供规则文件，**引擎与 UI 一行都不用改**。

1. 新建 `squid-platform/src/<os>.rs`，实现 `Platform`；在 `lib.rs::current()` 里返回它。
2. 决定 `app_sources()` 与 `leftover_roots()`：命名方式（bundle id / 名称 / 容器）决定残留匹配器怎么配。
3. 补 `safety::HARD_DENY` / `DENY_EXACT` 里该系统的禁区（Windows：`C:\Windows`、`C:\Program Files`；Linux：`/snap`、`/nix`、`/var/lib`）。
4. 新增 `crates/squid-rules/rules/<os>.toml`，并在 `RuleSet::builtins()` 里 `include_str!`。规则里的 `os = [...]` 字段决定它在哪个平台生效。
5. 回收站语义：`fs::trash_target_for` 需要该系统的回收站位置与命名规则（macOS 是 `~/.Trash` 与卷根 `.Trashes/<uid>`；Linux 是 XDG `~/.local/share/Trash/files`；Windows 需要 `$Recycle.Bin` 或 Shell API）。
6. 提权：macOS/Linux 用 `sudo` 重跑（`needs_admin` 的规则在非 root 时只报告不执行）；Windows 需要 UAC。

现状：macOS 完整；Linux 有 `.desktop`/Flatpak/Snap 与 XDG 残留实现，缺规则文件与真机验证；Windows 是占位实现。

## 测试策略

86 个测试（`cargo test --workspace`）。三类：

- **纯逻辑**：字节格式化、glob 与锚点、规则解析、名称匹配、安全策略拒绝表 —— 这些是最有价值的，因为它们把「曾经出过的事故」变成断言。
- **引擎级**：在临时目录 / `~/Library/Caches/squid-delete-test-*` 里搭沙盒，跑真实 scan + delete。
- **状态隔离**：删除类测试必须调用 `isolate_state()`，用 `SQUID_STATE_DIR` 把账本重定向到临时目录，否则测试会话会污染用户真实的 `squid history`。

约定：CI 门禁是 `cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace --no-fail-fast`。任何删除相关改动必须同时提供 Trash 往返（delete → undo → 字节一致）测试。

### 确认框背后的不变量：`Approval` 枚举

TUI 的「按 d → 确认 → 执行」曾经出过一次典型的静默失败：`App` 上有两个平行字段
`pending: Option<Vec<CleanupItem>>` 与 `pending_plan: Option<UninstallPlan>`，卸载流程只写了
后者对应的读逻辑，`pending_plan` 从来没被赋成 `Some`，于是 `confirm_delete()` 里
`let Some(plan) = self.pending_plan.take() else { return }` 直接返回——用户按了回车，什么也没发生，
什么也没说。

修复方式不是补一行赋值，而是让非法状态无法表达：两个字段合并成
`pending: Option<Approval>`，`Approval` 是 `Items { items, purge } | Plan(UninstallPlan) |
UndoSession(String)` 三个变体。`ask_delete()` 必须构造其中一个才能打开确认框；确认框能打开，
就必然有可执行的内容。`confirm_delete()` 对 `None` 的情况显式 toast「没有待确认的内容，请重新按 d」，
把「静默」变成「可见」。`app.rs` 里那三个回归测试直接驱动 `ask_delete() → confirm_delete() → poll()`
到任务结束，断言 `outcome.summary.removed == N`，覆盖卸载、清理/深度清理、以及空确认框三条路径。

## 关于 Mole（GPL v3）的边界

仓库根目录的 `Mole/` 是 GPL v3 的参考实现，已写入 `.gitignore`。squid 是净室实现：

- 没有任何一行 Rust 代码来自 Mole（Mole 也不是 Rust 写的）；
- 参考的是**行为**：哪些路径值得清理、哪些目录是雷区、卸载残留该去哪儿找；
- 规则表是本项目的 `macos.toml`，模式与文案均为本仓库独立编写；
- 分发 squid（MIT OR Apache-2.0）时不携带、不链接 GPL 代码。
