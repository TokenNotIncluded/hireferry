# `hfy` / 聘渡命令行客户端

包名是 `hireferry-cli`，命令是 `hfy`，本版为 `0.1.0`。客户端通过 HTTP API 连接聘渡，不依赖服务端 crate、数据库或私密 TOML。`hireferry` 继续负责启动服务、检查配置、初始化管理员和备份。

服务端在已有 HumanGrant schema 7 之上，通过 schema 8 保存 CLI 授权与刷新凭据哈希。HumanGrant 与官方 CLI 使用各自的授权协议及配置；`hfy` 不领取或保存 HumanGrant 签名凭据，现有 HumanGrant 接入继续按其原有权限规则运行。

## 安装、升级与卸载


### 一键安装（推荐）

网站托管的安装脚本会检测操作系统和 x64 / ARM64 架构，从 GitHub Release 下载匹配的包并校验 SHA-256。无需 Rust、Node.js 或管理员权限。

```sh
# Linux / macOS
curl -fsSL https://hireferry.lmm.best/install.sh | sh
```

```powershell
# Windows PowerShell
irm https://hireferry.lmm.best/install.ps1 | iex
```

安装后重新打开终端，运行 `hfy --version`。Unix 默认写入 `~/.local/bin/hfy`，需要时为 Bash / Zsh / POSIX shell 的用户配置添加 PATH；Fish 用户按输出提示添加 PATH。Windows 默认写入 `%LOCALAPPDATA%\HireFerry\bin\hfy.exe`，并更新当前用户的 PATH。脚本不进行登录，也不读取凭据。

重复执行同一命令升级。`HFY_VERSION` 可固定版本（如 `0.1.0`），`HFY_INSTALL_DIR` 可指定目录，`HFY_NO_PATH=1` 跳过 PATH 设置。例如：

```sh
curl -fsSL https://hireferry.lmm.best/install.sh | HFY_VERSION=0.1.0 HFY_INSTALL_DIR="$HOME/.local/bin" sh
```

```powershell
$env:HFY_VERSION = '0.1.0'
$env:HFY_INSTALL_DIR = Join-Path $env:LOCALAPPDATA 'HireFerry\bin'
irm https://hireferry.lmm.best/install.ps1 | iex
```

脚本还支持 `HFY_RELEASE_REPO` 和 `HFY_TAG_PREFIX` 覆盖下载来源；自托管站点通过服务端 `[cli_distribution]` 配置默认来源和标签前缀。默认标签为 `cli-v<版本>`。所有资产在确定版本后使用固定标签下载，校验不匹配时保留现有程序。

卸载前用 `hfy auth logout` 退出每个站点，再删除安装目录内的 `hfy` / `hfy.exe`；从用户 PATH 或 shell 配置中移除对应目录。不要把客户端配置目录当作程序安装目录删除。

### 安装 Agent Skill（跨平台）

安装 [Node.js 22.20+](https://github.com/vercel-labs/skills)，在项目目录执行：

```sh
npx skills add TokenNotIncluded/hireferry --skill hireferry
```

按提示选择需要的助手。默认只安装到当前项目；加入 `-g` 安装到用户范围。此命令同样适用于 Windows PowerShell。技能教助手使用 `hfy` 登录、检查身份和权限，不会绕过网页确认。安装后重启或重新加载助手，再让它“使用 HireFerry 技能，通过设备码登录并查看当前身份”。

不使用 Node.js 时，可下载 Release 包，使用随包的技能目录和文末手工安装教程；Unix 可运行 `./scripts/install-skill.sh --global`。CLI 本身不依赖 Node.js。

### 自动多平台发布

`.github/workflows/cli-release.yml` 在推送 `cli-v*` 标签后为 Linux / macOS / Windows 的 x64、ARM64 分别进行原生测试、构建和安装脚本冒烟验证，全部通过后才发布 GitHub Release。Linux 使用静态 musl 包；Unix 为 `.tar.gz`，Windows 为 `.zip`。Release 提供六个包、汇总 `SHA256SUMS` 和 `VERSION`，供网站安装脚本读取。

维护者先修改 `crates/hireferry-cli/Cargo.toml` 的版本，提交并推送，再创建对应标签：

```sh
git tag cli-v<版本>
git push origin cli-v<版本>
```

`workflow_dispatch` 可只构建当前分支的六平台包；在已有 CLI 标签上运行才进入发布步骤。已发布版本不会覆盖，修改后须升级版本。Actions 成功只能证明对应 runner 的构建、测试和安装检查，不能代替用户真实设备的钥匙环或浏览器授权验收。

### 从源码安装

使用仓库指定的 Rust 工具链，在仓库根目录执行：

```sh
cargo install --path crates/hireferry-cli --locked
hfy --version
hfy --help
```

代码合并到默认分支后，可直接从 Git 安装：

```sh
cargo install --git https://github.com/TokenNotIncluded/hireferry --locked hireferry-cli
```

安装指定分支时加 `--branch <branch>`；固定版本时加 `--rev <commit>`。上面的命令从 Git 构建，不要求 crates.io 已发布此包。

更新源码后升级，或卸载通过 Cargo 安装的客户端：

```sh
cargo install --path crates/hireferry-cli --locked --force
cargo uninstall hireferry-cli
```

卸载二进制不会自动撤销登录。卸载前，对每个已连接的站点执行 `hfy --server <origin> auth logout`。如果客户端已被删除，可在网站中撤销对应 CLI 连接。

### 平台范围

| 平台 | 客户端实现 | 验证范围 |
| --- | --- | --- |
| Linux | 浏览器和设备码登录；系统凭据服务或私有文件 | 本次本地构建与自动化验证环境 |
| macOS | 浏览器和设备码登录；系统 Keychain 或私有文件 | 有条件编译路径，未在真实 macOS 上验收 |
| Windows | 浏览器和设备码登录；Windows 凭据管理器，文件后备使用系统保护 | 有条件编译路径，未在真实 Windows 上验收 |

Linux 系统凭据保存使用 `secret-tool` 连接 Secret Service，适合已有桌面钥匙环的环境。没有可用钥匙环的服务器可用私有文件。手工技能安装和打包脚本使用 Bash，可在 Linux、macOS 或具备 Bash 的环境运行；Windows 原生界面尚未实测。

仓库 CI 定义了 Linux、macOS 和 Windows 的客户端构建、测试及压缩包上传任务。是否完成以及是否有可下载的 Actions artifact，应查看对应提交的运行结果；日常 CI 任务上传 Actions artifact；另有 `CLI release` 工作流在 CLI 标签推送后发布 GitHub Release。工作流定义本身不代表构建已成功或原生钥匙环已经实测。

### 本地二进制包与校验

仓库提供本地打包脚本，不会自动发布 GitHub Release：

```sh
./scripts/package-cli.sh
```

默认构建本机目标并在 `dist/hfy/` 写入 `hireferry-cli-0.1.0-<target>.tar.gz` （Windows 目标使用 `.zip`）和 `SHA256SUMS`。压缩包包含 `hfy`（Windows 目标为 `hfy.exe`）、LICENSE、CLI 文档、技能及手工安装脚本，不包含服务配置或数据库。

也可指定目标或打包已有二进制：

```sh
./scripts/package-cli.sh --target x86_64-unknown-linux-gnu
./scripts/package-cli.sh --binary /path/to/hfy --target x86_64-unknown-linux-gnu --out-dir ./packages
```

指定其他目标仍需要对应 Rust target 与构建工具，脚本不会安装这些依赖。`--binary` 模式只打包提供的文件，调用者应确认该文件与标注的平台一致。已有同名压缩包不会被覆盖。

在输出目录校验，再解包并安装 Linux / macOS 二进制：

```sh
cd dist/hfy
sha256sum -c SHA256SUMS
# macOS 可用：shasum -a 256 -c SHA256SUMS
```

核对文件后，用 `tar -xzf <archive.tar.gz>` 解包，再把包内的 `hfy` 复制到自己的 PATH 目录。升级时替换该二进制，卸载时只删除自己安装的文件。`BUILD-INFO.txt` 标明版本、目标和可获得的源码提交，并标记打包时源码是否有未提交修改。仓库存在打包脚本不表示已有公开发布包。

## 快速开始

默认连接 `https://hireferry.lmm.best`，默认只申请身份查询权限：

```sh
hfy auth login
hfy whoami --json
hfy capabilities --json
hfy auth status --json
hfy auth logout
```

登录成功不等于有 AI 或管理员业务权限。默认的 `identity` 权限用于识别账户、查看登录状态、AI 资格和当前能力。需要业务权限时必须明确申请，并由用户在网页上确认：

```sh
hfy auth login --scope mcp
hfy auth login --scope admin
```

`mcp` 仅用于满足现有 AI 资格的普通用户。`admin` 仅用于最近通过 Passkey 验证的管理员。两者不能组合，也不能通过刷新令牌扩大原授权。

连接自己的部署时使用 `--server`，后续命令使用相同站点，或将其写入客户端配置：

```sh
hfy --server https://recruit.example auth login --device
hfy --server https://recruit.example whoami --json
hfy --server https://recruit.example auth logout
```

生产站点要求 HTTPS。本机开发可以使用回环 HTTP，例如 `http://127.0.0.1:3000`。站点地址不能带路径、查询、用户名、密码或令牌。

## 命令

| 命令 | 行为 |
| --- | --- |
| `hfy auth login` | 交互式桌面环境选浏览器登录，其他环境选设备码 |
| `hfy auth login --device` | 显示验证地址和设备码，等待网页确认 |
| `hfy auth login --oauth` | 使用系统浏览器和本机回环回调 |
| `hfy auth status` | 向服务端检查已保存连接的状态 |
| `hfy auth refresh` | 使用刷新令牌更新当前连接 |
| `hfy whoami` | 查询服务端确认的当前身份、到期时间和资格 |
| `hfy capabilities` | 查询当前连接的权限及 AI 使用资格 |
| `hfy auth logout` | 撤销当前连接并删除当前站点的本机凭据 |

`--device` 与 `--oauth` 不能同时使用。登录支持 `--no-browser`，只显示网址；`--timeout <seconds>` 限制等待人工授权的时间。例如：

```sh
hfy auth login --device --no-browser --timeout 300
```

浏览器授权的回调需要回到运行 `hfy` 的设备。SSH、远程容器或另一台设备上的浏览器应选择设备码。

## 客户端配置与优先级

配置优先级从低到高是：**内置默认值 → 客户端 `config.toml` → `HFY_*` 环境变量 → 命令参数**。这份文件属于客户端自己的配置目录，不是服务端配置。

| 字段 | 默认值 | 环境变量 | 参数 |
| --- | --- | --- | --- |
| `server` | `https://hireferry.lmm.best` | `HFY_SERVER` | `--server` |
| `client_id` | `hireferry-cli` | `HFY_CLIENT_ID` | `--client-id` |
| `scope` | `identity` | `HFY_SCOPE` | `--scope` |
| `auth_timeout_seconds` | `600` | `HFY_AUTH_TIMEOUT_SECONDS` | 登录的 `--timeout` |
| `request_timeout_seconds` | `20` | `HFY_REQUEST_TIMEOUT_SECONDS` | `--request-timeout-seconds` |
| `poll_interval_seconds` | `5` | `HFY_POLL_INTERVAL_SECONDS` | `--poll-interval-seconds` |
| `storage` | `auto` | `HFY_STORAGE` | `--storage auto\|keyring\|file` |

`HFY_CONFIG_DIR` 或 `--config-dir` 选择整个客户端配置目录；命令参数优先。它们决定读取哪一份 `config.toml`，不从该文件本身再次重定向配置目录。设备码查询还必须遵守服务端返回的最小间隔，不能用本机设置提高查询频率。

客户端配置示例：

```toml
server = "https://hireferry.lmm.best"
client_id = "hireferry-cli"
scope = "identity"
auth_timeout_seconds = 600
request_timeout_seconds = 20
poll_interval_seconds = 5
storage = "auto"
```

配置里不写访问令牌、刷新令牌、Google secret 或服务端数据库路径。公共客户端 ID 可以配置，但必须与目标站点允许的客户端 ID 一致。

## 登录、续期与权限

浏览器方式使用系统浏览器、一次性授权码、S256 PKCE、随机 `state` 和短时本机回环监听。Google 或邮箱登录仍由聘渡网站处理；客户端没有 Google secret。

设备码方式会显示短码和验证网址。用户在网页登录后核对短码、账户及权限，再选择允许或拒绝。打开链接不会直接批准请求。CLI 遵守轮询间隔，收到 `slow_down` 后延长等待，连接超时后退避；拒绝或过期后不会无限创建新请求。

访问令牌默认有效 15 分钟，刷新令牌有效期默认为首次授权起 30 天。CLI 在访问令牌接近到期时自动续期，也可运行 `hfy auth refresh`。每次续期都会换发访问令牌与刷新令牌，旧令牌失效；重复使用已消费的刷新令牌会撤销同次授权产生的整组凭据。刷新不延长原来的最长登录期限，也不扩大权限。

客户端在本机串行处理同一连接的续期，避免并发使用同一刷新令牌。如果请求已经发出但结果因断网而不确定，不盲目重试旧刷新令牌；按错误提示重新登录。过期、撤销或封禁后，旧凭据不能恢复权限。

| 申请范围 | 可做的事 | 条件 |
| --- | --- | --- |
| `identity` | 查看自己的身份、资格和能力 | 有效普通用户或管理员身份；不要求具备 AI 资格 |
| `mcp` | 身份查询及普通用户既有业务权限 | 当前满足普通用户 AI 资格；每次业务请求重新检查 |
| `admin` | 身份查询及平台管理员既有业务权限 | 明确选择该范围，并完成最近的管理员 Passkey 认证 |

`hfy capabilities --json` 返回服务端判断。不要根据账户角色或登录成功自行推断业务权限。没有 AI 资格的招聘方仍可使用 `identity` 登录查看原因；不能通过改令牌、改配置或选择其他范围跳过收费及权限规则。当前 CLI 没有付款、发消息或修改岗位等业务子命令。

## JSON、标准输出与退出状态

所有命令支持全局 `--json`。stdout 输出单个 JSON 结果；stderr 用于网址、设备码、进度和提示。访问令牌、刷新令牌、设备秘密码、Cookie 和 PKCE 验证串均不输出。

```sh
hfy auth status --json
hfy whoami --json
hfy capabilities --json
hfy auth login --device --json
```

`--json` 不会跳过浏览器登录与人工授权。登录结果在授权结束后返回。成功结果包含身份、状态、到期时间、资格和权限等可公开给当前用户的信息，以及 `storage.backend` 和 `storage.location` 表示的保存位置，不包含凭据。已有有效且涵盖申请范围的连接可直接复用；登录结果的 `reused` 表示是否复用。例如，基础登录可复用以前明确批准的 MCP 连接，结果会保留并显示实际范围；需要改为只保留身份权限时，先退出该连接，再重新登录。

例如，尚未保存连接时，`hfy auth status --json` 会返回 `authenticated: false`。已登录后的 `hfy capabilities --json` 提供 `server`、`client_id`、`scope`、`ai_eligibility` 和 `capabilities`，其中 `capabilities.identity`、`capabilities.mcp`、`capabilities.admin` 是当前服务端判定的布尔值。`whoami` 返回的身份可能包含当前用户的邮箱；向他人转发结果前应只保留任务需要的字段。

- 退出状态 `0` 表示命令成功；`auth status` 在没有保存连接时正常返回 `authenticated: false`。
- 退出状态 `1` 表示执行失败，例如身份查询时未登录或网络错误。
- 退出状态 `2` 表示参数错误。

JSON 错误包含 `error.code`、`error.message`，必要时附带 `error.details`：

```json
{"error":{"code":"error_code","message":"A safe explanation of the failure"}}
```

脚本要同时检查退出状态和结果字段。不要把服务端不可达解释成「已退出登录」，也不要把 `authenticated: false` 当成有效身份。

## 凭据保存与退出

客户端按配置目录、站点来源和公共客户端 ID 隔离凭据，并绑定经服务端确认的账户 ID；同一凭据后来返回其他账户时会拒绝使用并移除。不同目录、协议、主机、端口或客户端 ID 不共用令牌。API 请求不会跟随重定向把凭据转发到其他地址。

| 环境 | 默认配置目录 |
| --- | --- |
| Linux / macOS，设置绝对路径 `XDG_CONFIG_HOME` | `$XDG_CONFIG_HOME/hireferry` |
| Linux / macOS，未设置上述路径 | `$HOME/.config/hireferry` |
| Windows | `%LOCALAPPDATA%\hireferry` |

`storage = "auto"` 优先使用可用的系统凭据服务；不具备该服务时使用私有文件，并给出提示。`keyring` 要求系统凭据服务可用，失败不会悄悄转为文件；`file` 明确选择文件保存。已有连接的存储方式会被记录，不会因暂时打不开钥匙环而默默读取另一份旧凭据。

Linux / macOS 文件目录权限为 `700`，凭据文件为 `600`，由当前用户持有；不安全权限与符号链接会被拒绝。Windows 优先使用凭据管理器，文件后备使用绑定当前用户的系统数据保护。macOS 优先使用系统 Keychain，无法使用时可选择私有文件后备。macOS 与 Windows 的真实系统保存、读取和删除行为仍需本机验收。

用独立目录可以隔离一个项目或测试连接：

```sh
hfy --config-dir ./private-hfy auth login --device
hfy --config-dir ./private-hfy auth status --json
hfy --config-dir ./private-hfy auth logout
```

不要把该目录加入 Git、共享盘或问题附件。AI 不应读取凭据文件，也不应把其中的令牌复制给其他工具。

退出会尝试撤销服务器上的整次授权，再删除本机凭据。网络失败时，命令明确报告本机删除与远程撤销的不同结果；本机删除不表示服务端已撤销。可在对应网站的 `/oauth/connections` 连接列表中撤销仍存活的授权，包括只允许身份查询的连接。退出 CLI 不会退出已有网页会话，也不会撤销其他独立设备的授权。

## 服务器设置

服务端通过 TOML 的 `[cli_auth]` 配置授权，修改后重启生效。默认配置：

```toml
[cli_auth]
enabled = true
client_id = "hireferry-cli"
allowed_scopes = ["identity", "mcp", "admin"]
access_token_minutes = 15
refresh_token_days = 30
code_seconds = 120
grant_minutes = 10
poll_seconds = 5
max_active_tokens = 10
rate_limit_requests = 100
rate_limit_window_seconds = 300
max_user_code_attempts = 30
max_pending_grants = 1000
```

| 字段 | 含义 / 范围 |
| --- | --- |
| `enabled` | CLI 授权开关 |
| `client_id` | 1–80 个 ASCII 字母、数字、`-` 或 `_` |
| `allowed_scopes` | 允许不重复的 `identity`、`mcp`、`admin`；必须保留 `identity` |
| `access_token_minutes` | 访问令牌有效期，1–1440 分钟 |
| `refresh_token_days` | 刷新授权最长有效期，1–365 天 |
| `code_seconds` | 浏览器一次性授权码有效期，30–600 秒 |
| `grant_minutes` | 设备码请求有效期，1–30 分钟 |
| `poll_seconds` | 设备码初始轮询间隔，5–60 秒 |
| `max_active_tokens` | 每个身份可用 CLI 授权数上限，1–100 |
| `rate_limit_requests` | 指定限流窗口允许的请求数，1–10000 |
| `rate_limit_window_seconds` | 限流窗口，10–3600 秒 |
| `max_user_code_attempts` | 设备码验证尝试限制，1–1000 |
| `max_pending_grants` | 待处理授权数量上限，1–100000 |

已有凭据不会因为配置修改而自动延长到期时间。`enabled = false` 阻止新授权和令牌兑换，包括续期；已有访问令牌在到期或撤销前仍可使用原有权限，连接查询与撤销继续可用。继续沿用网站的 Google、邮箱和管理员 Passkey 设置，不需要把 Google secret 发给 CLI。

授权设计参考 [RFC 8628](https://www.rfc-editor.org/rfc/rfc8628.html)、[RFC 8252](https://www.rfc-editor.org/rfc/rfc8252.html)、[RFC 7636](https://www.rfc-editor.org/rfc/rfc7636.html)、[RFC 9700](https://www.rfc-editor.org/rfc/rfc9700.html)。接口见 [API 文档](api.md)，网站认证见 [认证文档](auth.md)。

## 可选的真实浏览器验收

`tests/cli-browser.mjs` 同时启动真实服务端、`hfy` 进程和 Chromium，使用临时数据库及客户端配置检查浏览器授权、设备码确认、拒绝、续期、退出和网页撤销。它独立于不需要浏览器依赖的 `*.test.mjs` 测试。此脚本使用本地开发邮箱流程，不代表真实 Google、SMTP、实体 Passkey 或 macOS / Windows 原生凭据保存已经验收。

在仓库根目录构建两个程序，并将 Playwright、npm 缓存及匹配的 Chromium 安装到独立临时目录：

```sh
cargo build --locked --workspace
hfy_browser_deps="$(mktemp -d)"
npm --cache "$hfy_browser_deps/npm-cache" install \
  --prefix "$hfy_browser_deps" --no-save --package-lock=false playwright
PLAYWRIGHT_BROWSERS_PATH="$hfy_browser_deps/browsers" \
  node "$hfy_browser_deps/node_modules/playwright/cli.js" install chromium
```

安装浏览器需要联网，运行环境也需要 Chromium 的系统依赖；具体要求见 [Playwright 浏览器文档](https://playwright.dev/docs/browsers)。依赖准备好后，在同一终端运行：

```sh
PLAYWRIGHT_BROWSERS_PATH="$hfy_browser_deps/browsers" \
HFY_PLAYWRIGHT_MODULE="$hfy_browser_deps/node_modules/playwright/index.mjs" \
HFY_SERVER_BIN=target/debug/hireferry \
HFY_CLI_BIN=target/debug/hfy \
  node tests/cli-browser.mjs
```

`HFY_SERVER_BIN` 和 `HFY_CLI_BIN` 可改为所需构建的绝对路径；Windows 构建使用对应 `.exe` 文件。`HFY_PLAYWRIGHT_MODULE` 指向已安装的 Playwright 模块入口，避免向项目添加 Node 依赖。已有适配浏览器时，可使用 `HFY_BROWSER_EXECUTABLE` 指定其可执行文件。脚本完成后清理其临时服务数据和子进程；依赖目录可在不再需要时自行移除。验收是否通过应以该次脚本退出状态和对应提交的记录为准。

## 安装、升级与移除 Agent Skill

技能名为 `hireferry`，兼容本版 `hfy 0.1.x`。技能先检查客户端版本、状态、当前身份和能力，不会自动申请 AI 或管理员权限。

### Skills CLI

在希望使用技能的项目中执行：

```sh
npx skills add TokenNotIncluded/hireferry --skill hireferry -a codex
```

加 `-g` 表示用户级安装，用于多个项目：

```sh
npx skills add TokenNotIncluded/hireferry --skill hireferry -a codex -g
```

从已检查的本地源码安装，避免等待默认分支包含改动：

```sh
npx skills add /path/to/hireferry --skill hireferry -a codex --copy
```

检查、升级和移除：

```sh
npx skills list -a codex
npx skills check
npx skills update hireferry -p
npx skills remove hireferry -a codex
```

用户级安装改用 `npx skills update hireferry -g` 和 `npx skills remove hireferry -a codex -g`。`update` 用于已记录的远程来源；从本地路径安装时，更新本地源码后重新执行对应的 `skills add /path/to/hireferry ...` 命令。上述语法依据 [Skills CLI 命令文档](https://github.com/vercel-labs/skills/blob/main/docs/commands.md)。当前安装器要求 Node.js 22.20 或以上，首次 `npx` 安装需要能访问 npm 和源码站点。

### 不使用 Node.js 的手工安装

仓库和本地二进制包均附带安装脚本。在目标项目目录中运行脚本的绝对路径：

```sh
/path/to/hireferry/scripts/install-skill.sh
```

默认复制到当前项目的 `.agents/skills/hireferry`。它不联网，也不安装客户端程序。已有同名目录会被拒绝；明确升级与卸载使用：

```sh
/path/to/hireferry/scripts/install-skill.sh --force
/path/to/hireferry/scripts/install-skill.sh --uninstall
```

`--force` 与 `--uninstall` 只接受该脚本创建、且来源标记匹配的目录，拒绝符号链接。不会覆盖或删除其他来源的同名技能。通过 Skills CLI 安装的技能应继续用 Skills CLI 管理。

用户级或指定目录安装：

```sh
/path/to/hireferry/scripts/install-skill.sh --global
/path/to/hireferry/scripts/install-skill.sh --dest /chosen/path/hireferry
```

未设置 `CODEX_HOME` 时，`--global` 使用 `$HOME/.agents/skills/hireferry`。显式设置 `CODEX_HOME` 时，脚本尊重该自定义环境，使用 `$CODEX_HOME/skills/hireferry`。当前 [Codex 技能文档](https://developers.openai.com/codex/skills/) 列出的默认用户发现路径是 `$HOME/.agents/skills`；使用自定义或旧版路径时，应确认该 Codex 安装会扫描它，或用 `--dest` 直接选择实际发现目录。该脚本不修改 Codex 配置，也不创建跨目录符号链接。

技能遵循 [Agent Skills 格式](https://agentskills.io/specification)。它不创建账号、不持有凭据、不代替用户点击授权，也不把 CLI 令牌交给其他 MCP 客户端。安装后可在 Codex 中选择 `$hireferry`；若未显示，检查安装目录并重启对应客户端。

实际使用示例：**「使用 $hireferry，帮我登录并查看当前账户和 AI 使用资格。」** 代理会先检查版本和已有登录，必要时把网站登录与授权交给用户，再通过 `whoami --json` 和 `capabilities --json` 汇报服务端确认的身份与资格。
