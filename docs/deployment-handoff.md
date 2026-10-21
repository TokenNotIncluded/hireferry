# HireFerry 生产部署交接

本文件对应当前仓库的单服务实现：Rust / Axum、内嵌网页、SQLite、TOML、systemd 或 Docker。正式部署来源为 **https://hireferry.lmm.best**，支持邮箱为 **support@lmm.best**。配置、管理员密钥和业务数据库由部署方持有。

本文提供可以执行的部署步骤与真实接入核对项，不表示域名服务器、真实 Google、SMTP、物理 Passkey 或商户账户已在本次环境完成联调。本次交付未获得用户提及的 `production.env` 和另行提供的生产交接 README 内容；这不同于仓库自带的 README.md。**没有读取、复用或猜测其中的账号、客户端 ID、商户 ID、商品 ID、密码或密钥。**

部署方应在部署机器找到并核验实际交接配置，再将确认无误的值填入权限受保护的 TOML。应用只读取 `--config` 指定的 TOML，不自动加载 `.env`，没有按环境变量名猜测凭证的兼容层。

## 1. 七个正式接入路径

| 用途 | 方法与地址 | 接入要求 |
| --- | --- | --- |
| Google OAuth 回调 | GET `https://hireferry.lmm.best/auth/google/callback` | 在 Google Web OAuth 客户端中精确登记；注意没有 `/api` 前缀 |
| 服务条款 | GET `https://hireferry.lmm.best/terms` | 公开、中英双语 |
| 隐私政策 | GET `https://hireferry.lmm.best/privacy` | 公开、中英双语 |
| 退款政策 | GET `https://hireferry.lmm.best/refunds` | 公开，提供订单核对和支持入口 |
| 联系页面 | GET `https://hireferry.lmm.best/contact` | 显示 `support@lmm.best`，取自 site.support_email |
| Waffo 支付/退款通知 | POST `https://hireferry.lmm.best/api/webhooks/waffo` | 订阅 order.completed、refund.succeeded、refund.failed；必须通过网关签名 |
| 统一支付返回页 | GET `https://hireferry.lmm.best/billing/return` | order_id 标识本地订单；页面读取本人订单真实状态 |

其他入口：

- 首页：`https://hireferry.lmm.best/`
- 普通用户登录：`https://hireferry.lmm.best/login`
- 管理员：`https://hireferry.lmm.best/admin`，也可连续点击页面 Logo 5 次进入。
- Stripe 回调：`POST https://hireferry.lmm.best/api/webhooks/stripe`
- MCP：`POST https://hireferry.lmm.best/mcp`
- 健康检查：`GET https://hireferry.lmm.best/healthz`，正常返回 204。

Google 控制台中的应用首页、条款、隐私和支持信息使用上述地址。支付商户通知直接送到 Rust 服务；不要用认证跳转、验证码页面或会改写请求体的代理规则拦截回调。

## 2. 运行目录与 TOML

建议沿用仓库中的 [systemd 服务](../deploy/hireferry.service) 和 [Caddy 配置](../deploy/Caddyfile)：

| 内容 | 路径 / 约定 |
| --- | --- |
| 应用二进制 | `/usr/local/bin/hireferry` |
| 配置 | `/etc/hireferry/config.toml`，服务账户可读，权限 0600 |
| 服务账户 | `hireferry:hireferry`，无交互登录 shell |
| 工作目录 | `/var/lib/hireferry`，权限 0700 |
| 数据库 | `/var/lib/hireferry/data/hireferry.sqlite3` |
| 备份目录 | `/var/lib/hireferry/backups`，权限 0700 |
| 应用监听 | `127.0.0.1:3000`，由 Caddy 提供公网 HTTPS |

完整字段以 [生产模板](../config.example.toml) 为准。复制模板后保持以下非敏感基础配置：

```toml
bind = "127.0.0.1:3000"
public_url = "https://hireferry.lmm.best"
webauthn_rp_id = "hireferry.lmm.best"
database = "data/hireferry.sqlite3"
development = false
session_hours = 24

[site]
name = "HireFerry · 聘渡"
support_email = "support@lmm.best"
default_language = "zh"
source_url = "https://github.com/TokenNotIncluded/hireferry"
```

以上为节选，不替代完整模板。public_url 必须是 HTTPS 来源，不包含路径、查询、fragment 或账号密码；RP ID 必须等于主机名。数据库相对路径按服务工作目录解析。生产环境必须保持 development=false；开发验证码直显只允许显式开发模式及回环地址。

### 需要部署方确认的配置

| TOML 区域 | 必须确认的实际值 | 启用前的行为 |
| --- | --- | --- |
| google | client_id、client_secret、正式授权回调 | enabled=false 时不提供 Google 登录 |
| smtp | host、port、username、password、获准发件 from、TLS 方式；reply_to 使用 support@lmm.best | 生产未配置 SMTP 时邮箱登录不可用 |
| payments.stripe | secret_key、webhook_secret、live_mode、allowed_currencies；所属账户可收取所选币种 | 未配置不创建真实付款链接，币种子集外的订单不进入结账 |
| payments.waffo | merchant_id、store_id、一次性 product_id、私有 RSA PEM、对应 webhook 公钥、实际 tax_category、test/prod 模式、商品可用的 allowed_currencies | 任何必填值缺失时不得启用，仅支持实现明确列出的六种一次性收款币种 |
| pricing | 默认/可选币种、开通费、发布/修改模式和费率/固定金额、修改次数、每币种 AI 薪资门槛 | 旧记录保留币种金额，原岗位保留编辑条款和资格门槛 |
| limits | 招聘人数最大值、每人待付款上限、普通列表页大小/最大页码、工作台默认页大小、最近记录上限 | workbench_page_size 默认 25、范围 1–100；recent_list_limit 默认 200、范围 1–1000；免费人数修改的次数不设限 |
| webhooks | 全站启停、端点/队列/尝试上限、超时、历史保留期限 | 默认启用功能，但不会自动创建任何用户接收端点 |
| legal.values / legal.document_files | 实际运营方、版本、生效日期、退款和保留承诺，及可选的中英文替代文档路径 | 默认 Markdown 随二进制嵌入；替代文件在启动时加载并校验，价格与币种取业务配置 |

`[site].support_email` 与 `[smtp].reply_to` 是两个独立字段。页面联系方式不会替代 SMTP 的获准发件地址，也不会自动修改该地址。PEM 密钥可使用 TOML 多行字面量字符串保存；不要在日志、截图、Git 或工单中贴出密钥。

系统设置没有网页修改入口。`GET /api/config` 和远程 MCP `get_site_config` 只读公开的币种、费用、网关可用币种和业务限额，不暴露密钥。修改 TOML 后运行 check-config，再重启服务。

服务条款和隐私政策由 `/api/legal/{terms|privacy}?lang={zh|en}` 提供清洗后的 HTML、版本和生效日期。部署前按 [政策配置](legal-policies.md) 核实运营方与承诺值，并回读两个语言版本；外部 Markdown 文件相对 TOML 所在目录解析。未识别变量或无效文档会阻止配置加载。默认价格段落会区分免费、固定金额和按底薪比例，按币种精度显示并说明旧岗位条款保留。保留期、退款响应和注销时限是运营承诺，配置不会自动创建相应后台流程。

`[pricing].allowed_currencies` 控制新岗位和新委托的币种选择，`currency` 决定默认值及招聘方开通费币种。发布、修改分别支持 `salary_percent`、`fixed`、`free`；每个额外币种须配置固定价格及 `employer_ai_min_salary_minor`，这些是独立业务金额，不是汇率。费用字段均使用整数最小货币单位；USD 12.34 为 1234，JPY 1234 也为 1234。不要对所有币种统一乘除 100。

两个网关各有 `allowed_currencies` 子集，空数组表示该集成已实现的支持集；它不证明实际商户或商品已经开通全部币种。Stripe 当前实现 32 种币种，Waffo 为 CNY/EUR/GBP/HKD/JPY/USD 六种。按实际账户/商品收窄后，前端和后端都会限制支付方式。旧订单不改价、不换币；后续限制结账不会丢弃历史有效付款回调。原岗位继续使用创建时保存的编辑规则和 AI 薪资门槛。支持范围和外部约束见 [支付说明](payments.md)。

## 3. 构建和安装

从源码构建时，当前 CI 使用 Rust 1.99.1，Docker 构建镜像使用 Rust 1.99；Linux 需要 C 工具链、pkg-config、OpenSSL 开发包。SQLite 随依赖构建。运行期不需要 Node.js；Node.js 24 仅用于前端测试。

在源码根目录执行：

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev ca-certificates curl
rustup toolchain install 1.99.1 --profile minimal --component rustfmt
cargo +1.99.1 build --locked --release
```

也可以使用对应提交在 GitHub Actions 生成的 Linux 发布包，核对来源、提交与构建结果后安装其中的二进制。不要把临时 QA 数据库、测试账户或真实配置放入发布包。

首次安装：

```sh
sudo useradd --system --user-group --home-dir /var/lib/hireferry --shell /usr/sbin/nologin hireferry
sudo install -m 0755 target/release/hireferry /usr/local/bin/hireferry
sudo install -d -m 0750 -o root -g hireferry /etc/hireferry
sudo install -d -m 0700 -o hireferry -g hireferry /var/lib/hireferry
sudo install -d -m 0700 -o hireferry -g hireferry /var/lib/hireferry/data
sudo install -d -m 0700 -o hireferry -g hireferry /var/lib/hireferry/backups
sudo install -m 0600 -o hireferry -g hireferry config.example.toml /etc/hireferry/config.toml
sudoedit /etc/hireferry/config.toml
sudo -u hireferry /usr/local/bin/hireferry --config /etc/hireferry/config.toml check-config
```

服务账户或目录已存在时保留现有所有权与数据，不重复创建账户，也不覆盖已验证的生产配置。`check-config` 检查 TOML 格式、字段、基本范围和启用条件；它不连接 Google、SMTP 或支付商户，不能证明外部服务真实可用。

安装并启动 systemd 服务：

```sh
sudo install -m 0644 deploy/hireferry.service /etc/systemd/system/hireferry.service
sudo systemctl daemon-reload
sudo systemctl enable --now hireferry
sudo systemctl status hireferry --no-pager
curl --fail --silent --show-error --output /dev/null --write-out '%{http_code}\n' http://127.0.0.1:3000/healthz
```

健康检查预期为 204。启动时会创建并迁移 SQLite 表，包括退款和用户 Webhook 数据；迁移前应已有一致备份。用 `journalctl -u hireferry` 查看服务状态，不应在诊断材料中输出 TOML 内容。

## 4. HTTPS 与公开页面

先将 hireferry.lmm.best 的 DNS 记录指向目标服务器。安装 Caddy 后，将仓库 [Caddyfile](../deploy/Caddyfile) 中的站点块合入服务器现有配置，保留其他站点。该块代理到 127.0.0.1:3000，不要求额外静态文件服务。

```sh
sudo caddy validate --config /etc/caddy/Caddyfile
sudo systemctl reload caddy
curl --fail --silent --show-error --output /dev/null --write-out '%{http_code}\n' https://hireferry.lmm.best/healthz
```

HTTPS 健康检查预期 204。通过真实浏览器打开首页、terms、privacy、refunds、contact，分别确认中文和英文内容、支持邮箱、无登录也能阅读，以及手机宽度下可用。这些路径返回嵌入的页面，由前端渲染相应内容；只得到 HTTP 200 不能替代浏览器检查。

公网开放 443；80 供证书验证与 HTTPS 重定向。应用的 3000 端口只在回环监听。保持服务器时钟同步，支付验签和 Passkey 挑战依赖正确时间。

## 5. Google 与 SMTP

### Google

在真实 Google OAuth Web 客户端登记精确回调：

```text
https://hireferry.lmm.best/auth/google/callback
```

填入 `[google]` 的真实 client_id/client_secret 并启用，检查配置、重启后，用获准的账号完成浏览器授权和首次角色选择。Google 邮箱与既有邮箱账户相同时不会自动合并，当前没有账户合并或绑定页面。不能使用猜测的 Client ID 或另一个项目的密钥做验收。

### SMTP 与 Reply-To

将已验证的 SMTP 配置填入 `[smtp]`。`starttls=true` 使用 STARTTLS，常见端口 587；false 使用隐式 TLS，常见端口 465，不能据此启用明文 SMTP。具体端口和发件身份以服务商实际配置为准。

```toml
# 在完整生产模板的 [smtp] 区域中设置：
reply_to = "support@lmm.best"
```

实际验证码邮件会写入 Reply-To。`from` 必须是该 SMTP 账户获准的发件邮箱，不能从支持邮箱或域名推断。启用后向部署方控制的邮箱申请验证码，确认真实到达、From 身份、Reply-To 以及验证码可使用一次；生产响应和界面不得展示开发验证码。不能把本地验证码测试记为真实邮件投递成功。

## 6. 支付和退款接入

完整 API 字段、签名、去重和退款规则见 [支付说明](payments.md)。不要把测试环境的公钥、webhook secret 或商品 ID 用在正式模式。

### Stripe

填入实际 secret_key/webhook_secret，test key 对应 live_mode=false，live key 对应 true。在商户侧配置 snapshot webhook：

```text
POST https://hireferry.lmm.best/api/webhooks/stripe
checkout.session.completed
checkout.session.async_payment_succeeded
```

先在正确环境完成端到端结账、签名通知和重复通知验证。应用按订单保存的币种和整数最小单位处理已付款的一次性 Checkout；银行卡信息由网关托管。Stripe 退款通知与争议处理当前未实现，需要商户人工核对；不要把商户已发起退款等同于本站已入账退款。

### Waffo Pancake

使用该商户实际开通并支持订单币种的一次性商品、merchant/store 标识、RSA 私钥、匹配环境的 webhook 公钥和真实业务 tax_category。将 payments.waffo.allowed_currencies 限定为该商品确实支持的 CNY/EUR/GBP/HKD/JPY/USD 子集，并核对商品价格、税务和支付方式要求。base_url 为 `https://api.waffo.ai`，模式只能为 `test` 或 `prod`。商户身份与签名 priceSnapshot 动态价格必须获得网关支持；不能猜商品 ID 或用不同业务类别替代招聘服务。

同一通知地址订阅三项事件：

```text
POST https://hireferry.lmm.best/api/webhooks/waffo
order.completed
refund.succeeded
refund.failed
```

通知在验签后核对 storeId、mode、原本地订单、原 paymentId、币种和金额。成功退款原子记录 refund ID，累计不得超过原实收；重复结果不重复扣款。失败通知只记尝试，既不扣收入也不改变权益。

部分退款保留已履行权益；原待核对订单仍待核对。全额开通退款撤回相应开通，全额发布退款关闭对应岗位；修改退款不回滚 JD 或恢复编辑次数。原付款流水保持不变，管理员统计分开显示原实收、成功退款、净收入，按 test/live 隔离，并按退款首次成功记录时间归入现金流期间；某个退款日的净额可以为负。

本站没有发起退款 API 或按钮。退款由经过授权的商户操作完成，应用只接收结果通知。真实购买、退款、失败退款通知及商户侧重试仍需要实际凭证和明确的验收安排，本次没有进行真实扣款或退款。

### 统一支付返回

所有网关使用同一站内返回页：

```text
https://hireferry.lmm.best/billing/return?order_id=<local-order-id>&provider=stripe
https://hireferry.lmm.best/billing/return?order_id=<local-order-id>&provider=waffo
```

Stripe 取消返回额外加 canceled=1，仅为提示。页面会要求登录并通过本人订单 API 读取真实状态；别人持有返回链接也不能读取订单。浏览器先返回、webhook 后到时仍可能 pending，不能仅因此再次付款。遇到网关创建超时或 paid_needs_review，先按订单号核对本地与商户记录，不通过修改 SQLite 伪造支付成功。

## 7. 管理员 Passkey

在目标服务器交互式终端生成一次性口令：

```sh
sudo -u hireferry sh -c 'cd /var/lib/hireferry && /usr/local/bin/hireferry --config /etc/hireferry/config.toml admin-bootstrap'
```

口令 10 分钟有效，只能用一次。打开正式 `/admin` 输入后，只能绑定首把 Passkey，尚不能访问业务总控台。注册成功后才取得管理员身份，口令登录永久停用，CLI 也拒绝再生成首次口令。

在控制台登记至少第二把独立设备或安全密钥。最多 20 把，后续增删要求最近 10 分钟 Passkey 认证，不能删除最后一把。验证一次登出再登录、第二把密钥登录，以及归档会话只读。所有密钥丢失时没有网页密码重置或自动恢复命令；数据库备份也不能代替设备持有的私钥。

普通用户与管理员 AI 使用独立令牌。管理员只可在最近 Passkey 认证的真实浏览器会话中签发 `hf_admin_` 令牌，30 天有效，不能借助 Bearer 自行扩增凭证。不要将管理员令牌交给普通招聘方的客户端。完整权限见 [API 文档](api.md)。

## 8. 用户 Webhook 与外部 AI

用户 Webhook 属于用户主动配置的出站通知，与支付商户入站回调分开。默认最多 5 个端点、每次投递最多 5 次尝试；系统开关与队列、超时、保留策略在 `[webhooks]`。完整接收方规范见 [用户 Webhook](webhooks.md)。

端点及签名密钥只能由本人普通浏览器管理，写请求需要 CSRF；Bearer、WebMCP、管理员和首次引导会话都不能管理。服务进程内的后台任务投递通知，不依赖 Redis 或独立工作进程。接收方必须是公网 HTTPS/443，真实发送前会复核 DNS/IP 与权限；不存在用于部署方便而绕过检查的 localhost/HTTP 选项。

先在部署方控制的接收站点实现验签和事件 ID 去重，再从工作台发送测试通知、查看实际投递记录。测试通知排队不代表对端已收到。尝试重试、暂停和撤销，并确认失去匹配权限后旧事件不会继续外发。通知仅包含资源标识与状态，不发送完整资料正文；需要详情时使用另行授权的本人 API/MCP。

远程 MCP 使用 Streamable HTTP `POST /mcp`、手动 Bearer、协议 2025-11-25；成员和管理员各有 32 个工具，都包含只读公开配置工具 get_site_config。外部 AI 可以轮询消息、回复、管理匹配并记录委托进展，但本应用不附带 LLM、爬虫、外部招聘站账号或自动联系服务。浏览器 WebMCP 只在支持草案接口时注册；本地注册测试不等于真实浏览器原生执行已验收。

## 9. 备份、升级与恢复

使用 CLI 在线备份获得一致 SQLite 快照，不要只复制运行中处于 WAL 模式的主数据库文件：

```sh
sudo -u hireferry sh -c 'umask 077; cd /var/lib/hireferry && /usr/local/bin/hireferry --config /etc/hireferry/config.toml backup /var/lib/hireferry/backups/hireferry-001.sqlite3'
```

每次用新的备份文件名。数据库包含资料、会话、订单、退款、审计、Passkey 公钥，以及需要用于签名的用户 Webhook 密钥；连同 TOML 按敏感数据管理，保存服务器外备份，并演练恢复。

升级前记录当前代码/发布包版本，备份数据库和 TOML，核对新版本测试与迁移说明；停止服务、安装新二进制、check-config、再启动。检查 healthz、日志、真实登录、订单和会话后再确认升级。回退时必须考虑数据库迁移兼容性，必要时恢复与旧程序对应的一致备份；不要直接用旧二进制修改已迁移数据库。

本版使用 SQLite schema **8**。升级先应用 schema 4 的招聘人数与一次恢复字段及岗位 Webhook 触发器：旧草稿和已发布岗位默认人数为 1，旧关闭岗位保持关闭、人数为 0；旧版本已经删除的岗位按永久删除迁移，不进入可恢复回收站，清除标题、JD、关联订单内容快照及缓存付款链接，取消尚待付款的关联订单，保留账务关联。schema 5 建立管理员工作台的分页/筛选索引及 FTS5 trigram 索引，首次会为已有内容建索引。schema 6 为旧岗位和委托保存历史 CNY 币种，并为旧岗位保留原 CNY 300000 最小单位的 AI 门槛；绝不以当前配置重标旧金额。旧 orders 表在事务中重建为允许零金额订单，历史订单、账本、退款及事件关联保持原值；相关索引和 Webhook 触发器重新建立。schema 7 新增 HumanGrant 申请、授权、会话、防重放与限流记录。schema 8 新增 CLI 授权及访问、刷新凭据的哈希记录，不保存这些凭据的明文，并在账户停用时撤销相关授权。务必在升级前完成备份；具体生命周期、权限与恢复条件见 [岗位管理](jobs.md)。

Docker 部署方式见 [README](../README.md#docker)：容器以 UID/GID 10001 运行，配置只读挂载，数据库独立持久化。容器内监听 0.0.0.0:3000，但主机只映射 127.0.0.1:3000；仍由 HTTPS 代理提供公网入口。

## 10. 验证状态与交接界限

自动化测试、构建和截图的具体结果、数量及提交证据统一见 [验收记录](acceptance.md)。交接时应分别确认以下三层，不能相互替代：

| 验证层 | 已有或需要的证据 | 能说明什么 |
| --- | --- | --- |
| 本地自动化 | Rust、Node、ESM 语法、格式及构建记录；本地 RSA/HMAC/WebAuthn 签名和真实 SQLite 测试 | 业务事务、鉴权、幂等、清洗、退款与通知逻辑是否符合本地实现 |
| 浏览器视觉与流程 | 中英文、桌面/手机、资料、聊天、委托、匹配、订单/返回页及控制台截图与操作记录 | 页面可用性、响应式和本地流程；使用独立合成测试资料 |
| 部署和外部服务 | 实际 DNS/HTTPS、Google、SMTP/Reply-To、实体 Passkey、Stripe/Waffo 商户付款与退款通知、接收方 Webhook、原生 WebMCP/独立 AI | 正式服务器与真实外部账号、设备、网络之间是否成功接通 |

本次使用合成 QA 资料，不向用户发布默认测试账号或业务数据库。外部生产文件未提供的字段保持待部署方填写；没有任何一步可以通过返回 URL、手工改订单或模拟成功来替代真实付款确认。
