# HireFerry 文档

[项目首页](../README.md) · [English](i18n/README.en.md) · [繁體中文](i18n/README.zh-TW.md)

从产品概览进入所需专题。命令在仓库根目录执行，除非文中另有说明。

## 了解产品

| 文档 | 内容 |
| --- | --- |
| [产品与业务规则](product.md) | 服务流程、私聊与匹配、角色、委托、收费及权限 |
| [团队经理](staff.md) | 受限招聘业务角色、邮箱邀请及成员启停 |
| [业务员工与 AI 员工](team.md) | 独立员工身份、邮件邀请、业务权限及可撤销 AI 凭据 |
| [岗位管理](jobs.md) | 招聘人数、删除、一次恢复、永久删除及版本冲突 |
| [支付](payments.md) | 费用、币种、Stripe、Waffo、区域、回调及退款记录 |
| [优惠码](promo-codes.md) | 申请、审批、免费开通、发布减免与次数预占 |
| [公开政策](legal-policies.md) | 中英文条款与隐私模板、政策变量及自定义文本 |

## 使用桌面版、CLI 与 AI

| 文档 | 内容 |
| --- | --- |
| [桌面版与 CLI 下载矩阵](downloads.md) | 自动生成的系统、软件包、架构、大小和下载链接 |
| [桌面安装与发布](desktop.md) | Tauri 2、Linux 兼容范围、签名与 Release 流程 |
| [Arch Linux / AUR](aur.md) | 服务端与 CLI 分开安装、-git / -bin、systemd 初始化与自动发布 |
| [命令行客户端 `hfy`](cli.md) | 安装、登录、JSON、连接管理、升级与卸载 |
| [Agent Skill](../skills/hireferry/SKILL.md) | AI 使用 CLI 的操作约定 |
| [API 与 MCP](api.md) | HTTP、远程 MCP、WebMCP、字段、权限及分页 |
| [HumanGrant](humangrant.md) | 用户逐项授权、独立 AI 凭据、签名与撤销 |
| [HumanGrant 视觉资源](humangrant-assets.md) | 按钮、图标与确认页资源 |
| [Webhook](webhooks.md) | 事件、HTTPS 端点、验签、重试、容量及保留策略 |
| [快速反馈与增长控制](feedback-and-retention.md) | 网页和 CLI 反馈、管理员处理、容量与自动清理 |

## 自托管与开发

| 文档 | 内容 |
| --- | --- |
| [自托管与开发](self-hosting.md) | 本机启动、systemd、Docker、管理员初始化、备份与检查命令 |
| [部署交接](deployment-handoff.md) | 外部服务接入、上线核对、升级与恢复 |
| [认证与 Passkey](auth.md) | 用户登录、管理员引导、Passkey 及会话 |
| [生产配置模板](../config.example.toml) | 服务与业务配置字段 |
| [开发配置模板](../config.development.toml) | 仅回环访问的本机环境 |
| [CI 工作流](../.github/workflows/ci.yml) | Rust、网页资源和 CLI 的验证步骤 |
| [桌面发布工作流](../.github/workflows/desktop-release.yml) | 桌面版与 CLI 完整发布、校验与矩阵更新 |
| [CLI 发布工作流](../.github/workflows/cli-release.yml) | 六平台构建及版本发布 |

## 验收记录与进展

记录对应各自标注的提交。最新构建状态见 [GitHub Actions](https://github.com/TokenNotIncluded/hireferry/actions)。

| 文档 | 内容 |
| --- | --- |
| [验收记录](acceptance.md) | 测试、界面截图、构建与外部联调范围 |
| [CLI 验收](cli-validation.md) | 登录、凭据、脚本及浏览器验证 |
| [CLI 页面与浏览器缓存](browser-cache-and-cli.md) | 终端演示、复制交互、缓存更新与浏览器验证 |
| [CLI 发布说明](cli-release-notes.md) | 已记录的版本变化 |
| [页面导航验证](workspace-navigation-validation.md) | 页面切换与交互验证 |
| [聊天热路径验证](chat-hotpath-validation.md) | 聊天查询、负载与性能记录 |
| [团队功能验收](team-validation.md) | 团队权限、邀请、迁移与主分支兼容验证 |
| [剩余工作](remaining-work.md) | 已实现范围与仍需完成的验收 |

## 文档语言

| 语言 | 项目概览 |
| --- | --- |
| 简体中文 | [README](../README.md) |
| English | [README](i18n/README.en.md) |
| 繁體中文 | [README](i18n/README.zh-TW.md) |

三个概览采用相同结构。专题文档目前以简体中文为主，部分包含英文摘要。网页界面提供简体中文和英文。

补充或更新译文时，同步概览中的功能、命令与链接。共享插图位于 [`assets/readme`](assets/readme)，图中的简短英文标签由各语言正文补充说明。

## 许可与来源

[AGPL-3.0-or-later](../LICENSE) · [第三方声明](../THIRD_PARTY_NOTICES) · [粒子运行时声明](../static/runtime/NOTICE)
