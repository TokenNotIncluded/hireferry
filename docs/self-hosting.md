# 自托管与开发

[项目首页](../README.md) · [文档导航](README.md) · [部署交接](deployment-handoff.md)

HireFerry 使用一个 Rust 服务运行网站与 API。网页资源嵌入二进制，业务数据保存在 SQLite，服务设置由 TOML 管理。生产运行无需 Node.js。

## 本机运行

使用与 [CI](../.github/workflows/ci.yml) 一致的 Rust 1.99.1。Linux 构建需要 C 工具链、`pkg-config` 和 OpenSSL 开发包，SQLite 随 Rust 依赖构建。

Debian / Ubuntu：

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev ca-certificates
```

安装 [rustup](https://rustup.rs/) 后，在仓库根目录运行：

```sh
rustup toolchain install 1.99.1 --profile minimal --component rustfmt
cp config.development.toml config.toml
chmod 600 config.toml
cargo +1.99.1 run --locked -- --config config.toml serve
```

打开 [http://localhost:3000](http://localhost:3000)。开发模板关闭 SMTP，邮箱验证码显示在界面中。此行为仅适用于显式启用 `development = true` 并绑定回环地址的开发环境。

Google、SMTP 和支付网关在开发模板中默认关闭。按需配置并重启即可启用。开发模式不模拟支付成功，也不自动创建用户或示例业务数据。

完整字段见 [开发模板](../config.development.toml) 和 [生产模板](../config.example.toml)。

## Linux 生产部署

生产模板使用 `https://hireferry.lmm.best` 作为示例来源。自托管时将 `public_url`、`webauthn_rp_id`、反向代理域名及外部服务回调统一改为自己的域名。商户与邮件字段使用部署方已验证的配置。

程序只加载指定 TOML，不自动导入 `.env`。完整外部服务接入清单见 [部署交接](deployment-handoff.md)。

### 接入地址

以下地址以模板域名为例：

| 用途 | 方法与地址 |
| --- | --- |
| Google OAuth 回调 | GET `https://hireferry.lmm.best/auth/google/callback` |
| 服务条款 | GET `https://hireferry.lmm.best/terms` |
| 隐私政策 | GET `https://hireferry.lmm.best/privacy` |
| 退款政策 | GET `https://hireferry.lmm.best/refunds` |
| 联系支持 | GET `https://hireferry.lmm.best/contact` |
| Stripe 通知 | POST `https://hireferry.lmm.best/api/webhooks/stripe` |
| Waffo 支付与退款通知 | POST `https://hireferry.lmm.best/api/webhooks/waffo` |
| 支付返回页 | GET `https://hireferry.lmm.best/billing/return` |
| 远程 MCP | POST `https://hireferry.lmm.best/mcp` |

管理员入口为 `/admin`。接入外部系统前，先确认 HTTPS 来源与各服务登记的地址一致。

### 构建与安装

服务端正式发布矩阵包括 Linux x86_64 与 aarch64，使用 Ubuntu 22.04 构建，要求 glibc 与 OpenSSL 3。Arch Linux 可单独安装 `hireferry-server-bin` 或 `hireferry-server-git`，使用 `/usr/bin/hireferry`；初始化步骤见 [AUR 指南](aur.md)。下面的手动安装使用 `/usr/local/bin/hireferry`。

在仓库根目录执行：

```sh
cargo +1.99.1 build --locked --release
sudo useradd --system --user-group --home-dir /var/lib/hireferry --shell /usr/sbin/nologin hireferry
sudo install -m 0755 target/release/hireferry /usr/local/bin/hireferry
sudo install -d -m 0750 -o root -g hireferry /etc/hireferry
sudo install -d -m 0700 -o hireferry -g hireferry /var/lib/hireferry
sudo install -d -m 0700 -o hireferry -g hireferry /var/lib/hireferry/data
sudo install -m 0600 -o hireferry -g hireferry config.example.toml /etc/hireferry/config.toml
sudoedit /etc/hireferry/config.toml
```

设置真实 HTTPS `public_url`、对应的 `webauthn_rp_id` 和 `development = false`。使用本机反向代理时，服务监听 `127.0.0.1:3000`。

`public_url` 只接受来源，不附加路径、查询参数或账号密码。`database` 相对服务工作目录解析，模板对应 `/var/lib/hireferry/data/hireferry.sqlite3`。

配置文件权限设为 600，数据库与备份目录设为 700。密钥和业务数据库保留在部署环境中。每次修改配置后验证并重启服务。

### systemd 与 HTTPS

```sh
sudo -u hireferry /usr/local/bin/hireferry --config /etc/hireferry/config.toml check-config
sudo install -m 0644 deploy/hireferry.service /etc/systemd/system/hireferry.service
sudo systemctl daemon-reload
sudo systemctl enable --now hireferry
```

使用 [Caddyfile](../deploy/Caddyfile) 配置 HTTPS 反向代理。将 DNS 指向部署服务器，调整模板中的域名，验证配置后重载 Caddy。

公网入口使用 443，80 用于证书与 HTTPS 重定向，应用端口 3000 保持回环监听。服务日志可通过 `journalctl -u hireferry` 查看。`check-config` 检查本地设置，外部 DNS、邮件、Google 和支付商户另行联调。

### Google 与邮箱

Google OAuth Web 应用的授权回调必须匹配实际站点的 `/auth/google/callback`。应用首页、条款和隐私 URL 使用同一站点来源。

在 `[google]` 填写 `client_id` 和 `client_secret` 后启用。Google 邮箱与现有邮箱账户相同也不会自动合并，现有邮箱账户继续使用邮箱验证码登录。

在 `[smtp]` 配置主机、端口、用户名、密码及获准发件地址 `from`。`reply_to` 控制邮件回复地址，`[site].support_email` 控制页面联系方式。

`starttls = true` 使用 STARTTLS，通常为 587 端口。`false` 使用隐式 TLS，通常为 465 端口。生产环境未启用 SMTP 时，邮箱登录不可用。详见 [认证与 Passkey](auth.md)。

### 初始化管理员

在服务器交互式终端执行：

```sh
sudo -u hireferry sh -c 'cd /var/lib/hireferry && /usr/local/bin/hireferry --config /etc/hireferry/config.toml admin-bootstrap'
```

命令输出 10 分钟内有效的一次性口令。打开站点 `/admin`，输入口令并绑定首把 Passkey。服务端验证成功后才开放总控台，此后管理员使用 Passkey 登录，首次口令引导永久停用。

建议登记第二把独立设备或安全密钥。最多支持 20 把，增删需要最近 10 分钟内的 Passkey 认证，最后一把不能删除。凭证与 HTTPS 来源和 RP ID 绑定。更换域名前须规划登录迁移，全部密钥丢失时没有网页密码恢复或自动重置命令。

### 在线备份

`hireferry backup` 使用 SQLite 在线备份 API，可在服务运行期间生成一致快照。正在使用 WAL 时，只复制主数据库文件无法保证完整性。

```sh
sudo install -d -m 0700 -o hireferry -g hireferry /var/lib/hireferry/backups
sudo -u hireferry sh -c 'umask 077; cd /var/lib/hireferry && /usr/local/bin/hireferry --config /etc/hireferry/config.toml backup /var/lib/hireferry/backups/hireferry-001.sqlite3'
```

每次使用新文件名，将备份与 TOML 安全保存到服务器之外，并定期验证恢复流程。恢复时停止服务。数据库包含资料、订单、审计、会话和 Passkey 公钥，备份不能代替实际持有凭证的设备或安全密钥。

## Docker

[Dockerfile](../Dockerfile) 使用 Rust 1.99 构建，运行环境为 Debian Bookworm，包含 CA 证书及 OpenSSL 运行库。服务以 UID/GID `10001` 运行，配置与数据由外部挂载。

```sh
docker build -t hireferry:local .
mkdir -p docker-config
cp config.example.toml docker-config/config.toml
```

编辑 `docker-config/config.toml`，设置真实 HTTPS 域名、`development = false`、`bind = "0.0.0.0:3000"`。数据库路径保留为 `data/hireferry.sqlite3`，并填入需要启用的外部服务配置。

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

容器外仍需 HTTPS 反向代理。首次管理员口令通过下列命令获取：

```sh
docker exec -it hireferry hireferry --config /app/config/config.toml admin-bootstrap
```

升级时保留持久化的 `docker-data`。开发模式要求回环监听，不适用于上述容器内 `0.0.0.0` 的生产映射配置。

## 开发检查

前端检查使用 Node.js 24。以下命令在仓库根目录运行，覆盖格式、Rust 测试、浏览器资源及打包脚本：

```sh
cargo +1.99.1 fmt --all -- --check
cargo +1.99.1 test --locked --workspace
for asset in static/*.js static/runtime/*.js; do
  node --input-type=module --check < "$asset" || exit 1
done
node --check examples/humangrant-client.mjs
for suite in tests/*.test.mjs; do node "$suite" || exit 1; done
python3 tests/cli-scripts.test.py
cargo +1.99.1 build --locked --release --workspace
```

大数据量工作台回归使用独立合成资料，需显式运行：

```sh
cargo +1.99.1 test --locked --test workbench -- --ignored --nocapture
```

CLI 浏览器验收需要 Playwright 与 Chromium，安装和运行方式见 [CLI 浏览器验收](cli.md#可选的真实浏览器验收)。全部 CI 步骤以 [工作流](../.github/workflows/ci.yml) 为准。

### 验收记录

[验收记录](acceptance.md) 汇总对应提交的测试、浏览器与移动端截图、构建和 CI 证据。专题记录见 [CLI 验收](cli-validation.md)、[页面导航验证](workspace-navigation-validation.md) 和 [聊天热路径验证](chat-hotpath-validation.md)。

Google、SMTP、实体 Passkey、商户付款与 Waffo 退款通知需要使用实际部署配置完成端到端验收。真实 Webhook 接收站、浏览器原生 WebMCP 以及外部 AI 集成也需分别验证。

异常到账标记为 `paid_needs_review`，由管理员核对后通过商户后台处理。本站不主动发起退款，Stripe 退款与争议通知尚未实现。当前待办与外部验收项见 [剩余工作](remaining-work.md)。
