# User webhooks / 用户出站事件通知

聘渡可以把与本人有关的事件发送到用户自己管理的 HTTPS 接收端。生产站点为
`https://hireferry.lmm.best`，支持地址为 `support@lmm.best`。
本功能的实现依据为 [业务与投递 worker](../src/webhooks.rs)、
[系统配置](../src/config.rs)、[数据库迁移](../src/db.rs) 和
[回归测试](../tests/webhooks.rs)。支付网关向聘渡发送的入站付款、退款通知见
[支付文档](payments.md)，使用不同的接口和验签协议。

## 1. 启用与账户范围

用户在自己的工作台中管理接收端。求职者和招聘方都可以使用；账户必须完成身份选择且未被封禁。
普通用户 Webhook 不适用招聘方 MCP 的月薪门槛。

管理接口只接受本人的浏览器 Cookie。写操作同时要求正确的
`Origin: https://hireferry.lmm.best` 和 `X-CSRF-Token`；CSRF 值来自
`GET /api/session`。携带 `Authorization` 或 `X-HireFerry-AI` 的管理请求会被拒绝，
管理员会话和首次 Passkey 引导会话也不能代替普通用户创建端点。
这些凭据管理操作没有注册为 MCP 或 WebMCP 工具。

默认每个用户最多有 **5 个未撤销端点**，停用端点仍占用额度。每个端点独立生成
32 字节随机签名秘密，创建响应仅显示一次，格式为 `hf_wh_` 后接 64 个十六进制字符。
以后读取列表、修改端点或查看投递记录都不会返回秘密。保存好接收端副本；丢失后应撤销并新建端点。

发送方必须持有签名秘密才能生成 HMAC，因此服务端将它保存在受保护的 SQLite 中，
并非仅保存哈希。数据库及备份需要沿用部署说明中的访问权限。秘密不进入事件正文、
管理审计或应用日志；撤销端点会清空该端点保存的秘密。

## 2. 用户 API

以下路径均相对于 `https://hireferry.lmm.best`。JSON 请求使用
`Content-Type: application/json`；时间为 UTC Unix 秒。字段名区分大小写，
输入 DTO 拒绝未知字段。

| 方法与路径 | 请求 | 返回与规则 |
| --- | --- | --- |
| `GET /api/webhooks` | 无业务参数 | `{items, enabled, limits}`；`enabled` 为系统总开关 |
| `POST /api/webhooks` | `{name, url, events}` | `{endpoint, signing_secret}`；创建后默认启用 |
| `PATCH /api/webhooks/{id}` | `{name?, events?, enabled?}` | `{endpoint}`；只更新提供的字段 |
| `DELETE /api/webhooks/{id}` | 无业务参数 | `{revoked:true}`；撤销并取消尚未发送的投递 |
| `GET /api/webhooks/{id}/deliveries?limit=50` | `limit` 可省略，范围 1–100 | `{items}`；最新记录在前，无历史分页接口 |
| `POST /api/webhooks/{id}/test` | `{}` | `{delivery}`；仅排入一个不含业务资料的测试事件 |
| `POST /api/webhooks/{id}/deliveries/{delivery_id}/retry` | `{}` | `{delivery}`；只允许当前仍有权发送的 `failed` 记录 |

创建示例，接收端域名需要替换为自己真实管理的公网服务：

```json
{
  "name": "My HR integration",
  "url": "https://hooks.example.com/hireferry",
  "events": ["message.created", "match.updated", "job.updated"]
}
```

名称去除首尾空白后为 1–100 个字符，不允许控制字符。URL 最多 2,048 字节，
必须通过下文的出站地址限制。`events` 至少选择一种，最多六项；服务端校验支持的事件名，
排序并去重。URL 不能通过 PATCH 改动，迁移地址需要新建端点并撤销旧端点。

`Endpoint` 返回以下字段，没有账户登录信息或签名秘密：

```text
id, name, url, events, enabled, dropped_events, created_at, updated_at
```

`limits` 返回 `max_endpoints`、`max_attempts`、`max_pending_per_endpoint`。
`dropped_events` 是端点队列达到上限时被丢弃的合资格事件计数；这些事件不会自动补发。

`Delivery` 返回以下字段，不包含事件正文：

```text
id, event_id, event_type, status, attempts, max_attempts,
next_attempt_at, last_attempt_at, last_status_code, last_error,
created_at, delivered_at
```

其中 `last_attempt_at`、`last_status_code`、`last_error`、`delivered_at` 可以为 null。
`next_attempt_at` 仅在 `pending` 状态下表示下一次可投递时间；它不表示终态记录仍会重试。
创建测试或手动重试返回成功只表示已经排队，不代表接收端已经收到。

读取其他人的端点或投递返回 404。业务错误使用 `{"error":"说明"}`，
常见状态为 400、401、403、404、409、503；JSON 格式错误可能使用框架错误响应。
撤销后端点不再列出，也不能通过这些用户 API 继续读取该端点历史。

## 3. 六类业务事件与最小数据

事件由 SQLite 触发器写入业务操作所在的事务；业务事务回滚时，事件和投递也回滚。
只有事件发生时已启用、已订阅且有权读取该资源的端点会取得投递。
新建或重新启用端点不会导入以前的聊天记录或补发停用期间的事件。

| 事件 | 产生时机 | `data` 字段 | 可以收到的用户 |
| --- | --- | --- | --- |
| `message.created` | 新建聊天消息，包括本人发送的消息 | `id`：消息 ID 的字符串形式；`conversation_id` | 平台私聊的本人，或当前有效匹配会话的两位参与者 |
| `match.updated` | 建立匹配；修改阶段或介绍备注 | `id`：匹配 ID；`status` | 对应求职者和岗位所有者，且匹配仍有效 |
| `job.updated` | 建立岗位；修改状态、版本、JD 或招聘人数 | `id`：岗位 ID；`status` | 岗位所有者；明确有效匹配到该岗位的求职者 |
| `order.updated` | 建立订单；改变订单状态或累计退款金额 | `id`：订单 ID；`status` | 订单本人 |
| `mandate.updated` | 新建或更新求职委托 | `id`：委托所属用户 ID；`status` | 委托本人 |
| `outreach.updated` | 新建或更新平台记录的外部寻找进展 | `id`：进展记录 ID；`status`：该记录的 stage | 该求职者本人 |

状态名称直接来自对应业务资源。退款金额改变但订单状态仍相同时，也可能收到新的
`order.updated`；事件不包含金额，查看当前订单才能取得详细账务信息。
同样，免费修改 `headcount` 会产生 `job.updated`，但通知不增加人数、标题或 JD 字段。
人数从正数变为 0 时仍是 `published`，接收方应按当前权限读取岗位详情，不能仅凭相同的
`status` 判断岗位没有变化。相同人数的重复保存不产生新的岗位更新。

业务事件的 JSON 信封如下。示例 ID 与时间仅用于说明：

```json
{
  "id": "evt_00000000000000000000000000000000",
  "type": "message.created",
  "created_at": 1700000000,
  "data": {
    "id": "42",
    "conversation_id": "match_example"
  }
}
```

测试接口产生额外的 `webhook.test` 类型，仅发送到被测试的那个端点，
其 `data` 固定为 `{"test":true}`。它不是六类可订阅业务事件之一。

所有事件都不包含消息正文、Markdown 简历、JD、介绍备注、外联备注、邮箱、
头像、账户登录信息、付款凭据或签名秘密。订阅不会扩大平台 API 原有权限。

### 匹配与撤权

`introduced`、`interview`、`hired` 为有效匹配状态。`declined`、任一方封禁、
岗位删除都会使普通参与者失去匹配资源的投递权限。岗位仅为 `closed` 时，
已有有效匹配仍可沟通并接收其有权查看的事件。岗位所有者仍可获得自己岗位的状态变化。

`headcount=0` 只暂停新匹配，不撤销已有有效匹配；因此已有双方仍可收到有权访问的聊天及岗位事件。
首次删除进入回收站时，匹配对方不能再收到该岗位、匹配或匹配会话的通知；包括未发送的记录，
在发送前复查时也会被取消。首次删除恢复后，原有效匹配的历史按当前权限重新可读，
后续事件重新按当前权限分发；**已取消的投递不会因恢复而重新排队**。
永久删除不能恢复，岗位所有者仍可收到仅含 ID 和 `deleted` 状态的岗位通知。
完整人数、一次恢复和删除版本规则见 [岗位管理](jobs.md)。

平台私聊只通知该私聊的本人。建立匹配后使用新的匹配会话，既有平台私聊不会被复制、
合并或补发给另一方。被拒绝匹配的状态事件也不会为了「通知撤权」而绕过当前权限发送。
接收端不能依赖一定收到一条最终撤权事件。

worker 在领取记录时检查权限，并在 DNS 解析完成、开始实际 HTTP 请求前再检查一次。
已开始发送或已到达接收端的数据无法撤回；这种复查也不意味着接收端可以永久保留对资源的访问权。

## 4. 请求头与签名校验

投递使用 `POST`、`Content-Type: application/json`，并携带：

```text
X-HireFerry-Event: message.created
X-HireFerry-Delivery: whd_<stable delivery ID>
X-HireFerry-Timestamp: <Unix seconds for this attempt>
X-HireFerry-Signature: v1=<64 hexadecimal characters>
User-Agent: HireFerry-Webhooks/1
```

签名为：

```text
HMAC-SHA256(
    key = UTF-8 bytes of the COMPLETE "hf_wh_..." secret,
    message = UTF-8(decimal timestamp) + b"." + original HTTP body bytes
)
```

签名 key 使用创建时返回的**完整字符串**，包括 `hf_wh_` 前缀；不要把后面的十六进制部分解码为 key。
接收端必须在解析或重新序列化 JSON 之前取得原始请求字节，不能改变空格、字段顺序或编码后再验签。
`created_at` 是事件创建时间；请求头时间戳是本次尝试的时间，二者可以不同。

下面是仅依赖 Python 标准库的接收端校验函数。它不启动服务器、不发请求，也不包含真实密钥：

```python
import hashlib
import hmac
import json
import time


def verify_event(signing_secret, raw_body, timestamp_header,
                 signature_header, *, current_time=None):
    # Read the original bytes from the HTTP framework, before JSON parsing.
    if not isinstance(raw_body, bytes) or len(raw_body) > 8192:
        raise ValueError("Invalid webhook body")
    try:
        timestamp = int(timestamp_header)
    except (TypeError, ValueError):
        raise ValueError("Invalid webhook timestamp") from None
    if str(timestamp) != timestamp_header:
        raise ValueError("Invalid webhook timestamp")
    current_time = int(time.time()) if current_time is None else current_time
    if abs(current_time - timestamp) > 300:
        raise ValueError("Expired webhook signature")
    if not isinstance(signature_header, str) or not signature_header.startswith("v1="):
        raise ValueError("Invalid webhook signature")
    try:
        supplied_mac = bytes.fromhex(signature_header[3:])
    except ValueError:
        raise ValueError("Invalid webhook signature") from None
    if len(supplied_mac) != 32:
        raise ValueError("Invalid webhook signature")

    signed = timestamp_header.encode("ascii") + b"." + raw_body
    expected_mac = hmac.new(
        signing_secret.encode("utf-8"), signed, hashlib.sha256
    ).digest()
    if not hmac.compare_digest(expected_mac, supplied_mac):
        raise ValueError("Invalid webhook signature")

    event = json.loads(raw_body)
    if not isinstance(event, dict) or not isinstance(event.get("id"), str):
        raise ValueError("Invalid event envelope")
    return event
```

服务器端的等价 Rust 辅助函数为 `webhooks::verify_signature`。
两端时钟需要保持准确；五分钟窗口校验通过之后，仍必须做持久幂等处理。

### 幂等接收

签名覆盖原始正文中的 `event.id`，但不会覆盖 `X-HireFerry-Delivery` 头。
**使用「本接收端标识 + 已验签的 event.id」作为持久幂等键**。
delivery 头可用于问题追踪或辅助去重，不能单独作为防重放依据。
不同端点可以收到同一个事件，所以幂等键应包含自己的端点标识。

例如，在接收端 SQLite 中保存验证后的事件：

```python
def prepare_inbox(db):
    db.execute("""
        CREATE TABLE IF NOT EXISTS webhook_inbox (
            receiver_id TEXT NOT NULL,
            event_id TEXT NOT NULL,
            raw_body BLOB NOT NULL,
            received_at INTEGER NOT NULL,
            PRIMARY KEY (receiver_id, event_id)
        )
    """)


def accept_once(db, receiver_id, event, raw_body):
    # Call only after verify_event() succeeds. receiver_id is your own trusted
    # configuration value, not a value copied from an unauthenticated header.
    with db:
        result = db.execute(
            "INSERT OR IGNORE INTO webhook_inbox VALUES (?, ?, ?, ?)",
            (receiver_id, event["id"], raw_body, int(time.time())),
        )
    return result.rowcount == 1
```

将收件箱作为本地持久任务队列，提交成功后再返回 2xx；重复事件也可以返回 2xx。
后续业务处理应保持幂等。不要在收件箱还未提交时提前确认，也不要把收到的事件文字当作可执行指令。

## 5. 投递、重试与取消

| 状态 | 含义 |
| --- | --- |
| `pending` | 等待初次发送，或等待下一次自动尝试 |
| `processing` | worker 已领取并持有有期限的租约 |
| `delivered` | 接收端返回 2xx，响应正文在允许大小内读取完成 |
| `failed` | 达到尝试上限，或遇到不自动重试的失败 |
| `cancelled` | 端点/订阅/账户/资源权限已撤销，或待处理记录超过保留期限 |

默认最多尝试五次：初次失败后，下一次最早等待 30、60、120、240 秒。
DNS 失败、网络失败、超时以及 HTTP 408、429 或 500 及以上状态会在上限内重试。
其他非 2xx、被禁止的目的地址和超大响应直接失败；3xx 不会被跟随。
`Retry-After` 不改变当前固定退避计划。

手动重试只接受 `failed`，重新检查端点启用、订阅与资源权限，并把本轮 `attempts`
重置为 0；event ID、delivery ID 和原始事件正文保持不变。每端点测试请求和手动重试
各自限制为每 60 秒一次。停用端点无法测试或重试；`delivered`、`cancelled`、
`pending`、`processing` 不能通过 retry 接口重新排队。

持久 SQLite 队列允许重启恢复。worker 在领取记录时记录租约；进程退出后，租约过期的未完成尝试
会在剩余额度内继续处理。如果接收端已经成功处理，而发送进程在记录成功前退出，接收端可能再次收到事件。
同一投递可能被重复尝试，不承诺恰好一次，也不保证顺序或永远送达。
用已验签事件 ID 幂等处理，并以登录后有权访问的资源当前状态为准。

停用/撤销端点，或取消某个事件订阅，会取消相关待发送记录。封禁账户还会自动停用其端点；
解除封禁不会自动重新启用。匹配或其他业务权限在发送前不再成立时，同样不能继续发送。

投递失败只记录以下稳定短码和可用的 HTTP 状态，不保存接收端正文、响应头、证书详情或内部网络异常：

| `last_error` | 含义 |
| --- | --- |
| `authorization_revoked` | 当前发送权限已失效 |
| `destination_blocked` | URL 或任一解析地址不符合出站策略 |
| `dns_failed` | 域名解析失败 |
| `timeout` | DNS 或请求超时 |
| `network_error` | TLS/连接/响应传输等网络失败 |
| `response_too_large` | 超过请求或响应正文上限 |
| `http_error` | 接收端返回非 2xx，具体值见 `last_status_code` |
| `worker_interrupted` | 中断租约耗尽额度，或待处理记录超过保留期限 |

## 6. 系统 TOML 限制与保留

用户只能配置自己的端点；系统限制由管理员修改 TOML 后重启服务：

```toml
[webhooks]
enabled = true
max_endpoints = 5
max_attempts = 5
max_pending_per_endpoint = 1000
request_timeout_secs = 5
max_response_bytes = 16384
poll_interval_secs = 5
delivered_retention_days = 30
failed_retention_days = 90
```

配置范围分别为：端点数 1–5、尝试次数 1–5、待投递上限 1–1000、超时 1–15 秒、
响应正文 1024–65536 字节、空队列轮询 1–60 秒、两类保留时间各 1–365 天。
出站事件正文另有固定的 8192 字节上限。没有默认接收端或自动生成的示例数据。

待投递上限统计 `pending` 和 `processing`。队列满时，新事件对该端点的投递被丢弃并累加
`dropped_events`，不会覆盖已有记录。其他有剩余额度的合资格端点仍可收到该事件。

清理按投递的 **`created_at`** 计算，而非按最后完成时间计算。
`delivered` / `cancelled` 默认保留 30 天，`failed` 默认 90 天；仍待处理但超过失败保留期限的记录
会停止处理并进入清理。没有关联投递的事件正文也会清除，已撤销且无投递引用的过期端点随后清理。
worker 运行时大约每分钟执行一次维护。这不是永久事件档案。

全局 `enabled=false` 时不生成新的业务事件，也不启动发送 worker。
已有队列暂停，重新启用后仍需通过权限、保留时间和尝试额度检查；停用期间的业务事件不会补录。
用户仍可读取端点及历史、修改名称/订阅、停用或撤销端点，但不能新建、重新启用、测试或重试。

从旧版升级到当前 schema 5 时，会先应用 schema 4 引入的岗位人数与删除状态字段，再重建岗位更新触发器，
让已有数据库同样捕获 `headcount` 的变化。无需手工重建触发器或新增事件订阅；
`job.updated` 的事件名和最小数据格式保持一致。

## 7. SSRF 与出站网络边界

所有环境使用相同限制，`development=true` 不会允许 HTTP 或内网接收端：

- 只允许 HTTPS、443 端口，不允许用户名/密码或 URL fragment；拒绝控制字符、空白和反斜杠。
- 拒绝单标签主机、尾点域名，以及 localhost、`.local`、`.internal`、`.home`、`.lan`、`.test`、`.invalid`、`.onion` 等本地域名。
- 拒绝回环、私网、链路本地、CGNAT、文档/基准测试地址、多播、保留范围及常见 metadata 地址；显式拒绝 Azure `168.63.129.16`。
- IPv6 使用保守的全球单播范围检查，拒绝 IPv4 映射/转换、6to4、Teredo/协议保留及文档地址；因此部分特殊用途地址即使可达也不能作为接收端。
- 每次发送重新解析 DNS，最多接受 64 个不同地址；**全部**结果都必须为允许的公网地址。混合公网/私网结果整体拒绝。
- DNS 验证后的地址绑定到本次独立 reqwest client，避免验证后重新解析到其他地址；TLS 证书校验保持启用。
- 禁用代理与重定向，不携带聘渡登录 Cookie、Bearer token 或服务器服务凭据。只发送上述事件头、签名和最小正文。
- DNS 和 HTTP 阶段分别受超时限制，响应正文受大小限制，不接受「开发例外」绕过。

创建端点时执行 URL 语法和字面 IP 校验；域名的 DNS 与实际连接限制在每次 worker 投递时执行。
创建成功本身不表示域名已经解析成功、证书有效或接收端能响应。

## 8. 验证范围与上线联调

`tests/webhooks.rs` 使用真实 HTTP 路由、SQLite 事务和投递 worker，注入 transport 测试
管理权限、密钥不回显、端点上限、私聊隔离、回滚、DNS 后撤权、混合地址拒绝、HMAC、
有限重试、重启租约、配额/保留和六类业务事件。当前回归不会访问真实公网接收端；
测试运行结果以本次交付的统一测试记录为准。

生产联调仍需要用户自己控制的公网 HTTPS 接收端，验证实际 DNS、证书、签名、幂等入队、
2xx 确认、故障重试和撤权行为。`POST .../test` 只发送不敏感测试事件，成功排队后还需查看投递结果。
当前没有 OAuth 接收端授权、额外自定义请求头、mTLS、历史事件批量重放或自动恢复已取消记录。

Google、SMTP 与商户支付接入也需要各自真实凭据和联调，不能由 Webhook 测试结果代替。
用户提供的两处生产凭据路径在当前交付环境不存在；本次文档及实现没有从这些路径读取、
导入或验证生产密钥。

## English summary

Member webhooks send only resource identifiers and statuses to explicitly configured public HTTPS:443 endpoints.
Management requires the owning member's browser session and CSRF; Bearer, AI and administrator sessions cannot manage these credentials.
The full `hf_wh_...` secret is shown once and is used verbatim as the UTF-8 HMAC key.
Verify `timestamp + "." + raw_body`, enforce the five-minute timestamp window, and use constant-time comparison.
Deduplicate using your own receiver ID plus the **signed body event ID**; the delivery header alone is not signed.

The SQLite outbox is transactional, persistent and bounded. Permissions are checked when events are queued and again after DNS resolution before each HTTP request.
Private platform conversations never become shared match history. Delivery attempts can repeat and arrive out of order, and cancelled, expired or over-quota events are not replayed automatically.
Headcount changes emit `job.updated` with the same ID/status-only payload. Zero headcount keeps existing match access; deletion suspends counterpart access, and one-time recovery restores eligible history without replaying cancelled deliveries. Permanent deletion cannot be recovered.
All environments reject private/reserved destinations, DNS rebinding, proxies and redirects. Automated tests inject a transport; a real HTTPS receiver still needs deployment-time integration testing.
