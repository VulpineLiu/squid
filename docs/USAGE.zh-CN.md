# Squid 使用帮助

适用于 Squid 0.1.11。项目说明见 [`README.zh-CN.md`](../README.zh-CN.md)，版本历史见 [`CHANGELOG.md`](../CHANGELOG.md)。

## 1. 下载与安装

Squid 的桌面安装包发布在 origin 仓库的 `releases` 分支。GitHub 只镜像源码，不承载这个分支。

- 发布目录：<https://git.unissense.tech/mula/nex_squid/src/branch/releases>
- 更新清单：<https://git.unissense.tech/mula/nex_squid/raw/branch/releases/squid-update.json>

当前版本 0.1.11：

| 平台 | 安装包 |
| --- | --- |
| macOS 12+ | [`Squid-0.1.11-macOS-universal.dmg`](https://git.unissense.tech/mula/nex_squid/raw/branch/releases/Squid-0.1.11-macOS-universal.dmg) |
| Windows 10/11 x64 | [`Squid-Setup-0.1.11-win-x64.exe`](https://git.unissense.tech/mula/nex_squid/raw/branch/releases/Squid-Setup-0.1.11-win-x64.exe) |

两个安装包都是未签名构建。校验文件大小和 SHA256 的方法见发布目录中的 `SHA256SUMS.txt`。

### macOS

1. 打开 DMG，把 `Squid.app` 拖进“应用程序”。
2. 第一次启动请在访达中右键 `Squid`，选择“打开”。
3. 如果系统提示“已损坏，无法打开”，去掉下载隔离属性后再启动：

   ```bash
   xattr -dr com.apple.quarantine /Applications/Squid.app
   open /Applications/Squid.app
   ```

4. Squid 不需要 root，也不会安装后台服务、内核扩展或特权 helper。

### Windows

1. 运行 `Squid-Setup-<version>-win-x64.exe`。
2. SmartScreen 提示时选择“更多信息” > “仍要运行”。
3. 默认安装到 `%LOCALAPPDATA%\Programs\Squid`，不需要管理员权限。
4. 如果安装器无法把临时文件写入 `%TEMP%`（部分 360 / 电脑管家环境），使用发布目录里的 `一键安装.bat`：它先把安装包复制到 C 盘的 `%TEMP%`，再启动安装。
5. 如果启动后窗口空白，安装 Microsoft Edge WebView2 Evergreen Runtime 后重试：
   <https://developer.microsoft.com/microsoft-edge/webview2/>

## 2. 首次启动

- 界面语言默认跟随系统，可在“设置 → 常规”切换简体中文或 English。
- 首次启动会显示一次可选的权限检查卡片，不会强制弹出系统权限窗口，也不会阻止应用启动。
- 关闭主窗口只会隐藏窗口，Squid 继续驻留托盘；只有托盘菜单里的“退出 Squid”才会结束进程。
- “设置 → 启动”可以打开开机自启。它只写入当前用户级启动项，不需要管理员权限。

## 3. 桌面端

### 左侧菜单

- **维护**：概览、日常清理、应用卸载、深度清理、权限管理（仅 Windows）
- **保护**：安全、启动项
- **洞察**：存储空间、实时监控
- **底部**：历史记录、设置

macOS 使用 `⌘1`–`⌘8`，Windows 使用 `Ctrl+1`–`Ctrl+9` 按侧栏顺序切换页面。macOS 隐藏 Windows 专属的权限管理，后面的页面顺延。

### 日常清理

1. 打开“日常清理”，选择扫描范围。
2. 扫描完成后按分类检查结果；每个分类都可以单独取消勾选。
3. 选择删除方式：默认进废纸篓/回收站，也可以显式选择永久删除。
4. 点击清理，并在确认框中确认。

规则覆盖应用与浏览器缓存、日志、崩溃报告、开发缓存、升级安装包和废纸篓/回收站。macOS 内置 38 条规则，Windows 内置 30 条规则。所有删除命令都支持 `--dry-run`（CLI），桌面端则先扫描再选择。

### 应用卸载

1. 打开“应用卸载”，搜索应用名称。
2. 选中应用后，Squid 会把应用本体与偏好、缓存、容器或 AppData 残留组成同一个卸载计划。
3. 检查计划中的每一项，确认后执行。

残留匹配会排除常见文档后缀，避免把用户文档当成应用残留。

### 深度清理

“深度清理”查找 `node_modules` 等可重建构建产物、磁盘镜像、安装包和陈旧下载。默认项目根目录可以在配置里的 `purge_roots` 修改；`purge_min_age_days` 用来保护近期仍在使用的项目。

### 存储空间

选择一个目录后，Squid 逐层测量目录体积，并列出最大子项和最大文件。存储分析只在用户选择或配置目录后读取，文件内容不会上传。

### 实时监控与进程

- 每两秒采样 CPU、内存、网络、磁盘、电池、卷和高占用进程。
- “进程”页签列出全部进程及其签名状态与启动项来源。
- 结束进程前 Squid 会重新采样并核对名称，拒绝 Squid 自身、系统首进程和关键系统进程。
- 可以把实时监控固定成桌面小工具；小工具设置从顶部齿轮下拉打开。

### 安全

- 读取 XProtect / Defender 状态，不会替你关闭或修改系统防护。
- 按哈希、YARA 规则与启发式扫描恶意软件，发现分为“已确认 / 可疑 / 提示”三级。
- 已确认项可以隔离，也可以从隔离区恢复。
- 安全页支持按发布者、应用目录或提示类型信任正常软件；已确认威胁不会被白名单静默。
- 不上传文件名、哈希、扫描结果或被隔离的文件。

### 启动项与权限管理

- “启动项”列出 LaunchAgent、LaunchDaemon、登录项、注册表 Run/RunOnce、启动文件夹、计划任务与服务；当前用户拥有的条目可以停用和恢复。
- Windows 的“权限管理”按软件分组启动项，并提供右键菜单扩展、弹窗规则与后台监听。

### 历史记录与撤销

每次真实删除都会写入本地账本。仍在废纸篓/回收站中的内容可以在“历史记录”里还原；CLI 使用 `squid history` 和 `squid undo`。

### 软件更新

- 默认每天最多自动检查一次，也可以在“设置 → 更新”手动检查。
- 更新清单来自 origin 的 `releases` 分支，不来自 GitHub。
- 安装包必须匹配清单中的精确大小和 SHA256；带发布公钥的构建还会校验清单的 Ed25519 签名。
- 下载完成后可以安装并重启，或清除已下载文件。

## 4. CLI 与 TUI

安装：

```bash
cargo install --path crates/squid-cli --locked
```

或者从源码构建：

```bash
cargo build --release -p squid-cli
```

常用命令：

| 命令 | 作用 |
| --- | --- |
| `squid state` | 打开 TUI |
| `squid status` | 输出系统快照 |
| `squid clean --dry-run` | 预览可清理内容 |
| `squid clean -y` | 按规则执行清理 |
| `squid uninstall --list` | 列出已安装应用 |
| `squid uninstall <NAME>` | 生成应用本体加残留的卸载计划 |
| `squid purge <PATH>` | 清理项目构建产物 |
| `squid analyze <PATH>` | 输出目录体积树与最大文件 |
| `squid history` | 查看删除账本 |
| `squid undo [ID]` | 还原最近一次或指定废纸篓会话 |
| `squid whitelist add\|list\|remove` | 维护保护路径 |
| `squid rules` | 列出当前生效的规则 |
| `squid doctor` | 检查权限、配置、历史文件和生效路径 |
| `squid guard status` | 报告 XProtect / Defender 状态、规则包版本、启动项数量与隔离区占用 |
| `squid guard scan [PATHS…]` | 扫描恶意软件；`--full` 全盘，`--json` 输出 JSON，`-y` 自动隔离已确认项 |
| `squid guard startup` | 列出登录/启动条目；`--all` 含系统自带，`--risky` 只看需注意的 |
| `squid guard quarantine <ID>` / `restore <ID>` | 隔离或恢复指定发现 |
| `squid guard held` | 列出隔离区内容与原因 |

所有删除命令都支持 `--dry-run`。非交互环境必须显式使用 `-y/--yes`，否则 Squid 会拒绝执行删除。完整参数运行 `squid <command> --help`。

## 5. 配置与数据

| 用途 | macOS | Windows |
| --- | --- | --- |
| 配置与规则 | `~/Library/Application Support/squid/` | `%APPDATA%\squid\` |
| 历史账本 | `~/Library/Application Support/squid/history/` | `%APPDATA%\squid\history\` |
| 删除去向 | `~/.Trash` 或永久删除 | 回收站或永久删除 |

Linux 使用 XDG 目录。桌面端与 CLI/TUI 共用配置和历史。

常用配置（`config.toml`）：

```toml
delete_mode = "trash"
history_keep = 60
purge_min_age_days = 7
purge_max_entries = 200000
show_allocated = true
# 留空表示跟随系统；也可以设为 "en" 或 "zh"
language = ""
whitelist = ["~/Library/Caches/ImportantApp"]
disabled_rules = ["user-trash"]
purge_roots = ["~/code", "~/Projects", "~/Desktop"]
```

常用环境变量：

| 变量 | 作用 |
| --- | --- |
| `SQUID_CONFIG_DIR` | 覆盖配置目录 |
| `SQUID_STATE_DIR` | 覆盖状态与历史目录 |
| `SQUID_LANG` | 强制 CLI/TUI 使用 `zh` 或 `en` |
| `SQUID_UPDATE_URL` | 覆盖桌面端更新清单地址，适合测试发布通道 |
| `SQUID_TRUECOLOR` / `NO_COLOR` | 控制 TUI 颜色 |
| `SQUID_SYMBOL_CELLS` / `SQUID_CONSOLE_TRACE` | 排查 Windows 控制台字符宽度与排版 |

## 6. 安全边界

1. 桌面界面只能提交扫描结果中的索引，不能提交任意待删除路径。
2. 执行前重新构造并校验路径所有权、安全锚点、保护列表、白名单和锁定状态。
3. 默认删除到废纸篓/回收站；永久删除必须显式选择。
4. 系统根目录、`/usr`、`/System`、`/Applications` 等位置不能被递归删除，符号链接不会被跟随。
5. 每次真实删除写入本地账本，仍可还原的内容可以撤销。
6. 结束进程前重新采样并核对名称，拒绝关键系统进程。

## 7. 卸载

### macOS

1. 退出 Squid。
2. 把 `/Applications/Squid.app` 移到废纸篓。
3. 如需同时删除配置和历史：

   ```bash
   rm -rf "$HOME/Library/Application Support/squid"
   ```

### Windows

1. 在“设置 → 应用”中卸载 Squid。
2. 如需同时删除配置和历史，删除 `%APPDATA%\squid`。

### CLI

```bash
cargo uninstall squid-cli
```

## 8. 故障排查

### macOS 提示“已损坏，无法打开”

这是未签名构建被 Gatekeeper 隔离，不是文件损坏。右键“打开”仍然不行时：

```bash
xattr -dr com.apple.quarantine /Applications/Squid.app
```

### Windows 启动后窗口空白

安装 WebView2 Evergreen Runtime 后重新启动 Squid。

### Windows 安装器提示无法写入临时文件

安装包在非系统盘或受端点防护限制的目录运行时，NSIS 可能无法解压到 `%TEMP%`。把安装包复制到 C 盘的 `%TEMP%` 再运行，或使用发布目录里的 `一键安装.bat`。

### 清理结果比预期少

检查是否授予了完整磁盘访问（macOS），以及 `whitelist`、`disabled_rules` 和安全页的“已信任”列表。没有完整磁盘访问时，受 TCC 保护的位置会被跳过，安全页会把覆盖范围标为“受限”。

### 更新检查失败

确认能访问 origin 的发布通道：

<https://git.unissense.tech/mula/nex_squid/raw/branch/releases/squid-update.json>

代理或离线环境可以设置 `SQUID_UPDATE_URL` 指向可访问的镜像清单。

### TUI 排版异常

尝试设置 `SQUID_TRUECOLOR=0`、`NO_COLOR=1`，或在 Windows 控制台设置 `SQUID_SYMBOL_CELLS` / `SQUID_CONSOLE_TRACE` 排查字符宽度。

## 9. 更多文档

- [项目说明](../README.zh-CN.md)
- [更新日志](../CHANGELOG.md)
- [架构说明](ARCHITECTURE.md)
- [桌面端说明](DESKTOP.md)
- [发布与更新](RELEASE.md)
- [安全说明](SECURITY.md)
