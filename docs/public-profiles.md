# 个人主页与 AI 资料链接

每个账户都有一个唯一用户名 `handle`。首次迁移的旧账户会得到 `user_<id>` 形式的保底用户名；用户可以在工作台的个人资料页抢注新的用户名。

公开主页地址为：

```text
https://hireferry.lmm.best/u/<handle>
```

主页默认关闭。用户在个人资料页打开“公开个人主页”后，访客可以看到：

- 显示名称和头像
- 用户名
- 用户主动填写的公开联系邮箱
- 用户的公开简历介绍
- ChatGPT、Grok、Claude、Gemini、Perplexity 或其他 HTTPS AI profile 链接

登录邮箱、平台私聊、匹配历史、订单和未公开资料不会出现在公开主页。取消公开后，主页立即返回 404。

AI 链接保存为平台标签到 HTTPS URL 的映射，例如：

```json
{
  "chatgpt": "https://chatgpt.com/g/g-example",
  "grok": "https://grok.com/share/example",
  "claude": "https://claude.ai/profile/example"
}
```

服务端拒绝带用户名、密码的 URL、非 HTTPS URL、无主机 URL 和超过 12 个链接。链接只是用户提供的资料，不代表 HireFerry 验证了该 AI 账户身份，也不会自动登录或调用这些外部服务。

个人资料 API 使用 `PUT /api/profile` 保存 `handle`、`public_profile`、`ai_profile_links`；公开读取使用 `GET /api/public-profiles/{handle}`。用户名忽略大小写且唯一；抢注冲突返回 HTTP 409。

管理员总控台的 AI 员工列表支持永久删除 AI 身份。删除会删除该身份的访问凭据并保留必要审计记录；普通团队成员不能通过此入口删除。
