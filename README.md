# 聘渡 · HireFerry

**人才双向匹配服务。HR 作为服务。**

不只是发布招聘信息，我们亲自帮你找到对的人。

聘渡由平台 HR 团队居中服务：求职者提交资料与求职委托，招聘方提交岗位，双方分别与平台私聊。平台建立明确匹配后，双方才可查看对应资料、岗位和联系邮箱，并进入一段新的独立会话。

这是一个面向 Linux 自托管的小型 Rust 应用：单个服务、内嵌网页资源、SQLite 数据库、纯文本 TOML 配置。生产运行无需 Node.js 服务或外部数据库。界面支持中文与英文。

## 服务流程与隐私

1. 使用 Google 或邮箱验证码注册登录，首次选择求职者或招聘方；不要求手机号。
2. 求职者免费填写图片或安全 SVG 头像、联系邮箱和 Markdown 简历，并提交目标行业、岗位、薪资范围、地点及偏好。招聘方支付一次性开通费，再为岗位支付发布费。
3. 每个用户都有自己的平台私聊。平台管理员及其已授权 AI 可以读取业务资料、沟通需求、整理委托和推进匹配；其他用户没有全站人才或岗位浏览权限。
4. 平台把具有有效委托的求职者与已付费、正在招聘的岗位匹配。该求职者只能访问具体被介绍的岗位，招聘方只能访问被介绍的求职者。
5. 匹配产生独立的双方会话。此前各自与平台的私聊历史仍留在原会话中，不会复制给对方。平台可以在匹配会话中协调，并更新面试、录用或终止状态。

资料始终对平台可见，对无关用户保持私密。字段 `public_email` 保留兼容命名，表示供平台及匹配对象使用的联系邮箱；登录邮箱仅本人和平台管理员可见。旧 `visible` 字段不再授予公开访问权，没有公开简历或招聘目录。

将匹配改为 `declined`、删除岗位或封禁任一方，会撤销普通用户对相关匹配资料与会话的访问。单纯关闭岗位停止新的介绍，已有匹配会话可继续。管理员保留全部业务历史的读取权；归档会话返回 `writable: false`，包括管理员 AI 在内都不能继续发送。

## 已实现的工作台

- 首页服务介绍与收费说明，渡船、桥梁、罗盘三组粒子轮播；支持暂停、切换与减少动态效果偏好。
- 求职者资料、本人委托、外部寻找进展、平台私聊、独立匹配会话与未读提醒。
- 招聘方多个岗位、Markdown JD、固定月薪底薪、托管支付、自己的订单与匹配对象。
- 管理员总控台：完整用户/岗位资料、封禁与内容处理、委托、可视化匹配流程、平台私聊和历史会话、外部进展、订单、审计及按日期/历史累计的流水图表。正式和测试流水分开。
- 首次通过服务器命令行一次性口令绑定 Passkey，后续仅 Passkey 登录，支持多把密钥。
- 普通用户和平台管理员使用独立 MCP 身份；浏览器 WebMCP 绑定当前身份，AI 代发消息带标记。
- 所有系统设置通过 TOML 修改并重启生效，没有系统设置页面。

## 求职委托与外部寻找

求职者本人开启 `active` 委托，即允许平台围绕其需求开展站内匹配。多渠道外部联系还需要本人明确开启 `allow_external`；授权时间和是否通过 AI 提交会记录在案。求职者可暂停/关闭委托或撤回外部联系授权，管理员不能代为创建、扩大或重新激活这些同意。

平台可记录发现机会、已联系、收到回复、面试、录用与结束等进展，求职者能查看自己的记录。暂停或撤回授权后，只允许关闭已有外部进展，不能继续登记新的外部联系活动。资料不会因填入来源 URL 而被发送到外部站点。

**本应用不内置大模型、爬虫、外部招聘网站账号或自动联系服务。** 人工团队或另行部署的 AI 可以通过 MCP 读取获授权的需求、轮询消息、回复、建立匹配并记录实际进展；若要搜索其他网站或发送外部邮件，需要由其自身具备相应工具、账号和用户授权。本站保存委托与事实进展，不虚构已完成的外部求职接入。

## 收费规则

| 操作 | 默认费用 | 底薪 ¥3,000 的例子 |
| --- | --- | --- |
| 求职者注册及使用 | 免费 | ¥0 |
| 招聘方开通 | 一次性 ¥100 | ¥100 |
| 发布一个岗位，交由平台匹配 | 月薪底薪的 10% | ¥300 |
| 修改该岗位的 JD | 每次按原月薪底薪的 1% | 每次 ¥30，最多 3 次 |

底薪不含奖金、提成或补贴。岗位创建后标题与底薪固定；已验签付款回调确认后，岗位进入平台的可匹配岗位池。JD 修改先生成订单，付款确认后才替换现有内容。修改费按**原底薪**计算。金额使用人民币整数分，费率使用基点；规则在 `[pricing]` 配置，已有订单保留原金额和修改条款快照。

支持 Stripe 与 Waffo Pancake 的真实托管收银台，网站不接收银行卡号。支付返回页不能把订单标记为成功；只有已验签回调与 SQLite 事务可落账。缺少网关凭证时保持不可支付，不启用模拟成功。具体商户配置、签名和回调要求见 [支付说明](docs/payments.md)。

## 本机运行

需要 Rust 1.99 工具链；Linux 构建依赖 `pkg-config`、OpenSSL 开发包和 C 工具链。SQLite 已随 Rust 依赖编译。Debian / Ubuntu 可先安装：

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev ca-certificates
```

从 [rustup](https://rustup.rs/) 安装 Rust 后：

```sh
cp config.development.toml config.toml
chmod 600 config.toml
cargo run --locked -- --config config.toml serve
```

打开 **http://localhost:3000**。本机开发配置关闭 SMTP，邮箱验证码会直接显示在界面中；这仅在显式 `development = true` 且绑定回环地址时可用。该模式不会虚构支付成功，也不会自动创建用户、订单或示例数据。

Google、SMTP、支付网关默认关闭。需要使用时编辑 TOML 并重启；缺少商户凭证时无法支付。所有字段见 [生产模板](config.example.toml) 和 [开发模板](config.development.toml)。

## Linux 生产部署

下文统一使用示例域名 `https://hireferry.lmm.best`，部署时请替换为你的实际域名。

构建并安装单个二进制。网页与粒子运行时已经嵌入，不必另起静态资源服务。

```sh
cargo build --locked --release
sudo useradd --system --user-group --home-dir /var/lib/hireferry --shell /usr/sbin/nologin hireferry
sudo install -m 0755 target/release/hireferry /usr/local/bin/hireferry
sudo install -d -m 0750 -o root -g hireferry /etc/hireferry
sudo install -d -m 0700 -o hireferry -g hireferry /var/lib/hireferry
sudo install -d -m 0700 -o hireferry -g hireferry /var/lib/hireferry/data
sudo install -m 0600 -o hireferry -g hireferry config.example.toml /etc/hireferry/config.toml
sudoedit /etc/hireferry/config.toml
```

将 `public_url` 改为实际 HTTPS 站点来源，例如 `https://hireferry.lmm.best`，`webauthn_rp_id` 改为相同主机名 `hireferry.lmm.best`。保留 `development = false`；使用本机反向代理时保持 `bind = "127.0.0.1:3000"`。`database` 相对于服务工作目录，模板对应 `/var/lib/hireferry/data/hireferry.sqlite3`。

配置文件权限保持 **600**，数据库与备份目录保持 **700**。配置、SMTP 密码、Google secret、支付密钥均不要提交 Git。修改配置后重启服务。先验证配置，再启用服务：

```sh
sudo -u hireferry /usr/local/bin/hireferry --config /etc/hireferry/config.toml check-config
sudo install -m 0644 deploy/hireferry.service /etc/systemd/system/hireferry.service
sudo systemctl daemon-reload
sudo systemctl enable --now hireferry
```

[Caddyfile](deploy/Caddyfile) 提供 HTTPS 反向代理模板。安装 [Caddy](https://caddyserver.com/docs/install)，配置域名 DNS，将模板主机名改为实际域名，放入 Caddy 配置后验证并重载。公网入口为 HTTPS 443，80 用于证书与重定向；应用端口 3000 保持在回环地址。服务日志可用 `journalctl -u hireferry` 查看。

### Google 与邮箱

Google OAuth Web 应用的授权回调地址必须是：

```text
https://hireferry.lmm.best/auth/google/callback
```

以上 `https://hireferry.lmm.best` 为示例域名，请替换为实际域名；条款与隐私说明分别位于 `/terms` 和 `/privacy`。Google 控制台中的应用首页、条款和隐私 URL 也应使用实际部署地址。

在 `[google]` 填写 `client_id`、`client_secret` 并启用。Google 邮箱若与已有邮箱账户相同，不会自动合并身份；请使用原邮箱验证码登录。当前没有账户合并或 Google 绑定页面。

在 `[smtp]` 填写主机、端口、发件地址和凭证并启用。`starttls = true` 使用 STARTTLS，通常端口 587；`false` 使用隐式 TLS，通常端口 465，不是明文 SMTP。生产环境未配置 SMTP 时邮箱登录不可用。认证机制见 [认证与 Passkey](docs/auth.md)。

### 初始化管理员

在服务器交互式终端执行，命令会输出一条 **10 分钟内有效且只能使用一次**的口令：

```sh
sudo -u hireferry sh -c 'cd /var/lib/hireferry && /usr/local/bin/hireferry --config /etc/hireferry/config.toml admin-bootstrap'
```

打开 `https://hireferry.lmm.best/admin`，或在站点连续点击 Logo 5 次。输入口令后只会进入绑定首把 Passkey 的流程；服务端验证绑定成功后才能访问总控台。此后口令登录永久停用，`admin-bootstrap` 也会拒绝重新生成密码。

请在控制台登记第二把独立设备或安全密钥。最多支持 20 把；添加与删除需要最近 10 分钟内的 Passkey 认证，不允许删除最后一把。HTTPS 来源及 RP ID 与凭证绑定，部署后改变域名需要事先规划。全部密钥丢失时没有网页密码恢复或自动重置命令。

### 在线备份

CLI 使用 SQLite 在线备份 API，可在服务运行时得到一致快照；不要只复制正在使用 WAL 的主数据库文件。

```sh
sudo install -d -m 0700 -o hireferry -g hireferry /var/lib/hireferry/backups
sudo -u hireferry sh -c 'umask 077; cd /var/lib/hireferry && /usr/local/bin/hireferry --config /etc/hireferry/config.toml backup /var/lib/hireferry/backups/hireferry-001.sqlite3'
```

每次使用新的备份文件名，连同 TOML 安全保存到服务器之外，并定期演练恢复。恢复时停止服务。数据库包含用户资料、订单、审计、会话与 Passkey 公钥；数据库备份不能代替持有 Passkey 的设备或安全密钥。

## Docker

镜像使用 Rust 1.99 构建，运行在 Debian Bookworm，包含 CA 证书及 OpenSSL 运行库。服务以 UID/GID `10001` 运行，不携带配置、密钥或示例数据库。

```sh
docker build -t hireferry:local .
mkdir -p docker-config
cp config.example.toml docker-config/config.toml
```

编辑 `docker-config/config.toml`：使用真实 HTTPS 域名，`development = false`，容器内监听 `bind = "0.0.0.0:3000"`，数据库保持 `data/hireferry.sqlite3`。设置好 Google、SMTP 与网关，再准备挂载权限：

```sh
sudo install -d -m 0700 -o 10001 -g 10001 docker-data
sudo chown 10001:10001 docker-config/config.toml
sudo chmod 600 docker-config/config.toml
docker run -d --name hireferry --restart unless-stopped \
  -p 127.0.0.1:3000:3000 \
  --mount type=bind,src="$PWD/docker-config/config.toml",dst=/app/config/config.toml,readonly \
  --mount type=bind,src="$PWD/docker-data",dst=/app/data \
  hireferry:local
```

同样使用 HTTPS 反向代理。首次管理员口令可用 `docker exec -it hireferry hireferry --config /app/config/config.toml admin-bootstrap` 获取。持久化数据在 `docker-data`，不要在升级时丢弃该目录。`development` 模式要求回环监听，不适合作为该容器映射方案的配置。

## MCP、WebMCP 与 AI 资格

### 普通用户

求职者选择身份后可免费使用 AI 连接。招聘方需要已开通，并且至少有一个**发布费已支付、状态为 `published`、月薪底薪 ≥ ¥3,000** 的岗位。不是只填写金额或生成未支付订单即可开通。

创建个人令牌和每一次 Bearer 请求都会重新检查资格，包括直接请求 HTTP API。关闭最后一个符合条件的岗位后，既有招聘方 AI 令牌立即不可用；人工浏览器工作台仍可正常管理账号、支付和继续已有会话。WebMCP 的每次执行也检查资格，服务端对带 `X-HireFerry-AI: webmcp` 的请求再次验证。

在工作台「AI 连接」创建个人令牌，配置兼容客户端：

```text
URL: https://hireferry.lmm.best/mcp
Transport: Streamable HTTP
Authorization: Bearer <your-personal-token>
Protocol version: 2025-11-25
```

普通用户有 26 个远程 MCP 工具：本人资料/头像、岗位与匹配、订单/结账、委托/进展和会话。能发现某工具不代表能越过角色规则；例如招聘方不能修改求职者委托。令牌只显示一次，30 天有效，每个账户最多 10 个有效令牌，可撤销。

### 平台管理员

管理员通过真实 Passkey 会话在控制台「平台 AI」中签发独立 `hf_admin_` 令牌。创建要求最近 10 分钟内完成 Passkey 认证并通过 CSRF；令牌 30 天有效，平台最多保留 10 个有效令牌。令牌的列出、创建、撤销只允许管理员浏览器会话，Bearer 不能自行扩增凭证。

管理员使用相同 `/mcp` 地址，但发现的是另一组 26 个工具，可读取和管理全平台业务资料、委托、匹配、会话、外部进展、订单及统计。每项写入通过与管理页面相同的业务函数，记录具体管理员 AI 的审计身份。管理员令牌不能冒充普通用户、操作 Passkey、修改系统密钥，或把未支付订单伪造为已支付。

### 消息轮询与浏览器工具

外部 AI 可用 `get_updates` 读取增量消息，保存 `next_cursor`；若 `has_more` 为真继续分页，否则按 `poll_after_ms` 等待 5 秒。定期用 `list_conversations` 更新匹配、参与者、可写状态和未读数，用 `get_messages` 获取具体历史。发送时为同一消息重试保留相同 `client_id`，服务端据此避免重复写入；每条 AI 消息保留 `via_ai` 标记。

浏览器 WebMCP 在支持 `modelContext.registerTool` 时按登录角色注册工具，沿用当前 Cookie/CSRF。管理员和普通用户拥有不同工具集合，未登录或不具备资格时不注册；不支持该草案接口的浏览器仍能使用普通网页。WebMCP 不注册 Passkey、令牌签发或系统设置操作。

远程 MCP 为无状态 `POST /mcp` JSON-RPC；采用手动 Bearer 请求头，没有 OAuth 自动发现，也没有 SSE 推送。协议、全部 HTTP 接口、两组远程工具 schema 和 WebMCP 名称差异见 [API 文档](docs/api.md)。

## 测试与交付边界

```sh
cargo fmt --all -- --check
cargo test --locked
for asset in static/*.js static/runtime/*.js; do
  node --input-type=module --check < "$asset" || exit 1
done
node tests/sculpture.test.mjs
node tests/service-ui.test.mjs
cargo build --locked --release
```

上述脚本包含 `static/service-ui.js`。Node.js 24 用于前端检查，生产运行无需 Node。GitHub Actions 检查格式、Rust 测试、JavaScript 语法、粒子模型与服务界面测试并构建 Linux 发布包。测试范围包括邮箱与真实 WebAuthn 签名、付费事务、角色/匹配/聊天权限、管理员令牌、委托授权、撤销、重试幂等和输入清洗。

已验证 62 项 Rust 测试、13 项 Node 测试（共 75 项）、7 个 JavaScript 模块语法、Rust 格式检查和本地开发构建。测试日志、真实浏览器与移动端截图、发布构建状态由 [验收记录](docs/acceptance.md) 分别记录。视觉 QA 使用临时独立数据库，测试账号、聊天、匹配和流水不作为默认数据随包。

真实 Google OAuth、SMTP 投递、物理 Passkey 设备及两个网关的商户付款，仍需部署方凭证、域名和设备做端到端验收。密码学测试和软件认证器不能替代外部验收。自动退款与争议处理尚未实现；异常到账标记 `paid_needs_review`，由管理员核对并通过商户后台处理。外部 AI 的实际模型运行、第三方搜索与邮件投递也属于单独部署和授权范围。

## English

HireFerry is a private, platform-mediated recruiting service built with Rust, Axum and SQLite. People and employers each talk privately to the HR team. There is no public resume or job directory. A platform introduction grants a specific pair access to the relevant profile and job, and opens a separate conversation; earlier private HR conversations are never copied into it.

Job seekers are free. Employers pay CNY 100 once, then 10% of a role's fixed monthly base salary to activate it for matching. Each description update costs 1% of the original base salary, with at most three paid updates. Titles and salaries are immutable. Only verified payment callbacks settle orders. Employer AI access additionally requires at least one paid, currently published role with a monthly base salary of CNY 3,000 or more, checked on every AI request.

The server records job-search mandates and explicit consent for external outreach. Platform staff or a separately configured AI may poll messages, reply, manage introductions and record factual progress through MCP. The application does not include an LLM, crawler, external job-board integration or automatic outreach service. Administrators can read all business history, including archived conversations, but cannot send to archived rooms.

One Linux binary embeds the website. Configure everything in TOML, use the supplied systemd/Caddy templates, and protect the SQLite database and configuration. The CLI provides consistent online backups. Administrator enrollment uses a one-time shell secret followed by Passkeys only. Separate administrator AI tokens are issued from a recently authenticated Passkey session; member tokens cannot gain administrator rights. MCP `2025-11-25` exposes 26 tools per identity class with manual Bearer headers; browser WebMCP is feature-detected and uses the current authorized session.

Live Google, SMTP, physical authenticators, merchant checkout and any independently operated AI integrations require deployment-specific acceptance tests. See [API and MCP](docs/api.md), [acceptance](docs/acceptance.md), [authentication](docs/auth.md), [payments](docs/payments.md), and the supplied deployment files.

## 许可与致谢 / License and credits

[AGPL-3.0-or-later](LICENSE)。首页粒子运行时基于 [LIghtJUNction/particle-sculpture-skill](https://github.com/LIghtJUNction/particle-sculpture-skill)；保留其 [版权说明](static/runtime/NOTICE)。本站渡船、桥梁与罗盘为面向聘渡主题的程序化模型。界面视觉参考 [ix.dev](https://ix.dev/)，未复制其品牌资源。
