use crate::{AppResult, config::Config};

pub(crate) const PUBLIC_PAGES: &[&str] = &[
    "/",
    "/cli",
    "/downloads",
    "/humangrant",
    "/contact",
    "/refunds",
    "/terms",
    "/privacy",
];

pub(crate) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(crate) fn robots(cfg: &Config) -> String {
    // Allow crawlers to see noindex on account pages. Access control still
    // protects private data; robots.txt is not an authorization boundary.
    format!(
        "User-agent: *\nAllow: /\n\nSitemap: {}/sitemap.xml\n",
        cfg.public_url
    )
}

pub(crate) fn sitemap(cfg: &Config) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for path in PUBLIC_PAGES {
        xml.push_str(&format!(
            "  <url><loc>{}{}</loc></url>\n",
            escape(&cfg.public_url),
            path
        ));
    }
    xml.push_str("</urlset>\n");
    xml
}

pub(crate) fn known_page(path: &str) -> bool {
    PUBLIC_PAGES.contains(&path)
        || matches!(
            path,
            "/login"
                | "/admin"
                | "/app"
                | "/jobs"
                | "/people"
                | "/team/invite"
                | "/billing/return"
                | "/payment/return"
                | "/payment/success"
                | "/checkout/return"
        )
        || path.starts_with("/app/")
        || ["/jobs/", "/people/", "/human/", "/staff/invite/"]
            .iter()
            .any(|prefix| {
                path.strip_prefix(prefix)
                    .is_some_and(|id| !id.is_empty() && !id.contains('/'))
            })
}

fn metadata(path: &str, en: bool) -> (&'static str, &'static str) {
    match (path, en) {
        ("/", false) => (
            "AI 简历初筛与人才匹配",
            "聘渡 HireFerry 提供 AI 原生简历初筛与人才双向匹配。早期阶段求职和招聘都免费，可抢注唯一用户名、创建可选公开的个人主页，并连接 ChatGPT、Grok 等 AI profile。",
        ),
        ("/", true) => (
            "AI résumé screening and talent matching",
            "HireFerry connects people and roles with AI résumé screening and talent matching. Early access is free for job seekers and employers, with unique usernames, optional public profiles and AI profile links.",
        ),
        ("/cli", false) => (
            "hfy CLI 与 AI 技能安装",
            "安装 HireFerry 命令行工具 hfy，登录账户、查询身份与权限，并将 AI 助手连接到求职和招聘工作流。支持 Linux、macOS 和 Windows。",
        ),
        ("/cli", true) => (
            "hfy CLI and AI skill installation",
            "Install the HireFerry hfy CLI, sign in, inspect account permissions and connect your AI assistant. Available for Linux, macOS and Windows.",
        ),
        ("/downloads", false) => (
            "软件下载与安装",
            "下载 HireFerry 已正式发布的命令行工具和服务端软件，查看平台、架构、版本与校验值，以及桌面和移动客户端的发布状态。",
        ),
        ("/downloads", true) => (
            "Software downloads and installation",
            "Download released HireFerry CLI and server packages. Check platforms, architectures, versions, checksums and desktop or mobile release availability.",
        ),
        ("/humangrant", false) => (
            "HumanGrant 人类辅助登录",
            "了解 HireFerry HumanGrant：由本人审核 AI 助手请求的账户权限，确认授权范围并管理或撤销授权。",
        ),
        ("/humangrant", true) => (
            "HumanGrant human-assisted sign-in",
            "Review account permissions requested by an AI assistant with HireFerry HumanGrant. Approve a specific scope and manage or revoke access.",
        ),
        ("/contact", false) => (
            "联系与支持",
            "联系聘渡 HireFerry，咨询求职、招聘、账户、订单和退款，或申请资料更正与删除。",
        ),
        ("/contact", true) => (
            "Contact and support",
            "Contact HireFerry for job search, hiring, account, order and refund support, or to request profile corrections and deletion.",
        ),
        ("/refunds", false) => (
            "退款政策",
            "了解聘渡 HireFerry 招聘方开通、岗位发布和描述修改订单的退款咨询、人工核对与状态确认流程。",
        ),
        ("/refunds", true) => (
            "Refund policy",
            "Learn how to request a review of HireFerry employer activation, job publication and description update orders, and check refund status.",
        ),
        ("/terms", false) => (
            "服务条款",
            "阅读聘渡 HireFerry 服务条款，了解账户、人才匹配、招聘费用、AI 助手授权与服务使用规则。",
        ),
        ("/terms", true) => (
            "Terms of service",
            "Read the HireFerry terms for accounts, talent matching, employer fees, AI assistant authorization and use of the service.",
        ),
        ("/privacy", false) => (
            "隐私政策",
            "了解聘渡 HireFerry 如何处理账户、简历、岗位、匹配和会话信息，以及资料访问、更正与删除方式。",
        ),
        ("/privacy", true) => (
            "Privacy policy",
            "Learn how HireFerry handles account, résumé, role, match and conversation information, and how to request access, correction or deletion.",
        ),
        (_, false) => (
            "账户与工作台",
            "登录聘渡 HireFerry，管理你的账户、匹配与会话。",
        ),
        (_, true) => (
            "Account and workspace",
            "Sign in to HireFerry to manage your account, matches and conversations.",
        ),
    }
}

pub(crate) fn page(template: &str, path: &str, cfg: &Config) -> AppResult<String> {
    let en = cfg.site.default_language == "en";
    let t = |zh, english| if en { english } else { zh };
    let public = PUBLIC_PAGES.contains(&path);
    let (title, description) = metadata(path, en);
    let title = escape(&format!("{title} · {}", cfg.site.name));
    let description = escape(description);
    let canonical = escape(&format!("{}{path}", cfg.public_url));
    let mut head = format!(
        "<meta name=\"description\" content=\"{description}\">\n  <title>{title}</title>\n  <meta name=\"robots\" content=\"{}\">",
        if public {
            "index, follow, max-image-preview:large"
        } else {
            "noindex, follow"
        }
    );
    // Keep client-side navigation and language changes on the same catalog as
    // the initial HTML, without another request or an executable inline script.
    let catalog: std::collections::BTreeMap<_, _> = PUBLIC_PAGES
        .iter()
        .map(|path| (*path, [metadata(path, false), metadata(path, true)]))
        .collect();
    head.push_str(&format!(
        "\n  <meta name=\"hireferry-seo\" content=\"{}\">",
        escape(&serde_json::to_string(&catalog).expect("page metadata JSON"))
    ));
    if public {
        head.push_str(&format!(
            "\n  <link rel=\"canonical\" href=\"{canonical}\">\n  <meta property=\"og:type\" content=\"website\">\n  <meta property=\"og:site_name\" content=\"{}\">\n  <meta property=\"og:title\" content=\"{title}\">\n  <meta property=\"og:description\" content=\"{description}\">\n  <meta property=\"og:url\" content=\"{canonical}\">\n  <meta name=\"twitter:card\" content=\"summary\">",
            escape(&cfg.site.name)
        ));
    }
    let start = template
        .find("  <meta name=\"description\"")
        .expect("description marker");
    let end = template.find("</title>").expect("title marker") + "</title>".len();
    let mut html = format!("{}  {head}{}", &template[..start], &template[end..]);
    html = html.replacen(
        "lang=\"zh-CN\"",
        if en { "lang=\"en\"" } else { "lang=\"zh-CN\"" },
        1,
    );
    if public {
        let content = match path {
            "/terms" | "/privacy" => {
                crate::legal::document(cfg, &path[1..], if en { "en" } else { "zh" })?["html"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            }
            "/" => format!(
                "<div itemscope itemtype=\"https://schema.org/WebSite\"><meta itemprop=\"name\" content=\"HireFerry\"><meta itemprop=\"alternateName\" content=\"聘渡\"><link itemprop=\"url\" href=\"{}\"><h1>{}</h1><p>{description}</p><h2>{}</h2><p>{}</p><h2>{}</h2><p>{}</p><a class=\"button\" href=\"/login?role=seeker\">{}</a> <a class=\"button secondary\" href=\"/login?role=employer\">{}</a></div>",
                escape(&cfg.public_url),
                t(
                    "聘渡 HireFerry · AI 简历初筛与人才匹配",
                    "HireFerry · AI résumé screening and talent matching"
                ),
                t("说说你的期待", "Share your plans"),
                t(
                    "提交简历或岗位需求，和平台聊清你的期待。AI 自动初筛简历并匹配岗位。",
                    "Share a résumé or a role and tell the platform what matters to you. AI screens résumés and matches people with roles."
                ),
                t("合适再认识", "Meet your match"),
                t(
                    "先与平台私聊。匹配后共享资料并开始交流，在工作台查看匹配与沟通进展。",
                    "Start with a private conversation. Share profiles after a match and track introductions and conversations in your workspace."
                ),
                t("找工作", "Find work"),
                t("找人才", "Find people")
            ),
            "/cli" => format!(
                "<h1>{title}</h1><p>{description}</p><h2>{}</h2><p>Linux / macOS / Windows</p><p><a href=\"/install.sh\">install.sh</a> · <a href=\"/install.ps1\">install.ps1</a></p><h2>{}</h2><pre><code>hfy --server {} auth login --oauth\nhfy --server {} whoami\nhfy --server {} capabilities</code></pre>",
                t("安装 hfy", "Install hfy"),
                t("登录与权限", "Sign-in and permissions"),
                escape(&cfg.public_url),
                escape(&cfg.public_url),
                escape(&cfg.public_url)
            ),
            "/downloads" => {
                let catalog: serde_json::Value =
                    serde_json::from_str(include_str!("../static/downloads.json"))?;
                let links = catalog["assets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|asset| {
                        format!(
                            "<li><a href=\"{}\">{}</a> · {} · {} · {}</li>",
                            escape(asset["url"].as_str().unwrap_or_default()),
                            escape(asset["name"].as_str().unwrap_or_default()),
                            escape(asset["platform"].as_str().unwrap_or_default()),
                            escape(asset["architecture"].as_str().unwrap_or_default()),
                            escape(asset["version"].as_str().unwrap_or_default())
                        )
                    })
                    .collect::<String>();
                format!(
                    "<h1>{title}</h1><p>{description}</p><h2>{}</h2><ul>{links}</ul><p>{}</p><p><a href=\"/cli\">{}</a></p>",
                    t("已发布的软件包", "Released packages"),
                    t(
                        "桌面、Android 和 iOS 客户端尚未正式发布。工程构建和模拟器产物不属于正式安装包。",
                        "Desktop, Android and iOS clients are not formally released yet. Engineering builds and simulator artifacts are not release packages."
                    ),
                    t("CLI 安装与登录说明", "CLI installation and sign-in guide")
                )
            }
            "/refunds" => format!(
                "<h1>{title}</h1><p>{description}</p><h2>{}</h2><p>{}</p><p>{}</p>{}",
                t("如何提出退款问题", "How to ask about a refund"),
                t(
                    "如有误付、重复付款、付款后服务未生效或其他订单异常，请提供订单号与简要情况说明。平台会根据订单记录和实际服务情况进行人工核对。",
                    "For a mistaken or duplicate payment, a service that did not activate or another order issue, provide your order number and a brief description. HireFerry reviews the order record and the service provided."
                ),
                t(
                    "退款经人工核对后通过支付服务商处理。请在工作台订单页面查看状态，或联系支持核对。",
                    "After review, refunds are handled through the payment provider. Check your orders in the workspace or contact support to confirm the status."
                ),
                support(cfg)
            ),
            "/contact" => format!(
                "<h1>{title}</h1><p>{description}</p>{}<p>{}</p>",
                support(cfg),
                t(
                    "订单与退款咨询请附上订单号和简要说明。资料更正或删除可通过邮箱申请。",
                    "For orders and refunds, include your order number and a brief description. Email support to request profile corrections or deletion."
                )
            ),
            _ => format!("<h1>{title}</h1><p>{description}</p>"),
        };
        let nav = PUBLIC_PAGES
            .iter()
            .map(|p| format!("<a href=\"{p}\">{}</a>", escape(metadata(p, en).0)))
            .collect::<Vec<_>>()
            .join(" · ");
        let start = html.find("<div id=\"root\">").expect("root marker");
        let end = html.find("  <div id=\"toast-root\"").expect("toast marker");
        html.replace_range(start..end, &format!("<div id=\"root\"><main id=\"main\" class=\"page-width\"><article class=\"prose\">{content}<nav aria-label=\"{}\">{nav}</nav></article></main></div>\n", t("网站导航", "Site navigation")));
    }
    Ok(html)
}

fn support(cfg: &Config) -> String {
    let email = escape(&cfg.site.support_email);
    if email.is_empty() {
        return String::new();
    }
    format!("<p><a href=\"mailto:{email}\">{email}</a></p>")
}
