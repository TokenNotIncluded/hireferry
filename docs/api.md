# HTTP API、MCP 与 WebMCP

本文件描述当前实现。依据：[用户 API](../src/api.rs)、[访问控制](../src/access.rs)、[聊天与委托](../src/chat.rs)、[管理员业务](../src/admin.rs)、[管理员 AI 令牌](../src/admin_ai.rs)、[远程 MCP schema](../src/mcp.rs)、[浏览器注册代码](../static/app.js)。

聘渡由平台 HR 居中服务。账号资料、简历、头像与岗位均不提供公共目录。每位用户与平台的私聊独立保存；平台建立明确匹配后，双方才能读取对应资料，并进入新的独立匹配会话。管理员可以读取全平台业务历史，包含封禁、拒绝匹配与岗位删除后的归档会话；归档会话不可发送新消息。

## 1. 通用约定

- JSON 请求使用 `Content-Type: application/json`。时间戳为 UTC Unix 秒；统计日期为包含首尾日期的 `YYYY-MM-DD`。
- 所有金额使用整数人民币分。`salary_cents:300000` 代表月底薪 ¥3,000，不包含奖金、提成。
- 业务错误通常为 HTTP 400/401/403/404/409/429/503 与 `{"error":"说明"}`。JSON 提取及请求体超限等框架错误可能不同。无权读取的资料、岗位、头像通常返回 404。
- 全局请求体上限 3 MiB；头像、文本另有限制。输入 DTO 拒绝未知字段；无参数操作不要附加业务字段。
- 浏览器 Cookie 为 `hf_session`，HttpOnly、SameSite=Lax，HTTPS 下 Secure；默认 24 小时，可由 TOML 调整。
- Cookie 写请求携带 `Origin`（配置的 public_url 来源）和 `X-CSRF-Token`（来自 GET /api/session）。尚未登录的认证接口验证同源及浏览器挑战。
- Bearer 为 `Authorization: Bearer <token>`。普通用户、管理员两类令牌严格隔离；管理员不能冒充普通用户调用本人资料接口。
- 每次普通用户 Bearer 请求检查封禁、撤销、过期和 AI 资格。求职者免费；招聘方须已付开通费，并拥有至少一个发布费已付、状态为 published、底薪≥300000 分的岗位。
- WebMCP 请求携带 `X-HireFerry-AI: webmcp` 并使用当前 Cookie/CSRF，服务端再次检查 AI 资格。关闭最后一个合资格岗位后，已有招聘方 AI 令牌立即不可用；人工工作台仍可操作。
- `public_email` 是历史字段名，现表示向平台及明确匹配对象提供的联系邮箱。`visible` 兼容旧客户端，写入固定 false，不授予公开访问。登录邮箱 email 只向本人和管理员返回。

| 身份 | 业务范围 |
| --- | --- |
| 未登录 | 首页、条款、隐私说明、公开配置、登录流程、健康检查 |
| 普通用户 Cookie | 本人资料/岗位/订单、平台私聊、明确匹配对象和对应匹配会话；按角色操作 |
| 普通用户 Bearer / WebMCP | 同上，每次重新检查 AI 资格；不能自行签发令牌 |
| 管理员 Cookie | 全平台业务、会话历史、订单/统计/审计，以及 Passkey/管理员令牌管理 |
| 管理员 Bearer / WebMCP | 全平台业务操作；无 Passkey、令牌签发、系统配置或伪造付款操作 |

introduced/interview/hired 匹配授予对应双方访问权。declined、任一方封禁、岗位删除立即撤销普通参与者的匹配资料与会话访问；岗位仅 closed 时已有匹配仍可沟通。暂停委托阻止新匹配，不自动撤销已建立匹配。

## 2. 站点、登录与管理员认证

域名 `https://hireferry.lmm.best` 为部署示例，应替换为实际域名。Google 回调固定路径为 `/auth/google/callback`；条款和隐私页面为 `/terms`、`/privacy`。详细认证机制见 [认证说明](auth.md)。

| 方法与路径 | 输入 | 返回 / 条件 |
| --- | --- | --- |
| GET `/healthz` | 无 | 204，进程存活 |
| GET `/api/config` | 无 | {name,default_language,support_email,source_url,google_enabled,email_enabled,development,providers,pricing}；无服务密钥 |
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

## 3. 普通用户 HTTP API

这些业务接口接受本人 Cookie 或具备 AI 资格的个人 Bearer；个人令牌创建仅限 Cookie。

### 资料、身份与头像

| 方法与路径 | 输入 | 返回 |
| --- | --- | --- |
| GET `/api/ai/eligibility` | 用户或管理员身份 | {eligible,reason,role,min_salary_cents:300000} |
| GET `/api/profile` | 无 | 本人 User |
| PUT `/api/profile` | {name,public_email,resume_md,visible?} | 更新后的 User |
| POST `/api/profile/avatar` | multipart 首个文件，建议字段名 file | User |
| PUT `/api/profile/avatar` | {svg} | User |
| GET `/api/avatars/{id}` | 本人、明确匹配对象或管理员 | 清洗后的 PNG/SVG，private/no-store |
| POST `/api/role` | {role:"seeker"或"employer",provider?:"stripe"或"waffo"} | {user,order}；order 可为 null |
| POST `/api/markdown` | {text}，用户或管理员 | {html}，服务端清洗后的预览 |

身份选择后不能切换。招聘方可先选身份而不指定网关，此时无支付订单；指定已启用网关时创建/复用开通订单。新用户先通过浏览器注册并选择身份，再获取 AI 资格。

姓名≤120 UTF-8 字节，简历/Markdown≤80000 字节，拒绝 NUL。联系邮箱可为空，否则须有效；仅求职者可写非空简历。图片只接受 PNG/JPEG/WebP，原始字节≤2 MiB，宽高 1–2048 像素；解码并重编码为不超过 256×256 的 PNG，去除元数据与附加内容。SVG≤64000 字节，只允许静态白名单；脚本、事件属性、外链/xlink 引用拒绝。最终头像写入事务再次检查封禁。

### 岗位、匹配与订单

| 方法与路径 | 输入 | 返回 / 范围 |
| --- | --- | --- |
| GET `/api/jobs` | query q?、page? | {items:Job[],total}；仅介绍给本人的具体岗位 |
| GET `/api/jobs/mine` | query q?、page? | {items:Job[],total}；本人未删除岗位，含草稿 |
| GET `/api/jobs/{id}` | 无 | {job,jd_html,terms,employer}；本人岗位或精确匹配岗位 |
| POST `/api/jobs` | {title,jd_md,salary_cents,provider} | {job,order}；已付开通费招聘方，先生成待支付订单 |
| POST `/api/jobs/{id}/edit` | {jd_md,provider} | {job,order}；原内容保留到验签收款 |
| POST `/api/jobs/{id}/close` | 无业务参数 | Job；本人 published 岗位，无待支付修改 |
| GET `/api/people` | query q?、page? | {items:CounterpartyProfile[],total}；明确匹配的求职者 |
| GET `/api/people/{id}` | 无 | {user:CounterpartyProfile,resume_html}；本人求职资料或匹配对象 |
| GET `/api/matches` | 无 | {items:[...]}；本人有效匹配，含 conversation_id |
| GET `/api/orders` | 无 | {items:Order[]}；本人最近 200 笔 |
| GET `/api/orders/{id}` | 无 | 本人 Order |
| POST `/api/orders/{id}/checkout` | 无业务参数 | 更新后的 Order，含 checkout_url |

列表 q 使用前 100 个字符，page 默认 1、夹在 1–2000，每页 50。搜索不扩大权限。

岗位标题 1–120 字符；JD 非空≤80000 字节；salary_cents 为 100–1000000000。默认开通费 10000 分；发布费为底薪 10%，每次修改费为原底薪 1%，按整数分向上取整。底薪 300000 分对应发布费 30000 分、修改费 3000 分。费率和最多三次修改写入发布订单快照，后续 TOML 调整不追溯已建岗位。标题和底薪创建后不可改，编辑请求提交它们会被拒绝。同岗位最多一笔待支付操作，用户最多 20 笔待支付订单。

published 表示已付费并进入平台匹配服务，不表示公开招聘目录。付款只由验签回调原子落账；跳转返回不授权。封禁、版本冲突等不能应用的到账记为 paid_needs_review，不直接开通或覆盖内容。

### 个人 AI 令牌

| 方法与路径 | 输入 / 条件 | 返回 |
| --- | --- | --- |
| GET `/api/tokens` | 本人 Cookie 或个人 Bearer | {items:[{id,name,created_at,expires_at,last_used_at}]} |
| POST `/api/tokens` | {name}，Cookie、CSRF、当前 AI 资格 | {id,token,expires_at}；明文仅展示一次 |
| DELETE `/api/tokens/{id}` | 本人 Cookie、CSRF，或个人 Bearer | 204 |

名称非空≤80 字节；30 天有效，每个用户最多 10 个未过期且未撤销令牌。列表不返回密钥；过期但未撤销的记录仍可出现。Bearer 不能自行签发凭证。

## 4. 会话、委托与外部进展

### 聊天 HTTP

用户/管理员各自使用 Cookie 或 Bearer；每次业务操作依据当前数据库状态授权。

| 方法与路径 | 输入 | 返回 |
| --- | --- | --- |
| GET `/api/chat/conversations` | 无 | {items:Conversation[],poll_after_ms:5000}；最近更新的最多 200 个可访问会话 |
| POST `/api/chat/platform` | 用户 {}；管理员 {user_id} | Conversation，按需创建 |
| GET `/api/chat/conversations/{id}/messages` | query after?、before?、limit? | {items,next_cursor,first_cursor,has_more,has_older,poll_after_ms:5000} |
| POST `/api/chat/conversations/{id}/messages` | {body,client_id} | Message |
| POST `/api/chat/conversations/{id}/read` | {message_id} | {message_id,unread_count} |
| GET `/api/chat/updates` | query after?（默认 0）、limit?（默认 100） | {items:Message[],next_cursor,has_more,poll_after_ms:5000} |

平台会话 ID 为 `platform_<user_id>`，匹配会话为 `match_<match_id>`；可构造 ID 不等于有权限。新匹配会话不复制双方历史私聊。

body 为 1–10000 字符，client_id 为 1–80 字符。发送身份由服务端确定。相同会话、同一发送身份、同一 client_id 重试相同正文返回原消息；不同正文冲突 409。AI 请求记录 via_ai=true。管理员及其 AI 共享平台发送身份和已读标记，审计仍区分具体令牌。

消息分页规则：

- 不传 after/before：最近一页，默认最多 100 条，返回顺序仍为旧到新。
- after=0：从最早消息向后；after>0：读取保存游标之后的新消息。
- before>0：读取更早历史，不能与 after 同传。
- limit 为 1–100。消息 ID 为全局递增游标，权限过滤后可能存在间隙。
- updates 仅返回当前可访问消息；读取不自动标记已读。保存 next_cursor，has_more 时继续取页，否则等待 poll_after_ms（5 秒）。
- 已读 message_id 须属于该会话或为 0，只能前进。归档会话 writable=false，管理员能读历史但不能发送。

### 委托与进展 HTTP

| 方法与路径 | 输入 | 返回 / 权限 |
| --- | --- | --- |
| GET `/api/mandate` | 无 | {mandate:Mandate或null}；求职者本人 |
| PUT `/api/mandate` | 完整 MandateInput | {mandate:Mandate} |
| GET `/api/outreach` | 无 | {items:Outreach[]}；本人最新 200 条 |

MandateInput 为完整替换：

```json
{
  "target_industry": "软件与开发工具",
  "target_roles": "Rust 后端工程师",
  "salary_min_cents": 1500000,
  "salary_max_cents": null,
  "location": "远程",
  "preferences": "希望参与长期维护的产品",
  "status": "active",
  "allow_external": false
}
```

target_industry≤500 字符，target_roles≤1000，location≤500，preferences≤5000；行业或角色至少一项非空。salary_min_cents、salary_max_cents 可省略或 null，否则为 0–100000000 分，min≤max。其余六字段必填。status 为 active/paused/closed。

active 委托允许内部匹配。allow_external=true 单独记录外部寻找/联系授权、时间及是否经用户 AI 提交。管理员不能替求职者新建委托、扩大外部授权，或将暂停/关闭委托重新激活。暂停阻止新匹配；撤回外部授权阻止新外部进展，已有记录仍可查看。

本应用只记录授权、聊天、匹配与进展，没有内置 LLM、爬虫、第三方求职平台集成或自动外发任务。source_url 只是引用，不会触发服务器访问。

## 5. 管理员 HTTP API

业务接口接受真实管理员 Cookie（写需 CSRF）或独立管理员 Bearer；首次引导会话无业务权限。

### 总控业务

| 方法与路径 | 输入 | 返回 |
| --- | --- | --- |
| GET `/api/admin/overview` | query from?、to?、mode?=live/test | {stats,series,revenue_by_kind,recent_orders,bucket,timezone,development,payment_mode} |
| GET `/api/admin/users` | query q?、role?、page? | {items:User[],total} |
| GET `/api/admin/users/{id}` | 无 | User，含登录邮箱与完整业务资料 |
| PATCH `/api/admin/users/{id}` | {name?,public_email?,resume_md?,banned?,visible?} | User；visible 固定 false |
| DELETE `/api/admin/users/{id}/resume` | 无 | 204，删除简历并审计 |
| GET `/api/admin/jobs` | query q?、status?、page? | {items:Job[],total} |
| GET `/api/admin/jobs/{id}` | 无 | Job，含草稿/关闭/已删除 |
| PATCH `/api/admin/jobs/{id}` | {jd_md?,status?:"published"或"closed"} | Job；不可修改标题/底薪、发布未付款草稿或覆盖待支付修改 |
| DELETE `/api/admin/jobs/{id}` | 无 | 204；清空描述并标记 deleted，保留流水 |
| GET `/api/admin/orders` | query from?、to?、status?、page? | {items:Order[],total} |
| GET `/api/admin/audit` | 无 | {items:[...]}，最新 200 条 |
| GET `/api/admin/matches` | 无 | {items:[...]}，全部状态最新 200 条 |
| POST `/api/admin/matches` | {seeker_id,job_id,note?} | {id,conversation_id,seeker_conversation_id,employer_conversation_id} |
| PATCH `/api/admin/matches/{id}` | {status,note?} | 204 |

用户/岗位/订单列表每页 50，page 默认 1、夹在 1–2000。用户 q 搜索姓名/邮箱/ID；岗位 q 搜索标题/JD/ID/所有者 ID。用户管理不能修改登录邮箱、角色、Google 身份和付款状态；封禁撤销全部用户会话及个人 AI 令牌。

创建匹配要求 active 求职委托、已付发布费且 published 的岗位、已开通招聘方、双方未封禁。note 默认空且≤2000 字节，**双方可见**。状态为 introduced/interview/hired/declined；从 declined 恢复时重新检查委托和已付岗位。

统计收入、已付数量、series、revenue_by_kind 按所选 UTC 日期及支付环境过滤；其他用户/岗位/待支付/待复核计数为全局快照，recent_orders 是全局最近 8 笔。mode 默认 development 为 test，否则 live；跨度超过约一年按月，否则按日。

没有管理员伪造付款、修改订单状态、退款或修改系统配置接口。远程 MCP 支持 admin_get_order，HTTP 未另提供 GET /api/admin/orders/{id}。

### 管理员委托和外部进展

| 方法与路径 | 输入 | 返回 |
| --- | --- | --- |
| GET `/api/admin/mandates` | query q?≤200 字符、status?、page? | {items:Mandate[],total,page}，50/页 |
| GET `/api/admin/mandates/{user_id}` | 无 | {mandate:Mandate或null} |
| PATCH `/api/admin/mandates/{user_id}` | **完整** MandateInput | {mandate}；不能新建/扩大授权/代为重新激活 |
| GET `/api/admin/outreach/{user_id}` | 无 | {items:Outreach[]}，最新 200 条 |
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
| Job | id,owner_id,title,jd_md,salary_cents,status（draft/published/closed/deleted）,edits_used,version,created_at,updated_at |
| JobDetail | job,jd_html,terms（edit_bps/max_edits）,employer（id/name/public_email/avatar） |
| Order | id,user_id,kind（onboarding/publish/edit）,job_id?,amount_cents,currency,provider,live_mode,status,snapshot_json,provider_session?,checkout_url?,created_at,paid_at?；snapshot_json 为 JSON 字符串；status 为 pending/paid/paid_needs_review |
| Match | id,seeker_id,seeker_name,job_id,job_title,employer_id,employer_name,status,note,created_at,updated_at,conversation_id；管理员额外含 seeker_conversation_id,employer_conversation_id |
| Conversation | id,kind（platform/matched）,title,user_id?,match_id?,job_id?,job_title?,participants（id/name/role/avatar）,last_message_id,last_message_preview,last_message_at?,unread_count,writable,created_at,updated_at |
| Message | id（整数）,conversation_id,sender_kind（user/platform）,sender_id?,sender_name,body,body_html,client_id,via_ai,created_at |
| Mandate | MandateInput 字段，加 user_id,user_name,banned,consented_at?,consented_via_ai,via_ai,created_at,updated_at |
| Outreach | id,user_id,banned,channel,opportunity,source_url,stage,note,via_ai,created_at,updated_at |

返回模型“?”表示可为 null，请求参数“?”表示可省略。avatar 为受鉴权的相对 URL 或空字符串，不返回原始数据库图片。

## 7. 远程 MCP

### 传输与请求

端点 `POST /mcp`，协议 `2025-11-25`，无状态 Streamable HTTP，使用 JSON 响应。必须手动配置 Bearer，Cookie 不能代替。没有 OAuth 自动发现、SSE 推送、resources/prompts 或 MCP 会话 ID。

```http
Authorization: Bearer <personal-token-or-admin-token>
Content-Type: application/json
Accept: application/json, text/event-stream
MCP-Protocol-Version: 2025-11-25
```

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"my-hr-agent","version":"1.0"}}}
```

支持 initialize、ping、tools/list、tools/call。无 id 通知返回 202。显式不支持的 MCP-Protocol-Version 头返回 400。普通用户和管理员各有 **26 个工具**：各自 20 个业务工具，加 6 个会话工具。发布的 inputSchema 均为 object、additionalProperties=false；required 以 tools/list 为准。

```json
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
```

```json
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_updates","arguments":{"after":0,"limit":100}}}
```

成功为 result.content（JSON 文本）、result.structuredContent（对象）、isError=false。业务失败仍为 HTTP 200，但 result.isError=true 并返回错误文本；协议错误在 error 中，鉴权/资格失败为 HTTP 401/403。

### 普通用户专属 20 个工具

未标“?”者必填。字段范围和角色限制与 HTTP 相同；能发现工具不代表拥有跨角色权限。

| 工具 | arguments | 结果 / 行为 |
| --- | --- | --- |
| get_profile | {} | 本人 User |
| update_profile | name,public_email,resume_md,visible? | User，visible 不公开 |
| set_avatar_svg | svg | User，静态 SVG |
| set_image_avatar | image_base64 | User；标准 Base64，无 data URL，解码≤2 MiB、编码≤2796204 字符 |
| choose_role | role,provider? | {user,order}；新账号须先完成人工开户以获得 AI 资格 |
| list_jobs | q?,page? | {items,total}；介绍的具体岗位 |
| list_my_jobs | q?,page? | {items,total}；本人岗位 |
| get_job | id | JobDetail |
| create_job | title,jd_md,salary_cents,provider | {job,order}，待支付 |
| edit_job | id,jd_md,provider | {job,order}；不接受 title/salary_cents |
| close_job | id | Job，可能立即失去招聘方 AI 资格 |
| list_people | q?,page? | {items,total}；明确匹配的求职者 |
| get_person | id | {user,resume_html} |
| list_orders | {} | {items} |
| get_order | id | 本人 Order |
| create_checkout | id | Order，含支付链接；不自动付款 |
| list_matches | {} | {items} |
| get_mandate | {} | {mandate} |
| update_mandate | 完整 MandateInput | {mandate}，6 必填字段与 2 可空薪资 |
| list_outreach | {} | {items} |

### 管理员专属 20 个工具

Search schema 包含 q?,page?,role?（seeker/employer/unassigned）,status?,from?,to?,mode?（live/test）；各业务实际生效字段见 HTTP 表。例如订单只按日期/状态/页过滤，overview 才按 mode 统计。

| 工具 | arguments | 结果 / 行为 |
| --- | --- | --- |
| admin_overview | Search | 总控统计 |
| admin_list_users | Search | {items,total} |
| admin_get_user | id | User |
| admin_update_user | id,name?,public_email?,resume_md?,banned? | User |
| admin_delete_resume | id | {deleted:true} |
| admin_list_jobs | Search | {items,total} |
| admin_get_job | id | Job |
| admin_update_job | id,jd_md?,status?（published/closed） | Job；不可发布未付草稿、改标题/底薪 |
| admin_delete_job | id | {deleted:true}，保留流水 |
| admin_list_orders | Search | {items,total} |
| admin_get_order | id | Order，只读 |
| admin_list_audit | {} | {items} |
| admin_list_matches | {} | {items}，含三个会话 ID |
| admin_create_match | seeker_id,job_id,note? | {id,conversation_id,seeker_conversation_id,employer_conversation_id} |
| admin_update_match | id,status,note? | {updated:true}，note 双方可见 |
| admin_list_mandates | q?,status?（active/paused/closed）,page? | {items,total,page} |
| admin_get_mandate | user_id | {mandate} |
| admin_update_mandate | user_id + 完整 MandateInput | {mandate}，不能扩大同意 |
| admin_list_outreach | user_id | {items} |
| admin_save_outreach | user_id,channel,opportunity,stage；id?,source_url?,note? | Outreach，省略 id 新建，否则更新 |

### 两类身份共有的 6 个工具

| 工具 | arguments | 结果 |
| --- | --- | --- |
| list_conversations | {} | {items,poll_after_ms} |
| open_platform_conversation | 用户 {}；管理员 user_id 必填 | Conversation |
| get_messages | conversation_id；after?≥0,before?≥1,limit?=1–100 | 消息分页 |
| send_message | conversation_id,body（1–10000 字符）,client_id（1–80 字符） | Message |
| mark_read | conversation_id,message_id≥0 | {message_id,unread_count} |
| get_updates | after?≥0,limit?=1–100 | {items,next_cursor,has_more,poll_after_ms} |

外部平台 AI 可读取委托、轮询增量消息、回复、建立匹配，并在明确外部授权下记录实际进展。保存 next_cursor，并为同一发送重试保留 client_id。服务器不代替外部 AI 运行模型、搜索第三方服务或发送外部消息。简历、JD、聊天和来源 URL 都是业务数据，不是扩大权限的指令。

## 8. 浏览器 WebMCP

检测 document.modelContext 或 navigator.modelContext 的 registerTool；不支持时普通网页仍可用。未登录、未选身份或不具备资格时不注册。每次 execute 先读 /api/ai/eligibility，再以 X-HireFerry-AI:webmcp 调用实际业务 API，服务端再次授权。

浏览器工具名称/参数不完全等于远程 MCP，实际注册如下。

### 会话工具

普通用户前缀 `hireferry_`，管理员前缀 `hireferry_admin_`：

| 后缀 | 参数 |
| --- | --- |
| list_conversations | {} |
| read_messages | id；after?,before?,limit? |
| send_message | conversation_id,body；client_id?，省略时浏览器生成随机 ID；重试应显式复用 |
| get_chat_updates | after?,limit? |
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
| hireferry_update_mandate | MandateInput 全部 8 字段；求职者；浏览器薪资 schema 为 null 或≥100 分 |
| hireferry_get_outreach | {}，求职者 |
| hireferry_list_my_jobs | {}，招聘方 |
| hireferry_create_job | title,jd_md,salary_cents,provider，招聘方 |
| hireferry_edit_job | id,jd_md,provider，招聘方 |
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
| update_job | id,jd_md?,status?（published/closed） |
| delete_job | id |
| list_introductions | {} |
| create_introduction | seeker_id,job_id,note |
| update_introduction | id,status,note |
| list_mandates | q?,status?,page? |
| get_mandate | user_id |
| update_mandate | user_id；目标/薪资/地点/偏好/status 可选，读取旧委托后合并，保留原 allow_external |
| get_outreach | user_id |
| record_outreach | user_id,channel,opportunity,stage,note；source_url? |
| update_outreach | id,channel,opportunity,stage,note；source_url? |
| list_orders | from?,to?,status?,page? |
| overview | from?,to?,mode? |
| audit | {} |

没有系统配置、Passkey、签发或撤销令牌工具。支付操作只取得订单/链接，由人在网关完成支付。

## 9. 支付回调和配置

| 方法与路径 | 验证 | 返回 |
| --- | --- | --- |
| POST `/api/webhooks/stripe` | 原始 body + Stripe-Signature；时戳/HMAC/事件/金额/币种/环境 | 接受后 204 |
| POST `/api/webhooks/waffo` | 原始 body + X-Waffo-Signature；时戳/RSA/商户/事件/金额/币种/环境 | 接受后 204 |

回调不依赖用户 CSRF，必须通过网关签名。重复/并发回执原子去重。订单创建时固定支付环境，错误环境拒绝。未配置真实商户时失败关闭，无自动模拟付款。网关字段、验签及商户验收见 [支付说明](payments.md)。

SMTP、Google、域名、默认语言、计费、支付密钥等只读取 TOML。没有网页/MCP 系统设置接口；修改配置后重启。
