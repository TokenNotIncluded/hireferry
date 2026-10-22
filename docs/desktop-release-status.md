# 桌面发行验收记录

本记录于 **2026-10-22 UTC** 核对，用来区分已发布软件包、已完成的工程验证和仍需真实凭据完成的发布条件。持续更新的正式下载入口是 [下载矩阵](downloads.md)，其数据来自仓库的 `release/latest.json`。

## 当前正式产品入口

| 产品 | 核对时的正式版本 | 已发布资产 |
| --- | --- | --- |
| hfy CLI | [0.1.3](https://github.com/TokenNotIncluded/hireferry/releases/tag/cli-v0.1.3) | Windows、macOS、Linux，各 x64 / ARM64，共 6 个归档 |
| 服务端 | [0.1.0](https://github.com/TokenNotIncluded/hireferry/releases/tag/server-v0.1.0) | Linux x64 / ARM64，共 2 个归档 |
| 桌面 GUI | 尚无正式 Release | 已有构建验证产物；临时 Actions artifact 不是正式发布入口 |
| 移动端 | 尚无正式 Release | 不列出未发布的 APK、IPA 或商店下载地址 |

GitHub 仓库通用 `/releases/latest` 用于现有 CLI 安装器。桌面、服务端保持独立标签与下载入口；桌面发布使用 `--latest=false`，不会改变 CLI 的 latest。

## 已经完成的一轮桌面工程构建

证据：[Desktop and CLI packages，run 39735931535](https://github.com/TokenNotIncluded/hireferry/actions/runs/39735931535)，源码提交 `4bd3963c295e68960b00d7df51e878c000a2baea`。这是本次补齐之前的 PR 验证记录，不代表随后修改过的提交已经通过同样检查。

| 平台 / 架构 | 构建及单元测试 | 当时的原生界面启动验证 | 包格式 |
| --- | --- | --- | --- |
| Linux x64 | 通过 | WebKit 启动器通过 | `.deb`、`.rpm`、`.AppImage` |
| Linux ARM64 | 通过 | WebKit 启动器通过 | `.deb`、`.rpm`、`.AppImage` |
| Windows x64 | 通过 | 当时未运行 GUI smoke | NSIS `.exe`、`.msi` |
| Windows ARM64 | 通过 | 当时未运行 GUI smoke | NSIS `.exe` |
| macOS Intel x64 | 通过 | 当时未运行 GUI smoke | `.dmg`、`.app.tar.gz` |
| macOS Apple Silicon | 通过 | 当时未运行 GUI smoke | `.dmg`、`.app.tar.gz` |
| Flatpak x64 / ARM64 | 两者通过 | 运行时依赖、版本检查与启动器 smoke 通过 | 各一个 `.flatpak` |

该轮完整产物汇总成功，发布 job 被跳过。构建未配置维护者的 Apple / Windows 证书，因此没有被记为平台发行签名通过。CI artifact 的有效期和可下载性由 GitHub 保留策略决定，文档不把它当作永久安装地址。

## 此次新增的验证门槛

- 相关路径 PR 自动运行六平台原生构建、单元测试、各自 WebView 启动器 smoke，以及两架构 Flatpak；Windows / macOS GUI smoke 是本次新增，只有新提交的工作流成功后才能在上表新增结果。
- PR 与普通手动构建标记 `distribution=engineering`；工程清单禁止自动写入公开下载页或生成 AUR 二进制配方。PR 不接收签名 secrets。
- `desktop-vX.Y.Z` 正式标签构建必须先通过凭据完整性检查，再实际验签最终包。Windows 要求有效 Authenticode 与时间戳；macOS 要求 Developer ID 签名和 stapler 验证，验证对象是最终 DMG 与 `.app.tar.gz` 内的应用。
- 每个原生目标输出包含最终包名、大小、SHA-256、签名分类的报告。汇总再次检查报告与包字节一致；发布步骤再次验证完整目录，不允许工程清单转为正式发布。
- Linux 报告明确为 `checksum-only`，不把完整性校验称作平台发布者签名。Flatpak 是单文件分发，不表示已经上架 Flathub。
- AUR 验证工作流在 Arch `base-devel` 中以普通用户执行真实 `makepkg --printsrcinfo` 与配方文件安装边界测试；桌面 source 配方与 Release 二进制配方分别处理，后者只从正式清单取两个 Linux `.deb` 的校验和。

本地签名策略测试使用模拟的原生命令结果来验证拒绝条件，不代表已经有真实签名证书。此次环境只运行 Python 发布门槛测试、真实 makepkg 元数据生成和 bsdtar 文件安装测试；六平台运行与真实签名必须以对应 GitHub 工作流和最终包验签结果为准。

## 实际发布还需要什么

1. 本次最终提交的跨平台 PR 验证通过。此前成功 run 不能替代新提交的结果。
2. 维护者提供可用的 Windows 签名证书、时间戳服务及 Apple Developer ID / 公证凭据。当前 PFX 接口适用于可导出证书；硬件或云签名需按提供商接入，不假设已存在账户。
3. 按 [桌面发布说明](desktop.md)同步版本并触发正式标签工作流。只有完整、验签通过的产物才进入草稿、公开 Release、下载矩阵与 AUR binary 发布。
4. 若采用移动端发行，还需独立的移动签名、平台账号及安装/商店渠道验收；桌面验签不能替代这些条件。

桌面当前采用下载新包后安装的手动更新路径。应用内 updater 没有配置，下载目录 JSON 也不是 updater feed。

## 官方协议与平台依据

- [Tauri Windows 安装包](https://v2.tauri.app/distribute/windows-installer/)：NSIS / MSI 构建与 WebView2 选项。
- [Tauri Windows 签名](https://v2.tauri.app/distribute/sign/windows/)：平台签名、时间戳及硬件 / 云签名接入。
- [Tauri macOS 签名与公证](https://v2.tauri.app/distribute/sign/macos/)：Developer ID、证书与公证凭据。
- [Apple 自定义公证流程](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)：公证票据与 stapler 验证。
- [Tauri updater](https://v2.tauri.app/plugin/updater/)：独立更新签名和更新协议。
- [Tauri AUR 分发](https://v2.tauri.app/distribute/aur/)：源码与 Debian 二进制 payload 的 Arch 打包方式。
