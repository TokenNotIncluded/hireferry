# 消息、流式对话与 Agent 接入

## 早期阶段的异步平台 AI

聘渡目前仍在早期阶段。平台 AI 是异步、主动式服务，不是时刻在线的实时客服。用户给平台发消息后，服务器会先把消息放入持久化队列，自动接待会在后台确认收到，并提供简历与推荐岗位入口；后续连接的 MCP Agent 或平台工作人员可以继续处理匹配、岗位卡片和跟进。服务重启不会丢掉尚未处理的自动回复，但不承诺即时响应或全天候人工在线。

自动接待消息会带有 `AI 代发` 标记。它只确认接收和下一步路径，不把尚未完成的匹配伪装成结果。真正的岗位介绍、简历共享和匹配仍遵循现有权限与授权边界。

网页、REST、MCP 和 CLI 使用同一组会话、消息及权限检查。私聊只对当前参与者和获得平台业务权限的工作人员开放；匹配结束、账号停用或凭据撤销后，原有连接不能继续读取或写入。发送者、平台身份和 `via_ai` 标记由服务器确定。

## 能力发现与消息格式

`GET /api/chat/capabilities` 返回公开的协议能力、传输地址、限制、欢迎语和 OpenUI 组件规范地址，不返回任何用户数据。MCP 对应 `get_chat_capabilities`。

| `format` | 内容与显示 |
| --- | --- |
| `plain` | 原样显示文本，HTML 不执行 |
| `markdown` | 默认；标题、列表、表格、任务列表、引用、链接、代码和数学公式 |
| `openui` | OpenUI Lang 源码；遵循 `/static/chat-openui.json` 的 `hireferry@1` 组件规范 |

`body` 保存原始源码；消息同时有 `status`（`streaming`、`complete`、`cancelled`）和 `sequence`。旧客户端省略 `format` 时仍按 Markdown 接收，旧记录迁移为 `complete`。普通消息使用 `client_id` 去重；流式消息从开始到结束保持同一个消息 ID。

OpenUI 使用 [Open Intelligent UI](https://github.com/thesysdev/open-intelligent-ui) 所使用的 OpenUI Lang 渲染体系。HireFerry 的组件库是有版本的受控子集；Agent 应先读取组件规范再生成内容。它不自动执行消息中的任意 JavaScript、MCP 查询或业务变更。交互操作由网页接管，将操作文本与已填写的标量表单值放入可编辑草稿，由用户确认后发送。

## REST 会话与消息

需要当前登录会话或 `Authorization: Bearer <token>`。浏览器写操作继续要求 CSRF；CLI/个人令牌沿用当前账号的业务资格检查。HumanGrant 的签名授权走 MCP，并按 `chat.read`、`chat.write` 与会话资源收缩。

| 方法与地址 | 用途 |
| --- | --- |
| `GET /api/chat/conversations` | 最近可访问会话、参与者、未读数 |
| `GET /api/chat/conversations/{id}` | 直接读取已知会话，无需先在最近列表找到 |
| `POST /api/chat/platform` | 打开平台私聊；平台管理员传 `user_id` |
| `GET /api/chat/conversations/{id}/messages` | 消息快照与分页 |
| `POST /api/chat/conversations/{id}/messages` | 一次发送完整消息 |
| `POST /api/chat/conversations/{id}/read` | 提交 `message_id`，推进自己的已读位置 |
| `GET /api/chat/events` | 按事件游标读取增量 |
| `GET /api/chat/events/stream` | SSE 实时订阅 |

普通发送示例：

```json
{
  "client_id": "application-generated-unique-id",
  "format": "markdown",
  "body": "## 面试安排\n\n- 时间：周五下午\n- 方式：线上"
}
```

消息分页支持 `after`、`before` 和 `limit`（1–100）。`after` 与 `before` 不能同时使用；不传游标时获取最近一页。响应中的 `event_cursor` 与该快照在同一个数据库事务中取得：先获取快照，再从这个事件游标订阅，可避免连接期间漏掉变化。

## 流式输入

### 有序分块接口

| 操作 | 地址 | 请求 |
| --- | --- | --- |
| 开始 | `POST /api/chat/conversations/{id}/streams` | `{ "client_id": "…", "body": "第一段", "format": "markdown" }` |
| 追加 | `POST /api/chat/conversations/{id}/streams/{message_id}/append` | `{ "sequence": 1, "delta": "下一段" }` |
| 完成 | `POST /api/chat/conversations/{id}/streams/{message_id}/finish` | `{ "sequence": 2 }` |
| 取消 | `POST /api/chat/conversations/{id}/streams/{message_id}/cancel` | `{ "sequence": 2 }` |

开始消息记录 `sequence=0`，以后每次操作加 1。相同序号、相同内容可安全重试；相同序号但不同内容返回冲突。完成与取消使用下一个序号。取消会保留部分内容和取消状态，不把未完成的答案伪装成完成。

开始返回消息快照与 `event_cursor`；追加、完成和取消返回 `{ id, conversation_id, sequence, status, event_cursor }` 小回执，不在每一轮重复返回累积正文。只有创建流的同一身份可以继续操作；有权限进入会话不等于可以修改对方的输出。

### 单连接 NDJSON 输入和增量回执

`POST /api/chat/conversations/{id}/stream` 使用 `Content-Type: application/x-ndjson`。服务器逐行处理请求体，边接收边返回 NDJSON 回执；不等整个请求体结束才发布消息。

```jsonl
{"type":"start","client_id":"agent-reply-001","format":"markdown","body":"## 初步结论"}
{"type":"delta","sequence":1,"delta":"\n\n已收到你的材料。"}
{"type":"finish","sequence":2}
```

每行成功后返回：

```json
{"type":"ack","message_id":42,"sequence":1,"status":"streaming"}
```

HTTP 状态提交前会检查身份和会话。提交响应头之后发生的序号错误、权限变化或输入超时，通过 `type: "error"` 的 NDJSON 记录返回；客户端必须检查记录，不能只看 HTTP 200。缺少 `finish`/`cancel` 不会自动完成消息。

发生断连时，保存消息 ID、`client_id` 和已确认的序号，使用相同初始内容恢复开始请求，再从最后确认序号之后继续。若恢复回执已经是 `complete` 或 `cancelled`，停止发送后续记录；只有 `streaming` 状态可以追加。也可以改用有序分块接口完成或取消。不要为网络重试生成新的消息 ID。服务器对长时间无活动的流标记取消并释放活跃流名额；当前空闲期限是 15 分钟。

当前限制：完整消息 10,000 个字符、单次增量 4,000 个字符、每条流最多 4,096 个增量序号、每个发送身份最多 4 条活跃流。NDJSON 每行最多 65,536 字节、整个上传最多 1 MiB；30 秒无输入会终止这次上传。能力发现与服务器校验是最终依据。

浏览器对流式请求体的 HTTP 版本和 `duplex` 有额外限制；网页使用分块消息操作和 SSE 接收，CLI/服务器 Agent 可以直接使用 NDJSON。代理必须关闭响应缓冲并允许长连接；不要给事件流添加整包压缩。

## 流式输出、恢复与去重

订阅示例：

```sh
curl -N "$HFY_BASE/api/chat/events/stream?conversation_id=$CONVERSATION&after=123" \
  -H "Authorization: Bearer $HFY_TOKEN" \
  -H 'Accept: text/event-stream'
```

事件包括 `message.created`、`message.started`、`message.delta`、`message.completed`、`message.cancelled`。增量事件只带变化；开始和终止事件带对应消息快照。SSE `id` 是持久化事件 ID，`sequence` 是某一条消息内的序号，两者也都不是 `message_id`。

接收端应：

1. 读取最近消息，保存该快照的 `event_cursor`。
2. 从这个游标订阅，按 `message_id` 更新同一条消息，按 `sequence` 避免重复追加。
3. 保存每次成功处理后的事件 ID。重连传 `Last-Event-ID`；该请求头优先于 URL 中旧的 `after`。
4. 接受 `cursor` 事件推进高水位，即使该区间没有自己可见的消息；心跳不会创建消息。
5. 遇到 `resync`，重新读取会话快照及其 `event_cursor`，然后恢复订阅。遇到 `error`，检查当前授权，不要持续重放已失效的凭据。

事件日志保留最近 10,000 条。`GET /api/chat/events?after=123&limit=50` 返回 `{ items, next_cursor, has_more, resync_required }`，可加 `conversation_id` 限制范围。游标过旧时明确要求重新同步；不会静默丢弃增量。读取本身不推进已读标记。

## MCP 工具

`/mcp` 保持现有 MCP HTTP JSON-RPC 连接。原生连续事件订阅由上面的 SSE 提供；MCP 客户端通过紧凑事件分页或流式消息工具操作。

| 工具 | 用途 |
| --- | --- |
| `get_chat_capabilities` | 格式、欢迎语、OpenUI 库和传输能力 |
| `list_conversations` / `get_conversation` | 最近会话 / 指定会话 |
| `open_platform_conversation` | 打开平台私聊 |
| `get_messages` | 历史消息快照和分页 |
| `send_message` | 发送完整的 plain/Markdown/OpenUI 消息 |
| `begin_message` / `append_message` | 创建一条流式消息并有序追加 |
| `finish_message` / `cancel_message` | 提交明确的终止状态 |
| `get_chat_events` | 用持久化游标读取变化，避免重复整段历史 |
| `mark_read` | 推进当前身份的已读位置 |
| `get_updates` | 保留给旧客户端的完整消息增量接口 |

流式 MCP 操作使用 `conversation_id`、`message_id`、`sequence`；开始使用 `client_id` 和首段 `body`，追加使用 `delta`。一次追加一段有意义的文本即可，不必每个 token 调用一次工具。

HumanGrant 的读取工具需要 `chat.read`，写入工具需要 `chat.write`。授权某一个 `conversation_id` 时，发现结果会把可用资源写入枚举，并要求 `get_chat_events` 显式提供该会话；只有 `resource: "self"` 可以省略会话读取当前账号的集合。能力发现仅含公开配置，写权限也可以读取它，不因此获得消息读权限。

HumanGrant 的 `send_message` / `begin_message` 也只返回身份、序号和状态回执；需要消息内容时，必须另有 `chat.read` 授权并调用读取工具。

## CLI 与 Agent 软件配置

先完成正常登录：

```sh
export HFY_SERVER=https://your-hireferry.example
hfy --scope mcp auth login
hfy chat capabilities
hfy chat list
hfy chat open
hfy chat messages platform_ACCOUNT_ID --limit 20
hfy chat send platform_ACCOUNT_ID '你好，想咨询这个岗位。' --request-id request-001
hfy chat watch --conversation-id platform_ACCOUNT_ID --after 123
```

`hfy chat watch` 输出 JSON Lines，保存每条记录的 `cursor` 可继续读取。连接会定期重建并复用现有令牌刷新机制。它不长期持有凭据文件锁，因此其他 CLI 命令仍可读取、刷新或撤销连接。`--once` 收到第一条数据事件后退出。

实时 NDJSON 输入可以直接从 Agent 的标准输出连接：

```sh
your-agent-producing-ndjson | hfy chat stream platform_ACCOUNT_ID
```

上传命令即时输出服务器回执。`stream`、`watch` 与 `mcp serve` 的 stdout 由流式协议占用，结束时不会再追加一段美化 JSON。普通 `hfy chat ... --json` 继续返回单个结构化结果。

对支持 stdio MCP 的 Agent：

```sh
hfy mcp config
```

该命令生成如下结构，并带上当前服务器、client ID 与配置目录，**不复制 access token 或 refresh token**：

```json
{
  "mcpServers": {
    "hireferry": {
      "command": "hfy",
      "args": ["--server", "https://your-hireferry.example", "mcp", "serve"]
    }
  }
}
```

`hfy mcp serve` 将一行一条的 JSON-RPC 请求转发到已认证的 `/mcp`，保留请求 ID，通知不生成伪响应；每次请求复核保存的身份并按需刷新凭据。stdout 只输出协议消息，诊断走 stderr。平台工作人员须通过 `--scope admin` 登录，成员连接不会自动提升为平台身份。


## 配置欢迎语与快捷草稿

在服务使用的 TOML 配置中设置 `[chat.welcome.<audience>]`，检查配置后重启。欢迎区只显示在尚无消息且可写的会话中，不会写成一条历史消息。归档会话继续显示归档提示。

| audience | 应用位置 |
| --- | --- |
| `platform_seeker` | 求职者视角的平台私聊 |
| `platform_employer` | 招聘方视角的平台私聊 |
| `matched` | 普通参与者视角的匹配会话 |
| `staff` | 平台工作人员视角的会话，优先于其他分类 |

例如：

```toml
[chat.welcome.platform_seeker]
title = "先聊聊你的下一份工作"
title_en = "Tell us about your next role"
body_md = "可以先介绍你的**经历、方向和期待**。"
body_md_en = "Start with your **experience, direction, and expectations**."
prompts = [
  { label = "介绍我的方向", body = "我想寻找的新机会是：", label_en = "Describe my search", body_en = "For my next role, I am looking for: " },
  { label = "讨论简历", body = "想请你帮我看看简历，讨论适合的方向。" }
]

[chat.welcome.matched]
body_md = "在这里确认岗位内容、双方期待和下一步安排。"
prompts = []
```

省略的字段保持内置文案。英文界面先用 `title_en` / `body_md_en`，缺少时回退到对应基础字段，再回退到内置英文；快捷草稿同样由 `label_en` / `body_en` 回退到 `label` / `body`。`title` 或 `body_md` 可设为空字符串隐藏对应文案。省略 `prompts` 保留内置按钮；显式 `prompts = []` 隐藏按钮。

每个提示项的 `label` 与 `body` 必须非空，最多 6 项。标题最多 160 字符、欢迎正文最多 4000 字符、按钮文字最多 80 字符、草稿正文最多 2000 字符。未知 audience、未知字段或超限配置会在启动/检查时失败。

`GET /api/config` 的 `chat.welcome` 与 `GET /api/chat/capabilities` 的 `welcome` 返回这些公开配置。服务器为 Markdown 正文生成清洗后的 `body_html` / `body_html_en`；页面按文本显示标题和按钮文字。点击提示按钮只填入可编辑草稿，由使用者决定是否发送。

生产和开发配置模板都包含可取消注释的示例。可先验证配置：

```sh
hireferry --config config.toml check-config
```
