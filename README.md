<p align="center">
  <img src="docs/assets/readme/hero.svg" alt="HireFerry 渡船与粒子航线" width="100%">
</p>

<div align="center">
  <h1>聘渡 · HireFerry</h1>
  <p><strong>开源、自托管、AI 可协作的人才匹配平台。</strong></p>
  <p>求职委托 · 招聘岗位 · 人岗初筛 · 独立匹配会话 · MCP / HumanGrant · Rust + SQLite</p>

  [![CI](https://github.com/TokenNotIncluded/hireferry/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/TokenNotIncluded/hireferry/actions/workflows/ci.yml)
  [![CLI](https://img.shields.io/github/v/release/TokenNotIncluded/hireferry?label=CLI&style=flat-square)](https://github.com/TokenNotIncluded/hireferry/releases/latest)
  [![Rust](https://img.shields.io/badge/Rust-2024-292524?style=flat-square&logo=rust)](Cargo.toml)
  [![SQLite](https://img.shields.io/badge/SQLite-bundled-57534e?style=flat-square&logo=sqlite&logoColor=white)](docs/self-hosting.md)
  [![License](https://img.shields.io/badge/license-AGPL--3.0--or--later-292524?style=flat-square)](LICENSE)

  **简体中文** · [English](docs/i18n/README.en.md) · [繁體中文](docs/i18n/README.zh-TW.md)

  [官网](https://hireferry.lmm.best) · [公开岗位与简历](https://hireferry.lmm.best/discover) · [下载](docs/downloads.md) · [自托管](docs/self-hosting.md) · [文档](docs/README.md)
</div>

## 先选择你的入口

| 你是谁 | 直接开始 | HireFerry 负责什么 |
| --- | --- | --- |
| **求职者** | [免费注册](https://hireferry.lmm.best/login?role=seeker&utm_source=github&utm_medium=readme&utm_campaign=evergreen&utm_content=seeker) | 维护简历与求职委托、选择是否公开、查看机会与匹配会话 |
| **招聘方** | [免费注册](https://hireferry.lmm.best/login?role=employer&utm_source=github&utm_medium=readme&utm_campaign=evergreen&utm_content=employer) | 发布岗位、维护招聘人数、筛选候选人、建立介绍并推进面试 |
| **AI / 开发者** | [`POST /mcp/public`](docs/mcp-public.md) 或安装 [`hfy`](#hfy-cli) | 读取公开机会，或在本人明确授权后使用账户工具 |
| **自托管运营者** | [部署指南](docs/self-hosting.md) | 用自己的域名、数据库、邮件、支付账户和业务规则运行实例 |

HireFerry 不抓取第三方招聘平台的私有简历，不是 BOSS 直聘插件，也不会自动批量打招呼。AI 负责整理、检索和辅助初筛；权限、公开范围、匹配建立和最终决定仍由人及服务端业务规则控制。

它也不保证录用、到岗或招聘结果。

## 本质上是什么

HireFerry 把招聘协作拆成四个可检查的对象：

```text
求职者资料 + 求职委托
              │
              ├────► 获授权 AI / HR 做初筛
              │
招聘方资料 + 公开岗位
              │
              ▼
          建立具体匹配
              │
              ▼
独立会话 + 对应资料 + 后续阶段记录
```

服务器保存身份、权限、资料、岗位、匹配、会话、订单和审计；持续运行的 Agent 客户端通过 MCP 或 CLI 执行获授权的自动化循环。模型、外部搜索、邮件外联和人工 HR 服务不是隐藏在服务器里的“魔法”。

## 服务流程

<img src="docs/assets/readme/workflow.svg" alt="求职者和招聘方分别与平台沟通，平台建立介绍后开启新的匹配会话" width="100%">

1. **提交需求**：求职者填写简历和求职委托；招聘方发布岗位与招聘人数。
2. **受控初筛**：HR 或获授权 AI 按岗位条件检索候选人，不自动扩大资料可见范围。
3. **建立介绍**：平台创建一条具体匹配，双方获得对应资料和新的独立会话。
4. **共同推进**：双方记录沟通、面试、录用或结束状态；原平台私聊仍留在原会话。

岗位关闭或人数变为 0 会停止新的介绍，但不会强行删除既有会话。匹配拒绝、岗位删除或账号封禁会重新收紧访问。详细状态机见 [产品与业务规则](docs/product.md) 和 [岗位规则](docs/jobs.md)。

## 隐私边界

```text
明确公开的岗位 / 简历 ───────► 公开目录与公共 MCP

私有资料 ─► 平台与本人获授权 AI
              │
              └─ 建立具体匹配后 ─► 对应匹配对象
```

- 求职者必须主动开启公开主页，简历才进入公开目录。
- 岗位必须已发布且仍有招聘人数，才作为公开招聘机会展示。
- 登录邮箱、私聊、订单、审计和匹配历史不会进入公开目录。
- `public_email` 是匹配联系字段，不等于任意访客可读取。
- HumanGrant 权限有明确资源、范围和期限；服务端在每次业务请求重新检查当前资格。
- 管理员搜索、AI 工具发现和浏览器会话都不能代替业务授权。

完整说明见 [隐私与访问模型](docs/product.md#服务流程与隐私)、[身份认证](docs/auth.md) 和 [HumanGrant](docs/humangrant.md)。

## 公开目录与免费 MCP

[公开目录](https://hireferry.lmm.best/discover) 无需登录即可浏览仍在招聘的公开岗位，以及主动公开资料的求职者。

AI 可以使用只读公共 MCP：

```text
POST https://hireferry.lmm.best/mcp/public
```

它不需要账号、API token 或浏览器 Cookie，只能读取明确公开的岗位、简历和站点配置。来源 URL 本身不会触发外联、投递或消息发送。

JSON-RPC 示例、工具 schema、分页和隐私边界见 [公共 MCP 文档](docs/mcp-public.md)。支持 WebMCP 的浏览器打开 `/discover` 时也会注册同一组只读工具。

## AI 接入

| 方式 | 身份与权限 | 适合场景 |
| --- | --- | --- |
| **公共 MCP** | 无登录，只读公开资源 | 发现岗位、公开简历和站点配置 |
| **账户 MCP** | 独立凭据，按当前账户能力授权 | 个人资料、委托、岗位、消息和业务操作 |
| **HumanGrant** | 本人选择具体权限、资源和有效期 | 将有限能力交给外部 AI Agent |
| **WebMCP** | 沿用当前浏览器会话与 CSRF 保护 | 支持该接口的浏览器内助手 |
| **`hfy` CLI** | 浏览器登录或设备码登录 | 终端、脚本、Agent skill 和服务连接管理 |

所有 AI 写操作继续经过普通服务端校验并记录执行身份。工具出现在列表里，不代表当前用户一定能执行它。

安装 Agent skill：

```bash
npx skills add TokenNotIncluded/hireferry --skill hireferry
```

技能安装需要 Node.js 22.20+。命令、授权和恢复流程见 [CLI 文档](docs/cli.md) 与 [API / MCP 文档](docs/api.md)。

## hfy CLI

当前 CLI 包版本为 `0.1.4`，最低 Rust 版本为 1.88。

Linux / macOS：

```bash
curl -fsSL https://hireferry.lmm.best/install.sh | sh
```

Windows PowerShell：

```powershell
irm https://hireferry.lmm.best/install.ps1 | iex
```

安装器识别 x64 / ARM64，校验 SHA-256，并安装到用户目录。重复运行可升级。

```bash
hfy auth login
hfy whoami
hfy capabilities --json
```

无图形远程终端使用：

```bash
hfy auth login --device
```

Arch Linux 可安装 `hireferry-cli-bin`；开发版使用 `hireferry-cli-git`。账户连接、升级、卸载、反馈和 Agent 使用方式见 [CLI 指南](docs/cli.md) 与 [AUR 指南](docs/aur.md)。

## 下载与客户端形态

<img src="docs/assets/release-matrix.svg" alt="HireFerry CLI 与服务端发布矩阵" width="100%">

- `hfy` 为独立用户 CLI，提供 Windows、macOS、Linux 的 x64 / ARM64 构建。
- Linux 可下载服务端发布包，也可使用 Nix Flake 或自行编译。
- 手机和平板当前使用响应式 Web 工作台；没有原生移动包时，下载页不会伪造 APK 或 IPA。
- Tauri 2 桌面壳复用系统 WebView，具体可用产物以当前发布目录为准。

实际版本、大小、格式和 SHA-256 统一从 [下载矩阵](docs/downloads.md) 读取，不根据 README 猜测。

## 自托管快速开始

服务端最低 Rust 版本是 **1.88**；当前 CI 使用 **Rust 1.99.1**。Linux 还需要 C 工具链、`pkg-config` 和 OpenSSL 开发包。

```bash
git clone https://github.com/TokenNotIncluded/hireferry.git
cd hireferry

cp config.development.toml config.toml
chmod 600 config.toml

cargo run --locked -- --config config.toml serve
```

打开 `http://localhost:3000`。开发配置只监听本机，并提供适合本地测试的认证路径。

生产部署从 [`config.example.toml`](config.example.toml) 开始：

```text
配置域名、Cookie 与密钥
        ↓
选择 Google / SMTP / Passkey
        ↓
配置捐赠或收费模式与支付网关
        ↓
初始化管理员
        ↓
放到 Caddy / systemd / Docker 后面
        ↓
执行备份恢复和外部集成验收
```

生产运行不需要独立 Node.js 服务或外部数据库服务：网页资源内嵌在 Rust 二进制中，业务数据默认使用内置 SQLite，设置由 TOML 管理。

部署、Caddy、systemd、Docker、管理员初始化、在线备份和恢复见 [自托管指南](docs/self-hosting.md) 与 [部署交接](docs/deployment-handoff.md)。

## 默认免费，运营者拥有自己的实例

当前模板默认使用 **捐赠模式**：求职和招聘免费，站点 `/donate` 接受自愿捐赠。款项进入当前实例运营者自己配置的支付账户，不会自动转给 HireFerry 上游开源项目。

运营者可以切换到收费模式，为开通、岗位发布或 JD 修改配置不同币种和规则。历史订单与岗位条款保持原值，不会因以后改价而被重新计算。

```text
捐赠模式：新服务操作免费 → 可选自愿捐赠
收费模式：配置价格 → 创建订单 → 网关验签到账 → SQLite 事务生效
```

Stripe 与 Waffo Pancake 是可选集成。支付返回页本身不确认到账；真实权益只由验证后的支付事件改变。退款、币种、优惠码与边界见 [支付文档](docs/payments.md) 和 [产品规则](docs/product.md#收费与币种)。

## 架构

<img src="docs/assets/readme/architecture.svg" alt="网页、hfy 和 AI 连接 Rust Axum 服务，服务内嵌网页资源并使用 SQLite 与 TOML" width="100%">

| 层次 | 实现 |
| --- | --- |
| 入口 | Web、`hfy`、MCP、WebMCP、Webhook |
| 应用 | Rust 2024、Axum、统一业务权限与审计 |
| 数据 | 内置 SQLite、FTS5、在线备份 |
| 配置 | TOML，敏感配置文件应限制权限 |
| 可选集成 | Google、SMTP、Passkey、Stripe、Waffo、用户 Webhook |

一个二进制负责 HTTP 服务和内嵌网页；CLI 是独立 crate。外部 Agent 通过受控接口连接，不与数据库直接耦合。

## 验证到哪里

当前 CI 会检查：

- Rust 格式、服务端与 CLI 工作区测试；
- 100,000 个合成候选人的工作台查询；
- 浏览器资源、聊天、关系工作台、反馈语言和 WebMCP 生命周期；
- 真实 CLI 浏览器登录流程；
- skill、AUR、发布脚本与 Linux release bundle；
- macOS 与 Windows 原生 CLI 构建。

这些测试使用临时 SQLite、合成账户和受控传输。它们不证明某个生产实例已经完成：

- 真实 Google OAuth、SMTP 到达率和实体 Passkey；
- 真实 Stripe / Waffo 商户、退款与争议处理；
- 用户公网 Webhook 的实际验签与重试；
- 目标机器的 DNS、HTTPS、备份恢复、磁盘和并发负载；
- 当前版本全部桌面、手机、明暗主题和慢网视觉验收。

完整历史证据与仍待现场完成的项目见 [验收记录](docs/acceptance.md)。测试数量、编译成功或健康接口响应都不能单独证明生产部署完整可用。

## 仓库结构

```text
src/                         Rust / Axum 服务端
static/                      内嵌网页与浏览器运行时
crates/hireferry-cli/        hfy CLI
desktop/                     Tauri 2 桌面壳
docs/                        产品、API、部署与验收文档
deploy/                      服务部署材料
examples/                    HumanGrant 等集成示例
config.development.toml      本机开发模板
config.example.toml          生产配置模板
```

## 文档导航

| 目标 | 文档 |
| --- | --- |
| 产品、隐私、收费与状态机 | [产品规则](docs/product.md) · [岗位规则](docs/jobs.md) |
| 登录、CLI 与 AI 授权 | [认证](docs/auth.md) · [CLI](docs/cli.md) · [HumanGrant](docs/humangrant.md) |
| API、MCP 与 WebMCP | [API / MCP](docs/api.md) · [公共 MCP](docs/mcp-public.md) |
| 团队、员工与管理员 | [团队](docs/team.md) · [Staff](docs/staff.md) |
| 支付、优惠和 Webhook | [支付](docs/payments.md) · [优惠码](docs/promo-codes.md) · [Webhook](docs/webhooks.md) |
| 数据保留与反馈 | [数据生命周期](docs/data-lifecycle.md) · [反馈与增长控制](docs/feedback-and-retention.md) |
| 安装、下载和部署 | [下载](docs/downloads.md) · [自托管](docs/self-hosting.md) · [Nix](docs/nix.md) |
| 当前验证边界 | [验收记录](docs/acceptance.md) · [安全策略](docs/security.md) |
| 完整目录 | [文档首页](docs/README.md) |

## Contributing

提交前运行：

```bash
cargo fmt --all -- --check
cargo test --locked --workspace --no-fail-fast
npm ci --ignore-scripts
npm run check:chat
npm run test:chat
```

较大改动应同时更新对应产品规则、API、配置模板和验收文档。不要把真实简历、邮箱、支付密钥、Cookie、生产数据库或第三方账号凭据放入 Issue、PR、测试夹具或 Agent 提示词。

## License

[AGPL-3.0-or-later](LICENSE)。第三方组件与归属见 [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES)。
