# 团队成员功能验收

更新：2026-10-22。本次团队分支以主分支 `1f30f2c69f5fd4d8c5ff30be3291b320d39f9261` 为提交基线，保留并行更新的 CLI 分发、安装器及网站指南。以下是本地实际执行结果；生产部署、真实 SMTP 收件和外部 Google 登录不计入通过项。

## Rust 与配置

团队功能及关联业务回归共 **127 项通过，0 失败**，覆盖 12 个集成套件：

| 套件 | 通过数 |
| --- | ---: |
| `team_invitations` | 11 |
| `team_access` | 6 |
| `auth_flows` | 10 |
| `admin_ai` | 7 |
| `account_access` | 2 |
| `chat_flows` | 17 |
| `privacy_mcp` | 11 |
| `cli_auth` | 35 |
| `humangrant_migration` | 3 |
| `headcount_flows` | 6 |
| `currency_pricing` | 10 |
| `workbench` | 9 |

上述回归先在 `ce0145d` 基础上完成。整合 `7b43402` 后，再执行 `team_invitations`、`team_access`、`cli_distribution` 共 **19 项通过**，并重新构建最终服务端；两份配置模板 `config.development.toml` 与 `config.example.toml` 均通过 `check-config`。19 项包含重复验证的 17 项团队测试，不与 127 项直接相加。

新增后端测试实际通过邮箱验证码接口建立会话，并覆盖邀请的邮箱绑定、显式接受、CSRF、到期、撤销、重发、一次性使用及并发容量。SMTP 测试使用本地故障和阻塞连接，验证发送失败保留邀请、网络等待期间不持有数据库锁，以及发送过程中撤销后不返回可用的旧链接。没有向外部邮箱发送测试邮件。

权限回归包括真人员工、员工 AI、普通个人令牌、WebMCP、官方 CLI 和原平台 AI。员工不能管理团队、Passkey 或平台凭据，不能借客户封禁接口变更同事或待邀请账户的访问。退出登录、停用、封禁及 AI 凭据轮换后，会重新检查权限。历史消息保留内部作者，客户响应不含员工身份信息。

数据库验证涵盖 schema 9 升级、幂等执行、原有超级管理员会话保留及迁移失败回滚。Rust 格式检查和 `git diff --check` 通过。未把本次目标回归描述为整个 workspace 的完整测试；全量构建和其余测试以对应 GitHub Actions 结果为准。

## 前端

全部 14 个 `tests/*.test.mjs` 文件独立运行，**169 项通过，0 失败**，包含最终基线新增的 CLI 指南测试。新增团队测试覆盖安全回跳、权限入口、确认加入、邮件失败提示及过期响应；消息测试覆盖内部作者和客户显示边界。JavaScript 语法检查通过。

真实浏览器检查发现并修复了已登录状态冷启动时的配置请求竞态：初始配置与会话仍并行读取，等两者完成后才应用当前身份，避免身份变化误中止公共配置。新增 4 项确定性回归覆盖超级管理员/员工/普通用户的冷启动、快速切页、配置等待期间跨标签注销，以及后续身份变化仍取消私有请求。会话缓存命中时不增加登录状态请求。

桌面浅色（1440 × 1000）与手机深色（390 × 844）检查包括成员列表、邀请状态、邮件失败后复制链接、AI 凭据关闭后消失、过期邀请及员工工作台。使用静态服务和合成 API 响应的浏览器检查中，未发现横向溢出或未捕获的页面异常；邀请秘密未进入请求 URL。该部分验证页面表现，后端授权另由真实 Rust 测试验证。

## 真实本地服务与浏览器

另外使用新建的临时 SQLite 和配置启动实际 Rust 服务，由浏览器加载二进制中嵌入的前端，不替换 API 响应。7 项流程检查覆盖超级管理员创建仅链接邀请、新浏览器通过真实开发邮箱验证码登录、回到原邀请 fragment、勾选前不提交、明确加入后进入业务工作台、越权管理返回 403，以及超级管理员从页面停用后员工原会话立即失去业务权限。流程仅提交一次接受请求，未捕获页面异常为 0，邀请秘密不进入 HTTP URL。

本地现有 Chromium 在虚拟认证器注册期间发生原生进程崩溃，因此这项浏览器检查使用隔离测试库中的近期超级管理员会话种子。邀请和收件人登录走真实服务；这不证明实体 Passkey 端到端登录通过。验证码使用 development 模式，SMTP 关闭；没有使用生产凭据或发送外部邮件。临时数据库与配置在检查后删除。

## 复现与部署边界

```sh
cargo fmt --all -- --check
cargo test --locked --test team_invitations --test team_access --test cli_distribution
for suite in tests/*.test.mjs; do node "$suite" || exit 1; done
cargo build --locked -p hireferry --bin hireferry
./target/debug/hireferry --config config.development.toml check-config
./target/debug/hireferry --config config.example.toml check-config
```

部署时需备份原数据库，再启动包含 schema 9 的版本。真实邮件服务器接受、实际到达收件箱、Google OAuth 回跳和实体 Passkey 仍须在部署环境检查。SMTP 的 `sent` 表示服务器接受邮件，不表示已送达或员工已加入。操作方法与配置见 [团队成员](team.md)。
