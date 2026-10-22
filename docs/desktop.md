# 聘渡桌面版与多平台发布

![聘渡桌面版与 CLI 发布矩阵](assets/release-matrix.svg)

**[下载矩阵](downloads.md) · [所有 Releases](https://github.com/TokenNotIncluded/hireferry/releases) · [CLI 使用指南](cli.md)**

桌面版使用 **Tauri 2 + 系统 WebView**，提供聘渡网站的独立应用窗口。默认连接 `https://hireferry.lmm.best`，也可以连接自托管站点。应用不包含聘渡服务器或数据库；业务数据仍由所选站点管理，使用业务功能需要联网。

`hfy` 是独立的用户端 CLI，不需要安装桌面版。服务器程序仍名为 `hireferry`，与桌面程序 `hireferry-desktop`、用户 CLI `hfy` 分开发布。

## 选择安装包

| 系统 | 架构 | 桌面包 | 用户 CLI |
| --- | --- | --- | --- |
| Windows | x64 | NSIS `.exe`、`.msi` | `.zip` |
| Windows | ARM64 | NSIS `.exe` | `.zip` |
| macOS | Intel x64、Apple Silicon ARM64 | `.dmg`、`.app.tar.gz` | `.tar.gz` |
| Linux | x64、ARM64 | `.deb`、`.rpm`、`.AppImage`、`.flatpak` | 静态 musl `.tar.gz` |

每次完整桌面 Release 包含 **15 个桌面包和 6 个 CLI 包**。包的实际下载大小、版本、架构及 SHA-256 会由构建产物生成，见下载矩阵及 Release 附件 `downloads.json`。源码文档在首次成功发布前显示构建目标，不提前宣称已有可下载包。

优先选系统原生包，可以复用已安装的图形库和 WebView。Windows NSIS 按需下载 WebView2，不在每个安装包里嵌入完整浏览器；macOS 使用系统 WebKit。Flatpak 首次安装运行时占用的空间会大于应用包本身，但同一运行时可供多个应用共享。Tauri 的这些平台依赖见[官方前置条件](https://v2.tauri.app/start/prerequisites/)与 [Windows WebView2 安装选项](https://v2.tauri.app/distribute/windows-installer/#webview2-installation-options)。

## Linux 覆盖范围

Linux 使用多种包格式覆盖主流发行版系列，不能仅凭扩展名保证所有历史版本均能运行。

| 发行版系列 | 建议 | 条件与范围 |
| --- | --- | --- |
| Ubuntu、Debian、Mint、Pop!_OS、Zorin、elementary | `.deb` | 原生构建基线 Ubuntu 22.04；需要 WebKitGTK 4.1。Debian 12 和提供相同依赖的后续版本可安装 |
| Fedora、openSUSE、RHEL、Rocky、Alma | `.rpm`；必要时用 Flatpak | 安装器必须能满足所需库；旧企业发行版缺少 WebKitGTK 4.1 时使用 Flatpak |
| Arch、Manjaro、EndeavourOS、Gentoo | AppImage 或 Flatpak | 图形会话、宿主 glibc 与运行时需兼容；不是所有历史版本的兼容承诺 |
| NixOS、SteamOS、不可变桌面 | Flatpak | 通过发行版配置安装 Flatpak、运行时及桌面 portal |
| Alpine / musl | CLI 用 musl 压缩包；桌面使用可用的 Flatpak 环境 | glibc `.deb`、`.rpm` 和 AppImage 不适用于原生 musl 环境 |

Tauri 2 原生 Linux 构建依赖 WebKitGTK 4.1。原生包在 **Ubuntu 22.04 的 x64 / ARM64 运行器**分别构建，避免在较新宿主上编译后意外提高 glibc 最低要求。AppImage 仍有宿主 ABI 和图形环境限制；无法把较新的 glibc 二进制变成所有发行版通用程序。参见 [Tauri Debian 打包说明](https://v2.tauri.app/distribute/debian/#limitations)与 [AppImage 限制](https://v2.tauri.app/distribute/appimage/#limitations)。

Flatpak 使用独立共享运行时提供图形依赖，通过 `.flatpak` 附件分发。**生成 Flatpak 附件不表示已上架 Flathub。** 安装运行时需要可用的软件源；部分受限或旧系统需要先配置相应服务。参考 [Flatpak 安装帮助](https://flatpak.org/setup/)及 [Tauri Flatpak 文档](https://v2.tauri.app/distribute/flatpak/)。

本仓库的 CI 验证构建、打包、包集合和校验信息；没有把每个发行版都实际运行过的结果记录为通过。32 位 x86、ARMv7 和 RISC-V 暂不在当前发布矩阵中。

## 安装与校验

从同一个 Release 下载安装包及 `SHA256SUMS`，不要混用不同版本的校验文件。

```sh
# Linux：下载的那个文件显示 OK 才继续安装
sha256sum --ignore-missing --check SHA256SUMS
```

```sh
# macOS：本机只下载了部分附件时，其他条目提示缺失是正常的
shasum -a 256 -c SHA256SUMS
```

```powershell
# Windows：把结果与 SHA256SUMS 中对应文件名的值比较
Get-FileHash .\hireferry-desktop-*.exe -Algorithm SHA256
```

安装命令中的文件名请替换为下载矩阵给出的实际名称，并只选与 CPU 对应的包。

```sh
# Debian / Ubuntu 家族
sudo apt install ./hireferry-desktop-<version>-<target>.deb

# Fedora / RHEL 家族
sudo dnf install ./hireferry-desktop-<version>-<target>.rpm

# openSUSE
sudo zypper install ./hireferry-desktop-<version>-<target>.rpm

# AppImage
chmod +x ./hireferry-desktop-<version>-<target>.AppImage
./hireferry-desktop-<version>-<target>.AppImage

# Flatpak：先按发行版指南配置 Flatpak 和 Flathub 运行时源
flatpak install --user ./hireferry-desktop-<version>-<target>.flatpak
flatpak run best.lmm.hireferry
```

AppImage 需要的 FUSE 配置因发行版而异；不能挂载时可尝试 AppImage 的 `--appimage-extract-and-run`。Flatpak 在没有运行时源时，需要先添加发行版推荐的软件源。Windows 双击 NSIS / MSI；macOS 打开 DMG 后把应用拖进 Applications。

CLI 的一键安装、登录、升级与卸载见 [CLI 文档](cli.md)。它继续使用独立的 `cli-v<版本>` 发布通道。

## 连接站点与登录

启动器接受 HTTPS 站点的**来源地址**，例如 `https://hireferry.example`；本机开发可用 `http://localhost:3000` 或回环 IP。用户名密码、查询参数和业务路径不能写进站点地址。

远程业务窗口不获得本地文件、Shell 或 Tauri 命令权限。外部站点链接交给系统浏览器；本地启动器仅获得连接站点所需的最小权限。

桌面窗口内可使用站点提供的邮箱验证码登录。Google 等第三方提供方可能拒绝嵌入式 WebView；系统浏览器与桌面窗口的会话也不自动共享。此版本没有加入浏览器 OAuth 会话回传桥接，使用此类登录时应在浏览器里继续访问站点。不要把“浏览器成功登录”误认为桌面窗口也已登录。

桌面壳的更新通过下载新版本安装包完成，Flatpak 通过重新安装新附件更新。这里的“自动更新”指**发布文档和下载矩阵自动生成**，并没有配置未签名的应用内自动更新器。

## 本地构建

桌面包独立于服务器 Cargo workspace，避免普通服务器或 CLI 测试被迫安装 GTK / WebKit 开发依赖。请先按 [Tauri 官方前置条件](https://v2.tauri.app/start/prerequisites/)安装当前系统所需工具。

```sh
cd desktop
npm ci
npm run tauri -- dev
npm run tauri -- build
```

生成的原始包在桌面 crate 的 `target/.../release/bundle/`。统一发布文件名由 `scripts/release.py stage-desktop` 生成；GitHub CI 使用显式 Rust target，因此资产包含架构而不会互相覆盖。

## CI 与版本发布

发布目标由 [`release/targets.json`](../release/targets.json) 定义。CI 矩阵、预期包集合、Markdown、JSON 和 SVG 都读取这个目录，新增平台时不需要分别维护多张表。

1. 在 `desktop/src-tauri/Cargo.toml`、`desktop/src-tauri/tauri.conf.json` 和 `desktop/package.json` 中保持桌面版本一致；更新相应锁文件。
2. 提交源码，等待 [桌面发布工作流](../.github/workflows/desktop-release.yml) 的验证构建通过。手动运行可验证构建，不会把分支名称当作正式版本发布。
3. 创建与桌面版本完全一致的 `desktop-v<版本>` 标签并推送。CLI 仍可通过原有 `cli-v<版本>` 标签单独发布。
4. 各操作系统原生构建；Linux 另外生成 Flatpak。发布任务只在全部包成功后运行。
5. 汇总脚本拒绝缺失平台、重复包、额外陈旧文件或 SHA-256 不匹配，生成完整的 `downloads.json`、`downloads.md`、`release-matrix.svg`、`VERSION` 和 `SHA256SUMS`。
6. 资产先传入草稿 Release，确认上传完成后公开。已公开的版本不被覆盖，修复需提升版本。
7. 桌面版和独立 CLI Release 均从已发布清单自动更新下载文档，按各自稳定版本保留最新条目；仓库分支保护可能要求通过 PR 接受文档变更，Release 自身仍包含准确的矩阵与完整清单。

桌面 Release 使用 `--latest=false`，保留现有 CLI 一键安装器依赖的 GitHub `latest` 通道。用户从[下载矩阵](downloads.md)进入桌面 Release；不要用仓库通用 `/releases/latest` 代替桌面版下载入口。

### 校验和再生成

```sh
python3 scripts/release.py matrix desktop
python3 scripts/release.py matrix cli
python3 scripts/release.py matrix flatpak
# CLI 独立发布也生成经过验证的 JSON / Markdown / SVG
python3 scripts/release.py assemble-cli artifacts/cli-release dist/cli-release --tag cli-v0.1.2
python3 scripts/release.py docs
python3 scripts/release.py docs --check
python3 tests/release.test.py
```

发布后把经过验证的 `downloads.json` 传给文档生成器：

```sh
python3 scripts/release.py docs --manifest /path/to/downloads.json
```

该命令同时保存 `release/latest.json` 快照，包含发布时固定的目标目录、包清单和仓库地址。`docs --check` 按这个已发布目录验证并生成下载表；`matrix` 命令独立验证当前 `release/targets.json`。因此可以先提交新增架构或包格式的构建配置，已发布下载表仍保留原来真实存在的包；每个产品的稳定 Release 成功后自动合并其条目，无需删除或篡改历史发布记录。独立 CLI 发布不会移除桌面下载；较新的桌面包即使附带较旧 CLI，也不会把 CLI 行降级。清单保留各产品原始标签、下载地址与 `source_commits` 来源提交。

### 签名与发布凭据

Release 上传使用 GitHub Actions 自带的 `GITHUB_TOKEN`。最终发布任务和下载文档写回任务获得 `contents: write`；一般构建任务只读仓库。

初始工作流不导入签名证书，也不配置代码签名 secrets。Windows 包未签名；macOS 仅采用构建工具默认的本地签名方式，不包含 Apple Developer ID 签名或公证。系统可能提示来源未验证，生成的包也不等于已完成商店审核。维护者可在获得自己的证书与账户凭据后添加签名步骤。配置 Apple 签名及公证方式见 [Tauri macOS 签名](https://v2.tauri.app/distribute/sign/macos/)，Windows 见 [Tauri Windows 签名](https://v2.tauri.app/distribute/sign/windows/)。

SHA-256 用于检测下载损坏和包清单不一致；它不替代平台代码签名。应用内更新签名另有独立密钥要求，当前流程不配置自动执行更新。
