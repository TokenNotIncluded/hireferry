# 聘渡桌面版与多平台发布

![聘渡桌面版与 CLI 发布矩阵](assets/release-matrix.svg)

**[下载矩阵](downloads.md) · [所有 Releases](https://github.com/TokenNotIncluded/hireferry/releases) · [CLI 使用指南](cli.md)**

桌面版使用 **Tauri 2 + 系统 WebView**，提供聘渡网站的独立应用窗口。默认连接 `https://hireferry.lmm.best`，也可以连接自托管站点。应用不包含聘渡服务器或数据库；业务数据仍由所选站点管理，使用业务功能需要联网。

`hfy` 是独立的用户端 CLI，不需要安装桌面版。服务器程序仍名为 `hireferry`，与桌面程序 `hireferry-desktop`、用户 CLI `hfy` 分开发布。

## 当前可安装版本

下载入口以 [自动生成的下载矩阵](downloads.md) 为准。2026-10-22 核对时，正式 Release 为 **CLI 0.1.3、服务端 0.1.0**，尚无正式桌面 Release。本次发行分支已通过六平台原生启动、15 个桌面包与两架构 Flatpak 的工程验收，但临时 CI artifact 不作为正式下载安装入口。详细证据和待验证项见 [桌面发行验收记录](desktop-release-status.md)。

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
| Arch、Manjaro、EndeavourOS | AUR 源码配方；正式桌面发布后可用二进制配方 | 当前仓库提供 `hireferry-desktop-git`；AUR 上架状态及构建命令见 [AUR 指南](aur.md) |
| Gentoo | AppImage 或 Flatpak | 图形会话、宿主 glibc 与运行时需兼容；不是所有历史版本的兼容承诺 |
| NixOS、SteamOS、不可变桌面 | Flatpak | 通过发行版配置安装 Flatpak、运行时及桌面 portal |
| Alpine / musl | CLI 用 musl 压缩包；桌面使用可用的 Flatpak 环境 | glibc `.deb`、`.rpm` 和 AppImage 不适用于原生 musl 环境 |

Tauri 2 原生 Linux 构建依赖 WebKitGTK 4.1。原生包在 **Ubuntu 22.04 的 x64 / ARM64 运行器**分别构建，避免在较新宿主上编译后意外提高 glibc 最低要求。AppImage 仍有宿主 ABI 和图形环境限制；无法把较新的 glibc 二进制变成所有发行版通用程序。参见 [Tauri Debian 打包说明](https://v2.tauri.app/distribute/debian/#limitations)与 [AppImage 限制](https://v2.tauri.app/distribute/appimage/#limitations)。

Flatpak 使用独立共享运行时提供图形依赖，通过 `.flatpak` 附件分发。**生成 Flatpak 附件不表示已上架 Flathub。** 安装运行时需要可用的软件源；部分受限或旧系统需要先配置相应服务。参考 [Flatpak 安装帮助](https://flatpak.org/setup/)及 [Tauri Flatpak 文档](https://v2.tauri.app/distribute/flatpak/)。

本仓库的 CI 验证六个平台构建、包集合、校验和与启动器加载；Linux 还在两个架构运行 Flatpak 启动检查。验收记录区分旧工作流已有的测试结果与本次新增检查，只有对应提交的工作流成功后才能计为通过。并没有把每个 Linux 发行版都记录为实机运行通过。32 位 x86、ARMv7 和 RISC-V 暂不在当前发布矩阵中。

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

## 手动更新

桌面壳当前通过新安装包更新：

1. 打开 [下载矩阵](downloads.md)，选择“桌面版”对应 Release 和当前 CPU 架构。仓库通用 `releases/latest` 保留给 CLI，不是桌面更新入口。
2. 下载该 Release 的安装包和 `SHA256SUMS`，按前面的命令校验。Windows、macOS 正式 Release 还必须通过发布工作流的原生签名与公证验证。
3. 关闭桌面应用。Windows 运行新安装器；macOS 用新应用替换 Applications 中的旧版本；Linux 使用原有包管理器安装较新的本地包。业务数据保存在所连接的站点，不在安装包内。
4. 使用 Flatpak 附件时运行 `flatpak install --user --reinstall ./新版本.flatpak`；这不会建立 Flathub 上架关系。AUR 包由相应 AUR 配方更新后使用包管理器升级；`-git` 版本需重新构建以跟随源码。

当前没有安装 Tauri updater 插件或应用内自动更新端点。`release/latest.json` 是多产品下载目录，**不是** Tauri updater 的 `version/platforms/url/signature` 更新协议；不能把它当作更新 feed。将来启用自动更新需要单独的持久签名密钥、客户端公钥以及对应的 `.sig` 产物，不能用 SHA-256 或平台代码签名替代。[Tauri updater 官方说明](https://v2.tauri.app/plugin/updater/)列出了这一要求。

## 本地构建

桌面包独立于服务器 Cargo workspace，避免普通服务器或 CLI 测试被迫安装 GTK / WebKit 开发依赖。Rust 需要 **1.90 或更新版本**，与当前锁定的 Tauri opener 依赖要求一致。请先按 [Tauri 官方前置条件](https://v2.tauri.app/start/prerequisites/)安装当前系统所需工具。

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
2. 提交源码，等待 [桌面发布工作流](../.github/workflows/desktop-release.yml) 的相关路径 PR 验证通过。PR 不接收签名 secrets，产物标记 `distribution=engineering`。手动运行可以选择 `require_signing=true` 验证真实签名，但仍不公开发布。
3. 创建与桌面版本完全一致的 `desktop-v<版本>` 标签并推送。CLI 仍可通过原有 `cli-v<版本>` 标签单独发布。
4. 各操作系统原生构建；Linux 另外生成 Flatpak。Windows / macOS 在构建前检查签名凭据，缺失时正式标签任务失败；不能把未签名结果降级当正式 Release 继续。
5. 每个原生目标对最终包生成 `desktop-signing-<target>.json`，报告绑定具体包名、SHA-256 和大小。汇总脚本拒绝缺失平台、重复包、陈旧文件、字节不符或不满足正式签名要求的报告。工程模式清单不能导入公开文档或 AUR 二进制配方。
6. 资产先传入草稿 Release，确认上传完成后公开。已公开的版本不被覆盖，修复需提升版本。
7. 正式发布后调用下载文档与 AUR 更新工作流，`hireferry-desktop-bin` 只取本次 Linux `.deb` 的真实 URL 和 SHA-256。文档按各产品稳定版本保留最新条目；分支保护不允许直接写回时，文档更新任务会失败，维护者需从已发布清单重新生成并通过 PR 合入；Release 自身的清单仍可使用。

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

正式桌面发布需要维护者实际持有的平台凭据。脚本只检查配置是否完整，随后由操作系统验签；凭据存在本身不代表签名有效。

| 平台 | 仓库 secrets / variables | 构建与发布条件 |
| --- | --- | --- |
| Windows | secrets：`WINDOWS_CERTIFICATE`（base64 PFX）、`WINDOWS_CERTIFICATE_PASSWORD`、`WINDOWS_CERTIFICATE_THUMBPRINT`；variable：`WINDOWS_TIMESTAMP_URL` | 导入 CurrentUser 证书库，校验所选证书包含私钥且未过期；Tauri 用 SHA-256 与 RFC 3161 时间戳签名；最终 `.exe` / `.msi` 必须 `Get-AuthenticodeSignature` 为 Valid 且包含时间戳证书 |
| macOS | secrets：`APPLE_CERTIFICATE`（base64 P12）、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_SIGNING_IDENTITY`（Developer ID Application） | Tauri 导入维护者证书并完成签名；同时配置下方一组公证凭据；最终 DMG 内与压缩包解出后的 `.app` 都要通过 `codesign --verify --deep --strict` 和 `xcrun stapler validate` |
| Linux | 不要求 Apple / Windows 凭据 | 记录每个最终包的 SHA-256；结果明确为 `checksum-only`，不称作发布者代码签名 |

macOS 公证凭据**二选一**：

- Apple ID：`APPLE_ID`、`APPLE_PASSWORD`（app-specific password）、`APPLE_TEAM_ID`。
- App Store Connect API：`APPLE_API_ISSUER`、`APPLE_API_KEY`、`APPLE_API_KEY_BASE64`（P8 私钥文件的 base64）。工作流仅在临时目录生成文件并设置 `APPLE_API_KEY_PATH`，结束时删除。

凭据只传入对应平台的正式标签或主动要求验签的手动任务；PR 不接收。未启用签名的工程构建在 macOS 显式使用 ad-hoc 签名，Windows 结果标记 unsigned。对应 artifact 的清单、Markdown 和图片均标明工程状态，不能自动进入下载页。

当前 Windows 接口支持维护者已有的可导出 PFX 证书。新签发 OV / EV 证书常要求硬件或云签名；这类服务需要按证书颁发方接入 Tauri 的 `signCommand`，本工作流没有假设 HSM、Azure 账户或私钥存在，也不代为创建凭据。**代码签名不保证 SmartScreen 立即建立信誉。** 参考 [Tauri Windows 签名](https://v2.tauri.app/distribute/sign/windows/)与 [macOS 签名及公证](https://v2.tauri.app/distribute/sign/macos/)。

SHA-256 用于检测下载损坏和清单不一致；不替代平台代码签名。此仓库当前没有记录一轮带维护者真实证书的正式桌面发布结果，配置预检和模拟测试通过也不会被记录为平台签名验收通过。
