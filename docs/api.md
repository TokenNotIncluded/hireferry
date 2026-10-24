# HTTP API、MCP 与 WebMCP

本文件描述当前实现。依据：[用户 API](../src/api.rs)、[访问控制](../src/access.rs)、[CLI 授权](../src/cli_auth.rs)、[聊天与委托](../src/chat.rs)、[流式消息与事件](../src/chat_stream.rs)、[聊天能力发现](../src/chat_capabilities.rs)、[管理员业务](../src/admin.rs)、[管理员 AI 令牌](../src/admin_ai.rs)、[HumanGrant](../src/humangrant.rs)、[远程 MCP schema](../src/mcp.rs)、[支付验签](../src/payments.rs)、[支付/退款事务](../src/commerce.rs)、[用户 Webhook](../src/webhooks.rs)、[浏览器注册代码](../static/app.js)。

聘渡由平台 HR 居中服务。账号资料、简历、头像与岗位均不提供公共目录。每位用户与平台的私聊独立保存；平台建立明确匹配后，双方才能读取对应资料，并进入新的独立匹配会话。管理员可以读取全平台业务历史，包含封禁、拒绝匹配与岗位删除后的归档会话；归档会话不可发送新消息。

当前数据库版本为 schema 16。schema 16 增加唯一用户名、可选公开个人主页和 AI profile 链接；schema 15 增加原生邮件通知开关与持久化发送队列，见[邮件通知](email-notifications.md)；schema 14 增加流式消息和事件日志。schema 9 新增优惠申请、优惠码和订单额度预占记录；schema 10 新增有限期快速反馈，schema 11 新增团队员工邀请与经理身份，schema 12 增加反馈持久化配额、重复保护与清理索引；schema 13 新增独立的业务员工、绑定邮箱的邀请和 AI 员工凭据。schema 8 的 CLI 授权、访问及刷新凭据哈希记录，以及 schema 7 的 HumanGrant 申请、授权和防重放状态保持不变。此前 schema 6 中，旧岗位和委托迁移为历史 CNY，原订单/账本/退款币种和金额不变；零费用订单不写现金账本。迁移与恢复步骤见 [部署交接](deployment-handoff.md#9-备份升级与恢复)。

优惠码申请、审批、免费激活、发布预览与次数管理接口见 [优惠码说明](promo-codes.md)。`POST /api/role` 可携带 `promo_code`；`POST /api/jobs` 可携带 `promo_code`、防重试的 `request_key` 和已确认报价 `expected_amount_cents`。减免百分比 100 表示免费，不经过支付网关。

超级管理员可在 `/admin?tab=staff` 邀请人类团队经理（schema 11），通过 SMTP 或一次性邀请链接要求受邀者使用已验证邮箱接受；经理操作仅限已授权的招聘业务，不授予超级管理员财务及系统权限。接口见 [团队员工邀请](staff.md)。

超级管理员也可在「团队成员」中邀请具有财务及账户处理权限的真人业务员工，或签发可独立撤销的 AI 员工凭据。经理与业务员工使用独立成员记录；邀请或停用其中一种身份不会改写另一种身份。优惠码、反馈处理、团队和平台凭据管理不因此授予业务员工。详见 [团队 API](team.md#http-api)。

## 1. 通用约定

- JSON 请求使用 `Content-Type: application/json`。时间戳为 UTC Unix 秒；统计日期为包含首尾日期的 `YYYY-MM-DD`。
- 所有金额按记录币种的整数最小单位保存，沿用 `_cents` 字段名兼容客户端。`currency:"USD",salary_cents:300000` 表示 USD 3,000；JPY 的相同整数表示 JPY 300,000。底薪不包含奖金、提成。不要假设统一除以 100，应读取 `currency_decimals`。
- 业务错误通常为 HTTP 400/401/403/404/409/429/503 与 `{"error":"说明"}`。JSON 提取及请求体超限等框架错误可能不同。无权读取的资料、岗位、头像通常返回 404。
- 全局请求体上限 3 MiB；头像、文本另有限制。业务输入 DTO 拒绝未知字段；无参数操作不要附加业务字段。OAuth 表单参数按协议处理，见下方 CLI 授权说明。
- 浏览器 Cookie 为 `hf_session`，HttpOnly、SameSite=Lax，HTTPS 下 Secure；默认 24 小时，可由 TOML 调整。
- Cookie 写请求携带 `Origin`（配置的 public_url 来源）和 `X-CSRF-Token`（来自 GET /api/session）。尚未登录的认证接口验证同源及浏览器挑战。
- Bearer 为 `Authorization: Bearer <token>`。普通用户、管理员两类令牌严格隔离；管理员不能冒充普通用户调用本人资料接口。
- HumanGrant 的远程业务请求只走 `POST /mcp`，使用 `Authorization: HumanGrant <access_token>` 并由原申请密钥签名；它不能作为个人 Bearer 使用，也不能直接调用普通用户 HTTP 业务接口。具体签名要求见 [HumanGrant 说明](humangrant.md)。
- 每次普通用户业务 Bearer 请求检查封禁、撤销、过期和 AI 资格。求职者免费；招聘方须已开通，并拥有至少一个发布订单已完成、状态为 published、底薪达到该岗位 `ai_min_salary_cents` 的岗位。门槛使用岗位自己的币种；默认模板为 CNY 300000 最小单位，允许配置为零。免费发布的已完成订单可满足发布条件，`headcount=0` 仍满足这里的已发布条件。CLI 的身份查询凭据不要求 AI 资格，也不授予这些业务权限。
- WebMCP 请求携带 `X-HireFerry-AI: webmcp` 并使用当前 Cookie/CSRF，服务端再次检查 AI 资格。关闭或删除最后一个合资格岗位后，已有招聘方 AI 令牌立即不可用；可用人工工作台执行符合规则的回收站恢复。
- `public_email` 是历史字段名，现表示向平台及明确匹配对象提供的联系邮箱。`visible` 兼容旧客户端，写入固定 false，不授予公开访问。登录邮箱 email 只向本人、获准读取本人资料的 AI 和管理员返回。

| 身份 | 业务范围 |
| --- | --- |
| 未登录 | 首页、条款、隐私/退款/联系页面、公开配置、登录流程、健康检查；支付返回页不显示私人订单 |
| 普通用户 Cookie | 本人资料/岗位/订单、平台私聊、明确匹配对象和对应匹配会话；按角色操作 |
| CLI `identity` Bearer | 仅 `/oauth/userinfo` 身份、当前资格和能力查询；可续期或撤销自己的连接 |
| 普通用户 Bearer / WebMCP | 普通用户 Cookie 的业务范围，每次重新检查 AI 资格；不能自行签发令牌 |
| 普通用户 HumanGrant | 仅限本人明确批准的 MCP 操作与资源；每次检查签名、原申请密钥、授权期限、撤销和当前 AI 资格；无管理员权限 |
| 管理员 Cookie | 全平台业务、会话历史、订单/统计/审计，以及 Passkey/管理员令牌管理 |
| 管理员 Bearer / WebMCP | 全平台业务操作；无 Passkey、令牌签发、系统配置或伪造付款操作 |

introduced/interview/hired 匹配授予对应双方访问权。declined、任一方封禁、岗位删除立即撤销普通参与者的匹配资料与会话访问；岗位仅 closed 或 headcount=0 时已有匹配仍可沟通。首次删除恢复后，原匹配历史按当前权限重新可见。暂停委托或人数为 0 阻止新匹配，不自动撤销已有有效匹配。完整规则见 [岗位管理](jobs.md)。

## 2. 站点、登录与管理员认证

正式来源为 `https://hireferry.lmm.best`，支持邮箱为 `support@lmm.best`。配置和外部接入清单见 [部署交接](deployment-handoff.md)，认证机制见 [认证说明](auth.md)。以下七个路径用于正式接入；URL 并不表示已经完成外部联调。

| 用途 | 方法与完整地址 |
| --- | --- |
| Google OAuth 回调 | GET `https://hireferry.lmm.best/auth/google/callback` |
| 服务条款 | GET `https://hireferry.lmm.best/terms` |
| 隐私政策 | GET `https://hireferry.lmm.best/privacy` |
| 退款政策 | GET `https://hireferry.lmm.best/refunds` |
| 联系支持 | GET `https://hireferry.lmm.best/contact` |
| Waffo 支付及退款通知 | POST `https://hireferry.lmm.best/api/webhooks/waffo` |
| 统一支付返回页 | GET `https://hireferry.lmm.best/billing/return` |

Stripe 另外使用 `POST /api/webhooks/stripe`，远程 MCP 使用 `POST /mcp`。

| 方法与路径 | 输入 | 返回 / 条件 |
| --- | --- | --- |
| GET `/healthz` | 无 | 204，进程存活 |
| GET `/api/config` | 无 | {name,public_origin,default_language,support_email,source_url,google_enabled,email_enabled,humangrant_enabled,development,providers,payment_currencies,pricing,limits}；无服务密钥 |
| GET `/terms`、`/privacy`、`/refunds`、`/contact` | 无 | 中英双语公开说明页面；联系地址取 site.support_email |
| GET `/api/legal/{kind}` | kind 为 terms/privacy；query lang=zh/en，默认 zh | {html,version,effective_date}；未知文档或语言 404，不返回文件路径或配置密钥 |
| GET `/billing/return` | query order_id?、provider?、canceled? | 页面壳；登录后读取本人 GET /api/orders/{id}，query 不能改变订单状态 |
| GET `/api/session` | Cookie 可选 | {user,admin,bootstrap,csrf}；匿名 user/csrf 为 null |
| POST `/api/auth/email/start` | {email}，同源 | {sent:true}，设置浏览器挑战；仅回环 development 且 SMTP 未启用时有 dev_code |
| POST `/api/auth/email/verify` | {email,code}，同一挑战 Cookie、同源 | {user,csrf} 和登录 Cookie；8 位验证码 |
| GET `/api/auth/google/start` | 无 | 跳转 Google，state/PKCE 绑定浏览器 |
| GET `/auth/google/callback` | Google 返回 code、state | 验证后跳转 /app |
| POST `/api/auth/logout` | Cookie、CSRF | {ok:true}，撤销会话 |
| GET `/api/admin/auth/status` | 无 | {initialized,bootstrap_available} |
| POST `/api/admin/auth/bootstrap` | {password}，同源 | {bootstrap:true,admin:false,csrf}，只能绑定首把 Passkey |
| POST `/api/admin/passkeys/register/start` | {name}，管理员或首次引导 Cookie、CSRF | {options}，WebAuthn 注册选项 |
| POST `/api/admin/passkeys/register/finish` | {credential}，同一会话/挑战、CSRF | {ok:true,admin:true,bootstrap:false,csrf} 和新 Cookie |
| POST `/api/admin/passkeys/login/start` | {}，同源 | {options}，WebAuthn 登录选项 |
| POST `/api/admin/passkeys/login/finish` | {credential}，同一挑战、同源 | {ok:true,admin:true,bootstrap:false,csrf} 和新 Cookie |
| GET `/api/admin/passkeys` | 真实管理员 Cookie | {passkeys:[...]} |
| DELETE `/api/admin/passkeys/{id}` | 真实管理员 Cookie、CSRF、最近认证 | {ok:true}；不能删最后一把 |

CLI admin-bootstrap 口令一次性、10 分钟有效。首次引导会话 5 分钟有效，无总控台权限；首把 Passkey 登记后密码登录永久关闭。最多 20 把 Passkey，后续增删需最近 600 秒 Passkey 认证。Google 与已有相同邮箱账户不自动合并。

公开 `pricing` 包含默认 `currency`、`currency_decimals`、`allowed_currencies`、`currency_options`，以及 `onboarding_cents`、发布/修改的 `*_mode`、`*_bps`、`*_fixed_minor`、`max_edits` 和 `employer_ai_min_salary_minor`。每个 `currency_options` 项提供币种、精度和该币种的两项固定费用及 AI 门槛。`payment_currencies` 按已启用网关列出它当前允许的币种，例如 `{stripe:["USD","JPY"],waffo:["USD"]}`；它独立于新岗位的可选币种，旧订单仍可能使用其他历史币种。`limits` 提供 `max_job_headcount`、`max_pending_orders_per_user`、`list_page_size`、`list_max_page`、`workbench_page_size`、`recent_list_limit`。工作台默认页大小可配置为 1–100（模板 25），最近记录上限可配置为 1–1000（模板 200）。

价格配置同时发布原价与折扣价：`onboarding_original_minor` / `onboarding_minor`、`onboarding_discount_percent`，以及每个 `currency_options` 的 `publish_fixed_minor` / `publish_discount_percent` 和 `edit_fixed_minor` / `edit_discount_percent`。百分比是减免比例，不是剩余比例；`0` 表示原价，`100` 表示免费。工资比例模式也先按原始 `bps` 计算，再应用折扣。折扣后的最小单位金额为 0 时，只有折扣正好为 100% 才允许免费。首页、定价区、开通页和相关价格说明会把原价显示为删除线，再显示折扣价。

当开通、发布或修改的最终金额为 0，服务端直接创建 `provider="free"`、`status="paid"` 的订单或立即应用修改，不调用 Stripe/Waffo；支付页面显示“免费完成”。调用方不应为免费操作传入支付网关。

公开法律文档来自独立中英 Markdown 模板；`[legal.values]` 和 `[legal.document_files]` 配置公开政策值及完整替代文本。费用模式、币种、整数金额精度、修改次数与 AI 门槛从运行配置派生，不能被政策值覆盖。加载配置时校验所有文档和变量，渲染过滤原始 HTML 与不安全链接。详见 [政策配置](legal-policies.md)。

### HumanGrant 入口与确认

浏览器入口是 `/humangrant`，本人确认页是 `/human/{link}`。本人继续使用 Google 或邮箱验证码登录；有效登录直接显示申请内容。权限、资源和期限由本人明确选择。批准后，原申请 AI 可单次领取独立凭据；本人在「AI 连接」页面查看或撤销。管理员不能通过此流程授权。

| 方法与路径 | 输入 / 身份 | 返回 / 行为 |
| --- | --- | --- |
| GET `/api/humangrant/config` | 无 | 协议版本、客户端、权限范围与配置限制，无密钥 |
| POST `/v1/requests` | 原申请 AI 的签名请求 | 创建申请，返回独立的本人确认链接与状态地址 |
| GET `/v1/requests/{id}/status.md` | 原申请 AI 的签名请求 | 私有 Markdown 状态；不会批准或领取凭据 |
| POST `/v1/requests/{id}/claim` | 原申请 AI 的签名请求 | 本人批准后，单次领取独立 AI 凭据 |
| GET `/api/humangrant/review/{link}` | 本人 Cookie 可选 | 申请详情、当前账户和可授予权限；打开页面不授权 |
| POST `/api/humangrant/review/{link}/approve` | 本人 Cookie、同源、确认页的 `X-HG-CSRF`；`{permissions,expires_at}` | 再次检查实际账户并批准所选范围 |
| POST `/api/humangrant/review/{link}/deny` | 本人 Cookie、同源、确认页的 `X-HG-CSRF`；`{}` | 拒绝申请 |
| GET `/api/humangrant/grants` | 本人 Cookie | 本人授权列表与该列表的确认凭据 |
| DELETE `/api/humangrant/grants/{id}` | 本人 Cookie、同源、授权列表的 `X-HG-CSRF`；`{}` | 撤销本人对应授权 |

HumanGrant 的完整请求字段、取消、会话检查、签名与有效权限计算见 [HumanGrant 说明](humangrant.md)。本节的浏览器审批确认凭据与普通 API 的 `X-CSRF-Token` 不可互换。

### CLI 授权与连接管理

`hfy` 是公共客户端，默认 `client_id=hireferry-cli`，没有客户端密钥。设备码授权遵循 [RFC 8628](https://www.rfc-editor.org/rfc/rfc8628.html)；浏览器授权使用 [RFC 8252](https://www.rfc-editor.org/rfc/rfc8252.html) 的系统浏览器和回环回调，并强制 S256 PKCE。安装、配置与操作示例见 [CLI 文档](cli.md)。

默认 scope 是 `identity`，仅允许查询自己的身份、AI 资格和当前能力。普通用户明确申请 `mcp` 后，服务端按现有 AI 资格授予 `identity mcp`；管理员明确申请 `admin` 并通过最近的 Passkey 认证后，可获得 `identity admin`。两种业务范围不能组合。未选身份的新用户或暂不具备 AI 资格的招聘方可以使用 `identity`，不会因此获得 MCP、业务 API 或管理员能力。

| 方法与路径 | 输入 | 返回 / 条件 |
| --- | --- | --- |
| GET `/.well-known/oauth-authorization-server` | 无 | 发行者、各 OAuth 端点、支持的授权方式、scope 与 S256 能力 |
| POST `/oauth/device/code` | form：`client_id`、可选 `scope` | `device_code`、`user_code`、`verification_uri`、`verification_uri_complete`、`expires_in`、`interval` |
| GET `/oauth/device` | 可选 `user_code` | 网页设备确认页；登录后核对短码、账户及权限 |
| GET `/oauth/authorize` | query：`client_id`、`response_type=code`、`redirect_uri`、`state`、`code_challenge`、`code_challenge_method=S256`、可选 `scope` | 网页授权确认页；打开页面不授予权限 |
| GET `/api/cli/authorization` | 当前 Cookie、授权页面的请求参数 | 当前账户、申请范围、客户端、设备码或回调地址及有效期；`can_approve`、`approval_error`、`requires_recent_auth` 说明当前账户能否批准；不返回令牌 |
| POST `/api/cli/authorization` | Cookie、Origin、CSRF；JSON：授权参数和 `approve` | 允许或拒绝；浏览器方式返回带 `code` 或 `error`、`state`、`iss` 的回调地址 |
| POST `/oauth/token` | form：`client_id`、`grant_type` 及对应凭据，见下表 | `access_token`、`token_type=Bearer`、`expires_in`、`scope`、`refresh_token`、`refresh_expires_in` |
| GET `/oauth/userinfo` | CLI 访问令牌 Bearer | `{admin,user,scope,expires_at,ai_eligibility,capabilities}`；`user` 为 `{id,name,email,role}` 或 null |
| POST `/oauth/revoke` | form：`client_id`、`token`（访问或刷新令牌） | 200，撤销整个授权族；重复撤销或未知令牌不暴露状态 |
| GET `/oauth/connections` | 浏览器 | 当前账户的 CLI 连接管理页，未登录时先登录 |
| GET `/api/cli/connections` | 当前 Cookie | `{items:[{id,client_id,scope,created_at,expires_at,revoked}]}`；仅本人连接，最多 200 条 |
| DELETE `/api/cli/connections/{id}` | 当前 Cookie、Origin、CSRF | 204，撤销本人的整个授权族；其他身份的 ID 返回 404 |

这里的 form 指 `application/x-www-form-urlencoded`，不是 JSON。令牌端点按授权方式接收以下参数：

| `grant_type` | 其他参数 | 行为 |
| --- | --- | --- |
| `urn:ietf:params:oauth:grant-type:device_code` | `device_code` | 仅批准后换取令牌；每次查询遵守服务端间隔 |
| `authorization_code` | `code`、原 `redirect_uri`、`code_verifier` | 一次性兑换；验证客户端、精确回调和 PKCE |
| `refresh_token` | `refresh_token`、可选 `scope` | 轮换访问与刷新令牌；范围只能保留或收窄 |

浏览器回调仅允许 `http://127.0.0.1:<port>/callback` 或 `http://[::1]:<port>/callback`，端口由客户端临时监听决定。客户端校验 `state` 和返回的 `iss`。令牌接口忽略协议允许的未知参数，但不接受未知 scope、缺失必填项或其他授权类型。

当前账户不符合请求范围时，确认页仍展示账户与权限，并允许拒绝或切换账户；批准按钮不可用。这个展示状态不替代服务端检查，提交批准时仍重新验证会话、AI 资格及最近的管理员 Passkey 认证。

设备码签发、令牌兑换与撤销端点的协议错误使用 `{error,error_description}`；`/oauth/userinfo` 和浏览器连接管理沿用 `{error:"说明"}` 与 HTTP 状态。设备轮询的 `authorization_pending` 表示尚未决定；`slow_down` 要求之后每次查询间隔增加 5 秒；`access_denied` 或 `expired_token` 停止当前授权。HTTP 429 及限流提示不应通过快速重复请求绕过。响应设置禁止缓存，日志和 CLI 标准输出不得包含访问令牌、刷新令牌、设备秘密码或 PKCE 验证串。

默认访问令牌有效 15 分钟，刷新授权在最初批准起 30 天到期。续期会原子地使旧访问令牌和旧刷新令牌失效；再次使用已消费的刷新令牌会撤销整个授权族，客户端不能在结果不确定时盲目重试。刷新时重新检查账户状态、当前配置与对应 AI 权限，不扩大原授权或延长总期限。

只申请身份的连接通过 `/oauth/connections` 管理，不列入旧的个人或管理员 AI 令牌列表。具有业务范围的 CLI 访问令牌也出现在相应的既有令牌列表；撤销其中任一令牌同样撤销其刷新授权。网站会话、其他独立 CLI 连接和手动创建的令牌不会被同次退出自动撤销。

`[cli_auth] enabled=false` 阻止新的授权和所有令牌兑换，包括续期；已有访问令牌在到期或撤销前仍可查询身份、按原权限访问业务。撤销接口和网页连接管理继续可用。改变允许的范围或客户端 ID 可进一步限制现有连接，具体配置与范围见 [CLI 文档](cli.md#服务器设置)。

## 3. 普通用户 HTTP API

这些业务接口接受本人 Cookie 或具备 AI 资格的个人 Bearer；个人令牌创建仅限 Cookie。

### 资料、身份与头像

| 方法与路径 | 输入 | 返回 |
| --- | --- | --- |
| GET `/api/ai/eligibility` | 用户或管理员身份 | {eligible,reason,role,min_salary_cents,currency,currency_decimals,currency_thresholds?}；展示当前配置门槛，实际资格逐岗位按保存门槛计算 |
| GET `/api/profile` | 无 | 本人 User |
| PUT `/api/profile` | {name,public_email,resume_md,visible?} | 更新后的 User |
| POST `/api/profile/avatar` | multipart 首个文件，建议字段名 file | User |
| PUT `/api/profile/avatar` | {svg} | User |
| GET `/api/avatars/{id}` | 本人、明确匹配对象或管理员 | 清洗后的 PNG/SVG，private/no-store |
| POST `/api/role` | {role:"seeker"或"employer",provider?:"stripe"或"waffo"} | {user,order}；order 可为 null |
| POST `/api/markdown` | {text}，用户或管理员 | {html}，服务端清洗后的预览 |

身份选择后不能切换。招聘方可先选身份而不指定网关，此时无支付订单；指定当前默认币种可用的网关时创建/复用开通订单。`onboarding_cents=0` 时直接开通，`order=null`。已有待付款开通订单保留创建时的金额、币种和网关。新用户先通过浏览器注册并选择身份，再获取 AI 资格。

姓名≤120 UTF-8 字节，简历/Markdown≤80000 字节，拒绝 NUL。联系邮箱可为空，否则须有效；仅求职者可写非空简历。图片只接受 PNG/JPEG/WebP，原始字节≤2 MiB，宽高 1–2048 像素；解码并重编码为不超过 256×256 的 PNG，去除元数据与附加内容。SVG≤64000 字节，只允许静态白名单；脚本、事件属性、外链/xlink 引用拒绝。最终头像写入事务再次检查封禁。

### 公开目录与公开 MCP

这些接口不需要登录，只返回招聘方已发布且仍在招聘的岗位，以及求职者明确打开公开个人主页后的简历。公开主页链接由本人填写，可以包含 AI、GitHub、X、个人网站和其他 HTTPS 页面；登录邮箱、私聊、匹配历史、订单、草稿和关闭内容不会返回。

| 方法与路径 | 输入 | 返回 / 范围 |
| --- | --- | --- |
| GET `/api/public/jobs` | query `q?`、`page?` | 公开招聘岗位；返回 `jd_md`、清洗后的 `jd_html`、薪资、人数和有限的招聘方公开信息 |
| GET `/api/public/jobs/{id}` | 无 | 单个仍在招聘的公开岗位；暂停、关闭、删除或未付款岗位返回 404 |
| GET `/api/public/people` | query `q?`、`page?` | 明确公开的求职者简历摘要、handle、公开邮箱和主页链接 |
| GET `/api/public/people/{handle}` | 唯一 handle | 单个公开简历；handle 不区分大小写 |
| POST `/mcp/public` | MCP `2025-11-25` JSON-RPC | 无需 Authorization；只读公开工具 `hireferry_public_*`，免费且有界限流 |

公开 MCP 的资源 `hireferry://docs/mcp-public` 提供客户端说明。网页 `/discover` 也会在支持 `document.modelContext` 或 `navigator.modelContext` 的浏览器中注册同一组原生 WebMCP 只读工具。完整字段、调用示例和隐私边界见 [公开 MCP 开发者说明](mcp-public.md)。

### 岗位、匹配与订单

| 方法与路径 | 输入 | 返回 / 范围 |
| --- | --- | --- |
| GET `/api/jobs` | query q?、page? | {items:Job[],total}；仅介绍给本人的具体岗位 |
| GET `/api/jobs/mine` | query q?、page? | {items:Job[],total}；本人未删除岗位，含草稿 |
| GET `/api/my/jobs/trash` | 无 | {items:Job[]}；本人 status=deleted、delete_count=1 的最近记录，最多 limits.recent_list_limit 条 |
| GET `/api/jobs/{id}` | 无 | {job,jd_html,terms,employer}；本人岗位或精确匹配岗位；永久删除返回 404 |
| POST `/api/jobs` | {title,jd_md,salary_cents,currency?,provider?,headcount?} | {job,order}；currency 默认站点币种、headcount 默认 1；付费模式待支付，免费模式直接发布 |
| POST `/api/jobs/{id}/edit` | {jd_md,provider?} | {job,order}；遵循原发布条款，付费模式保留原内容到验签收款，免费模式立即应用 |
| PATCH `/api/jobs/{id}/headcount` | {headcount} | Job；已开通且未封禁招聘方的本人 draft/published，免费不限次数，不改 JD version/edits_used |
| POST `/api/jobs/{id}/close` | 无业务参数 | Job；本人 published 岗位，无待支付修改；设 closed、人数 0 |
| DELETE `/api/jobs/{id}` | {expected_version} | Job；首次进入回收站，恢复后再删为永久；免费 |
| POST `/api/jobs/{id}/restore` | {expected_version} | Job；仅首次删除可恢复一次；免费，不新建订单 |
| DELETE `/api/jobs/{id}/permanent` | {expected_version} | Job 墓碑；直接永久删除，免费且不可恢复 |
| GET `/api/people` | query q?、page? | {items:CounterpartyProfile[],total}；明确匹配的求职者 |
| GET `/api/people/{id}` | 无 | {user:CounterpartyProfile,resume_html}；本人求职资料或匹配对象 |
| GET `/api/matches` | 无 | {items:[...]}；本人有效匹配，含 conversation_id |
| GET `/api/orders` | 无 | {items:Order[]}；本人最近 limits.recent_list_limit 笔 |
| GET `/api/orders/{id}` | 无 | 本人 Order |
| POST `/api/orders/{id}/checkout` | 无业务参数 | 更新后的 Order，含 checkout_url |

带 q/page 的普通列表使用 q 前 100 个字符，page 默认 1，上限为 `[limits].list_max_page`，页大小为 `list_page_size`（默认 2000 页、每页 50）；返回 `limit`、`page`。回收站仅返回最近 `limits.recent_list_limit` 条，无 q/page 参数。搜索不扩大权限。

岗位标题 1–120 字符；JD 非空≤80000 字节；salary_cents 为 1–1000000000 个所选币种的最小单位。`publish_mode`、`edit_mode` 可为 `salary_percent`、`fixed`、`free`；比例模式按 `(salary_minor*bps+5000)/10000` 整数半入取整，结果为零时拒绝，应显式配置免费模式。固定模式使用该币种的独立固定价格，不做汇率换算。默认模板为 CNY 开通 10000 最小单位、发布 1000 基点、修改 100 基点、最多 3 次。

岗位的币种、原底薪、编辑模式/费率/固定金额/次数与 AI 门槛在创建时保存，后续配置不追溯旧岗位。标题、币种和底薪创建后不可改，编辑请求提交它们会被拒绝。付费模式必须选择当前币种可用的网关，创建订单和结账都会验证；免费模式允许省略 provider，返回 `provider="free",amount_cents=0,status="paid"` 的订单，不写现金账本。同岗位最多一笔待付款操作，每人的发布/修改待付款总数受 `max_pending_orders_per_user` 约束（默认 20）。

`headcount` 为人工维护的整数 0 到 `limits.max_job_headcount`（默认 1000000），创建默认 1；不乘入发布费或 JD 修改费，不限制候选匹配总量，也不在匹配/入职时自动扣减。人数为 0 保持 published，但禁止新增或从 declined 恢复匹配；已有匹配、会话与已满足条件的招聘方 AI 资格保留。人数单改允许已有待付款 JD 订单存在，不改变其内容版本。关闭/退款关闭岗位不能靠人数恢复招聘。

`delete_count` 为 0（从未删除）、1（已用首次删除机会，包括恢复后）、2（永久删除）。只有 status=deleted 且 delete_count=1 可以恢复。删除和恢复以 `expected_version` 对照当前 version；冲突返回 409。对仍处于 deleted 的普通删除重试原样返回，不会触发第二次删除。恢复后原旧删除请求的版本已失效，不能误永久删除。永久删除清空标题、JD 和关联订单的 snapshot_json，保留账务与审计；后台删除也必须带版本且直接永久。详见 [岗位状态与恢复](jobs.md)。

published 表示发布已完成，不表示公开招聘目录；只有人数大于 0 才能新匹配。现金付款只由验签回调原子落账，跳转返回不授权。已删除岗位、不再可履行的修改或当前网关不支持的币种不允许创建/复用 checkout，个人订单读取也隐藏其旧 checkout_url。首次删除取消待付款 JD 修改；永久删除取消全部尚待付款的关联订单并清空缓存付款链接。删除、封禁或版本冲突后不能应用的真实到账记为 paid_needs_review，保留实收记录但不复活岗位或覆盖内容。限制新的结账不会丢弃历史订单后来收到的有效签名回调。

订单保留原 `amount_cents`，累计成功退款在 `refunded_cents`；已结算订单净额为两者之差，不额外返回 net_amount_cents 字段。成功部分退款通常为 `partially_refunded`，全额为 `refunded`；原 paid_needs_review 部分退款后仍保持待核对。部分退款保留权益，全额退款按订单种类撤回开通或关闭岗位，JD 修改不回滚，具体规则见第 9 节。返回页允许暂时显示 pending，不能因为 URL 中有 success/canceled 等参数就推断实收或退款。

### 个人 AI 令牌

| 方法与路径 | 输入 / 条件 | 返回 |
| --- | --- | --- |
| GET `/api/tokens` | 本人 Cookie 或个人 Bearer | {items:[{id,name,created_at,expires_at,last_used_at}]} |
| POST `/api/tokens` | {name}，Cookie、CSRF、当前 AI 资格 | {id,token,expires_at}；明文仅展示一次 |
| DELETE `/api/tokens/{id}` | 本人 Cookie、CSRF，或个人 Bearer | 204 |

名称非空≤80 字节；30 天有效，每个用户最多 10 个未过期且未撤销令牌。列表不返回密钥；过期但未撤销的记录仍可出现。Bearer 不能自行签发凭证。

## 4. 会话、委托与外部进展

### 聊天 HTTP

用户/管理员各自使用 Cookie 或 Bearer；每次业务操作依据当前数据库状态授权。`GET /api/chat/capabilities` 只返回公开协议与欢迎语配置，可匿名读取。完整接入、流式帧、恢复和 CLI 示例见 [消息、流式对话与 Agent 接入](chat-agents.md)。

| 方法与路径 | 输入 | 返回 |
| --- | --- | --- |
| GET `/api/chat/capabilities` | 无；公开 | 格式、传输地址、限制、欢迎语与 OpenUI 组件规范地址 |
| GET `/api/chat/conversations` | 无 | {items:Conversation[],poll_after_ms:5000,event_cursor}；最近更新的最多 limits.recent_list_limit 个可访问会话 |
| GET `/api/chat/conversations/{id}` | 无 | 单个 Conversation，含 participants、writable、unread_count；可直接读取最近列表上限以外的已知会话，按当前权限懒创建，越权返回 404 |
| POST `/api/chat/platform` | 用户 {}；管理员 {user_id} | Conversation，按需创建 |
| GET `/api/chat/conversations/{id}/messages` | query after?、before?、limit? | {items,next_cursor,first_cursor,has_more,has_older,poll_after_ms:5000,event_cursor} |
| POST `/api/chat/conversations/{id}/messages` | {body,client_id,format?} | Message；format 默认 markdown，可选 plain、markdown、openui |
| POST `/api/chat/conversations/{id}/read` | {message_id} | {message_id,unread_count} |
| POST `/api/chat/conversations/{id}/streams` | {body,client_id,format?}；body 为首段非空文本 | streaming 状态的 Message，sequence=0，附 event_cursor |
| POST `/api/chat/conversations/{id}/streams/{message_id}/append` | {sequence,delta} | {id,conversation_id,sequence,status,event_cursor}；序号从 1 起，增量最多 4000 字符 |
| POST `/api/chat/conversations/{id}/streams/{message_id}/finish` | {sequence}；下一个序号 | 小回执，status=complete |
| POST `/api/chat/conversations/{id}/streams/{message_id}/cancel` | {sequence}；下一个序号 | 小回执，status=cancelled，部分内容保留 |
| POST `/api/chat/conversations/{id}/stream` | application/x-ndjson；start、delta、finish/cancel 帧 | 边接收边返回 NDJSON ack/error；响应头提交后的失败需读取 error 记录 |
| GET `/api/chat/events` | query conversation_id?、after?（默认 0）、limit?（默认 100） | {items,next_cursor,has_more,resync_required}；独立的事件游标 |
| GET `/api/chat/events/stream` | query conversation_id?、after?、limit?；Last-Event-ID? | SSE；Last-Event-ID 优先于 URL 的 after；都省略时从当前事件高水位开始 |
| GET `/api/chat/updates` | query after?（默认 0）、limit?（默认 100） | 兼容旧客户端的 {items:Message[],next_cursor,has_more,poll_after_ms:5000} |

平台会话 ID 为 `platform_<user_id>`，匹配会话为 `match_<match_id>`；可构造 ID 不等于有权限。新匹配会话不复制双方历史私聊。

body 为 1–10000 字符，client_id 为 1–80 字符。Message 保存原始 body、format、status（streaming / complete / cancelled）和 sequence；body_html 是安全的展示回退，OpenUI 客户端按能力发现的组件规范渲染源码。发送身份由服务端确定。相同会话、同一发送身份、同一 client_id 重试相同正文及格式返回原消息；正文或格式不一致冲突 409。AI 请求记录 via_ai=true。管理员及其 AI 共享平台发送身份和已读标记，审计仍区分具体令牌。

消息分页规则：

- 不传 after/before：最近一页，默认最多 100 条，返回顺序仍为旧到新。
- after=0：从最早消息向后；after>0：读取保存游标之后的新消息。
- before>0：读取更早历史，不能与 after 同传。
- limit 为 1–100。消息 ID 为全局递增游标，权限过滤后可能存在间隙。
- updates 仅返回当前可访问消息；读取不自动标记已读。保存 next_cursor，has_more 时继续取页，否则等待 poll_after_ms（5 秒）。
- 已读 message_id 须属于该会话或为 0，只能前进。归档会话 writable=false，管理员能读历史但不能发送。

实时客户端先读取消息快照，再从该快照的 `event_cursor` 订阅。这个值与快照在同一事务内获取，与 `message_id` / 消息分页的 `next_cursor` 不可混用。SSE 的 `message.delta` 只携带增量，开始/完成/取消携带消息快照；`cursor` 推进事件高水位。最近 10000 条事件以外的旧游标返回 `resync_required` / `resync`，客户端应重新读取快照。流式操作及长连接重新核验当前凭据和会话权限；断连不会自动完成消息。

### 委托与进展 HTTP

| 方法与路径 | 输入 | 返回 / 权限 |
| --- | --- | --- |
| GET `/api/mandate` | 无 | {mandate:Mandate或null}；求职者本人 |
| PUT `/api/mandate` | 完整 MandateInput | {mandate:Mandate} |
| GET `/api/outreach` | 无 | {items:Outreach[]}；本人最近 limits.recent_list_limit 条 |

MandateInput 为完整替换：

```json
{
  "target_industry": "软件与开发工具",
  "target_roles": "Rust 后端工程师",
  "salary_min_cents": 1500000,
  "salary_max_cents": null,
  "salary_currency": "CNY",
  "location": "远程",
  "preferences": "希望参与长期维护的产品",
  "status": "active",
  "allow_external": false
}
```

target_industry≤500 字符，target_roles≤1000，location≤500，preferences≤5000；行业或角色至少一项非空。salary_min_cents、salary_max_cents 可省略或 null，否则为 0–100000000 个 `salary_currency` 最小单位，min≤max。`salary_currency` 可省略：已有委托保留原币种，新委托采用当前默认币种；显式选择须在当前允许集合内。更换币种不做金额换算。其余六字段必填，status 为 active/paused/closed。

active 委托允许内部匹配。allow_external=true 单独记录外部寻找/联系授权、时间及是否经用户 AI 提交。管理员不能替求职者新建委托、扩大外部授权，或将暂停/关闭委托重新激活。暂停阻止新匹配；撤回外部授权阻止新外部进展，已有记录仍可查看。

本应用只记录授权、聊天、匹配与进展，没有内置 LLM、爬虫、第三方求职平台集成或自动外发任务。source_url 只是引用，不会触发服务器访问。

## 5. 管理员 HTTP API

业务接口接受真实管理员 Cookie（写需 CSRF）或独立管理员 Bearer；首次引导会话无业务权限。

### 总控业务

| 方法与路径 | 输入 | 返回 |
| --- | --- | --- |
| GET `/api/admin/overview` | query from?、to?、mode?=live/test、currency? | {stats,series,revenue_by_kind,recent_orders,growth,currency,currency_decimals,currency_totals,bucket,timezone,development,payment_mode} |
| GET `/api/admin/users` | query q?、role?、page? | {items:User[],total} |
| GET `/api/admin/users/{id}` | 无 | User，含登录邮箱与完整业务资料 |
| PATCH `/api/admin/users/{id}` | {name?,public_email?,resume_md?,banned?,ban_reason?,visible?} | User；visible 固定 false，ban_reason 只与 banned 一起提交 |
| GET `/api/admin/users/{id}/access` | 无 | 当前封禁状态、理由和操作者/时间 |
| POST `/api/admin/users/{id}/access` | {banned,reason}；reason 非空≤500 字节 | 访问状态；记录人工理由与审计，封禁撤销会话、个人令牌及未完成邮件挑战 |
| DELETE `/api/admin/users/{id}/resume` | 无 | 204，删除简历并审计 |
| GET `/api/admin/jobs` | query q?、status?、page? | {items:Job[],total} |
| GET `/api/admin/jobs/{id}` | 无 | Job，含草稿/关闭/已删除 |
| PATCH `/api/admin/jobs/{id}` | {jd_md?,status?:"published"或"closed",headcount?} | Job；仅 published/closed；人数单改独立免费；不可恢复 closed/deleted 招聘或发布未付款草稿 |
| DELETE `/api/admin/jobs/{id}` | {expected_version} | Job 墓碑；直接永久删除标题/JD/关联内容快照，保留账务 |
| POST `/api/admin/jobs/{id}/restore` | {expected_version} | Job；与本人恢复共用次数、封禁、状态和支付规则 |
| GET `/api/admin/orders` | query from?、to?、status?、page? | {items:Order[],total} |
| GET `/api/admin/audit` | query page? | {items,total,page,page_size} |
| GET `/api/admin/matches` | query page? | {items,total,page,page_size}，全部状态 |
| POST `/api/admin/matches` | {seeker_id,job_id,note?} | {id,conversation_id,seeker_conversation_id,employer_conversation_id} |
| PATCH `/api/admin/matches/{id}` | {status,note?} | 204 |

用户/岗位/订单/审计/匹配列表使用 `[limits].list_page_size` 和 `list_max_page`（默认每页 50、最多 2000 页），返回 `page`、`page_size`，page 默认 1。用户 q 搜索姓名/邮箱/ID；岗位 q 搜索标题/JD/ID/所有者 ID。用户管理不能修改登录邮箱、角色、Google 身份和付款状态；封禁撤销全部用户会话及个人 AI 令牌。

创建匹配要求 active 求职委托、发布订单已完成且 published、headcount>0 的岗位、已开通招聘方、双方未封禁。note 默认空且≤2000 字节，**双方可见**。从 declined 恢复时重查同一条件；既有 introduced/interview/hired 可在人数为 0 时继续推进阶段。匹配或记录入职不扣人数，也不按人数限制候选总数。

管理员只改人数不会更新 JD version 或 edits_used，另记 job_headcount_update 审计；实际修改 JD 或状态仍需先处理待付款订单，并增加 version。关闭时人数置 0；closed 不能改回 published，deleted 不能由 PATCH 恢复。首次删除只能通过带版本的 restore 接口恢复，永久删除无法恢复。

统计原实收、成功退款、净收入、已付数量、series、revenue_by_kind 按所选 UTC 日期、支付环境和币种过滤；currency 省略时采用站点默认币种，绝不把不同币种的最小单位相加。`currency_totals` 同时返回当前日期/环境下逐币种的独立合计；其他用户/岗位/待支付/待复核计数为全局快照，recent_orders 是全局最近 8 笔。active_jobs 只数 published 且 headcount>0、所属用户未封禁的岗位。mode 默认 development 为 test，否则 live；跨度超过约一年按月，否则按日。

`stats.gross_cents` 是原付款流水总额，`stats.refund_cents` 是成功退款总额，`stats.revenue_cents = gross_cents - refund_cents`。`series` 项为 `{date,revenue_cents,orders,gross_cents,refund_cents}`；`revenue_by_kind` 项为 `{kind,amount_cents,count,gross_cents,refund_cents}`，其中 amount_cents 为净额。付款按 ledger.created_at，退款按首次成功记录的本地 succeeded_at 归属时间；跨日退款可使某日净额为负，失败退款不计入。paid_orders、series.orders、revenue_by_kind.count 统计原付款笔数，不因退款变为负数。

没有管理员伪造付款、任意修改订单状态、发起退款或修改系统配置接口。退款状态只由验证后的网关通知更新。远程 MCP 支持 admin_get_order，HTTP 未另提供 GET /api/admin/orders/{id}。

### 岗位中心匹配工作台

以下只读接口仅接受管理员 Cookie、独立管理员 Bearer 或管理员 WebMCP 会话；匿名、普通用户及首次引导会话没有权限。
每个业务函数在持有数据库锁时再次核验管理员身份，撤销后的 AI 令牌或 WebMCP 会话不能继续查询。
普通管理列表继续提供页码分页，审计和匹配列表新增相同的分页元数据；工作台使用以下游标接口：

| 方法与路径 | query | 返回 |
| --- | --- | --- |
| GET `/api/admin/workbench/jobs` | q?,employer_id?,cursor?,limit? | `{items:WorkbenchJob[],total,limit,next_cursor}`；仅当前可招聘岗位 |
| GET `/api/admin/workbench/jobs/{id}` | 无 | WorkbenchJob；岗位不存在或已不可新匹配返回 404 |
| GET `/api/admin/workbench/candidates` | q?,job_id?,industry?,roles?,location?,salary_currency?,salary_min_cents?,salary_max_cents?,cursor?,limit? | `{items:CandidateSummary[],total,limit,next_cursor}` |
| GET `/api/admin/workbench/candidates/{id}` | job_id? | `{user,mandate,resume_html,match}`；完整个人详情，mandate/match 可为 null |
| GET `/api/admin/workbench/matches` | q?,status?,job_id?,seeker_id?,employer_id?,cursor?,limit? | `{items:Match[],total,limit,next_cursor,stage_counts}` |
| GET `/api/admin/workbench/matches/{id}` | 无 | 单个管理员 Match，含三个会话 ID；不存在返回 404 |

批量介绍使用 `POST /api/admin/workbench/introductions`，body 为 `{job_id,seeker_ids,note?}`。seeker_ids 为 1–100 个不重复 ID；响应 `{created,reused,matches}`，matches 每项含 created 与三类会话 ID。整批在同一事务中验证当前资格；既有有效匹配复用且不覆盖备注，declined 记录返回冲突，任何无效成员使整批不写入。

`limit` 省略时使用 `[limits].workbench_page_size`（模板 25），显式范围 1–100；`total` 是当前完整筛选结果数量，不随页游标缩小。
`next_cursor` 为下一页不透明游标，到末页为 null。岗位按 `created_at DESC,id DESC`，
候选按委托 `created_at DESC,user_id DESC`，匹配跟进按 `updated_at DESC,id DESC` 排序。
游标与全部筛选条件绑定，改变筛选后应从第一页重查；可以保留游标历史提供“上一页”，不使用全量预加载或 OFFSET 深分页。
这些查询读取当前数据，并不固定一个跨页数据库快照；新增、更新或撤权后应刷新列表，尤其更新匹配阶段后从首屏重新读取。

q 最多 120 个字符；industry/roles/location、各实体 ID 最多 80 个字符；不允许控制字符。
每段搜索文本最多八个空白分隔词，多个词同时匹配。关键词按字面内容搜索，不接受 FTS/SQL 语法。
至少三个字符的词使用 FTS5 trigram 索引；一到两个字符使用已转义的 LIKE，并与相同业务筛选组合。
cursor 最多 2048 字节，非法或筛选不一致的游标返回 400。未知 query/工具字段被拒绝。

岗位 q 搜索标题、JD 和招聘方名称，只返回 `published`、人数大于 0、有效发布付款及已开通未封禁招聘方的岗位。
候选 q 搜索姓名、简历和委托目标/地点/偏好，基础范围为未封禁求职者的 active 委托；完整简历不随列表返回。
候选 job_id 若提供，必须仍可新匹配：不存在返回 404、已有岗位失效返回 409。
同岗位的 introduced/interview/hired 候选被排除；declined 保留为 `history_match`，必须明确执行原匹配 PATCH 才能重新介绍。
薪资过滤是与候选期望区间重叠，null 边界不限制；最小/最大筛选为 0–100000000 个币种最小单位，最小值不得大于最大值。显式 `salary_currency` 决定比较币种；有金额筛选而未指定币种时，采用所选岗位币种，没有岗位时采用站点默认币种。仅传 job_id 不限制币种；只传 salary_currency 可按币种筛选。所有金额比较都附加严格相等的币种条件，不自动换汇。
若筛选可接受岗位固定底薪的人，将两个筛选值都设为该 `salary_cents`，并同时使用该岗位的 `currency`。

匹配 q 搜索双方姓名、岗位标题和介绍备注，可按四个阶段及精确岗位/用户筛选。
`stage_counts` 是 `{introduced,interview,hired,declined}`；应用文本和实体筛选，但忽略当前 status 筛选，便于显示各阶段数量。
管理员可以读取失效或归档匹配历史；单人详情同样允许查看暂停/封禁求职者的历史，不会把他们放回可选候选列表。
单人详情的 resume_html 使用与资料页相同的 Markdown 清洗器；match 只对应所传 job_id，不查询或返回其他岗位的全部匹配。

所有搜索与详情均不建立匹配、不修改委托/外联同意、不授权双方资料访问，也不发送消息。
新建/重开匹配继续使用原事务和当前资格校验；人数为 0、委托暂停、付款失效或封禁都会阻止新介绍。
完整行字段见下方模型表；索引及查询实现见 [workbench.rs](../src/workbench.rs)。

### 管理员委托和外部进展

| 方法与路径 | 输入 | 返回 |
| --- | --- | --- |
| GET `/api/admin/mandates` | query q?≤200 字符、status?、page? | {items:Mandate[],total,page}，50/页 |
| GET `/api/admin/mandates/{user_id}` | 无 | {mandate:Mandate或null} |
| PATCH `/api/admin/mandates/{user_id}` | **完整** MandateInput | {mandate}；不能新建/扩大授权/代为重新激活 |
| GET `/api/admin/outreach/{user_id}` | 无 | {items:Outreach[]}，最近 limits.recent_list_limit 条 |
| POST `/api/admin/outreach/{user_id}` | OutreachInput | Outreach |
| PATCH `/api/admin/outreach/records/{id}` | OutreachInput | Outreach；从原记录确定归属 |

OutreachInput 必填 channel、opportunity、stage，可选 source_url、note（默认空）。channel 非空≤120 字符，opportunity 非空≤500，note≤5000，source_url≤2048，仅允许无用户凭证的 HTTP/HTTPS URL。stage 为 identified/contacted/replied/interview/offer/closed。

新建/修改进展要求未封禁求职者的 active 委托、allow_external=true 及已记录同意。授权暂停/撤回后，只能将已有记录改为 closed。管理员可读封禁用户历史，求职者可读本人进展；记录应对应实际工作，不触发外部动作。

### 独立管理员 AI 令牌

| 方法与路径 | 输入 / 条件 | 返回 |
| --- | --- | --- |
| GET `/api/admin/tokens` | 真实非 bootstrap 管理员 Cookie | {items:[{id,name,created_at,expires_at,last_used_at}]} |
| POST `/api/admin/tokens` | {name}，Cookie、CSRF、最近 600 秒 Passkey 认证 | {id,token,expires_at} |
| DELETE `/api/admin/tokens/{id}` | 管理员 Cookie、CSRF | 204，撤销无需最近认证 |

带 Authorization 头的令牌管理请求被拒绝。密钥前缀 hf_admin_，仅创建时展示，数据库只存哈希。名称非空≤80 字节；30 天有效，全平台最多 10 个有效令牌。撤销/过期在请求和业务函数内再次核验；审计 actor 为 admin_ai:<令牌ID>。令牌不提供 Passkey 或系统密钥权限。

## 6. 返回模型

| 模型 | 字段 |
| --- | --- |
| User | id,email,name,role（unassigned/seeker/employer）,employer_paid,avatar,public_email,resume_md,visible,banned,created_at,updated_at。仅本人/管理员；不序列化 google_sub |
| CounterpartyProfile | id,name,avatar,public_email,resume_md,created_at；不含登录邮箱 |
| Job | id,owner_id,title,jd_md,salary_cents,currency,currency_decimals,ai_min_salary_cents,headcount（0–1000000）,status（draft/published/closed/deleted）,delete_count（0/1/2）,edits_used,version,created_at,updated_at；保存的删除前状态/人数不直接序列化 |
| JobDetail | job,jd_html,terms（edit_mode/edit_fixed_minor/edit_bps/max_edits/currency/currency_decimals/employer_ai_min_salary_minor）,employer（id/name/public_email/avatar） |
| WorkbenchJob | id,owner_id,employer_id（同 owner_id）,employer_name,title,salary_cents,currency,currency_decimals,headcount,status,jd_excerpt（最多240字符）,created_at,updated_at,active_matches；active_matches 为 introduced/interview/hired 数量，不扣招聘人数 |
| CandidateSummary | id,name,avatar,target_industry,target_roles,location,salary_min_cents?,salary_max_cents?,salary_currency,currency_decimals,resume_excerpt（最多240字符）,preferences_excerpt（最多240字符）,created_at（委托创建时间）,updated_at（用户资料更新时间）,mandate_updated_at,history_match?（id/status/conversation_id/updated_at）；没有登录邮箱或完整简历 |
| Order | id,user_id,kind（onboarding/publish/edit）,job_id?,amount_cents,refunded_cents,currency,currency_decimals,provider,live_mode,status,snapshot_json,provider_session?,checkout_url?,created_at,paid_at?；snapshot_json 为 JSON 字符串；状态含 pending/cancelled/paid/paid_needs_review/partially_refunded/refunded；个人读取隐藏已不可履行或网关币种不可用订单的 checkout_url |
| Match | id,seeker_id,seeker_name,job_id,job_title,employer_id,employer_name,status,note,created_at,updated_at,conversation_id；管理员额外含 seeker_conversation_id,employer_conversation_id |
| Conversation | id,kind（platform/matched）,title,user_id?,match_id?,job_id?,job_title?,participants（id/name/role/avatar）,last_message_id,last_message_preview,last_message_at?,unread_count,writable,created_at,updated_at |
| Message | id（整数）,conversation_id,sender_kind（user/platform）,sender_id?,sender_name,body,body_html,client_id,via_ai,created_at |
| Mandate | MandateInput 字段（含 salary_currency），加 currency_decimals,user_id,user_name,banned,consented_at?,consented_via_ai,via_ai,created_at,updated_at |
| Outreach | id,user_id,banned,channel,opportunity,source_url,stage,note,via_ai,created_at,updated_at |

返回模型“?”表示可为 null，请求参数“?”表示可省略。avatar 为受鉴权的相对 URL 或空字符串，不返回原始数据库图片。

## 7. 远程 MCP

### 传输与请求

端点 `POST /mcp`，协议 `2025-11-25`，无状态 Streamable HTTP，使用 JSON 响应。接受现有个人或管理员 Bearer 令牌，以及普通用户批准后的 HumanGrant 签名请求。Cookie 不能代替远程 MCP 凭据。MCP 连接不提供第三方客户端的 OAuth 自动接入、MCP 会话 ID 或 MCP GET/SSE 通道；提供只读 `resources/list` / `resources/read` 开发者文档资源。聊天的原生 SSE 位于 `/api/chat/events/stream`，增量消息也可通过 MCP 工具读写，见 [Agent 聊天协议](chat-agents.md)。

```http
Authorization: Bearer <personal-token-or-admin-token>
Content-Type: application/json
Accept: application/json, text/event-stream
MCP-Protocol-Version: 2025-11-25
```

HumanGrant 使用 `Authorization: HumanGrant <access_token>`，并按 [签名规范](humangrant.md) 为每次请求提供密钥标识、时间、随机数、正文摘要与签名。仅持有访问令牌不足以调用；服务端核对它是否绑定原申请密钥。普通用户和管理员 Bearer 继续使用上面的请求头。

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"my-hr-agent","version":"1.0"}}}
```

支持 initialize、ping、tools/list、tools/call、resources/list、resources/read。无 id 通知返回 202。显式不支持的 MCP-Protocol-Version 头返回 400。普通用户和管理员分别获得其业务工具、公开配置与下列会话工具；当前工具集合以 `tools/list` 为准。发布的 inputSchema 均为 object、additionalProperties=false；required 以 tools/list 为准。工作台 limit 默认值和招聘人数最大值按当前配置发布。HumanGrant 的 `tools/list` 只返回当前授权允许的工具，`tools/call` 在执行时继续检查具体资源和当前权限。HumanGrant 不开放管理员工具、`choose_role`、`create_checkout` 或 `update_mandate`。

```json
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
```

```json
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_updates","arguments":{"after":0,"limit":100}}}
```

成功为 result.content（JSON 文本）、result.structuredContent（对象）、isError=false。业务失败仍为 HTTP 200，但 result.isError=true 并返回错误文本；协议错误在 error 中，鉴权/资格失败为 HTTP 401/403。

### 共用公开配置工具

`get_site_config` 的 arguments 为 `{}`，结果与 `GET /api/config` 相同。可读取币种精度、各网关可用币种、收费模式和业务限额，不返回凭证，也不能修改配置。

### 普通用户专属 25 个工具

未标“?”者必填。字段范围和角色限制与 HTTP 相同；能发现工具不代表拥有跨角色权限。

| 工具 | arguments | 结果 / 行为 |
| --- | --- | --- |
| get_profile | {} | 本人 User |
| update_profile | name,public_email,resume_md,visible? | User，visible 不公开 |
| set_avatar_svg | svg | User；安全 SVG，可使用本地 SMIL 动画；脚本、事件处理器、CSS、嵌入 HTML 和外部引用会被拒绝 |
| set_image_avatar | image_base64 | User；标准 Base64，无 data URL，解码≤2 MiB、编码≤2796204 字符 |
| choose_role | role,provider? | {user,order}；新账号须先完成人工开户以获得 AI 资格 |
| list_jobs | q?,page? | {items,total}；介绍的具体岗位 |
| list_my_jobs | q?,page? | {items,total}；本人岗位 |
| get_job | id | JobDetail |
| create_job | title,jd_md,salary_cents,currency?,provider?,headcount? | {job,order}；付费模式待支付，免费模式已生效，人数默认 1，不乘入费用 |
| edit_job | id,jd_md,provider? | {job,order}；遵循原条款，不接受 title/currency/salary_cents |
| set_job_headcount | id,headcount | Job；整数 0–1000000，免费不限次数，人数 0 保留其他条件满足时的 AI 资格 |
| list_deleted_jobs | {} | {items}；本人最近 limits.recent_list_limit 个可恢复岗位 |
| delete_job | id,expected_version | Job；首次可恢复，恢复后再删永久；destructiveHint=true |
| restore_job | id,expected_version | Job；仅首次删除可恢复一次；不新建订单 |
| permanently_delete_job | id,expected_version | Job 墓碑；不可恢复；destructiveHint=true |
| close_job | id | Job，可能立即失去招聘方 AI 资格 |
| list_people | q?,page? | {items,total}；明确匹配的求职者 |
| get_person | id | {user,resume_html} |
| list_orders | {} | {items} |
| get_order | id | 本人 Order |
| create_checkout | id | Order，含支付链接；不自动付款 |
| list_matches | {} | {items} |
| get_mandate | {} | {mandate} |
| update_mandate | 完整 MandateInput | {mandate}，6 必填字段、2 可空薪资和可选 salary_currency |
| list_outreach | {} | {items} |

### 管理员专属 25 个工具

Search schema 包含 q?,page?,role?（seeker/employer/unassigned）,status?,from?,to?,mode?（live/test）,currency?；各业务实际生效字段见 HTTP 表。例如订单只按日期/状态/页过滤，overview 按 mode/currency 统计。

| 工具 | arguments | 结果 / 行为 |
| --- | --- | --- |
| admin_overview | Search | 总控统计 |
| admin_list_users | Search | {items,total} |
| admin_get_user | id | User |
| admin_update_user | id,name?,public_email?,resume_md?,banned?,ban_reason? | User；ban_reason 与 banned 一起提交 |
| admin_delete_resume | id | {deleted:true} |
| admin_list_jobs | Search | {items,total} |
| admin_get_job | id | Job |
| admin_update_job | id,jd_md?,status?（published/closed）,headcount? | Job；纯人数免费，不可发布未付草稿、改标题/底薪或复活关闭/删除岗位 |
| admin_delete_job | id,expected_version | Job 墓碑，直接永久删除并保留账务；destructiveHint=true |
| admin_restore_job | id,expected_version | Job；遵守本人相同一次恢复/支付规则 |
| admin_list_orders | Search | {items,total} |
| admin_get_order | id | Order，只读 |
| admin_list_audit | page? | {items,total,page,page_size} |
| admin_list_matches | page? | {items,total,page,page_size}，含三个会话 ID |
| admin_search_jobs | q?,employer_id?,cursor?,limit? | {items,total,limit,next_cursor}；当前可招聘岗位摘要 |
| admin_search_candidates | q?,job_id?,industry?,roles?,location?,salary_currency?,salary_min_cents?,salary_max_cents?,cursor?,limit? | {items,total,limit,next_cursor}；有效委托候选摘要，不建立匹配 |
| admin_search_matches | q?,status?,job_id?,seeker_id?,employer_id?,cursor?,limit? | {items,total,limit,next_cursor,stage_counts}；只读匹配跟进 |
| admin_introduce_candidates | job_id,seeker_ids,note? | {created,reused,matches}；1–100 个不重复候选 ID，整批原子校验，既有有效匹配复用，declined 冲突 |
| admin_create_match | seeker_id,job_id,note? | {id,conversation_id,seeker_conversation_id,employer_conversation_id} |
| admin_update_match | id,status,note? | {updated:true}，note 双方可见 |
| admin_list_mandates | q?,status?（active/paused/closed）,page? | {items,total,page} |
| admin_get_mandate | user_id | {mandate} |
| admin_update_mandate | user_id + 完整 MandateInput | {mandate}，不能扩大同意 |
| admin_list_outreach | user_id | {items} |
| admin_save_outreach | user_id,channel,opportunity,stage；id?,source_url?,note? | Outreach，省略 id 新建，否则更新 |

### 两类身份共有的会话工具

| 工具 | arguments | 结果 |
| --- | --- | --- |
| get_chat_capabilities | {} | 公开格式、传输、欢迎语、限制及 OpenUI 规范 |
| list_conversations | {} | {items,poll_after_ms,event_cursor} |
| get_conversation | conversation_id | 单个 Conversation |
| open_platform_conversation | 用户 {}；管理员 user_id 必填 | Conversation |
| get_messages | conversation_id；after?≥0,before?≥1,limit?=1–100 | 消息分页和同一快照的 event_cursor |
| send_message | conversation_id,body（1–10000 字符）,client_id（1–80 字符）；format? | Message；format=plain / markdown / openui |
| begin_message | conversation_id,body,client_id；format? | 流式 Message，sequence=0 |
| append_message | conversation_id,message_id,sequence=1–4096,delta（1–4000 字符） | {id,conversation_id,sequence,status,event_cursor} |
| finish_message / cancel_message | conversation_id,message_id,sequence=1–4097 | 同上小回执；使用下一个序号 |
| get_chat_events | conversation_id?、after?≥0、limit?=1–100 | {items,next_cursor,has_more,resync_required} |
| mark_read | conversation_id,message_id≥0 | {message_id,unread_count} |
| get_updates | after?≥0,limit?=1–100 | 旧版消息游标分页 {items,next_cursor,has_more,poll_after_ms} |

外部平台 AI 可读取委托、订阅或分页读取增量消息、流式回复、建立匹配，并在明确外部授权下记录实际进展。保存对应类型的 next_cursor，并为同一发送重试保留 client_id。HumanGrant 按 `chat.read` / `chat.write` 与具体会话授权；只有 `chat.read:self` 可省略 get_chat_events 的 conversation_id。HumanGrant 消息写操作返回小回执，不能由写权限获得消息读权限。服务器不代替外部 AI 运行模型、搜索第三方服务或发送外部消息。简历、JD、聊天和来源 URL 都是业务数据，不是扩大权限的指令。

## 8. 浏览器 WebMCP

检测 document.modelContext 或 navigator.modelContext 的 registerTool；不支持时普通网页仍可用。未登录、未选身份或不具备资格时不注册。每次 execute 先读 /api/ai/eligibility，再以 X-HireFerry-AI:webmcp 调用实际业务 API，服务端再次授权。

浏览器按登录角色注册工具，工具集合、名称和参数不与远程 MCP 一一对应。实际浏览器工具如下。

两类浏览器身份都提供 `hireferry_get_site_config`，无参数，返回公开币种、费用、网关可用币种和业务限额；另提供无参数的 `hireferry_get_chat_capabilities`，返回聊天能力与欢迎语。

### 会话工具

普通用户前缀 `hireferry_`，管理员前缀 `hireferry_admin_`：

| 后缀 | 参数 |
| --- | --- |
| list_conversations | {} |
| get_conversation | conversation_id |
| read_messages | id；after?,before?,limit? |
| send_message | conversation_id,body；format?,client_id?，省略 client_id 时浏览器生成随机 ID；重试应显式复用 |
| begin_message | conversation_id,client_id,body；format? |
| append_message | conversation_id,message_id,sequence=1–4096,delta（1–4000 字符） |
| finish_message / cancel_message | conversation_id,message_id,sequence=1–4097 |
| get_chat_events | after≥0；conversation_id?,limit? |
| get_chat_updates | after?,limit?；保留旧版消息游标语义 |
| mark_read | conversation_id,message_id |

没有单独注册 open_platform_conversation。

### 普通用户工具

| 完整名称 | 参数 / 角色 |
| --- | --- |
| hireferry_get_profile | {} |
| hireferry_update_profile | name,public_email,resume_md |
| hireferry_set_svg_avatar | svg |
| hireferry_set_image_avatar | base64,mime_type（image/png,image/jpeg,image/webp），转 multipart 上传 |
| hireferry_list_introductions | {} |
| hireferry_get_introduced_job | id |
| hireferry_get_introduced_person | id |
| hireferry_list_orders | {} |
| hireferry_checkout_order | id |
| hireferry_get_mandate | {}，求职者 |
| hireferry_update_mandate | MandateInput 原 8 字段，加可选 salary_currency；求职者；薪资为 null 或≥0 的整数最小单位 |
| hireferry_get_outreach | {}，求职者 |
| hireferry_list_my_jobs | {}，招聘方 |
| hireferry_create_job | title,jd_md,salary_cents,currency?,provider?,headcount?，招聘方；人数默认 1 |
| hireferry_edit_job | id,jd_md,provider?，招聘方；使用岗位保存的编辑条款 |
| hireferry_set_job_headcount | id,headcount，招聘方；整数 0–1000000，免费 |
| hireferry_list_deleted_jobs | {}，招聘方；本人可恢复回收站 |
| hireferry_delete_job | id,expected_version，招聘方；首次可恢复，恢复后再删永久 |
| hireferry_restore_job | id,expected_version，招聘方；仅一次恢复 |
| hireferry_permanently_delete_job | id,expected_version，招聘方；直接永久 |
| hireferry_close_job | id，招聘方 |

### 管理员工具

以下所有名称前缀为 `hireferry_admin_`：

| 后缀 | 参数 |
| --- | --- |
| list_users | q?,role?,page? |
| get_user | id |
| update_user | id,name?,public_email?,resume_md?,banned? |
| delete_resume | id |
| list_jobs | q?,status?,page? |
| get_job | id |
| update_job | id,jd_md?,status?（published/closed）,headcount? |
| delete_job | id,expected_version；直接永久 |
| restore_job | id,expected_version；仅合资格首次删除 |
| list_introductions | {} |
| search_jobs | q?,cursor?,limit?,employer_id?；当前可招聘队列 |
| search_candidates | **job_id 必填**；q?,cursor?,limit?,industry?,roles?,location?,salary_currency?,salary_min_cents?,salary_max_cents?；与远程 MCP 的可选 job_id 不同 |
| search_matches | q?,cursor?,limit?,status?,job_id?,seeker_id?,employer_id?；按最近更新分页 |
| introduce_candidates | job_id,seeker_ids,note?；1–100 个不重复候选，批量原子校验 |
| create_introduction | seeker_id,job_id,note |
| update_introduction | id,status,note |
| list_mandates | q?,status?,page? |
| get_mandate | user_id |
| update_mandate | user_id；目标/薪资/薪资币种/地点/偏好/status 可选，读取旧委托后合并，保留原 allow_external |
| get_outreach | user_id |
| record_outreach | user_id,channel,opportunity,stage,note；source_url? |
| update_outreach | id,channel,opportunity,stage,note；source_url? |
| list_orders | from?,to?,status?,page? |
| overview | from?,to?,mode?,currency? |
| audit | {} |

没有系统配置、Passkey、签发或撤销令牌、用户 Webhook 端点/密钥管理工具。支付操作只取得订单/链接，由人在网关完成支付。

三个工作台搜索工具标记为只读，使用与 HTTP 相同的筛选与当前授权；limit 范围 1–100。
没有为候选/匹配聚合详情另增浏览器工具，已有 get_user/get_mandate/get_job 继续提供相应单条业务资料。

## 9. 支付返回与网关回调

| 方法与路径 | 验证 | 返回 |
| --- | --- | --- |
| POST `/api/webhooks/stripe` | 原始 body + Stripe-Signature；时戳/HMAC/事件/金额/币种/环境 | checkout.session.completed、checkout.session.async_payment_succeeded；接受后 204 |
| POST `/api/webhooks/waffo` | 原始 body + X-Waffo-Signature；时戳/RSA/storeId/mode/支付关联/金额/币种 | order.completed、refund.succeeded、refund.failed；接受后 204 |

回调不依赖用户 CSRF，必须通过网关签名。重复/并发回执原子去重。订单创建时固定支付环境，错误环境拒绝。未配置真实商户时失败关闭，无自动模拟付款。网关字段、验签及商户验收见 [支付说明](payments.md)。

Stripe success_url 与 Waffo successUrl 统一生成 `/billing/return?order_id=<本地订单ID>&provider=<stripe或waffo>`。Stripe cancel_url 额外加 canceled=1。各参数仅标识查询对象或提示，页面通过登录用户的订单 API 读取真实状态。未登录时先登录再回到该固定站内路径；不存在通过返回 URL 结算订单的接口。

Waffo 退款先验证原订单、原 paymentId、金额、币种和环境；若附带原付款金额或商户 metadata，也必须一致。refund_id 按网关及 test/live 环境唯一，同一结果重复不再落账，累计成功退款不能超过原实收。退款比原付款回调先到时返回 409，可在付款记录后重试；未知本地订单返回 404，错误商户/金额/关联拒绝。

| 情况 | 订单与权益 |
| --- | --- |
| 部分成功退款 | 更新 refunded_cents；已履行订单为 partially_refunded，保留原权益；原 paid_needs_review 不被升级授权 |
| 全额开通退款 | refunded；不存在另一笔已履行且未全退的开通订单时撤回 employer_paid |
| 全额发布退款 | refunded；岗位人数置 0，非 deleted 岗位关闭，已 deleted 仍隐藏；停止新介绍和编辑，后续恢复内容也不重新开放招聘；未删除的原匹配依 closed 规则保留 |
| 全额 JD 修改退款 | refunded；保留已应用 JD、版本和修改次数，不回滚较新内容 |
| refund.failed | 记录尝试，不扣净收入，不改变权益；晚到失败不能撤销已确认成功退款 |

本站不调用商户退款接口、不提供退款按钮；Stripe 退款通知与争议处理未实现。`payment_refunds` 保存退款实体，`refund_events` 保存各结果事件，原 ledger 保持不变。

## 10. 用户主动配置的出站 Webhook

这些接口管理用户自己的通知接收端点，与上述支付商户回调不同。只接受已选 seeker/employer 身份且未封禁的本人 Cookie；所有带 Authorization 或 X-HireFerry-AI 的请求、管理员和 bootstrap 会话均拒绝，写操作需要 Origin 和 CSRF。不注册远程 MCP 或 WebMCP 凭证管理工具，也不要求招聘方具有 AI 资格。

| 方法与路径 | 输入 | 返回 |
| --- | --- | --- |
| GET `/api/webhooks` | 无 | {items:WebhookEndpoint[],enabled,limits:{max_endpoints,max_attempts,max_pending_per_endpoint}} |
| POST `/api/webhooks` | {name,url,events} | {endpoint,signing_secret}；密钥只在创建响应展示 |
| PATCH `/api/webhooks/{id}` | {name?,enabled?,events?} | {endpoint}；URL 不可修改，换地址须撤销后新建 |
| DELETE `/api/webhooks/{id}` | 无业务参数 | {revoked:true}，撤销端点并取消不可继续投递的队列 |
| GET `/api/webhooks/{id}/deliveries` | limit?，默认 50、范围 1–100 | {items:WebhookDelivery[]} |
| POST `/api/webhooks/{id}/test` | {} | {delivery}，排队一条测试通知，不代表远端已收到 |
| POST `/api/webhooks/{id}/deliveries/{delivery_id}/retry` | {} | {delivery}，仅失败投递可重试，重新检查权限 |

WebhookEndpoint 为 `{id,name,url,events,enabled,dropped_events,created_at,updated_at}`，不含签名密钥。WebhookDelivery 为 `{id,event_id,event_type,status,attempts,max_attempts,next_attempt_at,last_attempt_at?,last_status_code?,last_error?,created_at,delivered_at?}`。

name 为 1–100 字符，events 是非空受支持事件列表：message.created、match.updated、job.updated、order.updated、mandate.updated、outreach.updated。接收地址只允许公网 HTTPS/443、无账号密码和 fragment。通知仅带资源标识及状态；创建端点不自动获得更多资料权限。发送时重新检查可见范围、封禁、端点启用与当前授权；授权失效的旧事件不能因重试重新外发。完整签名、接收方去重、DNS/IP 防护、重试/取消/保留规则见 [用户 Webhook](webhooks.md)。

## 11. 生产配置边界

SMTP、Google、域名、默认语言、计费、支付密钥、出站 Webhook 系统限制及 `[chat.welcome.<audience>]` 欢迎语等只读取 TOML。没有网页/MCP 系统设置接口；修改配置后重启。`[site].support_email = "support@lmm.best"` 用于页面联系信息；`[smtp].reply_to = "support@lmm.best"` 写入验证码邮件 Reply-To，认证发件人 from 需另填服务商获准的地址。

本次交付未获得所提及的 production.env 和另行提供的生产交接 README 内容，没有读取或推测真实客户端/商户/商品 ID、邮件账号或密钥。应用不会自动导入环境文件；部署方应在部署机器将已验证值写入受保护 TOML，并进行外部端到端验收。详见 [部署交接](deployment-handoff.md)。
