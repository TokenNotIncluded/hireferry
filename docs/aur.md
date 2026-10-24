# Arch Linux / AUR

[下载矩阵](downloads.md) · [CLI](cli.md) · [自托管](self-hosting.md)

服务端、客户端 CLI 与桌面 GUI 独立打包，都面向 x86_64 与 aarch64：

| 组件 | 从 main 编译 | 安装正式 Release 二进制 |
| --- | --- | --- |
| 服务端 `hireferry` | `hireferry-server-git` | `hireferry-server-bin` |
| 客户端 `hfy` | `hireferry-cli-git` | `hireferry-cli-bin` |
| 桌面 GUI `hireferry-desktop` | 仓库已提供 `hireferry-desktop-git` 配方 | 正式桌面 Release 成功后生成 `hireferry-desktop-bin` |

同一组件的 `-git` 和 `-bin` 提供相同命令，因此互斥。不同组件使用独立的可执行文件与文档目录，可以同时安装，也可以混用不同变体。安装 CLI 或桌面 GUI 不会安装、编译或启动服务端。桌面 GUI 连接已有站点。

**桌面 AUR 当前状态：** 此次变更新增配方与发布流程，尚未把“仓库中有 PKGBUILD”视为 AUR 已上架。2026-10-22 核对时没有正式桌面 Release，因此不提交占位的 `-bin` URL 或校验和。main 合入后 AUR 工作流才能更新源码配方；`-bin` 必须等待完整、通过签名门槛的桌面 Release。实际 AUR 发布成功后才能使用对应包名安装。

```sh
# 连接现有站点，只需要 CLI
yay -S hireferry-cli-bin
hfy auth login

# 自托管服务端
yay -S hireferry-server-bin

# 开发版：各自只编译对应 Cargo package
yay -S hireferry-cli-git
yay -S hireferry-server-git
```

也可克隆对应 AUR 仓库，用 `makepkg -si` 构建安装。`-git` 使用锁定依赖，版本由 Cargo 版本、提交数和提交 ID 组成；`-bin` 固定版本和两个架构各自的 SHA-256，不在安装时查询 latest，也不跳过二进制校验。

## 桌面源码配方与二进制配方

源码配方位于 [`packaging/aur/hireferry-desktop-git`](../packaging/aur/hireferry-desktop-git/PKGBUILD)。在 Arch 系统上复制该目录，检查配方后运行 `makepkg --cleanbuild -si`。它使用独立的 `desktop/src-tauri/Cargo.toml`、锁定依赖与 `custom-protocol` feature，仅构建 `hireferry-desktop`，不要求 Node 构建步骤或服务器 Cargo workspace。Rust 需至少 1.90；运行依赖包含 GTK 3、WebKitGTK 4.1、libsoup 3、OpenSSL 和桌面图标支持。桌面菜单与 SVG 图标分别安装到标准的 applications 与 icons 目录。

二进制生成器只从经过验证的正式 `downloads.json` 选择两种 Linux 架构的 `.deb`；AUR 的 `makepkg` 下载并按 `sha256sums_<arch>` 校验后解出 Debian 容器，`package()` 再用 `bsdtar` 安装其中的 `data.tar.*`。配方不执行 `.deb` 的安装维护脚本，不查询 latest，不将校验设置为 SKIP。它只接受单个 payload，并检查桌面程序存在、服务端与 CLI 可执行文件不存在。

Windows / macOS 的签名策略属于整个正式桌面 Release 的门槛；Linux `.deb` 仍明确只提供 SHA-256 校验，没有因此宣称 Linux 包也具有 Authenticode 或 Apple 签名。签名工程产物和没有验签报告的候选清单不能用于生成公开二进制配方。

## 服务端初始化

包安装 `/usr/bin/hireferry`、`hireferry.service`、systemd sysusers 与 tmpfiles 配置。它不自动开启服务、不生成管理员凭据，也不写入生产密钥。配置模板位于 `/usr/share/hireferry/config.example.toml`。

```sh
sudo systemd-sysusers /usr/lib/sysusers.d/hireferry.conf
sudo systemd-tmpfiles --create /usr/lib/tmpfiles.d/hireferry.conf
# 首次初始化；已有配置时不要重复覆盖
sudo install -m 0640 -o root -g hireferry \
  /usr/share/hireferry/config.example.toml /etc/hireferry/config.toml
sudoedit /etc/hireferry/config.toml
sudo -u hireferry sh -c 'cd /var/lib/hireferry && /usr/bin/hireferry --config /etc/hireferry/config.toml check-config'
sudo systemctl enable --now hireferry.service
```

先配置自己的 HTTPS 域名、邮件、登录与支付集成，再启用服务。默认工作目录为 `/var/lib/hireferry`，数据库保持在该目录下。升级和卸载不删除配置或业务数据；升级后由运营者检查配置并决定何时重启。完整说明见[自托管指南](self-hosting.md)。

CLI 的 Agent Skill 位于 `/usr/share/hireferry-cli/skills/hireferry`，可用 `sh /usr/share/hireferry-cli/scripts/install-skill.sh --help` 查看手动安装方式。

## 发布流程

`release/targets.json` 定义二进制构建矩阵。三类产品独立版本化：

| 产品 | 版本来源 | 发布标签 | 发布后更新 |
| --- | --- | --- | --- |
| 服务端 | 根 `Cargo.toml` | `server-vX.Y.Z` | Linux x86_64 / aarch64 归档、下载矩阵、`hireferry-server-bin` |
| CLI | `crates/hireferry-cli/Cargo.toml` | `cli-vX.Y.Z` | 六个平台归档、下载矩阵、`hireferry-cli-bin` |
| 桌面 | Tauri / Cargo / npm 同步版本 | `desktop-vX.Y.Z` | 桌面与所含 CLI 归档、下载矩阵、`hireferry-desktop-bin` |

发布工作流汇总全部目标，通过完整性和 SHA-256 校验后才发布 GitHub Release。AUR 工作流下载已发布的 `downloads.json` 并用 `SHA256SUMS` 校验，然后生成固定版本的 `PKGBUILD` 和 `.SRCINFO`。旧版本重跑不会降级 AUR 包。修改 AUR 配方、公共 Arch 安装文件或生成器后，main 分支推送会更新三个 `-git` 包。PR 在 Arch `base-devel` 容器中以非 root 用户检查 `.SRCINFO` 和实际配方的文件安装边界；该测试不代替完整编译、安装和图形会话实测。

GitHub 仓库需要配置：

- `AUR_PUBLISH_ENABLED=true` 仓库变量。
- `AUR_SSH_PRIVATE_KEY`：已登记到维护者 AUR 账户的自动发布 SSH 私钥。
- `AUR_KNOWN_HOSTS`：通过可信渠道核实的 `aur.archlinux.org` 主机记录。工作流强制核验主机，不临时信任扫描结果。

发布仅推送配方、必要的 systemd 文件与桌面二进制包的许可证，不上传二进制到 AUR，不强制推送。每个包发布后核对 AUR 远端提交。也可手动生成并发布：

```sh
python3 scripts/aur.py --product server --variant git
python3 scripts/aur.py --product cli --variant git
python3 scripts/aur.py --product desktop --variant git
python3 scripts/fetch-release-manifest.py server-v0.1.0 dist/server-manifest
python3 scripts/aur.py --product server --variant bin --manifest dist/server-manifest/downloads.json
bash scripts/publish-aur.sh packaging/aur/hireferry-server-bin
```

日常验证在本地完成：`python3 tests/release.test.py`、`python3 tests/aur.test.py`、`python3 scripts/release.py docs --check`。实际构建可运行 `makepkg --cleanbuild`；ARM64 包需要在对应架构构建和运行后，才能声称完成该架构的运行验收。

桌面正式 Release 发布后，再执行以下同样的校验入口；版本必须换成实际已发布标签：

```sh
python3 scripts/fetch-release-manifest.py desktop-vX.Y.Z dist/desktop-manifest
python3 scripts/aur.py --product desktop --variant bin --manifest dist/desktop-manifest/downloads.json
```
