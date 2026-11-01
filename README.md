# InvoiceBox · 发票收取助手

用 IMAP 收邮件 → 自动抓出 PDF 发票（**附件** + **正文里的下载链接**）→ 解析日期 / 金额 / 销售方 / 发票号 → 按模板**批量重命名**、**批量导出**。

Tauri 2 桌面窗口，目标平台 **Windows**。前端是纯 HTML/CSS/JS，**没有前端打包步骤**；Rust 与安装包全部由 **GitHub Actions** 编译，本机不需要装 Rust。

## 功能一览

| 能力 | 说明 |
| --- | --- |
| 收信 | 任意 IMAP 邮箱，支持 SSL/TLS（993）与 STARTTLS（143） |
| 抓附件 | 按扩展名 / `application/pdf` / `%PDF` 魔数三重判定，不怕附件名乱写 |
| 抓链接 | 扫描正文与 HTML 里的下载链接，命中 `invoice/pdf/发票/账单…` 关键词才尝试，下载后校验内容类型 |
| 去重 | SHA-256 内容指纹，重复文件自动跳过（同一张发票转发多次也只会存一份） |
| 解析 | 中文数电票 / 增值税电子发票 / 英文 invoice，提取 4 个字段，缺字段自动降级到邮件信息 |
| 命名 | 模板占位符自由组合，非法字符净化、空字段不留双下划线、重名自动 `-2` 递增 |
| 批量 | 勾选后批量重命名（磁盘文件同步改名）、批量导出到指定文件夹、导出 CSV 清单、批量删除 |
| 微调 | 表格内直接改日期 / 金额 / 销售方 / 发票号，改完点「按模板重命名」即可生效 |

## 用 GitHub Actions 构建（本机零编译）

1. 把仓库推到 GitHub。
2. `Actions` → `build` → 等「构建 Windows 安装包」跑完。
3. 在该任务的 Artifacts 里下载 `InvoiceBox-windows-x64`，内含：
   - `InvoiceBox_0.1.0_x64_en-US.msi`（MSI 安装包）
   - `InvoiceBox_0.1.0_x64-setup.exe`（NSIS 安装包，体积更小）

打 tag（例如 `git tag v0.1.0 && git push --tags`）会自动创建一个 **draft Release** 并把两个安装包附上去。

构建流程见 [`.github/workflows/build.yml`](.github/workflows/build.yml)：`npm install` → `npx tauri icon` 生成全套图标 → `cargo test` → `tauri build --bundles msi,nsis` → 上传产物。

## 本地开发（可选，仅调试需要）

```bash
npm install
npm run dev      # 需要 Rust 工具链 + 平台 WebView 依赖，见 Tauri 文档
```

核心逻辑有单元测试（发票字段解析、命名模板、去重命名）：

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

## 使用

1. **设置**：填 IMAP 服务器 / 端口 / 加密方式 / 邮箱账号 / **授权码**，点「测试连接」确认能登录并列出文件夹。
2. **收信**：点「开始收取发票」，日志会实时显示扫描进度、识别到的 PDF、跳过原因。
3. **发票库**：勾选记录 → 「按模板重命名」或「导出选中」；字段不准可直接在表格里改。

### 授权码怎么拿

| 邮箱 | IMAP 服务器 | 备注 |
| --- | --- | --- |
| QQ 邮箱 | `imap.qq.com:993` | 网页版 设置 → 账号 → 开启 IMAP 服务 → 生成授权码 |
| 163 / 126 | `imap.163.com:993` | 同上，必须先开启 IMAP 服务 |
| Gmail | `imap.gmail.com:993` | 开启两步验证后创建「应用专用密码」 |
| Outlook | `outlook.office365.com:993` | 部分账号需管理员开启 IMAP |
| iCloud | `imap.mail.me.com:993` | 用 App 专用密码 |

> 这里填的是**授权码 / 应用专用密码**，不是邮箱登录密码。

## 命名模板

默认：`{date}_{amount}_{seller}_{no}`

```
2024-03-01_1300.00_腾讯科技（深圳）有限公司_24312000000123456789.pdf
```

| 占位符 | 含义 |
| --- | --- |
| `{date}` | 开票日期（解析不到时用邮件日期） |
| `{amount}` | 价税合计 / Total，统一成两位小数 |
| `{seller}` | 销售方名称（解析不到时用发件人显示名或域名） |
| `{no}` | 发票号码 / Invoice No. |
| `{from}` | 发件人 |
| `{subject}` | 邮件主题 |
| `{kind}` | `attachment` 或 `link` |

命名规则：`\ / : * ? " < > |` 与非可见字符替换为 `-`；连续分隔符折叠；空字段直接省略（不会出现 `__`）；同名自动追加 `-2`、`-3`。

## 数据存放位置

侧边栏「打开数据目录」可直达。

| 内容 | 路径 |
| --- | --- |
| Windows 全部数据 | `%APPDATA%\com.invoicebox.desktop\` |

目录内容：

- `config.json` —— 邮箱账号、授权码、命名模板（明文保存，仅在本机；Unix 下权限 `0600`）
- `invoices/` —— 重命名后的 PDF
- `invoices.json` —— 发票清单 + 去重指纹
- `发票清单.csv` —— 表格用清单（UTF-8 BOM，Excel 直接开）

## 已知限制

- **扫描件 / 图片型 PDF** 没有文字层，只能靠邮件信息命名，记录里会带 `warnings` 提示。
- **加密 PDF**（需要打开密码）无法提取文本。
- **需要登录态 / 验证码 / 手机号的下载链接**取不到：只支持匿名可访问的直链。
- 只实现 SSL/TLS 与 STARTTLS，不提供明文 IMAP。
- CI 只产出 Windows 安装包（MSI + NSIS）。
- 中文发票版式差异大，字段解析是启发式规则；识别不准就在表格里改，再重命名。
- 构建产物**未签名**：Windows SmartScreen、macOS Gatekeeper 会提示，需要手动放行。

## 项目结构

```
.
├── assets/icon.svg              # 图标源文件（CI 用 npx tauri icon 生成全套）
├── ui/                          # 纯静态前端（无打包器）
│   ├── index.html
│   ├── styles.css
│   └── app.js                   # 通过 window.__TAURI__ 调命令 / 监听进度事件
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json          # frontendDist 直接指向 ../ui
│   ├── capabilities/default.json
│   └── src/
│       ├── main.rs              # Builder / 命令注册
│       ├── commands.rs          # 12 个 tauri 命令（配置、测试、收信、重命名、导出…）
│       ├── config.rs            # config.json 读写（含 0600 权限）
│       ├── mail.rs              # IMAP 收信 + MIME 解析 + 链接下载
│       ├── pdf.rs               # PDF 取文本 + 字段启发式解析
│       ├── invoice.rs           # 落库流水线 + 命名模板渲染
│       ├── store.rs             # invoices.json 清单与去重
│       └── util.rs              # 文件名净化 / 哈希 / 日期
└── .github/workflows/build.yml  # Windows 安装包构建 + Release
```

## 排错

| 现象 | 排查 |
| --- | --- |
| `登录失败` | 用了登录密码而不是授权码；QQ/163 需先在网页端开启 IMAP 服务 |
| 收不到任何发票 | 检查「扫描最近多少天」「扫描文件夹」；把「每封邮件尝试下载链接数」调大（默认 2） |
| 文件名很丑 | 模板占位符拼错；或 PDF 是扫描件没有文字层，字段来自邮件兜底 |
| 导出按钮无反应 | 需要先勾选记录；取消文件夹选择会静默返回 |
| Windows 提示未知发布者 | 未签名构建，选择「仍要运行」 |
