# 搜索引擎索引

服务使用 `public_url` 生成 `/robots.txt`、`/sitemap.xml` 和页面的 canonical（规范网址）。站点地图仅列出首页、软件下载、CLI 安装、HumanGrant 介绍、联系、退款、条款和隐私页面，不包含查询参数、账户、简历、岗位详情或授权链接。

公开页面在初始 HTML 中提供正文、独立标题、说明和分享元信息；条款与隐私正文来自当前加载的法律文件。页面初始语言使用 `site.default_language`，浏览器语言切换和站内跳转同步更新元信息。各语言目前共用网址，不声明独立语言版本。

账户、后台、工作台、匹配资料、授权及支付返回页面通过 `X-Robots-Tag: noindex, follow` 禁止索引。`robots.txt` 允许抓取，使搜索引擎能读取该指令；私人数据仍由原有登录与权限控制保护。未知普通页面返回 HTTP 404。不要把私人岗位包装成公开 `JobPosting` 数据。

上线后确认站点地图及其中所有页面返回 200，并在 Google Search Console 提交完整的 `https://你的域名/sitemap.xml`。已验证的父域名资源也可提交子域名站点地图。可用网址检查工具执行实时测试并请求首页索引；提交成功不等于已经收录。

本地检查：`cargo test --locked --test seo --test static_assets`。站内跳转和语言切换还需在实际服务的浏览器页面检查 canonical、标题及 robots 元信息。
