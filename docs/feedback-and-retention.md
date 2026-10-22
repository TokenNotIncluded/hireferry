# 快速反馈与保留策略

主站和 `/oauth/connections` 右下角提供「快速反馈」。访客也能报告登录问题。反馈保存类型、纯文本描述、来源、当前路径、提交时间和处理状态。来源由服务端根据登录方式确定。路径不接受查询参数或 fragment。

描述限 2–2000 个 Unicode 字符和 8000 字节，请求体上限 16 KiB。服务端校验类别、路径和控制字符。网页只使用 `textContent` 显示回执，反馈正文以 JSON 字符串返回，不作为 HTML 或管理员指令执行。审计只保存事件、身份、反馈编号和状态，不复制反馈正文。

## 网站与 CLI

```sh
hfy auth login
hfy feedback send "连接页在手机上显示不完整" --category bug

hfy --scope admin auth login
hfy --scope admin feedback list --limit 25 --status open --category bug --source web --json
hfy --scope admin feedback list --before-id 123 --json
hfy --scope admin feedback get 123 --json
hfy --scope admin feedback resolve 123
hfy --scope admin feedback resolve 123 --reopen
```

反馈类型为 `bug`、`idea`、`other`，来源为 `web`、`cli`，状态为 `open`、`resolved`。普通 CLI 身份可以提交。查询和处理仅开放给当前有效的管理员身份。

## 管理员 MCP 与 HTTP

| 功能 | MCP | HTTP |
| --- | --- | --- |
| 分页与筛选 | `admin_list_feedback({limit,before_id,status,category,source})` | `GET /api/admin/feedback`，字段作为查询参数 |
| 单条详情 | `admin_get_feedback({id})` | `GET /api/admin/feedback/{id}` |
| 标记处理或重开 | `admin_update_feedback({id,status})` | `PATCH /api/admin/feedback/{id}` |
| 提交 | 网站和 CLI 提交入口 | `POST /api/feedback` |

提交内容为 `{category,message,page?}`。浏览器请求必须同源，已登录会话还需 CSRF。CLI 需有效身份授权。普通 MCP Token 不能作为 CLI 身份提交，也不能发现或调用管理员反馈工具。列表、详情、更新均在服务端复核管理员权限和令牌撤销状态。

分页按编号倒序，每页默认 25 条，最多 100 条，返回 `items` 与 `next_before_id`。详情与处理使用数字编号。已清理的编号返回 404。已关闭提交入口时，管理员仍能查询和处理历史记录。

## 默认配置

```toml
[feedback]
enabled = true
retention_days = 180
resolved_retention_days = 30
max_records = 10000
duplicate_window_seconds = 600
max_per_account_per_hour = 5
max_guest_per_hour = 30

[maintenance]
interval_minutes = 60
batch_size = 200
expired_credentials_grace_days = 30
human_grant_history_days = 90
```

修改 TOML 后重启生效。旧的 `retention_days` 配置键继续有效；升级不会覆盖部署者原有设置。

- 未处理反馈从提交或实际重开起保留 180 天，已处理反馈从状态变化起保留 30 天。重复写入相同状态不延长期限。
- 达到 10,000 条时，新提交返回 **503**。保留所有尚未到期的现有记录，不按编号或状态腾挪容量。容量下调到现有数量以下时同样停止接收，等待到期清理或管理员调整配置。
- 每个账号在固定的一小时窗口内最多提交 5 条。网页与 CLI 共享账号配额。所有访客共享每小时 30 条配额，不记录 IP 或 User-Agent。
- 同一身份、同一类别与相同的去首尾空白正文，在默认 10 分钟内再次提交返回 **409**。网页路径与 CLI 切换不会绕过检查。访客共享重复检查，不返回其他访客的反馈编号。重复请求不消耗新配额。
- 配额与正文指纹写入 SQLite，和反馈提交处于同一事务。服务重启、处理反馈、反馈过期都不会清空当前窗口。过期计数和指纹由后台有界清理。
- **429** 表示限流。**503** 表示反馈暂不可用或容量已满。网站和 CLI 保留失败内容，不把这些响应显示为提交成功。

## 后台清理

维护任务在服务启动后立即运行，空闲时默认每 60 分钟运行。每张表每批最多 200 条，允许配置 1–1000 条。存在积压时，间隔一秒继续处理。每个阶段释放数据库互斥锁，SQLite 工作在阻塞线程中执行。关闭反馈、CLI、HumanGrant 或 Webhook 功能不关闭维护任务。

反馈读取不触发删除。提交前可做一轮小批次清理；没有任何访问时，独立维护任务仍会完成清理。维护保留当前有效的授权与反重放信息，不自动执行 `VACUUM`。

完整表、缓存、文件和队列的生命周期见[数据生命周期](data-lifecycle.md)。资金、审计、业务消息与招聘履约记录不由反馈清理删除。

## 升级

数据库升级到 schema 12。新增独立配额、重复提交指纹表以及清理索引，保留 schema 10 的反馈内容和编号及 schema 11 的员工邀请。升级以事务提交；失败时版本号及新增对象一起回滚。升级前使用一致性备份，回退时恢复对应备份。
