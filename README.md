# 聘渡 · HireFerry

**AI 原生人才匹配平台。人类 HR 与 AI HR 负责第三方初筛和需求匹配。**

不只是发布招聘信息，我们亲自帮你找到对的人。

求职者免费，招聘方按次付费，无会员、无月费、无自动续费。

聘渡由人类 HR 与 AI HR 居中服务：求职者提交资料与求职委托，招聘方提交岗位，双方分别与平台私聊。平台建立明确匹配后，双方才可查看对应资料、岗位和联系邮箱，并进入一段新的独立会话。

这是一个面向 Linux 自托管的小型 Rust 应用：单个服务、内嵌网页资源、SQLite 数据库、纯文本 TOML 配置。生产运行无需 Node.js 服务或外部数据库。界面支持中文与英文，提供海蓝、紫罗兰、玫瑰、琥珀、青碧五种 Material 3 配色，以及浅色、深色和跟随系统三种外观；偏好保存在当前浏览器。

正式部署域名为 **https://hireferry.lmm.best**，支持邮箱为 **support@lmm.best**。生产接入与运维步骤见 [部署交接](docs/deployment-handoff.md)；本文中的正式地址不代表已完成线上部署或真实商户联调。

## 服务流程与隐私

1. 本人使用 Google 或邮箱验证码注册登录，首次选择求职者或招聘方；不要求手机号。AI 可通过 HumanGrant 发起访问申请，由本人确认权限和期限。
2. 求职者免费填写图片或安全 SVG 头像、联系邮箱和 Markdown 简历，并提交目标行业、岗位、薪资范围、地点及偏好。招聘方支付一次性开通费，再为岗位支付发布费。
3. 每个用户都有自己的平台私聊。平台管理员及其已授权 AI 可以读取业务资料、沟通需求、整理委托和推进匹配；其他用户没有全站人才或岗位浏览权限。
4. 平台把具有有效委托的求职者与已付费、招聘人数大于 0 的已发布岗位匹配。该求职者只能访问具体被介绍的岗位，招聘方只能访问被介绍的求职者。
5. 匹配产生独立的双方会话。此前各自与平台的私聊历史仍留在原会话中，不会复制给对方。平台可以在匹配会话中协调，并更新面试、录用或终止状态。

资料始终对平台可见，对无关用户保持私密。字段 `public_email` 保留兼容命名，表示供平台及匹配对象使用的联系邮箱；登录邮箱仅本人、获准读取本人资料的 AI 及平台管理员可见。旧 `visible` 字段不再授予公开访问权，没有公开简历或招聘目录。

将匹配改为 `declined`、删除岗位或封禁任一方，会撤销普通用户对相关匹配资料与会话的访问。首次删除期间也会隐藏双方匹配访问；恢复后按当前权限重新显示原匹配历史。招聘人数为 0 或单纯关闭岗位会停止新的介绍，已有匹配会话可继续。管理员保留业务历史的读取权；归档会话返回 `writable: false`，包括管理员 AI 在内都不能继续发送。

## 已实现的工作台

- 首页以同一张开放画布呈现平台、初筛匹配和收费三段介绍。滚轮与手机纵向滑动先原地切换文案和渡船、桥梁、罗盘粒子，第三段结束后再进入下方内容流；支持倒退、章节按钮、暂停和减少动态效果偏好。
- 求职者资料、本人委托、外部寻找进展、平台私聊、独立匹配会话与未读提醒。
- 招聘方多个岗位、Markdown JD、固定月薪底薪、免费不限次数修改招聘人数、一次恢复的回收站、托管支付、自己的订单与匹配对象。
- 管理员总控台：完整用户/岗位资料、封禁与内容处理、委托、以岗位为入口的候选筛选与分页、单人详情、匹配跟进列表、平台私聊和历史会话、外部进展、订单、审计及按日期/历史累计的原实收、已退款和净收入图表。正式和测试流水分开。
- 首次通过服务器命令行一次性口令绑定 Passkey，后续仅 Passkey 登录，支持多把密钥。
- 总控台「团队员工」可使用 SMTP 邮件或一次性链接邀请人类经理；经理通过独立邮箱账户处理招聘匹配和沟通，财务、封禁、密钥及员工管理保留给超级管理员，详见 [员工邀请](docs/staff.md)。
- 普通用户和平台管理员使用独立 MCP 身份；浏览器 WebMCP 绑定当前身份，AI 代发消息带标记。
- [HumanGrant](docs/humangrant.md) 提供「Continue with human beings」入口：本人逐项确认权限，AI 使用独立签名凭据，可在「AI 连接」中撤销。
- 用户可配置自己的公网 HTTPS Webhook，接收已获授权的业务事件通知，查看投递历史、暂停、撤销及重试失败投递。
- 全站及 CLI/OAuth 页面可快速提交问题与建议；管理员 CLI/MCP 可分页审阅及标记处理。反馈自动过期、限频并设置总量上限。
- 所有系统设置通过 TOML 修改并重启生效，没有系统设置页面。
- 顶栏色盘可直接点选海蓝、紫罗兰、玫瑰、琥珀或青碧配色，并单独切换浅色、深色或跟随系统。新访客默认浅色海蓝；选择保存在当前浏览器，刷新和切页后保留。颜色由 Google Material Color Utilities 生成，正文、按钮等配对颜色经过对比度检查。

### 平台匹配工作台

先从当前可招聘的岗位队列选择岗位，再按关键词、行业、目标岗位、地点和薪资范围筛选候选人。
列表使用游标分页，默认条数由 `[limits].workbench_page_size` 配置（模板为 25，单次范围 1–100）；列表只载入摘要，完整简历和单条匹配按需读取。
该岗位已有有效匹配的候选人不重复列入待选名单，已终止的历史会明确显示，恢复须再次通过原业务检查。
匹配跟进使用可搜索、按阶段筛选的列表，阶段数量随当前筛选计算，按最近更新时间排列。

搜索只向管理员及独立管理员 AI 开放。它不创建匹配，不扩大求职委托或外部联系同意，
也不让招聘方直接浏览尚未匹配的候选资料。SQLite schema 5 为这些查询建立普通索引及 FTS5 trigram 索引，schema 6 保存岗位和委托币种、岗位 AI 薪资门槛并支持零费用订单；schema 7 增加 HumanGrant 授权和防重放状态，schema 8 增加 CLI 授权与刷新凭据记录，schema 9 增加优惠申请、优惠码和次数预占，schema 10 新增有限期反馈箱，当前 schema 11 新增团队经理邀请及成员权限。
较短关键词仍采用受筛选条件约束的字面匹配。规模测试与实测结果见 [验收记录](docs/acceptance.md)，
接口和分页规则见 [工作台 API](docs/api.md#岗位中心匹配工作台)。

## 求职委托与外部寻找

求职者本人开启 `active` 委托，即允许平台围绕其需求开展站内匹配。多渠道外部联系还需要本人明确开启 `allow_external`；授权时间和是否通过 AI 提交会记录在案。求职者可暂停/关闭委托或撤回外部联系授权，管理员不能代为创建、扩大或重新激活这些同意。

平台可记录发现机会、已联系、收到回复、面试、录用与结束等进展，求职者能查看自己的记录。暂停或撤回授权后，只允许关闭已有外部进展，不能继续登记新的外部联系活动。资料不会因填入来源 URL 而被发送到外部站点。

**本应用不内置大模型、爬虫、外部招聘网站账号或自动联系服务。** 人工团队或另行部署的 AI 可以通过 MCP 读取获授权的需求、轮询消息、回复、建立匹配并记录实际进展；若要搜索其他网站或发送外部邮件，需要由其自身具备相应工具、账号和用户授权。本站保存委托与事实进展，不虚构已完成的外部求职接入。

## 收费规则

| 操作 | 默认费用 | 底薪 ¥3,000 的例子 |
| --- | --- | --- |
| 求职者注册及使用 | 免费 | ¥0 |
| 招聘方开通 | 一次性 ¥100 | ¥100 |
| 发布一个岗位，交由平台匹配 | 月薪底薪的 10% | ¥300 |
| 修改该岗位的 JD | 每次按原月薪底薪的 1% | 每次 ¥30，最多 3 次 |
| 修改招聘人数 | 免费，不限次数 | 设为 0 暂停，设正数恢复已发布岗位的招聘 |
| 删除、一次恢复、永久删除 | 免费 | 不自动退款，也不增加 JD 修改次数 |

上表为默认 CNY 模板示例，实际币种、费用和修改次数由 TOML 配置。`[pricing]` 的 `currency` 是默认币种，`allowed_currencies` 决定可选币种。发布和 JD 修改各自支持 `salary_percent`（基点费率）、`fixed`（固定金额）、`free`（免费）；开通费为零时直接开通。新增币种需显式配置它的固定价格和 AI 薪资门槛，不推算汇率。

底薪不含奖金、提成或补贴。岗位创建后标题、币种与底薪固定。金额按记录币种的整数最小单位保存；兼容字段名 `_cents` 不表示所有币种均有两位小数，例如 USD 12.34 存为 1234，JPY 1234 也存为 1234。接口返回币种和精度，求职委托保存薪资币种，薪资筛选只比较相同币种，收入按币种分别统计。

付费发布与 JD 修改在验签到账后生效；免费模式立即生效并记录零金额已完成订单，不调用网关、不写现金账本。按比例修改费以**原底薪**计算，人数不影响费用。原岗位保存创建时的币种、编辑计费方式、费率、固定费用、次数与 AI 薪资门槛；后续配置变化只影响新岗位。永久删除清除关联内容快照并保留必要账务字段。

当前 Stripe 集成支持 32 个明确实现的币种，Waffo 支持 CNY/EUR/GBP/HKD/JPY/USD 六种一次性商品币种。每个网关的 `allowed_currencies` 可进一步收窄到实际商户或商品可用的集合；未指定时采用该集成的支持集。前端按币种筛选支付方式，后端在创建订单和结账时复核。范围、精度、Waffo 商品要求及未启用的特殊币种见 [支付说明](docs/payments.md)。

支持 Stripe 与 Waffo Pancake 的真实托管收银台，网站不接收银行卡号。支付返回页不能把订单标记为成功；只有已验签回调与 SQLite 事务可落账。缺少网关凭证时保持不可支付，不启用模拟成功。具体商户配置、签名和回调要求见 [支付说明](docs/payments.md)。

两个网关统一返回 `/billing/return?order_id=<本地订单ID>&provider=<stripe或waffo>`；Stripe 取消返回额外带 `canceled=1` 提示，页面仍以当前账户的订单记录为准。Waffo 支持 `order.completed`、`refund.succeeded`、`refund.failed` 三种通知。成功退款更新订单累计退款和净收入，失败退款只记录；部分退款保留服务权益，全额开通退款撤回相应开通、全额发布退款关闭对应岗位，JD 修改退款不回滚文本或恢复修改次数。本站不主动发起退款。

## 优惠码与申请

招聘方可以在工作台的「招聘优惠」提交申请、查看审批结果，也可以输入已有优惠码。管理员在总控台的「优惠码」审核申请或直接创建优惠码。免费激活默认附带 1 次，可关闭；发布优惠次数单独设置，减免 100% 即免费发布。两组次数按整个优惠码累计，发布时会显示服务器确认的实付金额。

免费操作成功时扣次，付费优惠订单先预占次数，取消或到期后释放。完整操作、接口、额度与订单规则见 [优惠码说明](docs/promo-codes.md)。

## 招聘人数与回收站

`headcount` 是人工维护的计划招聘人数，创建时默认 1，允许 0 到 `[limits].max_job_headcount`（默认 1,000,000）。已开通且未封禁的招聘方可以免费修改自己的草稿或已发布岗位人数，不限次数，不生成订单，不改变标题、底薪、JD 版本或已用修改次数。设为 0 保持 `published`，暂停新匹配；改为正数即可恢复这类已发布岗位的招聘。匹配、面试、入职不自动扣人数，正数也不限制候选匹配总数。人数为 0 时，已有匹配仍能聊天和推进阶段。`[limits]` 同时配置每人待付款订单上限、普通页码列表的页大小和最大页码、工作台默认页大小，以及最近订单/回收站/匹配/会话/外联记录的数量上限 `recent_list_limit`（模板为 200，范围 1–1000）。

普通删除的第一次进入回收站，可免费恢复一次；恢复后再次删除就是永久删除，也可直接选择永久删除。删除与恢复请求必须带当前 `expected_version`，过期操作被拒绝，避免迟到的首次删除请求误变成永久删除。对仍处于已删除状态的普通删除重试不会增加删除次数。管理员的删除操作直接永久删除，恢复操作仍遵守同一次数与支付规则。

恢复内容不会创建新订单或要求重新付费。原已发布岗位只有当前开通及原发布付款仍有效，才恢复原发布状态和人数；权益失效时恢复为 `closed`、人数 0。原本关闭或已全额退款的岗位不能靠人数或恢复操作重获招聘权益。永久删除清空岗位标题、JD 及关联订单的内容快照，取消尚未支付的相关订单，保留金额、付款/退款和审计等必要记录；它不会自动退款。完整状态、支付竞态和接口约定见 [岗位管理](docs/jobs.md)。

## 公开政策

服务条款与隐私政策使用独立的中英 Markdown 模板，经 `/api/legal/terms`、`/api/legal/privacy` 返回清洗后的 HTML 和版本元数据。`[legal.values]` 配置运营方、版本、生效日期、退款窗口及保留期限；`[legal.document_files]` 可加载相对配置文件的完整替代文档，修改后通过 `check-config` 并重启即可生效。当前币种、费用模式、金额精度、修改次数及 AI 门槛直接由业务配置生成，不能被政策变量覆盖；已有岗位仍保留原条款。默认文本保留运营方需实际履行的承诺，配置本身不会自动执行退款、账户注销或到期清理。详见 [政策配置](docs/legal-policies.md)。

## 官方命令行客户端 `hfy`


[网站安装与技能教程](https://hireferry.lmm.best/cli) · [GitHub Releases](https://github.com/TokenNotIncluded/hireferry/releases/latest)

Linux / macOS（x64、ARM64 自动识别）：

```sh
curl -fsSL https://hireferry.lmm.best/install.sh | sh
```

Windows PowerShell（x64、ARM64 自动识别）：

```powershell
irm https://hireferry.lmm.best/install.ps1 | iex
```

脚本下载最新 CLI Release，校验 SHA-256，再安装到用户目录；无需管理员权限、Rust 或 Node.js。重新运行同一命令即可升级。Linux/macOS 默认目录为 `~/.local/bin`；Windows 为 `%LOCALAPPDATA%\HireFerry\bin`。修改用户 PATH 后重新打开终端。

技能安装适用于 Linux、macOS、Windows。先安装 [Node.js 22.20 或更高版本](https://github.com/vercel-labs/skills)，然后在项目目录运行：

```sh
npx skills add TokenNotIncluded/hireferry --skill hireferry
```

按提示选择 Codex、Claude Code、Cursor 等使用的助手；加 `-g` 可供所有项目使用。技能不会替你批准账户授权。随后运行 `hfy auth login --oauth`；远程终端用 `hfy auth login --device`。

`hireferry-cli` 是独立的 Rust 客户端包，安装后使用 `hfy` 命令。它通过 HTTPS 连接已部署的聘渡，不需要服务端数据库、TOML 配置或 Google secret。现有 `hireferry` 命令继续负责启动服务、初始化管理员和备份。

在仓库中安装客户端：

```sh
cargo install --path crates/hireferry-cli --locked
hfy auth login
hfy whoami
hfy capabilities --json
hfy auth status --json
hfy auth refresh
hfy auth logout
hfy feedback send "页面有问题" --category bug
```

更新源码后可用 `cargo install --path crates/hireferry-cli --locked --force` 升级；先退出各站点登录，再用 `cargo uninstall hireferry-cli` 卸载。需要二进制压缩包时运行 `./scripts/package-cli.sh`，脚本会生成带文档和技能的本地包及 `SHA256SUMS`，本地打包本身不发布 Release；推送 `cli-v<版本>` 标签后，`CLI release` 工作流自动为六个平台构建、测试并发布 Release。

也可直接从 Git 构建，不要求 crates.io 已发布此包：

```sh
cargo install --git https://github.com/TokenNotIncluded/hireferry --locked hireferry-cli
# 本地打包与校验；macOS 可用 shasum -a 256 -c SHA256SUMS
./scripts/package-cli.sh
cd dist/hfy
sha256sum -c SHA256SUMS
```

`hfy auth login` 在可交互的桌面环境中使用浏览器登录，其他环境使用设备码。也可以明确选择：

```sh
hfy auth login --device
hfy auth login --oauth
hfy --server https://your-hireferry.example auth login --device
```

浏览器登录通过系统浏览器、一次性授权码和 S256 PKCE 完成；设备码适合 SSH、容器或另一台设备确认。两种方式都需要在网站上登录并确认授权。默认只申请 `identity` 身份查询权限；`--scope mcp` 才申请普通用户 AI 权限并检查资格，`--scope admin` 则必须明确授权并完成最近的管理员 Passkey 认证。

访问令牌默认 15 分钟有效，刷新授权默认 30 天；客户端自动续期，每次刷新会更换令牌，重复使用旧刷新令牌会撤销整次授权。用户可在网站的 `/oauth/connections` 查看和撤销自己的 CLI 连接。CLI 不打印访问令牌或刷新令牌；`--json` 结果写入 stdout，网址、设备码和提示写入 stderr。

管理员可用 `hfy --scope admin feedback list --json` 分页查看反馈，并用 `hfy --scope admin feedback resolve <ID>` 标记处理；管理员 MCP 也提供 `admin_list_feedback` 和 `admin_update_feedback`。反馈保留、频率限制和其他短期记录清理策略见[反馈与增长控制](docs/feedback-and-retention.md)。

客户端配置按「默认值 → 客户端配置文件 → `HFY_*` 环境变量 → 命令参数」覆盖。凭据优先保存在系统钥匙环，不可用时使用受限的私有文件。Linux 是本次本地验证平台；macOS、Windows 有对应实现，尚未进行真实系统验收。配置字段、存储细节、从 Git 安装和打包校验见 [CLI 使用说明](docs/cli.md)。当前业务操作继续使用网站或已有 MCP 工具。

Linux / macOS 的配置目录为 `$XDG_CONFIG_HOME/hireferry`，未设置时为 `$HOME/.config/hireferry`；Windows 为 `%LOCALAPPDATA%\hireferry`。客户端只读取该目录中的 `config.toml`，也可用 `HFY_CONFIG_DIR` 或 `--config-dir` 指定独立目录。私有文件后备保存在该目录内；登录结果中的 `storage.backend` 和 `storage.location` 明确给出实际存储方式与位置。Unix 目录权限为 `700`、凭据文件为 `600`。

### 安装 Agent Skill

仓库附带名为 `hireferry` 的技能，指导 AI 使用上述命令，不包含账号或凭据：

```sh
npx skills add TokenNotIncluded/hireferry --skill hireferry -a codex
```

Windows PowerShell：

```powershell
npx.cmd skills add TokenNotIncluded/hireferry --skill hireferry -a codex
```

默认安装到当前项目的 `.agents/skills/hireferry`；加 `-g` 安装到用户级目录，可用于多个项目。项目技能升级使用 `npx skills update hireferry -p`，移除使用 `npx skills remove hireferry -a codex`；用户级升级和移除分别使用 `update hireferry -g` 与 `remove hireferry -a codex -g`。安装器当前要求 Node.js 22.20 或以上，见 [Skills CLI](https://github.com/vercel-labs/skills)。

不用 Node.js 时，在目标项目中运行 `/path/to/hireferry/scripts/install-skill.sh`；升级加 `--force`，移除加 `--uninstall`。脚本默认拒绝覆盖，支持 `--global`、`--dest`，并尊重显式设置的 `CODEX_HOME`。目录规则与完整命令见 [技能安装说明](docs/cli.md#安装升级与移除-agent-skill)。技能先检查 `hfy 0.1.x`、身份与能力，不会自动提升权限。

手工安装默认使用当前项目的 `.agents/skills/hireferry`；`--global` 在显式设置 `CODEX_HOME` 时使用 `$CODEX_HOME/skills/hireferry`，否则使用 `$HOME/.agents/skills/hireferry`。安装或更新后，若当前 Codex 会话尚未发现技能，检查目录并重启对应客户端。安装技能不授予任何业务操作权限，登录和授权仍由本人完成。

使用示例：**「使用 $hireferry，帮我登录并查看当前账户和 AI 使用资格。」**

## 本机运行

需要 Rust 1.99 工具链；Linux 构建依赖 `pkg-config`、OpenSSL 开发包和 C 工具链。SQLite 已随 Rust 依赖编译。Debian / Ubuntu 可先安装：

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev ca-certificates
```

从 [rustup](https://rustup.rs/) 安装 Rust 后：

```sh
cp config.development.toml config.toml
chmod 600 config.toml
cargo run --locked -- --config config.toml serve
```

打开 **http://localhost:3000**。本机开发配置关闭 SMTP，邮箱验证码会直接显示在界面中；这仅在显式 `development = true` 且绑定回环地址时可用。该模式不会虚构支付成功，也不会自动创建用户、订单或示例数据。

Google、SMTP、支付网关默认关闭。需要使用时编辑 TOML 并重启；缺少商户凭证时无法支付。所有字段见 [生产模板](config.example.toml) 和 [开发模板](config.development.toml)。

## Linux 生产部署

生产模板已使用正式域名 `https://hireferry.lmm.best`。其余商户和邮件字段必须由部署方填入已验证值；本次交付没有取得所提及的 `production.env` 或另行提供的生产交接 README 内容，未读取或推测其中的 ID、账号、商品及密钥。程序只加载指定 TOML，不自动导入 `.env`。完整清单见 [部署交接](docs/deployment-handoff.md)。

### 正式接入地址

| 用途 | 方法与完整地址 |
| --- | --- |
| Google OAuth 回调 | GET `https://hireferry.lmm.best/auth/google/callback` |
| 服务条款 | GET `https://hireferry.lmm.best/terms` |
| 隐私政策 | GET `https://hireferry.lmm.best/privacy` |
| 退款政策 | GET `https://hireferry.lmm.best/refunds` |
| 联系支持 | GET `https://hireferry.lmm.best/contact` |
| Waffo 三类支付/退款通知 | POST `https://hireferry.lmm.best/api/webhooks/waffo` |
| 统一支付返回页面 | GET `https://hireferry.lmm.best/billing/return` |

Stripe 回调为 `POST https://hireferry.lmm.best/api/webhooks/stripe`；远程 MCP 为 `POST https://hireferry.lmm.best/mcp`。首页为 `https://hireferry.lmm.best/`，管理员入口为 `/admin`。

构建并安装单个二进制。网页与粒子运行时已经嵌入，不必另起静态资源服务。

[GitHub CI](.github/workflows/ci.yml) 的预编译发布包在 **Ubuntu 24.04、x86_64** 环境构建。
其他 Linux 发行版、旧运行库或不同架构应在目标环境从源码构建，或使用下文的 Docker 构建方式；
该预编译包不承诺在任意 Linux 上直接兼容。

```sh
cargo build --locked --release
sudo useradd --system --user-group --home-dir /var/lib/hireferry --shell /usr/sbin/nologin hireferry
sudo install -m 0755 target/release/hireferry /usr/local/bin/hireferry
sudo install -d -m 0750 -o root -g hireferry /etc/hireferry
sudo install -d -m 0700 -o hireferry -g hireferry /var/lib/hireferry
sudo install -d -m 0700 -o hireferry -g hireferry /var/lib/hireferry/data
sudo install -m 0600 -o hireferry -g hireferry config.example.toml /etc/hireferry/config.toml
sudoedit /etc/hireferry/config.toml
```

保持 `public_url = "https://hireferry.lmm.best"`、`webauthn_rp_id = "hireferry.lmm.best"` 和 `development = false`；使用本机反向代理时保持 `bind = "127.0.0.1:3000"`。`public_url` 只接受来源，不附加路径、查询、fragment 或账号密码。`database` 相对于服务工作目录，模板对应 `/var/lib/hireferry/data/hireferry.sqlite3`。

配置文件权限保持 **600**，数据库与备份目录保持 **700**。配置、SMTP 密码、Google secret、支付密钥均不要提交 Git。修改配置后重启服务。先验证配置，再启用服务：

```sh
sudo -u hireferry /usr/local/bin/hireferry --config /etc/hireferry/config.toml check-config
sudo install -m 0644 deploy/hireferry.service /etc/systemd/system/hireferry.service
sudo systemctl daemon-reload
sudo systemctl enable --now hireferry
```

[Caddyfile](deploy/Caddyfile) 已配置该正式域名的 HTTPS 反向代理。安装 [Caddy](https://caddyserver.com/docs/install)，将域名 DNS 指向部署服务器，合入现有 Caddy 配置后验证并重载。公网入口为 HTTPS 443，80 用于证书与重定向；应用端口 3000 保持在回环地址。服务日志可用 `journalctl -u hireferry` 查看。`check-config` 只检查本地配置，不验证 DNS、邮件投递、Google 或支付商户可用性。

### Google 与邮箱

Google OAuth Web 应用的授权回调地址必须是：

```text
https://hireferry.lmm.best/auth/google/callback
```

Google 控制台中的应用首页为 `https://hireferry.lmm.best/`，条款和隐私 URL 分别为 `https://hireferry.lmm.best/terms`、`https://hireferry.lmm.best/privacy`，支持联系方式为 `support@lmm.best`。

在 `[google]` 填写 `client_id`、`client_secret` 并启用。Google 邮箱若与已有邮箱账户相同，不会自动合并身份；请使用原邮箱验证码登录。当前没有账户合并或 Google 绑定页面。

在 `[smtp]` 填写已验证的主机、端口、认证用户名、密码及获准发件地址 `from`，将 `reply_to = "support@lmm.best"` 后启用。验证码邮件实际包含这个 Reply-To；`from` 仍必须使用邮件服务商允许的发件身份，不能仅凭支持邮箱推测。`[site].support_email` 控制页面联系方式，`[smtp].reply_to` 控制邮件回复地址，两者分别配置。`starttls = true` 使用 STARTTLS，通常端口 587；`false` 使用隐式 TLS，通常端口 465，不是明文 SMTP。生产环境未配置 SMTP 时邮箱登录不可用。认证机制见 [认证与 Passkey](docs/auth.md)。

### 初始化管理员

在服务器交互式终端执行，命令会输出一条 **10 分钟内有效且只能使用一次**的口令：

```sh
sudo -u hireferry sh -c 'cd /var/lib/hireferry && /usr/local/bin/hireferry --config /etc/hireferry/config.toml admin-bootstrap'
```

打开 `https://hireferry.lmm.best/admin`，或在站点连续点击 Logo 5 次。输入口令后只会进入绑定首把 Passkey 的流程；服务端验证绑定成功后才能访问总控台。此后口令登录永久停用，`admin-bootstrap` 也会拒绝重新生成密码。

请在控制台登记第二把独立设备或安全密钥。最多支持 20 把；添加与删除需要最近 10 分钟内的 Passkey 认证，不允许删除最后一把。HTTPS 来源及 RP ID 与凭证绑定，部署后改变域名需要事先规划。全部密钥丢失时没有网页密码恢复或自动重置命令。

### 在线备份

服务端 `hireferry backup` 使用 SQLite 在线备份 API，可在服务运行时得到一致快照；不要只复制正在使用 WAL 的主数据库文件。

```sh
sudo install -d -m 0700 -o hireferry -g hireferry /var/lib/hireferry/backups
sudo -u hireferry sh -c 'umask 077; cd /var/lib/hireferry && /usr/local/bin/hireferry --config /etc/hireferry/config.toml backup /var/lib/hireferry/backups/hireferry-001.sqlite3'
```

每次使用新的备份文件名，连同 TOML 安全保存到服务器之外，并定期演练恢复。恢复时停止服务。数据库包含用户资料、订单、审计、会话与 Passkey 公钥；数据库备份不能代替持有 Passkey 的设备或安全密钥。

## Docker

镜像使用 Rust 1.99 构建，运行在 Debian Bookworm，包含 CA 证书及 OpenSSL 运行库。服务以 UID/GID `10001` 运行，不携带配置、密钥或示例数据库。

```sh
docker build -t hireferry:local .
mkdir -p docker-config
cp config.example.toml docker-config/config.toml
```

编辑 `docker-config/config.toml`：使用真实 HTTPS 域名，`development = false`，容器内监听 `bind = "0.0.0.0:3000"`，数据库保持 `data/hireferry.sqlite3`。设置好 Google、SMTP 与网关，再准备挂载权限：

```sh
sudo install -d -m 0700 -o 10001 -g 10001 docker-data
sudo chown 10001:10001 docker-config/config.toml
sudo chmod 600 docker-config/config.toml
docker run -d --name hireferry --restart unless-stopped \
  -p 127.0.0.1:3000:3000 \
  --mount type=bind,src="$PWD/docker-config/config.toml",dst=/app/config/config.toml,readonly \
  --mount type=bind,src="$PWD/docker-data",dst=/app/data \
  hireferry:local
```

同样使用 HTTPS 反向代理。首次管理员口令可用 `docker exec -it hireferry hireferry --config /app/config/config.toml admin-bootstrap` 获取。持久化数据在 `docker-data`，不要在升级时丢弃该目录。`development` 模式要求回环监听，不适合作为该容器映射方案的配置。

## MCP、WebMCP 与 AI 资格

### 普通用户

求职者选择身份后可免费使用 AI 连接。招聘方需要已开通，并且至少有一个**发布订单已完成、状态为 `published`、底薪达到该岗位保存的币种薪资门槛**的岗位。门槛在 `[pricing]` 和各币种金额配置中设置；默认模板为 CNY 3,000，新岗位可配置为零。免费发布的已完成订单也可满足发布条件；未支付草稿不能满足资格。旧岗位保留原门槛，每次 AI 请求仍重新检查当前状态和权限。

创建个人令牌和每一次业务 Bearer 请求都会重新检查资格，包括直接请求 HTTP API。HumanGrant 的批准、领取凭据和每次业务调用也检查当前资格。**招聘人数为 0 不影响已发布广告的 AI 资格**，可继续通过 AI 调整人数；关闭或删除最后一个符合条件的岗位后，既有招聘方 AI 令牌立即不可用，此时可用人工浏览器工作台恢复符合规则的回收站内容。WebMCP 的每次执行也检查资格，服务端对带 `X-HireFerry-AI: webmcp` 的请求再次验证。

在工作台「AI 连接」创建个人令牌，配置兼容客户端：

```text
URL: https://hireferry.lmm.best/mcp
Transport: Streamable HTTP
Authorization: Bearer <your-personal-token>
Protocol version: 2025-11-25
```

使用个人 Bearer 的普通用户有 32 个远程 MCP 工具：公开配置、本人资料/头像、岗位与招聘人数、回收站、匹配、订单/结账、委托/进展和会话。`get_site_config` 返回币种精度、可用支付方式、费用和业务限额，不返回系统密钥。能发现某工具不代表能越过角色规则；例如招聘方不能修改求职者委托。令牌只显示一次，30 天有效，每个账户最多 10 个有效令牌，可撤销。

### HumanGrant：本人确认后连接 AI

登录页新增 **Continue with human beings** 入口，说明页为 `/humangrant`。AI 使用自己的密钥发起申请，并将 `/human/{link}` 确认链接交给本人。已登录的用户直接查看申请；未登录的用户先完成原有 Google 或邮箱登录，再返回同一申请。打开链接不会自动授权。

确认页展示实际账户、AI 身份、用途、具体权限、资源和到期时间。权限默认全部不选；用户选择允许的范围并明确同意后，原申请 AI 才能单次领取独立凭据。浏览器登录 Cookie 不会交给 AI。用户可以在「AI 连接」查看和撤销授权，过期、封禁或当前业务权限变化也会限制后续操作。

HumanGrant 使用同一 `/mcp` 端点，凭据为 `Authorization: HumanGrant <access_token>`，每次请求还必须由原申请密钥签名。现有个人 Bearer 连接继续保留。HumanGrant 只授予普通用户的获批业务权限，不提供管理员访问、角色变更、结账或求职委托同意变更。配置、请求格式、签名方式和接入示例见 [HumanGrant 说明](docs/humangrant.md)。

### 平台管理员

管理员不使用 HumanGrant。管理员通过真实 Passkey 会话在控制台「平台 AI」中签发独立 `hf_admin_` 令牌。创建要求最近 10 分钟内完成 Passkey 认证并通过 CSRF；令牌 30 天有效，平台最多保留 10 个有效令牌。令牌的列出、创建、撤销只允许管理员浏览器会话，Bearer 不能自行扩增凭证。

管理员使用相同 `/mcp` 地址，发现 34 个工具，可读取公开配置、处理快速反馈，并管理全平台业务资料、岗位与恢复、委托、匹配、会话、外部进展、订单及统计。`admin_search_jobs`、`admin_search_candidates`、`admin_search_matches` 提供工作台摘要、筛选和游标分页；`admin_introduce_candidates` 将最多 100 位有效候选人加入同一岗位，整批在同一事务内校验。普通管理列表支持配置页大小，审计和匹配列表也可翻页。每项写入通过与管理页面相同的业务函数，记录具体管理员 AI 的审计身份。管理员令牌不能冒充普通用户、操作 Passkey、修改系统密钥，或把未支付订单伪造为已支付。

### 消息轮询与浏览器工具

外部 AI 可用 `get_updates` 读取增量消息，保存 `next_cursor`；若 `has_more` 为真继续分页，否则按 `poll_after_ms` 等待 5 秒。定期用 `list_conversations` 更新匹配、参与者、可写状态和未读数，用 `get_messages` 获取具体历史。发送时为同一消息重试保留相同 `client_id`，服务端据此避免重复写入；每条 AI 消息保留 `via_ai` 标记。

浏览器 WebMCP 在支持 `modelContext.registerTool` 时按登录角色注册工具，沿用当前 Cookie/CSRF。管理员和普通用户拥有不同工具集合，未登录或不具备资格时不注册；不支持该草案接口的浏览器仍能使用普通网页。WebMCP 不注册 Passkey、令牌签发或系统设置操作。

远程 MCP 为无状态 `POST /mcp` JSON-RPC；接受现有 Bearer 令牌，或独立凭据与签名齐全的 HumanGrant 请求。Cookie 不能代替这两类远程凭据。不提供第三方 MCP 客户端的 OAuth 自动接入，也没有 SSE 推送。`hfy` 的登录授权用于官方 CLI，不会自动把凭据交给其他 MCP 客户端。协议、全部 HTTP 接口、两组远程工具 schema 和 WebMCP 名称差异见 [API 文档](docs/api.md)。

## 用户 Webhook

用户在自己的工作台「Webhook」页面登记公网 HTTPS 接收地址、选择事件，并保存只显示一次的签名密钥。支持 `message.created`、`match.updated`、`job.updated`、`order.updated`、`mandate.updated`、`outreach.updated`。通知携带资源标识及状态，不附带完整简历、聊天正文或登录邮箱；需要详细资料时，由接收方使用另行授权的 API/MCP 身份读取。

端点管理仅接受本人普通浏览器 Cookie，写操作需要 CSRF；普通 Bearer、管理员身份和 WebMCP 均不能创建、读取管理页面或修改这些外发凭证。每次投递会重新检查当前权限，撤销、封禁或匹配失效不会继续发送原本排队的无权事件。接收方应验签并按事件 ID 去重；投递会重试，不是恰好一次保证。

全站启停、端点/队列上限、超时与保留期限在 TOML 的 `[webhooks]` 中配置。默认每位用户最多 5 个端点，每次投递最多 5 次尝试，成功/取消记录保留 30 天，失败保留 90 天；没有自动创建的接收地址。服务内自带后台投递任务，不需额外消息队列。精确请求、签名、SSRF 防护与重试规则见 [用户 Webhook](docs/webhooks.md)。

## 测试与交付边界

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
for asset in static/*.js static/runtime/*.js; do
  node --input-type=module --check < "$asset" || exit 1
done
node tests/home-story.test.mjs
node tests/sculpture.test.mjs
node tests/service-ui.test.mjs
node tests/humangrant-ui.test.mjs
node tests/theme.test.mjs
node tests/cli-auth.test.mjs
python3 tests/cli-scripts.test.py
cargo build --locked --release --workspace
```

上述脚本覆盖服务界面、HumanGrant 确认流程、全站外观模块、CLI 授权页面辅助逻辑、技能安装与打包。Node.js 24 用于前端检查，生产运行无需 Node。GitHub Actions 检查格式、Rust 测试、JavaScript 语法、粒子模型、服务界面与系统/浅色/深色外观测试，并构建 Linux 发布包。测试范围包括邮箱与真实 WebAuthn 签名、付费/退款事务、免费招聘人数、删除/恢复及版本竞态、角色/匹配/聊天权限、管理员令牌、委托授权、工作台分页与访问边界、撤销、重试幂等、输入清洗、Webhook 权限/投递和支付返回页。可选的 CLI 真实浏览器测试需要另行准备 Playwright 与 Chromium，运行方式见 [CLI 浏览器验收](docs/cli.md#可选的真实浏览器验收)，本次实际结果见 [CLI 本地验收记录](docs/cli-validation.md)。

大数据量工作台回归使用独立合成资料，单独运行，不包含在普通 `cargo test` 中；CI 会显式执行：

```sh
cargo test --locked --test workbench -- --ignored --nocapture
```

最新测试数量、通过状态、各轮日志、浏览器与移动端截图、GitHub CI 和发布构建证据统一见 [验收记录](docs/acceptance.md)，以对应提交的已完成记录为准。视觉 QA 使用临时独立数据库，测试账号、聊天、匹配和流水不作为默认数据随包。

真实 Google OAuth、SMTP 投递、物理 Passkey 设备、两个网关的商户付款和 Waffo 真实退款通知，仍需部署方凭证、域名和设备做端到端验收。密码学测试和软件认证器不能替代外部验收。本站不发起退款；Stripe 退款通知与争议处理尚未实现。异常到账标记 `paid_needs_review`，由管理员核对并通过商户后台处理。用户 Webhook 的真实接收站点与浏览器原生 WebMCP 执行也需单独验证；外部 AI 模型、第三方搜索与外部邮件属于独立部署和授权范围。

## English

HireFerry is a private, platform-mediated recruiting service built with Rust, Axum and SQLite. People and employers each talk privately to the HR team. There is no public resume or job directory. A platform introduction grants a specific pair access to the relevant profile and job, and opens a separate conversation; earlier private HR conversations are never copied into it. The bilingual interface supports system, light and dark appearance, with the preference saved in the browser.

Job seekers are free. TOML controls the default and selectable currencies, activation charge, publication and edit fee modes (`salary_percent`, `fixed`, `free`), edit allowance, and per-currency employer AI salary threshold. The CNY 100 activation, 10% publication, 1% edit and three-edit allowance are template defaults. Amounts are integer minor units with explicit currency precision. Titles, currencies and salaries are immutable; each job retains its original edit terms and AI threshold when configuration changes. Paid actions require verified receipts; free actions create fulfilled zero-value orders without gateway calls or cash ledger entries. Employer AI access checks current activation, publication and the job's recorded salary threshold on every request.

Planned headcount defaults to 1 and accepts integers from 0 to 1,000,000. Updates are free and unlimited: zero pauses new introductions without closing the listing, consuming a paid edit, changing fees or removing otherwise valid AI eligibility. Existing matches and conversations continue, and headcount is never decremented automatically. The first deletion can be recovered once for free; deleting again after recovery is permanent. Version checks reject stale actions. Permanent deletion clears the title, description and associated order content snapshots while preserving necessary financial records. Recovery does not create a new bill or restore refunded hiring rights. See [job management](docs/jobs.md).

The server records job-search mandates and explicit consent for external outreach. Platform staff or a separately configured AI may poll messages, reply, manage introductions and record factual progress through MCP. The application does not include an LLM, crawler, external job-board integration or automatic outreach service. Administrators can read all business history, including archived conversations, but cannot send to archived rooms.

The administrator workbench starts from a recruitable job, then searches active candidate mandates with keyword, industry, role, location and currency-aware salary filters. Lists use bounded cursor pagination and short summaries; individual resumes and introduction details load on demand. Follow-up lists support stage filters and recent-update ordering. Schema 5 added query indexes and FTS5 trigram search; schema 6 added recorded currencies, job AI thresholds and zero-value orders. Schema 7 adds HumanGrant authorization and replay protection; schema 8 adds CLI authorizations and refresh credential records; schema 9 adds approved promotion codes and reserved publication allowances; schema 10 adds bounded short-lived feedback and the current schema 11 adds manager invitations and scoped staff access. Every result checks current business eligibility. Discovery never grants candidate consent or automatically creates introductions.

One Linux binary embeds the website. Configure everything in TOML, use the supplied systemd/Caddy templates, and protect the SQLite database and configuration. The server command `hireferry backup` provides consistent online backups. CI binaries are built on Ubuntu 24.04 x86_64; use a target-environment source build or the supplied Docker build for other runtime or architecture requirements. Administrator enrollment uses a one-time shell secret followed by Passkeys only. Separate administrator AI tokens are issued from a recently authenticated Passkey session; member tokens cannot gain administrator rights. MCP `2025-11-25` exposes 32 member tools and 34 administrator tools with manual Bearer headers. It also accepts signed HumanGrant access and exposes only the permitted member tools. Browser WebMCP is feature-detected and uses the current authorized session.

HumanGrant lets a member review the AI identity, choose permissions and expiry, and revoke access later; the AI gets a separate credential bound to its own signing key. Administrator access still uses Passkeys and separate administrator tokens. Cookie sessions cannot replace remote MCP credentials.

The production origin is `https://hireferry.lmm.best`, with support at `support@lmm.best`. Both gateways return to `/billing/return`; a redirect never confirms payment. Waffo payment and refund events are verified and settled transactionally, with successful refunds deducted by their own cash-flow date. The application does not initiate refunds or process Stripe refund/dispute notifications. Member-owned outbound HTTPS webhooks provide permission-checked event notifications; only the member's signed-in browser can manage their destinations and signing secrets.

The separate `hireferry-cli` Rust package installs `hfy`. Use `hfy auth login`, or select `--device` / `--oauth`, then query `hfy whoami`, `hfy capabilities`, or `hfy auth status`. Login defaults to identity access only; member MCP and administrator access require an explicit scope and browser consent. Access tokens default to 15 minutes, with rotating refresh credentials valid for up to 30 days. Use `hfy auth refresh` or `hfy auth logout`, and manage connections at `/oauth/connections`. Add `--json` for machine-readable stdout; prompts go to stderr and credentials are never printed. Linux is locally validated; macOS and Windows paths still need native acceptance. See [the CLI guide](docs/cli.md) for configuration, storage, installation, upgrade, removal and local package checksums. Install the included Agent Skill with `npx skills add TokenNotIncluded/hireferry --skill hireferry -a codex`. On Windows PowerShell, use `npx.cmd` instead of `npx`.

Live Google, SMTP with Reply-To, physical authenticators, merchant checkout/refunds, native browser WebMCP and independently operated webhook/AI integrations require deployment-specific acceptance tests. The referenced external production handoff files were not available in this environment; no merchant IDs, products or credentials were inferred. Copy verified values into the protected TOML on the deployment machine. See [deployment handoff](docs/deployment-handoff.md), [API and MCP](docs/api.md), [acceptance](docs/acceptance.md), [authentication](docs/auth.md), [HumanGrant](docs/humangrant.md), [payments](docs/payments.md) and [member webhooks](docs/webhooks.md).

## 许可与致谢 / License and credits

[AGPL-3.0-or-later](LICENSE)。首页粒子运行时基于 [LIghtJUNction/particle-sculpture-skill](https://github.com/LIghtJUNction/particle-sculpture-skill)；保留其 [版权说明](static/runtime/NOTICE)。本站渡船、桥梁与罗盘为面向聘渡主题的程序化模型。界面视觉参考 [ix.dev](https://ix.dev/)，未复制其品牌资源。

主题颜色通过 Google 官方 [Material Color Utilities](https://github.com/material-foundation/material-color-utilities) 0.3.0 的 `SchemeVibrant` 生成强调色，共用 `SchemeMonochrome` 中性页面背景；`static/theme-palettes.json` 保存种子色和完整颜色角色，`scripts/generate-theme-palettes.mjs` 可重新生成 CSS。浏览器运行时无需加载该依赖。


招聘方开通付款支持独立的多币种定价：`pricing.onboarding_cents` 是默认币种价格，`pricing.onboarding_amounts` 配置其他币种的整数最小单位价格。付款弹窗可切换币种和 Waffo Pancake 全球／中国区域，金额实时更新；不会自动推算汇率或修改已有订单。

`payments.waffo.checkout_regions` 配置每个区域的币种和支付方式，默认全球为银行卡、Apple Pay、Google Pay，中国为微信支付、支付宝。可直接编辑 TOML 调整方式，无需改前端。`mode = "test"` 必须搭配测试密钥、测试商品和测试回调公钥；界面及 Waffo 收银台明确提示不会真实扣款。详细设置见 [付款配置](docs/payments.md#waffo-regions-and-test-mode)。
