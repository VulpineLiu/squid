# Nex 应用视觉规范

版本：1.0
状态：Nex 系列应用统一标准
基准实现：`nex_shift` 的主色系

## 1. 原则

Nex 系列应用共用一套暖色品牌体系。Shift 已经使用的橙色是主品牌色，
Squid 以及后续 Nex 应用不得再各自定义蓝色或其它颜色的主操作色。

- 主操作、选中态、焦点环和进度使用 Nex 主色。
- 页面底色、卡片和正文使用暖中性色，不使用冷蓝色底。
- 绿色、红色、黄色只表达成功、危险和警告，不作为品牌主色。
- 分类色可以保留各自的语义色，但不得替代主操作色。
- 新界面必须引用 `src/styles.css` 中的 `--nex-*` 常量，不写新的主色十六进制值。

## 2. 品牌常量

这些值同时写在 `src/styles.css` 的 `:root` 中，是运行时唯一来源。

| 常量 | 值 | 用途 |
| --- | --- | --- |
| `--nex-primary` | `#F26A33` | 主按钮、选中态、焦点、进度 |
| `--nex-primary-hover` | `#E45C2A` | 主色悬停 |
| `--nex-primary-strong` | `#D94F1F` | 主色深阶、主按钮渐变终点 |
| `--nex-primary-pressed` | `#C94A1D` | 按下态 |
| `--nex-primary-soft` | `#FFF0E7` | 主色浅底、权限提示 |
| `--nex-primary-tint` | `#FFF7F2` | 选中卡片、弱高亮 |
| `--nex-ink` | `#342019` | 正文 |
| `--nex-ink-strong` | `#24120A` | 标题 |
| `--nex-muted` | `#7D6256` | 次级文字 |
| `--nex-muted-soft` | `#A98F80` | 说明文字 |
| `--nex-line` | `rgba(89, 51, 34, 0.10)` | 分隔线 |
| `--nex-paper` | `#FBF7F2` | 面板 |
| `--nex-cream` | `#FFF6EE` | 浅色表面 |
| `--nex-canvas` | `#F5EEE7` | 应用底色 |

主色高亮需要透明度时使用 `rgba(var(--nex-primary-rgb), α)`，其中
`--nex-primary-rgb` 为 `242, 106, 51`。

## 3. 图标渐变

应用图标的品牌渐变沿用 Shift：

- 起点 `#FFB25C`
- 中段 `#FF6B35`
- 终点 `#C66BFF`

图标主体的描边和阴影必须使用暖色，不得保留旧版蓝色。

### 侧栏品牌锁定

侧栏标识直接使用打包进应用的 `src-tauri/icons/128x128.png`，不得维护第二份
图标资源。默认视觉为 37 × 37 px、12 px 圆角。

- 一级标题固定为 `Squid`，使用 `--nex-text-lg` / `--nex-weight-display`。
- 中文二级标题固定为 `给你的电脑洗个澡`；英文为
  `Give your computer a bath`，使用 `--nex-text-3xs` / `--nex-weight-body`。
- 所有 Nex 应用使用同一锁定结构：应用图标、产品名、产品定位短句。

## 4. 字体规范

字体与 Shift 使用同一套系统字体栈和字号阶梯。所有新界面必须引用下面的
常量，不得再写 7–10px 等阶梯外的字号。

| 常量 | 值 | 用途 |
| --- | --- | --- |
| `--nex-font-sans` | `-apple-system, BlinkMacSystemFont, "SF Pro Display", "Segoe UI", "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif` | 界面正文 |
| `--nex-font-mono` | `"SF Mono", "Cascadia Code", ui-monospace, monospace` | 路径、哈希、配置值 |
| `--nex-text-3xs` | `11px` | 最小辅助文字与标签 |
| `--nex-text-2xs` | `12.5px` | 正文说明、列表文字 |
| `--nex-text-xs` | `13.5px` | 面板标题 |
| `--nex-text-sm` | `14.5px` | 强调正文 |
| `--nex-text-md` | `16px` | 区块标题 |
| `--nex-text-lg` | `18px` | 品牌与重点标题 |
| `--nex-text-xl` | `21px` | 对话框标题 |
| `--nex-text-hero` | `clamp(28px, 3vw, 38px)` | 页面主标题 |

字重沿用 Shift 的层级：`540` 正文、`620` 中等、`650` 半粗、`680` 粗体、
`720` 重体、`760` 品牌展示。对应常量为 `--nex-weight-body`、
`--nex-weight-medium`、`--nex-weight-semibold`、`--nex-weight-bold`、
`--nex-weight-heavy`、`--nex-weight-display`。

行高常量：`--nex-leading-tight: 1.1` 用于标题，
`--nex-leading-body: 1.55` 用于说明正文。

## 5. 组件规则

- 主按钮：`#FF9664 → --nex-primary → --nex-primary-strong` 的暖色渐变。
- 次按钮：暖白表面、暖棕文字，悬停时使用主色描边。
- 选中卡片：`--nex-primary-tint` 背景与 10% 主色描边。
- 焦点环：`rgba(var(--nex-primary-rgb), 0.55)`，宽度 2px。
- 进度条、开关、勾选框：主色或主色渐变。
- 正文不使用纯黑；标题使用 `--nex-ink-strong`。
- 更新面板面向使用者，只展示当前系统版本、版本状态、更新说明、进度与操作；
  更新地址、安装包文件名和哈希留在后端校验流程中，不进入常规界面。

## 6. 旧变量兼容

现有组件仍引用 `--blue`、`--blue-deep`、`--blue-soft`。这三个变量已经
分别别名为 `--nex-primary`、`--nex-primary-strong`、`--nex-primary-soft`，
因此不会再渲染蓝色。新增代码应直接使用 `--nex-*` 常量；迁移旧组件时
可以逐步移除 `--blue-*` 别名，但不得改回独立颜色值。

## 7. 验收

- 全应用不出现蓝色主按钮、蓝色选中态或蓝色焦点环。
- 页面底色为 `#F5EEE7`，面板为暖白，不使用冷蓝渐变。
- Settings、首次启动权限卡和更新面板共享同一套主色。
- 应用图标与界面使用同一组暖色品牌常量。
- 正文和标签不小于 11px，标题使用统一字号阶梯，不出现 7–10px 的正文。
