# 安全说明

squid 是一个「以删除为职责、以不删错为底线」的工具。它拥有当前用户的完整文件权限，因此威胁模型与普通的 CLI 不同：**最大的风险不是被攻击，而是它自己删错东西。**

## 权限边界

- squid 以当前用户身份运行，**不需要** root，也不安装任何 launch agent / 服务 / 内核扩展。
- 非 root 时 `SafetyPolicy::user_only()` 生效，家目录之外的路径一律不删除；只有显式用管理员权限运行时才切换到 `SafetyPolicy::system()`。
- 需要管理员权限的规则（`needs_admin`）在普通权限下只报告、不执行；`squid clean --user-only` 可以彻底跳过它们。
- 默认不联网。清理、卸载、深度清理与实时监控全都在本地完成，代码里没有 HTTP 客户端
  （下载规则包与应用更新走 `curl` / `Invoke-WebRequest` 子进程，见下）。**squid 从不上传
  任何遥测、路径、哈希、扫描结果或被隔离的文件。**

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

安全功能新增的外部命令，同一套规则（不经过 shell、参数不来自不可信输入）：

  | 命令 | 用途 | 是否修改系统 |
  | --- | --- | --- |
  | `/usr/bin/xprotect status/version`、`spctl --status` | 读取 XProtect 与 Gatekeeper 状态 | 否，只读。**`xprotect update` 永不调用**：那台机器要不要向 Apple 取定义，不是第三方 App 能替用户决定的 |
  | `powershell.exe`（Defender / SecurityCenter2 查询）、注册表 `Microsoft\Windows Defender` 的 `Signature Updates` | 读取 Defender / 第三方杀软与定义版本 | 否，只读；不会开启、关闭或更新任何防护 |
  | `codesign -dvvv --display` | 读取发布者的签名信息 | 否，只读 |
  | `launchctl bootstrap/kickout` | 启用或停用**当前用户**的 LaunchAgent | 是，仅在启动项页确认之后；系统级条目在首版只读 |
  | `schtasks /query /v /fo csv` | 列举计划任务 | 否，只读（首版不提供启停）|
  | `curl` / `Invoke-WebRequest` | 下载规则包与更新清单 | 否，只下载；见「规则包的下载与验签」|

## 安全功能（guard）的边界

恶意软件扫描是 0.1.6 加入的能力，它的风险模型和「删错东西」不同：**它会移动文件**。
因此下面几条是被测试钉住的硬边界，而不是承诺。

- **扫描本身绝不删除或移动任何东西。** 唯一的移动来自隔离，而隔离必须由用户点下确认，
  或由一个默认关闭的开关（`security.auto_quarantine`）触发。
- **隔离只接受后端发出的 finding id。** 前端提交路径的能力被拿掉了：`security_quarantine`
  拿 id 去查后端自己存下的那次扫描，再核对哈希、大小与文件身份；扫描之后文件被换过就
  拒绝移动。界面伪造一个路径出来，伪造不了一个 id。
- **只有 `confirmed` 可能自动隔离。** 这一级只能由精确 SHA-256 命中或签名级 YARA 规则到达；
  启发式（无签名、路径可疑、开机自启）的上限是 `suspicious`，永远自动隔离不到它。
- **隔离是可逆的。** 对象进入 `<state>/security/quarantine/`，去掉执行位、权限收到仅属主，
  账本记录原路径、模式、时间戳与哈希；恢复要求原字节、原权限一致，恢复前要确认。
  永久删除只能从隔离区发起。
- **不跟随符号链接，不隔离系统目录、XProtect、squid 自身和隔离区自身。**
- **读不到不等于干净。** 每个扫描根的结果都记录在覆盖范围里（walked / denied / absent），
  有缺口时界面必须同时列出未检查的范围，不能只显示「安全」。系统防护读不到时显示
  「无法确认」，不显示绿色。
- **清理白名单与安全排除项是两份独立配置。** 把一个目录从清理里保护起来，不会让它
  从此不被扫描；反过来，`config.toml` 的 `[security] scan_exclusions` 会让里面的威胁
  不再被报告，所以新增排除项要二次确认。

## 规则包的下载与验签

- 内置规则离线可用；更新只是**下载**，没有请求携带任何本地信息——没有文件名、没有哈希、
  没有扫描结果、没有被隔离的文件。
- 规则包用**独立于安装包的 Ed25519 密钥**签名（`SQUID_INTEL_PUBKEY`）。轮换发布密钥不应该
  顺手改变谁能推规则，泄漏了安装包密钥也不应该变成一条规则注入路径。
- 清单必须验签，包必须核对大小与 SHA-256，而且必须在**装进去之前**先编译一遍：一份
  YARA 拒绝的规则包永远不能成为 `current`，否则下一次扫描会在最不该失败的时候失败。
- manifest 与规则源码存在同一个文件里，读到什么就一起验什么，避免「新包配旧 manifest」。
- 验签失败、损坏或主动回滚时继续使用上一版可用规则，并且清空判定缓存——旧规则给出的
  结论不是新规则给的结论。
- 未配置 feed 的构建（比如从源码直接 `cargo build`）不会联网，只用内置规则。

## 什么不属于安全问题

- 规则没有覆盖到某个 App 的缓存 → 功能缺口，开 issue 即可。
- 某个恶意样本没被扫出来 → squid 不是 XProtect / Defender 的替代品，也不做实时拦截；
  它是本地按需扫描 + 可逆隔离 + 启动项与进程的可见性。系统防护仍在做实时那部分。
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
