# 安全说明

squid 是一个「以删除为职责、以不删错为底线」的工具。它拥有当前用户的完整文件权限，因此威胁模型与普通的 CLI 不同：**最大的风险不是被攻击，而是它自己删错东西。**

## 权限边界

- squid 以当前用户身份运行，**不需要** root，也不安装任何 launch agent / 服务 / 内核扩展。
- 非 root 时 `SafetyPolicy::user_only()` 生效，家目录之外的路径一律不删除；只有显式用管理员权限运行时才切换到 `SafetyPolicy::system()`。
- 需要管理员权限的规则（`needs_admin`）在普通权限下只报告、不执行；`squid clean --user-only` 可以彻底跳过它们。
- 不联网：`Cargo.toml` 的依赖树里没有 HTTP 客户端，squid 不上传任何遥测、路径或文件名。

## Full Disk Access（macOS）

`~/Library/Safari`、`~/Library/Mail`、`~/Library/Messages` 等目录受 TCC 保护。终端未授权时，squid 只会读不到、跳过它们，不会报错崩溃。`squid doctor` 会列出被拒绝的目录。若希望这些规则真正命中，请在 系统设置 → 隐私与安全性 → 完整磁盘访问权限 中勾选你运行 squid 的终端。

注意：**授权 Full Disk Access 增加的是读取能力，不是删除范围**——上面那张安全表（`HARD_DENY`/`DENY_EXACT`/锚点）在任何权限级别下都同样生效。

## 删除的多重防线

1. 默认 `Trash`：删除等于 `rename` 到回收站，`squid undo` 可以还原。
2. `--dry-run`：所有破坏性命令都支持。
3. 非交互确认：stdin 不是 TTY 且没有 `-y/--yes` 时直接拒绝执行删除（保护 CI 与 `curl | sh` 式误用）。
4. 删除时二次校验：不复用扫描结论，逐条重新检查锁、白名单、安全策略与锚点。
5. 永不跟随符号链接：`walkdir::follow_links(false)` + `symlink_metadata` 判定类型，避免把「指向 `~/Documents` 的软链接」当成缓存删掉。
6. 账本：每次真实删除写一个会话文件（含每条的原始路径与废纸篓目标），`history_keep` 控制保留数量。
7. 白名单：`squid whitelist add <glob>` 之后该路径在任何面板中都不再被选中。

## 用户输入与外部数据

- **配置文件与规则文件按 TOML 解析，绝不当作代码执行。** 解析失败时降级为默认值，错误通过 `squid doctor` / 引擎的 `settings_errors` 报告，不会 panic。
- 规则模式经 `globset` 编译，`*` 不跨越路径分隔符；`~` 只在家目录下展开。
- 卸载残留的名称匹配区分大小写归一化，且显式排除了文档后缀（`.pdf`、`.png`、`.mp3`…），避免把用户文档当成 App 残留。
- 外部命令一律用 `Command::new(program).args([...])` 直接传参，**不经过 shell**，参数也不来自不可信输入，因此不存在命令注入面。清单如下：

  | 命令 | 用途 | 是否修改系统 |
  | --- | --- | --- |
  | `sw_vers`、`sysctl`、`ps`、`df`、`diskutil`、`pmset`、`ioreg`、`vm_stat`、`tmutil list` | 系统信息与体积探测 | 否，只读 |
  | `open`、`xdg-open`、`explorer` | 打开废纸篓 / 在访达中显示 | 否 |
  | `git check-ignore`、`git ls-files`、`tmutil listlocalsnapshots` | 判断构建产物是否被 git 忽略（被跟踪的文件绝不删除）、列举 APFS 本地快照 | 否 |
  | `launchctl`、`lsregister` | 卸载前停掉 App 的 launchd 任务、注销 Launch Services 注册 | 是，仅 `squid uninstall` 确认之后 |
  | `brew uninstall --cask --force` | App 由 Homebrew 安装时优先交给 brew 处理 | 是，仅 `squid uninstall` 确认之后，且只在检测到对应 cask 时 |
  | `flatpak` / `snap` | Linux 应用清单与卸载建议 | 清单只读；卸载建议以提示形式给出 |

## 什么不属于安全问题

- 规则没有覆盖到某个 App 的缓存 → 功能缺口，开 issue 即可。
- 磁盘空间没有你预期释放得多 → 可能是 APFS 快照或 purge 的年龄过滤，先用 `squid analyze` 对照。
- 需要 Full Disk Access 才能读到的目录 → macOS 的限制，见上。

## 报告漏洞

如果你发现 squid **可能删除用户未主动选择的数据**（例如某条规则在默认配置下选中了 `~/Documents`、锚点校验可被绕过、`undo` 覆盖已有文件），请视为安全问题：

- 通过仓库的 private issue / security contact 私下披露；
- 附上最小复现（哪个命令、哪个规则 id、哪个路径）；
- **先跑 `--dry-run` 采集证据**，不要提供真实删除的日志。

我们会在修复版本发布后再公开细节。

## 你自己的责任

- squid 不是备份工具。重要数据请依赖 Time Machine 或对象存储。
- `--permanent` 跳过回收站，`undo` 也救不回来；除非你确切知道自己在做什么，否则不要用。
- 自定义规则里 `mode = "contents"` 会清空目录内容，`patterns` 只写一个字面目录（例如 `~/Library`）会被安全策略拒绝——如果你绕过或删除了 `crates/squid-core/src/safety.rs` 的守卫，squid 就不再保护你。
