# Android 与 iOS 客户端

移动客户端复用 `desktop/` 下的 Tauri 2 Rust 核心和静态启动页；手机上使用单个系统 WebView。默认连接 `https://hireferry.lmm.best`，也可手动输入带可信 TLS 证书的自托管 HTTPS 站点。业务数据保留在所选服务器，离线时只能打开本地连接页。

**调试包、模拟器包和正式分发包是不同产物。** PR 工作流 `Mobile engineering builds` 生成 Android 调试 APK/AAB 和 iOS 模拟器 `.app.tar.gz`。这些包用于工程验证，不进入桌面 `downloads.json`，也不表示已经上架 Google Play、App Store 或 TestFlight。可下载产物和通过的实际运行记录以对应 GitHub Actions run 的附件为准。

## 平台与产物

| 平台 | 本轮构建目标 | 产物 | 安装范围 |
| --- | --- | --- | --- |
| Android 7.0 / API 24 起 | ARM64；PR 另含 x86_64 模拟器架构 | `.apk`、`.aab` | 调试 APK 可侧载；AAB 交由商店处理，不能直接安装 |
| iOS 15 起 | iPhone/iPad ARM64 | 维护者签名 `.ipa` | 需要实际 Apple 签名身份、匹配配置文件及分发渠道 |
| iOS 模拟器 | 根据 macOS runner 架构构建 ARM64 或 x86_64 | `.app.tar.gz` | 只适用于相应架构的 Xcode Simulator，不能安装到真机 |

Rust 使用共享 `lib.rs` 的移动入口，桌面 `main.rs` 继续生成 `hireferry-desktop`，并保留 `--version` / `--smoke-test`。菜单和桌面多窗口代码通过 `cfg(desktop)` 隔离。平台原生工程由锁定的 Tauri CLI 生成到 `desktop/src-tauri/gen/`，构建脚本再加入环境变量签名配置和现有 SVG 生成的图标；生成目录不提交到版本控制。参见 [Tauri 项目结构](https://v2.tauri.app/start/project-structure/)与[移动平台前置条件](https://v2.tauri.app/start/prerequisites/#configure-for-mobile-targets)。

## 登录、返回和权限边界

在应用 WebView 中使用聘渡的邮箱验证码登录。服务端发放的 HttpOnly 会话 Cookie 留在该 WebView 的所选站点 Cookie 存储中。应用不读取或导出登录 Cookie，不将它放进 URL、localStorage、日志或 native command 参数。

Google 登录需要支持 `/auth/app` 的新版服务器。应用把所选站点的 Google 登录入口导向 WebView 内的 `/auth/app` 过渡页，由服务器创建短时授权请求，并把校验凭据放进该 WebView 的 HttpOnly Cookie。随后只将 `/auth/app/authorize?request=…` 打开到系统浏览器。浏览器自己完成 Google OAuth、显示当前身份并要求明确批准；原 WebView 使用自己的 Cookie 轮询同源兑换接口，服务器原子消费授权后设置普通用户会话。管理员或 bootstrap 权限不通过该入口转移，已有 CLI token 也不参与。旧版自托管服务器需先升级；也可继续使用邮箱验证码或在浏览器中独立使用服务。

批准页的“返回应用”使用固定 `hireferry://login-return` 唤回应用。协议由移动端 deep-link 插件注册；Rust 只接受无账号、端口、路径、查询或片段的固定标记（也接受末尾斜杠），不会接受 server、redirect、code 或 session。暖启动保持原 WebView 和原站点，只提醒现有 `/auth/app` 页面立即再轮询一次；系统若已回收应用，冷启动仍打开本地连接页，需要重新连接并发起登录。唤回链接本身不证明登录成功，也不授予任何原生命令权限。见 [Tauri Deep Linking](https://v2.tauri.app/plugin/deep-linking/)。

手机站点左下方的“连接”控件只会回到内置连接页。控件至少有 44px 点击高度，避开底部安全区和右侧反馈入口；聊天输入区展开时会自动上移。它直接导航到构建时固定的本地资源地址，不接收服务器地址、跳转目标、code 或 token，也不调用原生命令。这个本地导航与上面的系统登录唤回协议相互独立。这个 DOM 控件只是方便返回，所选网站可能改变自己的 DOM。若页面加载失败或控件消失，重新启动应用会重新打开本地连接页，不会自动连接历史站点。

只有本地连接页拥有 `connect` 和 `open_browser` 两个命令权限，且 Rust 再检查窗口标签和本地 URL。远程站点即使复用同一个窗口，也不拥有 native capability；不允许从远程内容打开文件、执行 shell 或更改本地设置。工作页只允许所选精确 origin 内的 HTTPS 导航；其他 HTTPS 网页交给系统浏览器，危险协议和含账号密码的 URL 被拒绝。移动端不依赖 Tauri 桌面的 `on_new_window` 回调（该回调不支持 Android/iOS）。普通 `target="_blank"` 链接通过同一导航规则处理。参见 [Tauri Capabilities](https://v2.tauri.app/security/capabilities/)和 [Opener](https://v2.tauri.app/plugin/opener/)。

## Android 本地构建

需要 Rust **1.90 或更新版本**、Node.js / npm、JDK 21、Android SDK command-line tools 22.0。与锁定的 `@tauri-apps/cli` 2.12.2 模板一致，本脚本固定 API 37（SDK 包路径为 `platforms;android-37.0`）、Build Tools 37.0.0、NDK 29.0.13846066；安装时使用 Android SDK Manager，不隐式接受缺少的 SDK 版本。旧版 command-line tools 16.0 无法识别新的 major.minor SDK 包路径；CI 显式使用 22.0（Google 下载编号 15859902），不依赖 setup action 的旧默认值。

```sh
cd desktop
npm ci
cd ..

# ANDROID_HOME 指向已安装的 SDK；JAVA_HOME 指向 JDK 21。
sdkmanager 'platforms;android-37.0' 'build-tools;37.0.0' 'ndk;29.0.13846066' 'platform-tools'
export NDK_HOME="$ANDROID_HOME/ndk/29.0.13846066"

python3 scripts/mobile.py doctor android
python3 scripts/mobile.py build android --variant debug --target aarch64 --format both
```

脚本检查前置工具、安装指定 Rust target、运行 `tauri android init`、生成图标、构建 APK/AAB 并验证签名。它把验证后的包、SHA-256 和 `mobile-artifacts.json` 放在 `artifacts/mobile/`。调试包 application ID 为 `best.lmm.hireferry.debug`，与正式包 `best.lmm.hireferry` 分开；调试签名可能随开发机器或 CI runner 改变，因此不能作为可靠的正式更新渠道。

要验证模拟器，可构建 `--target x86_64`（或重复指定 ARM64 + x86_64 得到通用包），然后运行：

```sh
python3 scripts/mobile.py smoke-android artifacts/mobile/hireferry-android-x86_64-debug.apk
```

设备必须已经通过 `adb devices` 可见；请使用隔离的测试模拟器。冒烟检查会安装调试 APK、启动 Activity、操作实际 WebView 连接表单，通过 `adb reverse` 连接隔离的本机测试服务，尝试远程 `connect` 并确认拒绝，检查返回与反馈按钮不重叠且反馈可点击，再验证暖启动登录唤回保留页面、返回连接设置、冷启动唤回保持本地页面。它检查 AndroidRuntime 致命异常，并保存各阶段 UI XML、截图和记录；失败时也保留 logcat 与进程退出原因。测试服务没有真实账号，也不访问生产或创建业务数据。该检查不能替代真实 Google 账号授权、真机键盘、文件选择、无障碍、断网恢复等测试。

Android 发布版使用可信 HTTPS。调试版额外允许明确的本机 loopback HTTP，供 `adb reverse tcp:3000 tcp:3000` 连接宿主开发服务；`http://192.168.*` 等明文 LAN 地址仍被 Rust 拒绝。生产版不放开全局 cleartext transport。

### Android 正式签名

不会自动生成或冒充正式签名密钥。维护者须安全保存长期签名身份，并在构建环境设置：

| 环境变量 | 内容 |
| --- | --- |
| `HF_ANDROID_KEYSTORE_PATH` | 真实 keystore 文件的绝对路径 |
| `HF_ANDROID_KEYSTORE_PASSWORD` | keystore 密码 |
| `HF_ANDROID_KEY_ALIAS` | 签名 key alias |
| `HF_ANDROID_KEY_PASSWORD` | key 密码 |
| `HF_ANDROID_CERT_SHA256` | 预期签名证书 SHA-256，可含冒号 |

```sh
python3 scripts/mobile.py build android --variant release --target aarch64 --format both
```

脚本通过环境变量把凭据交给 Gradle，不写进版本库中的配置。APK 用 `apksigner verify`，AAB 用 `jarsigner` / `keytool` 检查签名；证书摘要必须与维护者给出的值一致。缺少凭据或证书不匹配即失败，不把未签名包标成可安装正式包。Google Play 仍需 Play Console 账号、正确配置 Play App Signing、商店资料与审核；参见 [Tauri Android 签名](https://v2.tauri.app/distribute/sign/android/)和 [Google Play 打包](https://v2.tauri.app/distribute/google-play/)。

## iOS 本地构建

iOS 原生编译需要 **macOS 和完整 Xcode**，Linux 上不能把交叉编译命令等同于可用 IPA。还需 CocoaPods 以及锁定的 npm CLI。无分发证书时可以先构建模拟器：

```sh
cd desktop && npm ci && cd ..
brew install cocoapods
python3 scripts/mobile.py doctor ios
python3 scripts/mobile.py build ios --simulator --variant debug
```

模拟器架构默认跟随宿主，Apple Silicon 使用 `aarch64-sim`，Intel 使用 `x86_64`。产物是可供 `xcrun simctl install` 安装的 `.app` 压缩包，不是用于 iPhone 侧载的 IPA；无需填写假的 Team ID 或生成假的 Apple 分发身份。

真机分发需要实际 Apple Developer Team、签名证书和匹配 `best.lmm.hireferry` 的 provisioning profile，可来自本机 Xcode 签名配置，也可使用锁定 CLI 支持的 `IOS_CERTIFICATE`（Base64 p12）、`IOS_CERTIFICATE_PASSWORD`、`IOS_MOBILE_PROVISION`（Base64 profile）环境变量。`APPLE_DEVELOPMENT_TEAM` 必须为真实 Team ID。

```sh
python3 scripts/mobile.py build ios --variant release --target aarch64 \
  --build-number 1 --export-method app-store-connect
```

`release-testing` 和 `debugging` 只适用于匹配的 Ad Hoc / Development 配置文件；设备注册、证书和 profile 由维护者管理。构建后脚本展开 IPA，验证 codesign、Team ID、bundle ID 和嵌入的配置文件存在。`mobile-artifacts.json` 记录实际包摘要和签名检查结果。它不自动上传 App Store，也不宣称通过 Apple 审核；参见 [Tauri iOS 签名](https://v2.tauri.app/distribute/sign/ios/)和 [App Store 构建](https://v2.tauri.app/distribute/app-store/#ios)。

## CI 与发布凭据

`mobile-build.yml` 对修改客户端、脚本和对应工作流的 PR 自动运行，不读取发行签名 secrets。Android job 构建 ARM64 + x86_64 调试 APK/AAB，然后启动 API 35 模拟器侧载 APK，执行上述连接、远程权限隔离和返回闭环。iOS job 在 macOS runner 上构建无发行签名的模拟器应用，执行安装和启动并保存截图。测试附件保留 14 天，文件名和 manifest 明确标记工程测试。

`mobile-release.yml` 只允许维护者手动触发，生成签名产物附件，不自动发布到商店。请先在仓库设置 **`mobile-signing` environment**，限制可使用的分支/标签并配置所需审批，再提供凭据：

- Android secrets：`ANDROID_KEYSTORE_BASE64`、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS`、`ANDROID_KEY_PASSWORD`；environment variable：`ANDROID_CERT_SHA256`。
- iOS secrets：`IOS_CERTIFICATE`、`IOS_CERTIFICATE_PASSWORD`、`IOS_MOBILE_PROVISION`；environment variable：`APPLE_DEVELOPMENT_TEAM`。

Android keystore 只解码到 runner 临时目录并在任务结束清理。iOS 临时 keychain/profile 由 Tauri 的签名实现管理。不要把签名文件、密码或会话 token 提交到仓库。维护者仍需管理商店账号、版本递增、签名身份连续性和最终发布选择。
