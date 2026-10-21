# HumanGrant 界面与素材

HumanGrant 在聘渡中的入口表示：AI 发起申请，用户登录并明确选择权限，AI 再获得独立访问凭据。

## 新设计

图形由一个人形和一个允许标记组成。人形表示授权人，右侧的勾表示具体同意。按钮保留协议要求的英文名称 **Continue with human beings**，下方用当前界面的语言说明用途。

本目录的 SVG 为聘渡重新绘制，随本仓库许可发布。没有复制上游 HumanGrant 图标，也没有使用外部图片、脚本或字体文件。

| 素材 | 用途 |
| --- | --- |
| `static/humangrant/mark-light.svg` | 浅色背景的独立标记，透明背景 |
| `static/humangrant/mark-dark.svg` | 深色背景的独立标记，透明背景 |
| `static/humangrant/continue-light.svg` | 浅色按钮展示图，368 × 64 |
| `static/humangrant/continue-dark.svg` | 深色按钮展示图，368 × 64 |
| `static/humangrant/consent-light.svg` | 浅色确认流程插画，480 × 320 |
| `static/humangrant/consent-dark.svg` | 深色确认流程插画，480 × 320 |

按钮展示图用于说明和复用。实际网页按钮使用 HTML 文本和内联 SVG，以便键盘操作、缩放及屏幕阅读器读取。装饰插画不承载权限或状态信息。

## 页面

- `/login`：新增 HumanGrant 入口，保留现有登录方法。关闭服务配置后，不显示这个入口。
- `/humangrant`：说明 AI 申请、用户确认、AI 继续三个步骤；提供可复制的 AI 接入说明，以及本站确认链接输入框。
- `/human/{link}`：显示服务、申请者标识、用途、当前账户、具体权限、资源和到期时间。权限默认全部不选。用户另需勾选明确同意，才能提交允许操作。
- `/app?tab=agents`：在原 AI 连接页显示 HumanGrant 授权、权限、创建时间、到期时间与撤销入口。

HumanGrant 页面支持系统深色偏好。标记和插画也支持页面已有的 `html.dark` 或 `data-theme="dark"` 标记。小屏幕下，说明和确认区域按阅读顺序排列。权限、按钮与说明均为真实文字，不能只靠颜色判断。

完成后的确认页显示实际批准范围和授权截止时间，并按当前授权状态说明是否可继续使用。撤销、到期或权限减少后，原批准范围明确标为历史记录；未获批准的申请权限不进入这份清单。

## 登录恢复

用户从确认页进入邮箱或 Google 登录后，页面会恢复同一申请。浏览器只在当前标签页保存一个严格匹配 `/human/{link}` 的同源路径；保存记录一小时后失效。外部 URL、查询串、片段、短链接和编码路径不会作为登录后目标。

这份浏览器记录不提供任何权限。确认页和提交操作仍由服务器重新检查当前登录、申请状态和账户权限。切换账户后，旧确认凭据不能使用。

## 检查

```sh
node --input-type=module --check < static/humangrant-ui.js
node --input-type=module --check < static/app.js
node tests/humangrant-ui.test.mjs
node tests/service-ui.test.mjs
```

专项界面测试覆盖链接恢复、文本转义、明确选择权限、确认凭据、账户不匹配、申请结束和授权撤销。它们不替代真实签名、账户登录和服务器权限测试。
