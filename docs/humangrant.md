# HumanGrant：由本人确认的 AI 登录

聘渡实现 [HumanGrant Protocol 0.1 Draft](https://github.com/LIghtJUNction/humangrant/tree/c4a730480d2aae93a636889a144a025fe98becb8)：AI 使用自己的 Ed25519 密钥申请访问，本人登录聘渡、逐项选择权限和有效期，AI 再领取独立凭据。协议实现以该固定版本为依据，后续草案变动需重新核对兼容性。

登录页新增 **Continue with human beings** 入口。它进入使用说明或待确认的申请；本人身份仍由现有 Google／邮箱验证码流程验证。打开链接、登录成功或浏览器记住返回地址，都不会自动批准申请。管理员继续使用 Passkey，HumanGrant 不提供管理员权限。

## 使用流程

1. AI 读取本站 `GET /api/humangrant/config`，生成本地 Ed25519 密钥，并签名提交所需的动作、资源、用途和时间。
2. AI 把返回的 `human_authorization_url` 交给本人。链接指向本站 `/human/{link}`，只用于人工确认，不包含 AI 凭据。
3. 本人在网页完成登录与身份选择，核对账户、申请方指纹、用途和权限。权限默认均不勾选，可以减少权限、缩短时间，也可以拒绝。
4. 本人明确批准后，原申请密钥才能领取一次 AI 凭据。`approved` 表示已批准；领取完成后状态才变为 `succeeded`。
5. AI 用原密钥和独立凭据签名访问 `/mcp`。本人可以在工作台的 HumanGrant 授权列表撤销；后续操作会重新检查撤销、到期、账户资格和资源访问权。

求职者可免费使用 AI。招聘方沿用本站规则：已完成开通，并至少有一个履约完成、正在招聘且底薪达到该岗位记录门槛的岗位；默认人民币门槛为 ¥3,000，实际门槛与币种按现行配置及岗位快照确定。申请不能扩大账户原有权利，匹配撤回、账户封禁或资格失效同样会收回相关访问。封禁会永久撤销已有授权，恢复账户后需重新申请。

## 配置与部署

现有配置文件没有 `[humangrant]` 时使用以下默认值：

```toml
[humangrant]
enabled = true
client_ids = ["hireferry-web", "hireferry-cli"]
request_ttl_seconds = 600
max_grant_seconds = 86400
request_limit_per_hour = 12
service_limit_per_hour = 600
```

| 配置 | 含义与允许范围 |
| --- | --- |
| `enabled` | 开启入口与服务；关闭后已有 HumanGrant 凭据也不能继续使用 |
| `client_ids` | 允许的客户端标识；1–64 个不重复标识，每个最多 64 字符 |
| `request_ttl_seconds` | 待确认申请有效时间，60–3600 秒 |
| `max_grant_seconds` | AI 授权有效时间上限，60–86400 秒；未指定时采用该上限与一小时中的较小值 |
| `request_limit_per_hour` | 每个密钥身份每小时最多成功创建的申请数，1–1000 |
| `service_limit_per_hour` | 全站每小时最多成功创建的申请数，1–100000，防止轮换密钥绕过总量限制 |

这些限制及已用 nonce 保存在 SQLite，重启不会重置仍有效的记录。请求有效期与授权有效期是两个不同的期限；网页只允许在申请给定的截止时间内缩短授权。

此接入由聘渡自身处理，不需要部署第三方 HumanGrant 服务或申请 OAuth client secret。生产使用 HTTPS，`public_url` 必须是用户实际访问的准确来源；反向代理须保留请求路径和签名相关请求头，不能重新序列化请求正文。签名绑定配置的来源，不信任任意 Host／转发头来决定验签地址。服务器时钟需保持准确。

**同一 SQLite 数据库只运行一个聘渡服务实例。** 当前业务共用一个 `AppState` 数据库锁，撤销检查和业务读取／写入在该实例内按同一锁排序；这个锁不协调其他进程。不要把多个独立服务实例指向同一数据库来做负载均衡。

数据库启动时在主分支 schema 6 的基础上迁移到 schema 7，新增 `hg_requests`、`hg_grants`、`hg_sessions`、`hg_nonces`、`hg_limits`，不转换已有用户登录或 Bearer 令牌。升级前按既有备份流程备份数据库。旧版程序无法打开 schema 7；需要回退时使用升级前备份，不能手动改版本号绕过保护。

本版本采用人工转发授权链接。`POST /v1/requests/{id}/email` 返回 `503 mail_unavailable`，不会发送邮件。邮箱验证码登录仍按原 SMTP 配置工作。

## 动作与资源

每次申请包含 1–16 个不重复的 `{action, resource}`。`resource: "self"` 表示本人**当前有权访问**的集合，不表示全站数据。岗位、人才、订单、会话也可申请一个具体 ID；单一 ID 不能用于列表查询，工具发现会把对应参数限制为获准 ID。没有通配符权限。

| 动作 | MCP 工具 | 资源与影响 |
| --- | --- | --- |
| `profile.read` | `get_profile` | 仅 `self`；本人完整资料，包含登录邮箱、联系邮箱、简历、角色和账户状态 |
| `profile.write` | `update_profile`、`set_avatar_svg`、`set_image_avatar` | 仅 `self`；修改业务资料／头像，不能改登录身份或账户角色 |
| `jobs.read` | `list_jobs`、`list_my_jobs`、`get_job` | `self` 或当前可读岗位 ID；列表须 `self` |
| `jobs.write` | `create_job`、`edit_job`、`close_job` | 招聘方自己的 `self` 或岗位 ID；创建须 `self`，发布／修改仍遵守原付费流程 |
| `people.read` | `list_people`、`get_person` | 招聘方可用 `self` 查看明确匹配的人才；具体 ID 仍检查本人或有效匹配关系 |
| `orders.read` | `list_orders`、`get_order` | 本人订单集合或具体本人订单 ID |
| `chat.read` | `list_conversations`、`open_platform_conversation`、`get_messages`、`get_updates` | 当前可访问会话；集合／更新查询须 `self`；消息查询也可限具体会话 ID |
| `chat.write` | `send_message`、`mark_read` | 当前可访问会话或具体会话 ID；发送消息与标记已读，消息标明 AI 代发 |
| `matches.read` | `list_matches` | 仅 `self`；本人匹配与进展 |
| `mandate.read` | `get_mandate`、`list_outreach` | 仅求职者 `self`；读取现有委托与外部寻找记录 |

`choose_role`、`update_mandate`、`create_checkout`、令牌签发与管理员工具不在 HumanGrant 授权范围。尤其不能用一次登录申请替本人同意新的外部联系或完成付款。

写入成功不会附带返回未获准读取的旧内容。HumanGrant 的资料／头像更新与 `mark_read` 返回 `{ "updated": true }`；岗位创建／修改返回 `updated`、`job_id`、`order_id`，关闭岗位返回 `updated`、`job_id`。需要完整岗位、订单或资料时，另行取得对应读取权限。旧 Bearer 调用的返回格式保持兼容。

当前所有 HumanGrant 签名请求（包括 MCP）的原始正文上限为 **16 KiB**。JSON 与 base64 编码同样计入长度；大简历或图片应由本人使用网页上传，不能把普通 HTTP 上传接口的较大限额套用到签名请求上。服务端的工具发现是权限提示，真正检查发生在每次业务数据库访问处。

## 可运行的客户端

仓库提供 [examples/humangrant-client.mjs](../examples/humangrant-client.mjs)，使用 Node.js 22+ 的内置 `crypto`、`fetch` 和文件 API，无额外依赖。将示例域名替换成实际部署来源，私有状态文件放在仓库之外：

```sh
node examples/humangrant-client.mjs request \
  --origin https://hireferry.lmm.best \
  --state "$HOME/.config/hireferry/agent.json" \
  --permission profile.read:self \
  --permission matches.read:self \
  --purpose "读取我的资料和已确认的匹配，为我整理下一步建议"
```

将输出的 `human_authorization_url` 交给本人，等待本人确认，再执行：

```sh
node examples/humangrant-client.mjs status \
  --state "$HOME/.config/hireferry/agent.json"

node examples/humangrant-client.mjs claim \
  --state "$HOME/.config/hireferry/agent.json"

node examples/humangrant-client.mjs session \
  --state "$HOME/.config/hireferry/agent.json"

node examples/humangrant-client.mjs call \
  --state "$HOME/.config/hireferry/agent.json" \
  --tool get_profile --arguments '{}'
```

重复 `--permission` 可申请其他明确权限。客户端默认 `client_id=hireferry-cli`；省略 `--ttl` 时使用服务端默认期限，显式指定时不能超过服务端上限。本地开发可使用配置一致的 `http://localhost:3000`；非回环 HTTP 来源会被拒绝。

状态文件含私钥与领取后的凭据，客户端以 `0600` 权限原子保存并使用文件锁，凭据不输出到终端。文件绑定最初来源，后续不能把它带到另一个来源。读取结果会打印，可能包含获准的私人资料，不宜写入公开日志。

客户端不自动轮询、批准、打开浏览器、发信或重复领取。`status` 初始轮询间隔为 5 秒，过快访问会收到 `429` 及 `Retry-After`，间隔逐步增加到最多 60 秒。应遵守服务返回的间隔。领取只允许成功一次：如果成功响应丢失，不可通过再次领取恢复凭据；本人撤销旧授权，然后以新状态文件创建新申请。可用 `cancel --state ...` 取消尚未领取的申请。

## HTTP 契约与签名

| 端点 | 身份与用途 |
| --- | --- |
| `GET /api/humangrant/config` | 公开协议版本、客户端标识、动作与限额 |
| `POST /v1/requests` | AI 原密钥签名，创建申请；`target_service` 固定为 `hireferry` |
| `GET /v1/requests/{id}` | 原密钥签名，读取结构化状态 |
| `GET /v1/requests/{id}/status.md` | 原密钥签名，读取 Markdown 状态 |
| `POST /v1/requests/{id}/claim` | 原密钥签名，已批准后领取一次独立凭据 |
| `POST /v1/requests/{id}/cancel` | 原密钥签名，取消未完成申请 |
| `POST /v1/requests/{id}/email` | 本版本不可用，返回 `503` |
| `GET /v1/session` | 原密钥＋AI 凭据签名，读取当前有效授权及审计标识 |
| `POST /mcp` | 原密钥＋AI 凭据签名，仅执行当前获准工具 |
| `GET /api/humangrant/review/{link}` | 只读人工确认内容；实际登录后才有可提交的确认凭据 |
| `POST /api/humangrant/review/{link}/approve` | 真实本人 Cookie、同源 `Origin`、`X-HG-CSRF`，提交权限子集和截止时间 |
| `POST /api/humangrant/review/{link}/deny` | 同上，明确拒绝 |
| `GET /api/humangrant/grants` | 真实本人 Cookie，列出授权及用于撤销的确认凭据 |
| `DELETE /api/humangrant/grants/{id}` | 真实本人 Cookie、同源 `Origin`、授权列表的 `X-HG-CSRF`，只撤销本人授权 |

AI 请求用 `Authorization: HumanGrant anonymous` 创建／查询／领取；会话与 MCP 使用 `Authorization: HumanGrant <token>`。它不是 Bearer token，也不是浏览器 Cookie，不能拿它调用普通 `/api/*` 来绕过动作范围。

签名覆盖方法、`public_url` 加原始路径、原始正文摘要、Content-Type、公钥及 Authorization，顺序固定：

```text
("@method" "@target-uri" "content-digest" "content-type" "x-hg-key" "authorization")
```

`X-HG-Key` 为 32 字节 Ed25519 公钥的无填充 base64url；key ID 为 `ed25519:` 加公钥 SHA-256 的无填充 base64url。`Content-Digest` 使用原始正文 SHA-256 的标准 base64，`Content-Type` 必须是 `application/json`。签名输入中的算法为 `ed25519`，标签为 `humangrant-0.1`。签名生命周期最多 60 秒，最多容忍 30 秒未来时间，nonce 为 24 随机字节。

GET 正文为空，仍签名空正文摘要。服务器拒绝重复签名头、查询串、编码路径、反斜杠、双斜杠和结尾斜杠。不要手工拼接或猜测签名格式，参考客户端与 [上游公开测试向量](https://github.com/LIghtJUNction/humangrant/blob/c4a730480d2aae93a636889a144a025fe98becb8/tests/fixtures/signature.json)。

已验证的 nonce 在业务操作前独立提交，后续业务失败也不能重放该签名。凭据数据库仅保存哈希，且与原密钥、申请、授权人和客户端绑定。浏览器确认凭据与实际 Cookie、账户和申请绑定，切换账户或换一条申请不能沿用。协议／确认响应禁止缓存；审计记录 AI 身份、授权和操作名称，不记录私钥或明文凭据。

## 设计与验证入口

入口标记、按钮图标和确认插画均为本项目重新绘制，提供浅色／深色 SVG，详细尺寸和使用位置见 [素材说明](humangrant-assets.md)。

```sh
cargo test --locked
node --test tests/humangrant-ui.test.mjs
node --test tests/humangrant-client.test.mjs
```

Rust 集成测试使用真实本地邮箱挑战、Cookie 和签名，覆盖权限缩减、撤销、密钥绑定、防重放、单次领取、账户切换、到期、资源隔离和写入回执。Node 测试覆盖签名公开向量、状态文件、客户端契约与确认页面行为。验收结果与外部服务的验证边界记录在 [验收说明](acceptance.md)。
