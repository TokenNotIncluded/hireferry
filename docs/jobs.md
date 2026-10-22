# Job management / 招聘人数与岗位回收站

招聘人数可以免费修改，不限次数。人数为 0 表示暂不招人；第一次删除可以免费恢复一次，
恢复后再次删除就是永久删除。人数、关闭、删除分别处理，不增加付费 JD 修改次数，也不自动退款。

本文依据当前 [岗位事务](../src/commerce.rs)、[用户 API](../src/api.rs)、
[管理员业务](../src/admin.rs)、[权限检查](../src/access.rs)、[MCP](../src/mcp.rs)、
[数据库迁移](../src/db.rs) 和 [用户 Webhook](../src/webhooks.rs)。
完整调用约定见 [API 文档](api.md)，支付验签和退款见 [支付文档](payments.md)。

## 1. 招聘人数与费用

`headcount` 是招聘方人工维护的计划招聘人数，必须是 **0–1,000,000 的整数**，创建岗位时省略则为 1。
已开通且未封禁的招聘方可以修改本人 `draft` 或 `published` 岗位的人数：

- 修改免费，不限次数，不生成订单或新增收费。
- 不改变标题、月薪底薪、`version` 或 `edits_used`。
- 可以在待付款 JD 修改订单存在时修改人数；人数不属于那笔订单的内容版本。
- 匹配、面试和录用不会自动扣减人数；正数不限制候选匹配总数。
- 真正改变人数时更新 `updated_at`，并记录独立的 `job_headcount_update` 审计，包含修改前后人数。

发布费仍为一个岗位原月薪底薪的 10%，每次付费 JD 修改为原月薪底薪的 1%，默认最多 3 次。
**费用不乘招聘人数。** 例如，底薪 ¥3,000 的岗位无论计划招 1 人还是 10 人，默认发布费都是 ¥300，
每次 JD 修改都是 ¥30。付费修改次数用完后，人数仍能免费修改。费率和最大 JD 修改次数由 TOML 配置；
已创建订单按其已保存条款结算。

## 2. 暂停、关闭与删除的区别

| 当前状态 | 新建匹配或恢复 declined 匹配 | 已有有效匹配及聊天 | 可用的主要操作 |
| --- | --- | --- | --- |
| `draft` | 不允许 | 正常业务尚未建立匹配 | 修改人数、支付原发布订单、删除 |
| `published`，人数大于 0 | 满足委托、付款和账户条件时允许 | 保留 | 免费改人数、付费改 JD、关闭、删除 |
| `published`，人数为 0 | 不允许 | 保留，也可继续推进阶段 | 免费改为正数、付费改 JD、关闭、删除 |
| `closed` | 不允许 | 原有效匹配仍可沟通 | 删除；不能靠人数改回招聘中 |
| `deleted`，`delete_count=1` | 不允许 | 普通参与者的匹配访问暂时撤回 | 免费恢复一次，或永久删除 |
| `deleted`，`delete_count=2` | 不允许 | 普通参与者不能再进入该岗位的匹配资源 | 永久状态，不能恢复 |

这里的有效匹配为 `introduced`、`interview`、`hired`，同时要求双方未被封禁。
`declined` 本身不授予双方资料或聊天权限。新建匹配和将 declined 重新激活时，还要求求职者有
`active` 委托、招聘方已开通、岗位为 `published`、发布付款有效且人数大于 0。
已有效的匹配可以在人数为 0 时继续从介绍推进到面试或录用，不要求重新开放招聘。

把人数设为 0 不等于关闭，改成正数即可恢复该已发布岗位的新匹配。
`closed` 则不能通过人数修改或管理员 PATCH 改回 `published`。
全额发布退款会关闭岗位并置人数为 0；若岗位当时已经删除，退款不会把它重新显示出来。
部分退款按支付规则保留既有权益。

管理员的“招聘中岗位”统计只计算 `published`、人数大于 0 且所有者未被封禁的岗位。
平台没有公开岗位或简历目录；“已发布”表示发布付款及平台匹配状态，不扩大公开可见范围。

## 3. 一次恢复与永久删除

`delete_count` 属于同一个岗位，含义如下：

| 值 | 含义 |
| --- | --- |
| `0` | 尚未使用首次删除机会 |
| `1` | 已使用首次删除机会；恢复后仍保持 1 |
| `2` | 永久删除，只有账务关联墓碑 |

第一次普通删除保存删除前的状态和人数，把岗位设为 `deleted`、人数 0、`delete_count=1`。
标题和 JD 暂时保留，只有本人和有权管理的后台可读取。本人通过回收站查找，普通岗位列表不再列出它。
回收站列出最近 200 条可恢复岗位，不包含永久删除墓碑。

恢复只接受当前仍为 `deleted` 且 `delete_count=1` 的岗位。恢复后计数不归零；
下一次普通删除就直接永久删除。也可以在首次删除之前或回收站内主动选择永久删除。
删除和恢复不创建付款订单、不收取费用，也不重置已用 JD 修改次数。

本人删除、永久删除和恢复内容要求账户未封禁、身份仍为招聘方，不要求再次支付开通费。
**恢复内容与恢复招聘权益分开判断**，具体按下一节执行。
管理员删除直接使用永久删除，不经过一次可恢复删除；管理员恢复也不能绕过次数、封禁或支付检查。

### 为什么必须提交 `expected_version`

普通删除、显式永久删除、恢复，以及对应管理员 API，都要求 JSON 中带岗位当前的非负整数版本：

```json
{"expected_version": 7}
```

用本人岗位列表、回收站或管理员列表返回的 `Job.version` 填入，不能省略或猜测。
内容/状态变更、删除和恢复会增加 `version`；人数单独修改不会增加它。
服务器在同一个 SQLite 写事务中比较版本和执行操作。

例如，岗位为版本 7 时：

1. 第一次删除提交 7，返回回收站岗位版本 8、删除计数 1。
2. 如果响应丢失，再次提交同一普通删除，岗位仍在回收站时原样返回；不会升级为永久删除。
3. 恢复提交当前版本 8，返回版本 9，删除计数仍为 1。
4. 此时迟到的旧删除请求仍带 7，会返回 409；只有用户基于版本 9 发起的新删除才会永久删除。

已经永久删除的重复删除请求也只返回已有墓碑。恢复操作没有重复删除的特殊返回规则；
恢复成功后的旧恢复请求会发生版本/状态冲突。
遇到 409 时先重新读取状态并让用户确认当前动作，不应自动换用新版本重复危险操作。

## 4. 恢复时保留什么状态

恢复先验证所有者仍为未封禁的招聘方，再检查删除前状态和当前原始发布订单：

| 删除前状态与当前条件 | 恢复结果 |
| --- | --- |
| 原为 `published`，招聘方当前已开通，原发布订单为 `paid` 或 `partially_refunded` | 恢复 `published` 和原人数，包括原人数 0 |
| 原为 `draft`，原发布订单仍为 `pending` | 恢复 `draft` 和原人数，可按原订单继续支付 |
| 原为 `closed` | 恢复 `closed`，人数 0 |
| 原发布权益已全额退款、开通失效，或原草稿款项已到账但需人工核对等其他情况 | 内容恢复为 `closed`，人数 0 |

恢复不会新建发布订单、再次计费或授予已经失效的权益。
即使恢复为 `closed`，原匹配历史仍按当前有效匹配权限重新可见；这不等于重新接受新匹配。

首次删除即时撤回普通参与者对该岗位的匹配资料与匹配会话访问。
恢复后，原有效匹配可以重新访问其同一个会话的历史；不会新建另一份聊天，也不会把双方与平台的独立私聊合并进来。
被拒绝的匹配或封禁账户不会因为岗位恢复而重新取得权限。
管理员可以审阅归档会话，但不能向删除、拒绝或封禁导致的只读会话发送消息。

## 5. 待付款、迟到回调与数据保留

进入回收站后，本站拒绝为关联订单创建或复用 checkout；本人订单列表和单条订单读取
（包括 MCP `get_order`）也隐藏原 `checkout_url`。
首次删除保留待付款的发布订单，数据库保留链接供合法恢复为待付款草稿后继续使用。
删除会使旧 JD 修改订单的内容版本失效，因此同时取消尚未付款的 JD 修改订单并清空链接；
恢复后可以创建新的修改订单，旧订单不会一直占用待付款名额。该取消不收取费用。
已关闭或退款的岗位、已失去开通资格的账户，以及内容版本已变化的旧修改订单，也不再显示或创建付款链接。
网关自身可能仍持有此前已经打开的收银台；本站隐藏链接不等于已撤销网关托管页面。

永久删除将关联 `pending` 订单置为 `cancelled`，清空缓存付款链接和全部 `snapshot_json`，
同时清除岗位标题、JD、保存的删除前状态和人数。岗位记录保留为 `deleted`、人数 0 的墓碑。
付款创建在外部网络请求前后都会复查账户、订单与岗位状态，避免等待网关期间删除后又返回可用链接。

若用户在旧网关页面完成付款，验签通过且金额、币种、提供方等条件匹配的真实款项仍需如实入账。
由于删除、取消、封禁或内容版本冲突而不能履行的款项记为 `paid_needs_review`，
不会复活岗位、覆盖已经恢复的内容或增加 JD 修改次数。管理员需要根据商户记录核对处理；本站不自动退款。
已经记录的款项按支付事件及款项标识去重。

待付 JD 的旧版本也不会因为恢复而重新适用：删除和恢复递增版本，迟到的修改款项只能进入核对流程，
不能把旧 JD 写回当前岗位。单独修改人数不改变版本，因此不会使正常的待付 JD 内容过期。

永久删除仍保留订单金额、币种、提供方与付款标识、实收/退款流水、审计、岗位底薪和关联 ID，
供账务核对。匹配及既有聊天留作按权限管理的历史；它们不是岗位内容字段，永久删除岗位不会自动清除这些记录。
以前已经导出的资料、到达外部接收端的事件或独立备份也不会被此操作追溯抹除。

## 6. HTTP、MCP 与管理员入口

HTTP 路径相对于 `https://hireferry.lmm.best`。JSON 使用 `Content-Type: application/json`。
普通浏览器写操作要求本人 Cookie、正确 Origin 和 `X-CSRF-Token`。
Bearer / WebMCP 则同时检查当前 AI 资格，不会通过免费操作绕过角色或匹配边界。

| 本人 API | 请求 | 返回 |
| --- | --- | --- |
| `POST /api/jobs` | `{title,jd_md,salary_cents,provider,headcount?}` | `{job,order}` |
| `PATCH /api/jobs/{id}/headcount` | `{headcount}` | `Job` |
| `GET /api/my/jobs/trash` | 无业务参数 | `{items: Job[]}` |
| `DELETE /api/jobs/{id}` | `{expected_version}` | `Job`；首次回收，恢复后第二删永久 |
| `POST /api/jobs/{id}/restore` | `{expected_version}` | 恢复后的 `Job` |
| `DELETE /api/jobs/{id}/permanent` | `{expected_version}` | 永久删除后的 `Job` 墓碑 |

`Job` 新增返回 `headcount`、`delete_count`，保留 `status`、`version`、`edits_used` 等字段。
内部保存的 `deleted_from_status`、`deleted_headcount` 不直接序列化。

| 管理员 API | 请求 | 规则 |
| --- | --- | --- |
| `PATCH /api/admin/jobs/{id}` | `{jd_md?,status?,headcount?}` | 只处理已发布/关闭岗位；纯人数修改独立审计，不增加 JD 版本或修改次数；不能发布草稿或复活关闭/删除岗位 |
| `DELETE /api/admin/jobs/{id}` | `{expected_version}` | 直接永久删除，返回 `Job` 墓碑 |
| `POST /api/admin/jobs/{id}/restore` | `{expected_version}` | 使用与本人恢复相同的规则，返回 `Job` |

管理员草稿当前为只读/可永久删除；后台的删除能力不会绕过版本校验。
管理员修改实际 JD 或状态仍会增加版本，并在待付款相关订单存在时要求先处理。
关闭时人数归零；只有改变人数本身不受这项待付款限制。

远程 MCP 共 **32 个普通用户工具、32 个管理员工具**。`get_site_config` 向两类客户端提供公开币种、精度、计费方式和业务限额。与本功能直接相关的普通工具为
`create_job`（可选 `headcount`）、`set_job_headcount`、`list_deleted_jobs`、`delete_job`、
`restore_job`、`permanently_delete_job`；管理员使用 `admin_update_job`、`admin_delete_job`、`admin_restore_job`。
删除/永久删除工具标记为破坏性操作；JSON 参数与 HTTP 共用业务验证。
浏览器 WebMCP 按登录角色提供相应的 `hireferry_` 前缀工具，复用相同业务授权；
其注册集合和命名不与远程工具目录一一对应，32/32 仅为上述远程 MCP 的数量。

招聘方 AI 资格要求已开通，且至少有一个发布费有效、`published`、月薪底薪不低于 ¥3,000 的岗位。
**人数为 0 仍是 published，保留满足其他条件的 AI 资格**，所以可以通过 AI 免费恢复人数。
关闭或删除最后一个合资格岗位会失去 AI 权限；此时本人仍可在普通浏览器中使用允许的回收站操作。
回收站的 MCP 工具不会作为资格检查的例外。

## 7. 工作台、Webhook 与数据库升级

平台从当前可招聘岗位进入匹配工作台，只载入一页候选摘要，再按需读取单人资料。
基础候选范围是 active 委托且未封禁的求职者；行业、岗位、地点和期望薪资可以继续筛选。
当前岗位已有有效匹配的候选不重复列入，已终止的匹配保留历史标记，重新介绍仍需显式执行原阶段更新。
搜索和预览不会自行授予匹配权限、扩大委托或发出外部联系。
名单与详情仅管理员及其独立 AI 可用，招聘方不能借此直接浏览未匹配求职者。
匹配跟进按最近更新时间分页，已删除或关闭岗位的历史仍可审阅；聊天是否可写继续使用原授权规则。
精确接口、游标和薪资区间规则见 [工作台 API](api.md#岗位中心匹配工作台)。

岗位人数实际变化会发出 `job.updated`，但通知正文仍只含资源 `id` 和 `status`。
人数变化不要求增加订阅类型；接收方收到通知后应按当前授权读取详情。
人数 0 保留已有匹配的通知权限；删除时匹配对方无权接收，恢复只恢复当前权限，
不重放已经取消的通知。具体签名与投递规则见 [用户 Webhook](webhooks.md)。

本版数据库 `user_version` 为 **10**。schema 9 的优惠码与独立发布次数记录见 [优惠码说明](promo-codes.md)，schema 10 新增有限期快速反馈。schema 4 引入人数和删除生命周期字段；从旧版升级时会先补齐这些字段。
旧草稿、已发布岗位默认人数 1；旧关闭和删除岗位人数归零。
旧版本已经删除的岗位按永久删除处理、`delete_count=2`，不会获得一次恢复机会；
同时清除其标题、JD、关联订单快照与付款链接，取消尚待付款的关联订单，保留账务关联。
岗位更新触发器随后重建，以捕捉人数变化。schema 5 追加工作台的分页/业务筛选索引，以及用户、岗位、委托、匹配备注的
FTS5 trigram 索引；首次升级对已有内容建立索引，后续由事务触发器维护，查询仍连接当前业务表检查资格。
这些索引与资料同处受保护的 SQLite 数据库，不改变私密资料的授权范围。

升级前先用 CLI 备份，再替换程序；旧版本不能直接操作已迁移数据库。
见 [部署、备份与恢复](deployment-handoff.md#9-备份升级与恢复)。
相关回归代码包括 [人数与付款](../tests/headcount_flows.rs)、
[删除生命周期](../tests/job_lifecycle.rs)、[管理员、匹配、MCP 与 Webhook](../tests/headcount_admin_mcp.rs)。
本轮实际运行结果、日志和页面证据统一记录在 [验收记录](acceptance.md)，本文不把测试定义视为已通过结果。

## English summary

Planned headcount is a manually maintained integer from 0 to 1,000,000, defaulting to 1.
Changing it is free and unlimited, independent of pricing and paid description revisions.
Zero keeps the role published and preserves existing matches, conversations and otherwise valid employer AI eligibility;
it only blocks new introductions and reactivation of declined matches. Counts do not decrement automatically or cap the number of candidates.

The first normal deletion enters a private, recoverable trash state. Recovery is free once, but does not reset the deletion counter;
deleting again is permanent. Explicit permanent deletion and administrator deletion are permanent immediately.
All delete/restore endpoints require the current `expected_version`; a delayed first-delete retry cannot become a permanent deletion after recovery.
Recovery rechecks current payment rights and can return the content closed with zero headcount instead of reopening recruitment.

Deleted roles cannot start or reuse checkout through HireFerry. Permanent deletion cancels pending orders and clears the title,
description, order snapshots and cached checkout links, while retaining accounting references and authorized historical records.
A valid late payment is recorded for review without resurrecting the role. Schema 4 migrates older deletions as permanent.
