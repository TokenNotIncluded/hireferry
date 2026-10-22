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

## 本次提交的桌面工程验收

证据：[Desktop and CLI packages，run 39827315512](https://github.com/TokenNotIncluded/hireferry/actions/runs/39827315512)，分支提交 `bb2f513dd14f9d2c16e760b65f9498d8de06fe82`，2026-10-22 全部构建与汇总成功。该记录对应桌面发行分支；随后把移动端共享 Tauri 代码合入的集成提交仍须独立运行同样工作流。

| 平台 / 架构 | 构建、单元测试及原生界面启动 | 已生成并校验的包 | 本轮签名状态 |
| --- | --- | --- | --- |
| Linux x64 | 通过，真实 WebKit 启动器 | `.deb`、`.rpm`、`.AppImage` | SHA-256 完整性校验 |
| Linux ARM64 | 通过，真实 WebKit 启动器 | `.deb`、`.rpm`、`.AppImage` | SHA-256 完整性校验 |
| Windows x64 | 通过，原生 WebView2 启动器 | NSIS `.exe`、`.msi` | unsigned，工程验证 |
| Windows ARM64 | 通过，原生 WebView2 启动器 | NSIS `.exe` | unsigned，工程验证 |
| macOS Intel x64 | 通过，原生 WebKit 启动器 | `.dmg`、`.app.tar.gz` | ad-hoc；未使用 Developer ID / 公证 |
| macOS Apple Silicon | 通过，原生 WebKit 启动器 | `.dmg`、`.app.tar.gz` | ad-hoc；未使用 Developer ID / 公证 |
| Flatpak x64 / ARM64 | 两者通过，运行时与启动器 smoke | 各一个 `.flatpak` | 单文件工程包，未上架 Flathub |

本轮实际生成 **15 个桌面包与 6 个 CLI 归档**。汇总步骤成功验证全部包名、大小、SHA-256，以及与最终包字节绑定的签名分类报告，生成 Markdown、SVG 和 JSON。macOS 最终 DMG 挂载验证已修正许可证分页与确认交互；两种架构均通过。正式发布、下载页写回和 AUR 发布 job 均按工程构建规则跳过。

验收产物保存在上述 run 的 Actions artifacts 中，清单标记 `distribution=engineering`。它们不是正式 Releases 下载入口；其有效期及下载权限受 GitHub artifact 保留策略控制。不能将 Windows unsigned、macOS ad-hoc 结果写成维护者证书签名或公证成功。

AUR 另有 [run 39827315453](https://github.com/TokenNotIncluded/hireferry/actions/runs/39827315453) 成功记录：真实 Arch `base-devel` 下的配方元数据与安装边界验证通过。本次没有向 AUR 发布包。

历史基线 [run 39735931535](https://github.com/TokenNotIncluded/hireferry/actions/runs/39735931535) 只作追溯，不能替代上述提交的结果。

## 此次新增的验证门槛

- 相关路径 PR 自动运行六平台原生构建、单元测试、各自 WebView 启动器 smoke，以及两架构 Flatpak；Windows / macOS GUI smoke 已在上述提交实际通过。
- PR 与普通手动构建标记 `distribution=engineering`；工程清单禁止自动写入公开下载页或生成 AUR 二进制配方。PR 不接收签名 secrets。
- `desktop-vX.Y.Z` 正式标签构建必须先通过凭据完整性检查，再实际验签最终包。Windows 要求有效 Authenticode 与时间戳；macOS 要求 Developer ID 签名和 stapler 验证，验证对象是最终 DMG 与 `.app.tar.gz` 内的应用。
- 每个原生目标输出包含最终包名、大小、SHA-256、签名分类的报告。汇总再次检查报告与包字节一致；发布步骤再次验证完整目录，不允许工程清单转为正式发布。
- Linux 报告明确为 `checksum-only`，不把完整性校验称作平台发布者签名。Flatpak 是单文件分发，不表示已经上架 Flathub。
- AUR 验证工作流在 Arch `base-devel` 中以普通用户执行真实 `makepkg --printsrcinfo` 与配方文件安装边界测试；桌面 source 配方与 Release 二进制配方分别处理，后者只从正式清单取两个 Linux `.deb` 的校验和。

本地签名策略测试使用模拟的原生命令结果来验证拒绝条件，不代表已经有真实签名证书。本地运行 Python 发布门槛测试；真实 makepkg 元数据、bsdtar 安装边界、六平台原生启动及最终包验证以上述 GitHub 工作流为证。维护者真实签名与公证尚未运行。

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
