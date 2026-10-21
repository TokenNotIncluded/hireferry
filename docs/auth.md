# Authentication / 认证与管理员 Passkey

This document describes the implemented service behavior. Configuration lives in
TOML; changing it requires restarting HireFerry. It is not editable in the web
console.

## 用户登录与隐私

普通用户使用 Google 或邮箱验证码。首次登录创建 `unassigned` 账户，再通过界面选择求职者或招聘方。选择招聘方会创建开通订单，收到有效支付通知后才开通招聘权限。

登录邮箱是账户身份标识，默认不在公开资料中返回。`public_email` 是用户明确填写的公开联系邮箱；简历只有在用户开启公开状态后才进入人才目录。网站不要求手机号。管理员为服务运营可以查看账户及内容；公开 API、普通会话和 MCP 令牌不能访问他人私密资料。

### 邮箱验证码

配置 `[smtp]` 的 `enabled`、`host`、`port`、`username`、`password`、`from` 和 `starttls`。

- `starttls = true`：TLS 必需的 STARTTLS，常见端口 587。
- `starttls = false`：隐式 TLS，常见端口 465；不会回退为明文连接。
- 生产环境关闭 SMTP 时，邮箱登录返回不可用。
- 仅当 `development = true` 且 SMTP 关闭时，API 返回 `dev_code`，前端显示本地验证码。开发模式必须使用回环绑定与 localhost 来源，不能用于公网部署。

验证码为 8 位随机数字，10 分钟有效，只能在发起请求的浏览器中使用。数据库保存验证码与随机挑战组合的哈希，不存储明文验证码。再次申请会使旧码失效；每个挑战最多尝试 5 次。

服务端限制同邮箱 60 秒内最多申请 1 次、1 小时 10 次，并设置全局申请限制与验证请求限制。验证码不会写入应用日志。发信失败会撤销对应挑战，无法凭一个尚未投递的挑战继续登录。

### Google

创建 Google OAuth Web 应用，在 TOML 填写客户端 ID 和 secret。授权回调 URI 必须精确配置为：

```text
https://YOUR-HOST/auth/google/callback
```

本地开发可单独登记 `http://localhost:3000/auth/google/callback`。

流程使用浏览器绑定的随机 state、PKCE 和一次性服务端挑战；后端向 Google 交换授权码，并从 Google 用户信息接口取得 `sub` 与已验证邮箱。客户端提交的邮箱或用户标识不能用于创建已验证身份。成功后跳转 `/app`。

已有 Google 身份按 `sub` 匹配。如果首次 Google 登录的邮箱已经属于其他账户，服务拒绝自动合并，提示改用邮箱验证码登录。当前版本没有「连接 Google」或账户合并页面。这避免把仅凭同名邮箱的第三方身份绑定到既有账户。

参考：[Google OpenID Connect](https://developers.google.com/identity/openid-connect/openid-connect)。

## 管理员首次开通

管理员不属于普通用户，也不能通过 Google、邮箱验证码或个人 MCP 令牌登录总控台。

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

## 个人 AI 令牌

在登录后的「AI 连接」页面创建令牌。必须由交互式浏览器会话授权创建，已有 Bearer 令牌不能再创建令牌。每个账户最多 10 个有效令牌，30 天过期，只显示一次，数据库保存哈希，可立即撤销。

使用方式是 `Authorization: Bearer <token>`。令牌可使用该用户的业务工具，并受相同所有权、角色、支付与封禁检查限制。它不能使用管理员 API。MCP 为 `2025-11-25` 无状态 Streamable HTTP，当前需要手动配置请求头，没有 OAuth 自动发现。令牌不应放入公开仓库、网页源码或链接中。

WebMCP 使用当前浏览器会话，通过草案 `modelContext` API 提供工具。在支持的浏览器中同样受服务端授权与计费规则约束；浏览器未实现接口时不注册工具。

## API 约定

| 方法与路径 | 请求 / 行为 |
| --- | --- |
| `GET /api/session` | 返回 `user`、`admin`、`bootstrap`、`csrf`；未登录时 `user` 和 `csrf` 为 null |
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

两个 Passkey `start` 响应都把标准浏览器请求选项放在 `{ "options": { "publicKey": ... } }` 中。挑战 ID 通过 HttpOnly Cookie 关联，不接受客户端提供的服务器验证状态。前端负责标准 base64url 与 ArrayBuffer 转换。

## 验证范围

`cargo test --test auth_flows` 覆盖真实 HTTP 路由、SQLite 会话与权限、验证码重试、首次管理员注册、后续登录、多把 Passkey、删除/撤销和跨来源/CSRF 拒绝。WebAuthn 测试使用软件认证器产生并验证真实密码学签名。`cargo test auth::tests` 覆盖限流、哈希、挑战一次性与过期等底层约束。

这些测试没有发送真实邮件、访问真实 Google 账户、操作实体认证器或调用真实商户扣款。上线前仍需用实际 HTTPS 域名检查 SMTP 投递、Google 回调、至少两把独立 Passkey 的注册和登录，以及浏览器跨设备行为。需要保持服务器时钟准确。

## English overview

Users authenticate with a browser-bound, one-time email code or Google's server-side authorization-code flow with state and PKCE. Sign-in addresses are private; public contact addresses and résumé publication are separate choices. Existing accounts are not automatically merged by an email returned from a new Google identity.

Administrators begin with a local, single-use 10-minute secret. It grants a 5-minute enrollment session only, never console access. A verified first Passkey completes setup. Later sign-ins require Passkeys; additions and removals require recent authentication, and the last credential cannot be removed. Store a second independent credential before deployment.

Session secrets and personal AI tokens are stored only as hashes. Cookies are HttpOnly and SameSite=Lax, with Secure on HTTPS. Browser mutations require CSRF and origin validation. Suspension is checked on every authenticated request. Personal MCP tokens never grant administrator privileges.
