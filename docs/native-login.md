# 应用与系统浏览器之间的登录返回

移动客户端使用系统浏览器完成 Google 登录，业务页面继续运行在应用 WebView。两者的 Cookie 存储彼此独立；不能通过在浏览器里登录就假定应用也已登录。本功能通过一次性的、明确确认的请求为原 WebView 创建自己的会话。

## 用户流程

1. 在应用中点击 Google 登录，原生导航策略将当前 WebView 带到同一站点的 `/auth/app`。
2. 过渡页请求一个有效期 **5 分钟**的连接，显示核对编号。服务端把随机校验凭据放在该 WebView 的 HttpOnly Cookie 中，页面脚本无法读取它。
3. 点击“打开浏览器确认”。系统浏览器打开 `/auth/app/authorize?request=…`，链接仅包含随机请求编号。用户可以使用已有浏览器会话，也可以在该页通过 Google 或邮箱验证码登录。
4. 浏览器显示账户、站点、相同核对编号和应用将获得的账户权限。用户勾选确认并点击“确认登录应用”之后，服务端才批准连接。
5. 点击固定的 `hireferry://login-return` 唤回应用，或手动切回应用。该链接不含 code、token、服务器地址或跳转目标，不导入任何身份；原 WebView 通过自己的 HttpOnly 凭据领取正常的 `hf_session` Cookie，然后进入 `/app`。

应用挂起时轮询暂停；恢复可见或收到原生固定返回事件时立即重新检查。常态轮询间隔为 3 秒，每个网络请求有 15 秒截止时间。冷启动只回到本地连接页，用户需要重新发起登录；没有尝试用外部 URI 恢复身份。

Google 的浏览器授权仍走现有服务端 PKCE 和挑战 Cookie。新增的 `app_request` 只允许引用服务器中尚未结束的请求；Google 成功或取消后只返回固定的应用确认页，没有开放任意 `redirect_uri`。

## 权限与生命周期

- 目前仅支持普通用户账户。总管理员、引导管理员、活动团队员工和经理不能通过此流程转移会话，必须直接在应用中登录。员工和经理按数据库中的实际成员身份检查，不只检查会话上的 `admin` 标志。
- 浏览器确认需要有效 Cookie 会话、准确的请求 Origin 和 CSRF token；Bearer/API/CLI 凭据不能批准连接。没有自动同意步骤。
- 服务端只保存校验凭据的哈希。公开请求编号和核对编号不具有兑换能力；另一个 WebView 的 Cookie 也不能使用它。
- 批准和领取均在事务中重新检查账户与浏览器会话。注销、封禁、过期或在批准后成为员工/经理都会阻止领取。
- 领取只成功一次，并发请求也只产生一个会话。新会话继承源会话的认证时间，有效期不晚于源会话，不把一次确认当成重新认证。
- 应用和浏览器会话创建后彼此独立，日常退出只结束当前会话。原有账户封禁和会话管理规则继续生效。
- 取消可与领取并发执行。服务端保留本次领取所签发会话的哈希；即使兑换已提交、响应还没送达，取消也只撤销这一份新应用会话，保留浏览器原会话。前端收到取消确认前不会显示取消成功；取消失败时停止继续轮询，允许重试。
- 页面、状态和授权响应均为 `Cache-Control: no-store`，并保留网站现有的 `no-referrer`、Origin 和 CSP 策略。请求编号不写进审计描述；会话和校验凭据不会出现在 JSON、URL、localStorage 或原生调用参数中。

## 路由契约

| 路由 | 调用方与行为 |
| --- | --- |
| `GET /auth/app` | WebView 登录过渡页 |
| `GET /auth/app/authorize?request=…` | 系统浏览器确认页；请求必须仍然有效 |
| `POST /api/auth/app/start` | 同源 WebView 创建或恢复当前连接；校验凭据只通过 HttpOnly Cookie 发送 |
| `GET /api/auth/app/context?request=…` | 返回当前浏览器自己的可确认身份、站点和核对编号 |
| `POST /api/auth/app/decide` | `{request, approve}`，要求浏览器 Cookie、Origin 与 CSRF |
| `POST /api/auth/app/redeem` | `{request}`，要求同源及发起 WebView 的校验 Cookie；返回 pending / denied / complete |
| `POST /api/auth/app/cancel` | `{request}`，凭发起 Cookie 或本次新签发的会话撤销连接 |

成功领取仅返回 `{"status":"complete","redirect":"/app"}`，会话通过 `Set-Cookie` 发送。请求创建上限为全站每 5 分钟 100 个；恢复同一未过期请求不占用新名额。新增表使用幂等创建，保持既有数据库版本与升级路径。

## 部署与验收

服务端与移动端应一起验收。旧服务端不具备 `/auth/app` 路由，不能宣称已经支持 Google 登录返回；邮箱验证码仍可直接在 WebView 使用。Google 本身仍要求维护者已配置真实 Google OAuth 客户端和允许的 HTTPS 回调，本文没有提供或生成凭据。

可自动验证的部分使用临时 SQLite、测试会话和两个独立 Chromium Cookie 上下文：

```sh
cargo test --locked --test app_login --test auth_flows --test cli_auth
cargo build --locked --bin hireferry
HFY_PLAYWRIGHT_MODULE=playwright node tests/app-login.browser.mjs
```

Rust 回归覆盖不匹配 Origin、缺少/错误校验 Cookie、CSRF、过期、拒绝、撤销、并发领取、取消竞态、实际员工身份、审批后身份变更和认证时间继承。浏览器回归覆盖独立 Cookie 存储之间的真实 HTTP 流程、显式确认、邮箱验证码、取消与迟到响应、中英文和窄屏布局。Google 外部账号与真实设备登录仍需要对应开发者配置和实际设备验收；本测试不会调用生产邮箱或登录生产账户。
