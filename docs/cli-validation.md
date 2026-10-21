# hfy 0.1.0 本地验收记录

当前整合版本以主分支 `aa5e88db01ea43c5e19abc61be083c0d198c7b97` 为基础，保留其 HumanGrant schema 7、币种、费用、工作台、账户控制、页面导航与主题切换修改；官方 CLI 授权顺延为 schema 8。

整合 HumanGrant 后重新运行完整 workspace、HumanGrant 与 schema 8 迁移测试，结果记录如下。随后合入的主题更新没有修改 Rust 源码或配置；已重新检查全部前端测试，重建嵌入最新页面资源的服务端，并再次运行 35 项 CLI 授权测试。安装、升级和卸载使用独立客户端的实际 release 二进制；两次整合均未修改 CLI Rust 源码或其依赖版本。

## 已完成的本地验证

验证环境为 Linux x86_64、Rust 1.99.1、Node.js 24。所有账户、数据库、安装目录和客户端凭据均在隔离临时目录创建，未使用生产账户或修改用户已有技能。

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check`、`git diff --check` | 通过 |
| `cargo test --locked --workspace` | 整合后 228 项通过，唯一默认忽略的大数据测试另行运行 |
| `tests/cli_auth.rs` | 35 项通过，包含权限不足恢复、与 HumanGrant 的凭据及 MCP 权限隔离 |
| HumanGrant 流程与迁移测试 | 17 项流程、3 项迁移通过；版本 7 的授权与会话升级后继续有效 |
| 客户端测试 | 26 项通过：18 项单元、3 项输出、5 项会话测试 |
| `cargo test --locked --test workbench -- --ignored --nocapture` | 十万候选人、千岗位的独立查询回归通过 |
| 前端语法与 `tests/*.test.mjs` | 16 个 JavaScript 文件、HumanGrant 示例客户端语法和 10 组测试通过 |
| `python3 tests/cli-scripts.test.py` | 8 项安装、升级、卸载与打包脚本测试通过 |
| Skill 创建器格式校验 | `skills/hireferry` 验证通过 |
| 两份服务端示例配置的 `check-config` | 通过 |
| `cargo install --path crates/hireferry-cli --locked` | 在独立前缀实际构建并安装；`hfy --version` 返回 `0.1.0` |
| 源码安装的 `--force` 升级与 `cargo uninstall hireferry-cli` | 在同一独立前缀实际执行并通过 |
| `scripts/package-cli.sh --binary … --target x86_64-unknown-linux-gnu` | 实际 release 二进制打包、SHA-256 校验、解包、版本与 JSON 状态查询均通过 |
| Skills CLI 1.7.4 的本地来源安装 | Codex 项目与用户级安装、列出、内容比对、移除通过；手工安装另外覆盖 `CODEX_HOME` |

构建时依赖已缓存，因此本次 Cargo 验证附加了 `--offline`，并使用任务独立的缓存目录。没有修改现有锁文件中的依赖版本。一次完整编译遇到临时空间不足；清理本任务不再使用的重复编译缓存后，相同完整 workspace 命令成功执行并通过全部测试。

## 真实服务端、CLI 与浏览器

`tests/cli-browser.mjs` 实际启动编译后的 `hireferry`、`hfy` 和 Chromium，完成 11 个检查阶段，退出状态为 0。该验收已在包含最新主题页面资源的最终构建上重新通过。浏览器使用 Playwright 1.62.1 与本地 Chromium 133.0.6943.0；测试辅助脚本会移除浏览器包中关闭 Web 来源检查的预置参数。

- 从 OAuth 授权页进入真实邮箱验证码登录，返回原授权事务；未选择业务角色的账户可完成基础登录。
- 运行 `auth status`、`whoami`、`capabilities`，复用有效登录并执行真实刷新轮换。
- 在网页连接列表检查日期并撤销连接，确认 CLI 凭据随即失效。
- 在浏览器批准设备码，CLI 完成兑换并通过 `auth logout` 撤销。
- OAuth 和设备码分别在网页拒绝，CLI 返回明确错误并结束。
- 没有 MCP 资格时显示账户与申请范围，禁止批准，保留拒绝和切换账户；实际拒绝后 CLI 正常结束。
- 两种登录分别发送 Ctrl-C，得到结构化取消结果；OAuth 本地监听被关闭。
- 限时等待到期后正常退出，验证回调监听关闭。
- 检查所有 CLI stdout/stderr，未发现测试中实际生成的访问、刷新或设备秘密凭据泄漏；stdout 始终为单个 JSON 结果。

邮箱验证码来自服务端仅限回环开发模式的既有流程。这不是伪造登录输出，也不代表外部 SMTP 投递或 Google 账户联调完成。重跑方式见 [真实浏览器验收](cli.md#可选的真实浏览器验收)。

## 授权与权限覆盖范围

[服务端授权测试](../tests/cli_auth.rs) 覆盖错误 PKCE、回调地址和客户端，授权码重复兑换及过期，设备轮询 pending/slow_down/denied/expired、并发领取，刷新轮换、并发重放、绝对到期与全族撤销，账户封禁后恢复仍不得恢复旧凭据，连接所有权和 Cookie/CSRF 限制。schema 8 迁移测试已通过：保留 schema 7 的 HumanGrant 数据，CLI 迁移失败时回滚到有效的 schema 7。

[客户端测试](../crates/hireferry-cli/tests/)及其模块测试覆盖错误 state/issuer、跨来源重定向拒绝、服务器及客户端隔离、账户绑定、权限受限文件、并发续期、响应丢失后禁止重放旧刷新令牌、输出错误不泄漏凭据。基础身份、普通用户 MCP 和管理员授权分开校验；AI 资格、招聘方付费和岗位条件、最近 Passkey 规则继续由服务端执行。

## 仍需外部验收或发布步骤

- 生产部署、生产数据库备份与升级、线上 OAuth 配置调整均未执行。新客户端连接线上站点前，需要该站点部署包含这些端点的服务端。
- 沿用网站现有 Google、SMTP 和 Passkey 配置。CLI 是公共客户端，不需要 Google client secret，也不要求新建 Google 原生客户端。
- macOS Keychain、Windows 凭据管理器/DPAPI、Linux 桌面 Secret Service 的真实用户环境验收仍需在对应系统完成。本次真实登录使用 Linux 私有文件存储。
- CI 已增加原生平台客户端任务及浏览器回归，但本地通过不代表远程 CI 或原生钥匙环验收已经通过；本次没有反复触发或等待远程 CI。
- 没有发布 crates.io 包、GitHub Release 或生产二进制。打包脚本和校验和用于可重复的本地交付；正式发布另行执行。
- 默认分支远程 Skills 安装、检查、更新和移除在推送完成后验证；其结果由交付消息记录，与本记录中的本地来源安装分开报告。
