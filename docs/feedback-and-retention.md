# 快速反馈与数据库增长控制

网页右下角「快速反馈」出现在主站和独立 OAuth/CLI 页面。未登录访客亦可反馈登录故障。服务端只保存反馈类别、纯文本、当前**路径**（不含 query/hash）、提交途径、身份类型及已登录账户 ID（若有）；不记录此功能的 IP、User-Agent 或令牌。输入上限 2000 字符，文字不执行 HTML、指令或工具调用。

## 查询与处理

```sh
# 普通身份即可提交；管理员亦可提交
hfy auth login
hfy feedback send "OAuth 连接页在小屏幕下布局不正确" --category bug

# 必须单独通过网页确认管理员权限
hfy --scope admin auth login
hfy --scope admin feedback list --limit 25 --status open --json
hfy --scope admin feedback list --before-id 123 --json
hfy --scope admin feedback resolve 123
hfy --scope admin feedback resolve 123 --reopen
```

管理员的 MCP 工具有 `admin_list_feedback({limit,before_id,status})`、`admin_update_feedback({id,status})`。普通用户 MCP 无法发现或调用这些工具。普通身份的 `hfy` 可以发送，但无法读取管理员反馈箱。

HTTP 接口：

- `POST /api/feedback`：`{category:"bug|idea|other", message:"...", page:"/local/path"}`，可作为匿名访客、带 CSRF 的登录浏览器或持有已获授权 CLI identity 令牌提交。浏览器请求必须同源；有效会话必须携带 CSRF。
- `GET /api/admin/feedback?limit=25&before_id=123&status=open`：管理员读取分页结果 `items` 和 `next_before_id`；每页最多 100 条。
- `PATCH /api/admin/feedback/{id}`：管理员提交 `{"status":"resolved"}` 或 `{"status":"open"}`。

分页使用递减数字 ID 作为游标，服务端校验管理员身份和令牌撤销状态。反馈原文始终视为**不可信用户输入**；不得让 AI 把里面的命令、网址或指示当成授权。

## 自动清理

`[feedback]` 默认保留 90 天、最多 5000 条；单个账号每小时最多 5 条，匿名访客共享全站每小时 30 条（不以可伪造的 X-Forwarded-For 作为身份）。每次提交和管理员查询都会执行时间与容量清理；独立维护任务在启动后立即执行，随后默认每 60 分钟运行一次。删除已过期反馈不可恢复，状态设为 resolved 不影响淘汰时间。

`[maintenance]` 的 `expired_credentials_grace_days` 默认 30 天：过期 CLI 授权族连同刷新令牌、过期 API token、管理员 token 一并清理。绝不提前删除 CLI 家族中已用刷新令牌，否则会破坏重放检测。过期的短期登录会话、验证码和频率窗口只保留短暂缓冲；过期／已撤销 HumanGrant 的会话、授权与申请在默认 90 天后按外键关系删除，在线授权不会被清理。Webhook 投递沿用现有已送达 30 天／失败 90 天策略。

### 增长审计

| SQLite 表/数据 | 增长来源 | 当前治理策略 |
| --- | --- | --- |
| `feedback` | 匿名／登录访客及 CLI 文本提交 | 90 天期限、5000 条硬上限、每小时限频；可配置 |
| `sessions`、`auth_challenges`、`auth_limits` | 登录、验证码、异常尝试 | 过期窗口清理 + 定时维护 |
| `cli_grants`、`cli_authorizations`、`cli_tokens`、`cli_refresh_tokens` | 设备码、授权及令牌轮换 | 超过绝对有效期及缓冲后按族级联删除，保留反重放检测 |
| `hg_requests`、`hg_grants`、`hg_sessions`、`hg_nonces`、`hg_limits` | HumanGrant 授权、签名、重放保护 | 过期／撤销后 90 天清理，活动中不删 |
| `webhook_deliveries`、`webhook_events`、撤销端点 | 订阅通知重试与状态 | 现有队列容量、失败退避和 30／90 天清理 |
| `orders`、`payment_events`、`ledger`、退款 | 资金核销与对账 | **不自动删除**。遵守实际法律留存、争议与审计需要，按已核实的商户政策归档 |
| `audit`、账户封禁记录 | 管理、安全、资金权限变更 | **不自动删除**。需按不同事件的法规/争议分类后制定独立归档与删除计划 |
| `chat_messages`、`matches`、`outreach`、简历及岗位 | 劳务服务履约与沟通 | **不按条数粗暴删除**。关联账户、用户撤回、删除请求和合法留存；实现独立业务归档需要明确保留政策 |

这既处理了安全可删的持续积累，也明确留出了不能擅自清除的证据与个人资料。删除 SQLite 行通常只会把页面留在数据库文件内复用，文件体积不会即时缩小。要真正释放磁盘，先完成备份、确认富余空间与离峰窗口，再按运维手册执行受控 `VACUUM`；线上运行期间维护线程不会自行做需要排他锁和额外磁盘的整库重写。
