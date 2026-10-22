# Authentication / 认证与管理员 Passkey

This document describes the implemented service behavior. Configuration lives in
TOML; changing it requires restarting HireFerry. It is not editable in the web
console.

## 生产来源与支持地址

本项目约定的生产来源为 `https://hireferry.lmm.best`，支持邮箱为
`support@lmm.best`。对应配置为 `public_url = "https://hireferry.lmm.best"`
与 `webauthn_rp_id = "hireferry.lmm.best"`；来源不带路径或查询参数。
本地开发仍使用 `http://localhost:3000` 与 RP ID `localhost`。

这些是部署配置约定，不代表真实 Google、SMTP 或线上域名联调已经完成。
用户提供的两处凭据文件路径在当前交付环境不存在，因此本次没有从这些路径
读取或导入生产密钥。部署人员需要在实际服务器上配置可用的 TOML 凭据，
不能把模板中的空值当作已经完成接入。

## 用户登录与隐私

普通用户使用 Google 或邮箱验证码。首次登录创建 `unassigned` 账户，再通过界面选择求职者或招聘方。选择招聘方会创建开通订单，收到有效支付通知后才开通招聘权限。

资料、简历、头像和岗位不进入公开目录。`public_email` 是历史字段名，表示供平台及明确匹配对象使用的联系邮箱；旧 `visible` 字段不授予公开访问。平台建立明确匹配后，双方才能读取对应资料，并进入独立匹配会话，此前各自与平台的私聊不会复制给对方。

登录邮箱是账户身份标识，不向匹配对象返回。本人、获得本人资料读取权限的 AI，以及平台管理员可以读取它。网站不要求手机号。管理员为服务运营可以查看账户及业务历史；普通会话和 MCP 身份仍受当前角色、匹配关系、封禁和资源权限约束。

### 邮箱验证码

配置 `[smtp]` 的 `enabled`、`host`、`port`、`username`、`password`、`from`、`reply_to` 和 `starttls`。依据：[配置结构及校验](../src/config.rs)、[验证码邮件构建](../src/auth.rs)。

`from` 是 SMTP 服务允许使用的发件邮箱；`reply_to` 是收件人点击「回复」时使用的地址，两者分别配置。`reply_to` 默认和示例值均为 `support@lmm.best`，当前验证码邮件会发送这个 Reply-To 头。可在 TOML 中改为另一个有效邮箱，或显式设为空字符串以省略该头；非空值会在配置加载时进行邮箱格式校验。更改后需要重启服务。

```toml
[smtp]
enabled = false
host = ""
port = 587
username = ""
password = ""
from = "" # 填写实际 SMTP 服务允许的发件邮箱。
reply_to = "support@lmm.best"
starttls = true
```

此片段没有生产凭据，不能直接启用发信。支持邮箱和 Reply-To 的配置也不会自动创建邮箱、配置 DNS 或验证 SMTP 服务。

- `starttls = true`：TLS 必需的 STARTTLS，常见端口 587。
- `starttls = false`：隐式 TLS，常见端口 465；不会回退为明文连接。
- 生产环境关闭 SMTP 时，邮箱登录返回不可用。
- 仅当 `development = true` 且 SMTP 关闭时，API 返回 `dev_code`，前端显示本地验证码。开发模式必须使用回环绑定与 localhost 来源，不能用于公网部署。

验证码为 8 位随机数字，10 分钟有效，只能在发起请求的浏览器中使用。数据库保存验证码与随机挑战组合的哈希，不存储明文验证码。再次申请会使旧码失效；每个挑战最多尝试 5 次。

服务端限制同邮箱 60 秒内最多申请 1 次、1 小时 10 次，并设置全局申请限制与验证请求限制。验证码不会写入应用日志。发信失败会撤销对应挑战，无法凭一个尚未投递的挑战继续登录。

### Google

创建 Google OAuth Web 应用，在 TOML 填写客户端 ID 和 secret。授权回调 URI 必须精确配置为：

```text
https://hireferry.lmm.best/auth/google/callback
```

本地开发可单独登记 `http://localhost:3000/auth/google/callback`。

生产授权来源使用 `https://hireferry.lmm.best`。条款和隐私页面分别为
`https://hireferry.lmm.best/terms` 与 `https://hireferry.lmm.best/privacy`。
Google 登录仍从 `/api/auth/google/start` 发起；授权回调固定为
`/auth/google/callback`，不使用 `/api/auth/google/callback`。

流程使用浏览器绑定的随机 state、PKCE 和一次性服务端挑战；后端向 Google 交换授权码，并从 Google 用户信息接口取得 `sub` 与已验证邮箱。客户端提交的邮箱或用户标识不能用于创建已验证身份。成功后跳转 `/app`。

已有 Google 身份按 `sub` 匹配。如果首次 Google 登录的邮箱已经属于其他账户，服务拒绝自动合并，提示改用邮箱验证码登录。当前版本没有「连接 Google」或账户合并页面。这避免把仅凭同名邮箱的第三方身份绑定到既有账户。

参考：[Google OpenID Connect](https://developers.google.com/identity/openid-connect/openid-connect)。

### HumanGrant：由本人确认的 AI 访问

登录页提供 **人类辅助登录 / Human Login** 入口，`/humangrant` 说明如何让 AI 发起申请。本人仍通过现有 Google 或邮箱验证码登录。打开 `/human/{link}` 时，已有有效登录会直接进入权限确认；未登录时先登录，再返回原申请。

确认页显示当前账户、原申请 AI 的身份、服务、用途、具体权限、资源和期限。权限默认不勾选，用户还须明确同意。服务器在提交时重新检查账户与申请，并验证绑定该登录、账户和申请的 `X-HG-CSRF`；仅打开链接或切换账户不能取得授权。

批准后，只有原申请密钥可以单次领取独立 AI 凭据。后续远程 MCP 请求同时使用 `Authorization: HumanGrant <access_token>` 和该密钥的请求签名。AI 不获得本人的浏览器 Cookie。服务器持续检查有效期、撤销、封禁、AI 资格及实际操作权限。用户可以在「AI 连接」中查看授权并随时撤销。

现有个人 Bearer 令牌继续可用。HumanGrant 不用于管理员登录或管理员 AI，也不能扩大本人当前的业务权限。配置、完整协议接口与签名示例见 [HumanGrant 说明](humangrant.md)。

## 管理员首次开通

超级管理员不属于普通用户，也不能通过 Google、邮箱验证码、HumanGrant 或个人 MCP 令牌取得超级管理员权限。受邀员工通过自己的邮箱或 Google 账号进入业务总控台，团队、Passkey 与凭据管理仍仅由超级管理员操作，详见 [团队成员](team.md)。

1. 在持有服务器权限的交互式终端运行 `hireferry --config /path/to/config.toml admin-bootstrap`。命令生成 32 字节随机秘密并仅向当前终端输出；数据库只保存哈希。不要把输出写入共享日志。
2. 在 10 分钟内访问 `/admin`，或连续点击 Logo 5 次，输入该一次性口令。
3. 口令消耗后只取得一个 5 分钟的引导会话。它不能读取用户、订单、统计或执行管理员操作，只允许注册首把 Passkey。
4. 浏览器调用 WebAuthn，服务端验证返回的注册凭证。成功后注销引导会话、删除引导口令，换发真正的管理员会话并进入控制台。

已有任意管理员 Passkey 时，密码引导 HTTP 路径与 CLI 生成命令都不再允许使用。第一次注册未完成且已超时时，服务器操作员可重新生成口令；重新生成会使以前的引导会话失效。

### 后续登录与多把 Passkey

后续登录只有 WebAuthn Passkey。服务端保存挑战状态并验证来源、RP ID、签名及认证器返回的安全属性；挑战在 5 分钟后失效且只能消费一次。生产必须使用 HTTPS，`webauthn_rp_id` 必须等于 `public_url` 主机名。来源不得带路径、查询或凭证。

最多注册 20 把具名 Passkey。增加或删除密钥需要最近 10 分钟内的认证；超时请重新用已有 Passkey 登录。服务端拒绝删除最后一把密钥。删除密钥会撤销其他管理员会话以及尚未完成的登录挑战，保留执行删除的当前会话。

请在独立设备或硬件安全密钥上登记备用凭证。数据库仅包含凭证公钥等服务端材料，不包含可代替设备的私钥。全部凭证丢失时，没有网页密码恢复入口或自动化离线重置命令；重新运行 `admin-bootstrap` 不会绕过既有 Passkey。改变域名或 RP ID 也会影响已有凭证，应先制定迁移方案。

服务使用 [webauthn-rs](https://docs.rs/webauthn-rs/latest/webauthn_rs/) 验证协议，不依赖客户端声称「验证成功」。注册、登录和删除操作写入管理员审计记录。

## 会话、CSRF 与封禁

会话由 32 字节随机 token 标识，SQLite 只存 token 的 SHA-256 哈希。Cookie 设置 `HttpOnly`、`SameSite=Lax`、`Path=/`；HTTPS 配置下还设置 `Secure`。普通与完整管理员会话使用 `session_hours`，默认 24 小时，可配置 1–168 小时。登录与权限升级会轮换会话。

浏览器写操作使用 `x-csrf-token`，并检查请求来源。登录接口在还没有会话时使用来源检查和各自的浏览器挑战绑定。会话和 CSRF token 不写入 URL；日志不记录口令、验证码或 Cookie 内容。退出登录会删除服务端会话。

每次认证都重新检查会话过期时间和账户封禁状态。封禁账户无法继续使用旧会话或个人令牌；管理员和普通用户的权限分别验证，引导会话不能被当作完整管理员会话。

### 账户停用与恢复

管理员在账户详情中填写原因后，可以停用或恢复普通账户。`POST /api/admin/users/{id}/access` 接受 `{"banned":true,"reason":"原因"}`（恢复时为 `false`）；原因必须非空且不超过 500 个字符。需要完整管理员权限，浏览器写请求必须通过 CSRF 与来源校验。`GET` 同一路径读取当前状态、原因、变更时间及操作人。用户不能操作自己或其他人的封禁状态；管理员 MCP 同样经过权限复核。

停用在一个数据库事务中生效：删除该账户全部浏览器会话、撤销所有个人 AI 令牌、作废待验证的邮箱验证码，并停用该账户的 Webhook、取消待发送或正在领取的投递。发送器在领取及 DNS 解析后再次检查权限。已经发出的网络请求无法收回。聊天、匹配资料和 MCP 操作也会检查当前账户状态，历史记录保留供管理员审计。邮箱登录申请返回中性响应但不会生成验证码或发送邮件。

恢复只解除账户状态限制，**不会恢复旧会话、令牌、验证码或 Webhook 开关**。用户必须重新登录、重新创建所需的 AI 令牌并主动启用 Webhook。封禁前已经开始的 Google 授权流程也不能在恢复后换取会话，需要重新发起 Google 登录。每次停用和恢复写入包含原因、前后状态与操作人的审计记录；直接更新数据库封禁字段也触发凭据撤销，避免其他后台路径遗漏撤销。

旧的 `PATCH /api/admin/users/{id}` 仍接受 `banned` 和可选的 `ban_reason` 以兼容既有 MCP 客户端；未提供原因时使用明确的兼容操作原因。新界面使用独立的停用与恢复入口，普通资料保存不会隐式解除停用。管理员账户由 Passkey 和独立管理员令牌管理，不属于可停用的普通用户列表；删除管理员 Passkey、撤销管理员令牌仍使用对应的专门入口。

## 个人 AI 令牌

在登录后的「AI 连接」页面创建令牌。必须由交互式浏览器会话授权创建，已有 Bearer 令牌不能再创建令牌。每个账户最多 10 个有效令牌，30 天过期，只显示一次，数据库保存哈希，可立即撤销。

使用方式是 `Authorization: Bearer <token>`。令牌可使用该用户的业务工具，并受相同所有权、角色、支付与封禁检查限制。它不能使用管理员 API。MCP 为 `2025-11-25` 无状态 Streamable HTTP。远程连接接受 Bearer 或 HumanGrant 签名凭据，Cookie 不能替代；不提供第三方 MCP 客户端的 OAuth 自动接入。令牌不应放入公开仓库、网页源码或链接中。

WebMCP 使用当前浏览器会话，通过草案 `modelContext` API 提供工具。在支持的浏览器中同样受服务端授权与计费规则约束；浏览器未实现接口时不注册工具。

## 官方 CLI 授权

独立客户端 `hfy` 支持设备码与浏览器授权码登录。浏览器方式要求 S256 PKCE、随机 `state` 和本机回环回调；两种方式均需网站登录与明确确认。默认 `identity` 仅用于身份、资格与能力查询，不赋予业务权限，也不要求先满足 AI 资格。明确申请 `mcp` 时，普通用户才获得现有 AI 范围并检查资格；明确申请 `admin` 时，管理员须完成最近的 Passkey 认证。已有 Bearer 令牌不能代替浏览器同意签发新授权。

服务端在 `[cli_auth]` 中配置客户端 ID、允许范围、有效期、数量上限和限流。访问令牌默认 15 分钟有效，刷新授权默认从登录起最多 30 天。刷新令牌每次轮换，旧访问令牌同时失效；重复使用已消费的刷新令牌会撤销该次授权的全部凭据。刷新不能扩大范围，不会延长原登录的最长有效期。CLI 用本地锁协调续期；网络导致刷新结果不确定时不盲目重放旧令牌。

用户可在 `/oauth/connections` 查看和撤销自己的 CLI 授权，包括只有身份查询权限的连接。管理页面使用真实浏览器会话与 CSRF，不接受 Bearer 自行创建新授权；关闭新登录后仍可撤销。退出 CLI 撤销整次授权，其他独立授权与网页会话保留。账户停用会撤销其全部 CLI 授权，包含身份查询凭据；恢复账户不会恢复旧访问或刷新令牌。客户端不打印凭据，也不会自动把凭据配置给第三方 MCP 客户端。安装、命令、输出约定与配置见 [CLI 使用说明](cli.md)。

## API 约定

| 方法与路径 | 请求 / 行为 |
| --- | --- |
| `GET /api/admin/users/{id}/access` | 管理员读取账户状态、最近原因和操作人 |
| `POST /api/admin/users/{id}/access` | `{ "banned": true, "reason": "原因" }`；事务内停用或恢复并写入审计 |
| `GET /api/session` | 返回 `user`、`admin`、`super_admin`、`staff`、`bootstrap`、`csrf`；`admin` 表示业务工作台权限，`super_admin` 仅代表 Passkey 超管；未登录时 `user`、`staff` 和 `csrf` 为 null |
| `POST /api/auth/email/start` | `{ "email": "name@example.com" }`；设置浏览器挑战 Cookie |
| `POST /api/auth/email/verify` | `{ "email": "name@example.com", "code": "12345678" }`；消费挑战并建立会话 |
| `GET /api/auth/google/start` | 发起 Google 授权并设置挑战 Cookie |
| `GET /auth/google/callback` | 消费 state；成功以 303 跳转 `/app` |
| `POST /api/auth/logout` | CSRF 验证后删除当前会话 |
| `GET /api/admin/auth/status` | 返回是否已初始化、是否存在有效的首次引导口令 |
| `POST /api/admin/auth/bootstrap` | `{ "password": "one-time-secret" }`；只建立引导会话 |
| `POST /api/admin/passkeys/register/start` | `{ "name": "My security key" }`；需要 CSRF 和最近认证 |
| `POST /api/admin/passkeys/register/finish` | `{ "credential": <WebAuthn registration JSON> }`；需要 CSRF |
| `POST /api/admin/passkeys/login/start` | `{}`；创建一次性登录挑战 |
| `POST /api/admin/passkeys/login/finish` | `{ "credential": <WebAuthn authentication JSON> }`；验证并建立管理员会话 |
| `GET /api/admin/passkeys` | 读取已登记密钥的 ID、名称、创建时间及最后使用时间 |
| `DELETE /api/admin/passkeys/{id}` | 需要 CSRF、完整管理员会话和最近认证；保留至少一把 |

HumanGrant 的代理请求、本人确认、凭据领取与撤销接口见 [HumanGrant 说明](humangrant.md)。浏览器审批与授权列表使用各自返回的 `X-HG-CSRF`，不能用普通会话的 CSRF 值代替。

两个 Passkey `start` 响应都把标准浏览器请求选项放在 `{ "options": { "publicKey": ... } }` 中。挑战 ID 通过 HttpOnly Cookie 关联，不接受客户端提供的服务器验证状态。前端负责标准 base64url 与 ArrayBuffer 转换。

## 验证范围

`cargo test --test auth_flows` 覆盖真实 HTTP 路由、SQLite 会话与权限、验证码重试、首次管理员注册、后续登录、多把 Passkey、删除/撤销和跨来源/CSRF 拒绝。WebAuthn 测试使用软件认证器产生并验证真实密码学签名。`cargo test auth::tests` 覆盖限流、哈希、挑战一次性、过期以及封禁后的 Google 授权代次校验。`cargo test --test account_access` 覆盖管理员权限、CSRF、原因校验、凭据和 Webhook 撤销、解除封禁后旧凭据不能复活，以及不影响其他账户。

这些测试没有发送真实邮件、访问真实 Google 账户、操作实体认证器或调用真实商户扣款。上线前仍需用实际 HTTPS 域名检查 SMTP 投递、Google 回调、至少两把独立 Passkey 的注册和登录，以及浏览器跨设备行为。需要保持服务器时钟准确。

## English overview

Users authenticate with a browser-bound, one-time email code or Google's server-side authorization-code flow with state and PKCE. Profiles and roles have no public directory. Contact details and résumés are shared only with the platform and explicitly introduced counterparties; earlier private conversations remain separate. Sign-in addresses are visible to the account owner, an AI authorized to read that owner's profile, and platform administrators. Existing accounts are not automatically merged by an email returned from a new Google identity.

The production origin is `https://hireferry.lmm.best`, with the exact Google callback `https://hireferry.lmm.best/auth/google/callback`. SMTP messages default to `Reply-To: support@lmm.best`; the sending address is configured separately. These defaults do not establish a live provider connection. The two supplied credential-file paths were unavailable in the delivery environment, and no production credentials were imported from them.

HumanGrant adds AI access after explicit member approval. The member chooses permissions and expiry; the original AI key can claim a separate credential once. Each use requires a signed request, and access can be revoked. Existing Bearer tokens remain available. Remote MCP accepts Bearer or signed HumanGrant credentials, not a browser Cookie. See [HumanGrant](humangrant.md) for the protocol and setup.

Administrators do not use HumanGrant. They begin with a local, single-use 10-minute secret. It grants a 5-minute enrollment session only, never console access. A verified first Passkey completes setup. Later sign-ins require Passkeys; additions and removals require recent authentication, and the last credential cannot be removed. Store a second independent credential before deployment.

Session secrets and personal AI tokens are stored only as hashes. Cookies are HttpOnly and SameSite=Lax, with Secure on HTTPS. Browser mutations require CSRF and origin validation. Suspension is checked on every authenticated request. Suspending a member deletes their sessions, revokes personal AI tokens and pending email challenges, disables webhooks and cancels queued delivery. Restoring access never revives those credentials or webhook switches; a fresh sign-in is required, including for a Google flow started before suspension. Reasoned administrative changes are audited. Personal MCP tokens never grant administrator privileges.
