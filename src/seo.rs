use crate::{AppResult, AppState, api, config::Config};
use serde_json::{Value, json};

pub(crate) const PUBLIC_PAGES: &[&str] = &[
    "/",
    "/cli",
    "/downloads",
    "/humangrant",
    "/contact",
    "/refunds",
    "/terms",
    "/privacy",
    "/discover",
    "/for-employers",
    "/for-job-tools",
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

pub(crate) fn sitemap(state: &AppState) -> String {
    let cfg = &state.cfg;
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
    if let Ok(db) = state.conn() {
        if let Ok(mut jobs) = db.prepare(
            "SELECT j.id FROM jobs j JOIN users u ON u.id=j.owner_id WHERE j.status='published' AND j.headcount>0 AND u.banned=0 ORDER BY j.created_at DESC,j.id",
        ) {
            if let Ok(rows) = jobs.query_map([], |row| row.get::<_, String>(0)) {
                for id in rows.flatten() {
                    xml.push_str(&format!(
                        "  <url><loc>{}/discover/jobs/{}</loc></url>\n",
                        escape(&cfg.public_url),
                        escape(&id)
                    ));
                }
            }
        }
        if let Ok(mut people) = db.prepare(
            "SELECT handle FROM users WHERE role='seeker' AND banned=0 AND public_profile=1 AND resume_md<>'' ORDER BY updated_at DESC,handle",
        ) {
            if let Ok(rows) = people.query_map([], |row| row.get::<_, String>(0)) {
                for handle in rows.flatten() {
                    xml.push_str(&format!(
                        "  <url><loc>{}/discover/people/{}</loc></url>\n",
                        escape(&cfg.public_url),
                        escape(&handle)
                    ));
                }
            }
        }
    }
    xml.push_str("</urlset>\n");
    xml
}

pub(crate) fn known_page(path: &str) -> bool {
    PUBLIC_PAGES.contains(&path)
        || is_public_profile_path(path)
        || is_public_directory_path(path)
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

pub(crate) fn is_public_directory_path(path: &str) -> bool {
    if path == "/discover" {
        return true;
    }
    ["/discover/jobs/", "/discover/people/"]
        .iter()
        .any(|prefix| {
            path.strip_prefix(prefix)
                .is_some_and(|id| !id.is_empty() && !id.contains('/'))
        })
}

pub(crate) fn is_public_directory_detail_path(path: &str) -> bool {
    path != "/discover" && is_public_directory_path(path)
}

fn plain_text(value: &str, limit: usize) -> String {
    let mut text = String::with_capacity(value.len());
    let mut in_tag = false;
    for ch in value.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

fn iso_timestamp(value: Option<&Value>) -> Option<String> {
    let seconds = value?.as_i64()?;
    chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0).map(|date| date.to_rfc3339())
}

pub(crate) fn public_detail_data(state: &AppState, path: &str) -> Option<Value> {
    if let Some(id) = path.strip_prefix("/discover/jobs/") {
        return api::public_job_data(state, id).ok();
    }
    if let Some(handle) = path.strip_prefix("/discover/people/") {
        return api::public_person_data(state, handle).ok();
    }
    None
}

fn json_ld_script(value: Value) -> String {
    let json = serde_json::to_string(&value)
        .expect("public JSON-LD is serializable")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    format!("\n  <script type=\"application/ld+json\" data-hireferry-jsonld>{json}</script>")
}

fn public_detail_metadata(path: &str, detail: &Value) -> (&'static str, String, Value) {
    if path.starts_with("/discover/jobs/") {
        let title = detail["title"].as_str().unwrap_or("公开岗位");
        let employer = detail["employer"]["name"].as_str().unwrap_or("公开招聘方");
        let description = format!("{employer}正在公开招聘 {title}，岗位说明和招聘人数可公开查看。");
        let mut posting = json!({
            "@context": "https://schema.org",
            "@type": "JobPosting",
            "title": title,
            "description": plain_text(detail["jd_html"].as_str().unwrap_or_default(), 500),
            "url": detail["url"].as_str(),
            "hiringOrganization": {"@type": "Organization", "name": employer}
        });
        if let Some(date) = iso_timestamp(detail.get("created_at")) {
            posting["datePosted"] = Value::String(date);
        }
        if let (Some(currency), Some(cents), Some(decimals)) = (
            detail["currency"].as_str(),
            detail["salary_cents"].as_i64(),
            detail["currency_decimals"].as_i64(),
        ) {
            let divisor = 10_i64.pow(decimals.clamp(0, 6) as u32) as f64;
            posting["baseSalary"] = json!({
                "@type": "MonetaryAmount",
                "currency": currency,
                "value": {"@type": "QuantitativeValue", "value": cents as f64 / divisor, "unitText": "MONTH"}
            });
        }
        ("公开岗位", description, posting)
    } else {
        let name = detail["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| detail["handle"].as_str().unwrap_or("公开简历"));
        let handle = detail["handle"].as_str().unwrap_or_default();
        let description = format!(
            "{name} 的公开求职主页（@{handle}），包含本人主动公开的简历内容和个人主页链接。"
        );
        let same_as = detail["ai_profile_links"]
            .as_object()
            .into_iter()
            .flat_map(|links| links.values())
            .filter_map(Value::as_str)
            .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
            .map(Value::from)
            .collect::<Vec<_>>();
        let mut person = json!({
            "@context": "https://schema.org",
            "@type": "Person",
            "name": name,
            "identifier": handle,
            "url": detail["url"].as_str(),
            "description": plain_text(detail["resume_html"].as_str().unwrap_or_default(), 500)
        });
        if !same_as.is_empty() {
            person["sameAs"] = Value::Array(same_as);
        }
        ("公开简历", description, person)
    }
}

pub(crate) fn is_public_profile_path(path: &str) -> bool {
    let Some(handle) = path.strip_prefix("/u/") else {
        return false;
    };
    matches!(handle.len(), 3..=32)
        && handle.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

fn metadata(path: &str, en: bool) -> (&'static str, &'static str) {
    match (path, en) {
        ("/", false) => (
            "投了几百份简历石沉大海？让 AI 持续帮你找工作",
            "投了几百份简历却没有回应？填写目标、地点和薪资期待，连接 HireFerry AI 后 7×24 持续筛选岗位，做简历与岗位匹配。早期阶段求职和招聘都免费。",
        ),
        ("/", true) => (
            "Hundreds of résumés and no reply? Let AI keep looking",
            "Set your target role, location and salary expectations. Connect HireFerry AI to keep screening roles and matching opportunities around the clock. Early access is free for job seekers and employers.",
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
        ("/discover", false) => (
            "公开岗位与简历目录",
            "浏览明确公开的招聘岗位和求职简历，也可以通过免费公开 MCP 让 AI 读取这些信息。",
        ),
        ("/discover", true) => (
            "Public jobs and résumés",
            "Browse explicitly public roles and résumés, or let an AI read the same information through the free public MCP endpoint.",
        ),
        ("/for-employers", false) => (
            "AI 自动筛简历与免费发布岗位",
            "正在找 AI 自动筛简历、智能简历筛选或人岗匹配工具？HireFerry 让招聘方早期免费发布岗位，授权 AI 按岗位要求整理公开候选人线索；不抓取 BOSS 直聘等第三方平台，邮箱、订单和私聊保持私密。",
        ),
        ("/for-employers", true) => (
            "AI resume screening and free job publishing",
            "Looking for AI resume screening, intelligent candidate filtering or job matching? HireFerry lets recruiters publish roles free during early access and authorize AI to organize public candidate signals; it does not scrape BOSS Zhipin or other third-party platforms, and keeps email, orders and private chats private.",
        ),
        ("/for-job-tools", false) => (
            "本地求职工具与 HireFerry 公开发现层",
            "让 JobSync、Ollama、CLI 和 Agent 保留私密投递与草稿，HireFerry 只承接主动公开的主页、岗位和只读 MCP，帮助人与 AI 发现真实机会。",
        ),
        ("/for-job-tools", true) => (
            "Local-first job tools and HireFerry public discovery",
            "Keep private applications and drafts in JobSync, Ollama, CLIs or Agents. HireFerry adds explicit public profiles, roles and read-only MCP so people and AI can discover real opportunities.",
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

pub(crate) fn page(
    template: &str,
    path: &str,
    cfg: &Config,
    public_detail: Option<&Value>,
) -> AppResult<String> {
    let en = cfg.site.default_language == "en";
    let t = |zh, english| if en { english } else { zh };
    let public = PUBLIC_PAGES.contains(&path)
        || is_public_profile_path(path)
        || is_public_directory_path(path);
    let detail_path = is_public_directory_detail_path(path);
    let indexable = public && (!detail_path || public_detail.is_some());
    let (base_title, base_description) = metadata(path, en);
    let mut title = base_title.to_owned();
    let mut description = base_description.to_owned();
    let mut json_ld = None;
    if let Some(detail) = public_detail {
        let (kind, detail_description, structured) = public_detail_metadata(path, detail);
        title = if kind == "公开岗位" {
            detail["title"].as_str().unwrap_or(kind).to_owned()
        } else {
            detail["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| detail["handle"].as_str().unwrap_or(kind))
                .to_owned()
        };
        description = detail_description;
        json_ld = Some(structured);
    }
    let title = escape(&format!("{title} · {}", cfg.site.name));
    let description = escape(&description);
    let canonical = escape(&format!("{}{path}", cfg.public_url));
    let mut head = format!(
        "<meta name=\"description\" content=\"{description}\">\n  <title>{title}</title>\n  <meta name=\"robots\" content=\"{}\">",
        if indexable {
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
    if indexable {
        head.push_str(&format!(
            "\n  <link rel=\"canonical\" href=\"{canonical}\">\n  <meta property=\"og:type\" content=\"website\">\n  <meta property=\"og:site_name\" content=\"{}\">\n  <meta property=\"og:title\" content=\"{title}\">\n  <meta property=\"og:description\" content=\"{description}\">\n  <meta property=\"og:url\" content=\"{canonical}\">\n  <meta name=\"twitter:card\" content=\"summary\">",
            escape(&cfg.site.name)
        ));
    }
    if let Some(structured) = json_ld {
        head.push_str(&json_ld_script(structured));
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
                "<div itemscope itemtype=\"https://schema.org/WebSite\"><meta itemprop=\"name\" content=\"HireFerry\"><meta itemprop=\"alternateName\" content=\"聘渡\"><link itemprop=\"url\" href=\"{}\"><h1>{}</h1><p>{description}</p><h2>{}</h2><p>{}</p><h2>{}</h2><p>{}</p><p>{}</p><a class=\"button\" href=\"/login?role=seeker&return_to=%2Fapp%3Ftab%3Dprofile\">{}</a> <a class=\"button secondary\" href=\"/login?role=employer&return_to=%2Fapp%2Fjobs%2Fnew\">{}</a></div>",
                escape(&cfg.public_url),
                t(
                    "聘渡 HireFerry · AI 简历初筛与人才匹配",
                    "HireFerry · AI résumé screening and talent matching"
                ),
                t("说说你的期待", "Share your plans"),
                t(
                    "填写求职目标、地点和薪资期待，连接 AI 后 7×24 持续筛选岗位并匹配机会。",
                    "Set your target role, location and salary expectations. Connect AI to keep screening roles and matching opportunities around the clock."
                ),
                t("合适再认识", "Meet your match"),
                t(
                    "先与平台私聊。匹配后共享资料并开始交流，在工作台查看匹配与沟通进展。",
                    "Start with a private conversation. Share profiles after a match and track introductions and conversations in your workspace."
                ),
                t("求职者免费使用", "Free for job seekers"),
                t("注册独一无二的名字", "Register a unique name"),
                t("免费创建岗位", "Create a free role")
            ),
            "/for-employers" => format!(
                "<h1>{}</h1><p>{}</p><h2>{}</h2><ul><li>{}</li><li>{}</li><li>{}</li></ul><h2>{}</h2><p>{}</p><p><a class=\"button\" href=\"/login?role=employer&return_to=%2Fapp%2Fjobs%2Fnew\">{}</a> <a class=\"button secondary\" href=\"/discover\">{}</a></p>",
                t(
                    "把岗位交给 AI，少收一堆不匹配简历。",
                    "Let AI sort the signal from the noise."
                ),
                description,
                t("招聘方早期阶段权益", "Early-access recruiter benefits"),
                t(
                    "免费开通、无需月费，免费发布岗位",
                    "Free activation and role publishing, with no monthly fee"
                ),
                t(
                    "只公开你主动发布的岗位内容",
                    "Only explicitly published role content is public"
                ),
                t(
                    "公开 MCP 可让 AI 直接读取岗位",
                    "The public MCP lets AI read the role directly"
                ),
                t(
                    "AI 自动筛简历，先看公开证据",
                    "AI resume screening starts with public evidence"
                ),
                t(
                    "如果你正在找 BOSS 直聘自动筛简历、AI 自动筛简历或人岗匹配工具，HireFerry 走的是自己的公开岗位与公开主页路径：不抓取第三方简历、不自动打招呼，只有本人主动公开的内容才会被 AI 读取。",
                    "If you are looking for BOSS Zhipin resume screening, AI resume screening or job matching, HireFerry uses its own public roles and profiles: it does not scrape third-party résumés or send automatic greetings, and AI reads only what people explicitly publish."
                ),
                t("免费发布一个岗位", "Post one role free"),
                t("先看公开目录", "View public directory")
            ),
            "/for-job-tools" => format!(
                "<h1>{}</h1><p>{}</p><h2>{}</h2><ul><li>{}</li><li>{}</li><li>{}</li></ul><p><a class=\"button\" href=\"/discover\">{}</a> <a class=\"button secondary\" href=\"/login?role=seeker&amp;return_to=%2Fapp%3Ftab%3Dprofile\">{}</a></p>",
                t(
                    "让本地工具负责私密投递，让公开层负责被发现。",
                    "Let local tools keep applications private. Let the public layer make people discoverable."
                ),
                description,
                t("互补关系", "Complementary layers"),
                t(
                    "私密资料、草稿和投递记录继续留在本地工具。",
                    "Private data, drafts and applications stay in local tools."
                ),
                t(
                    "HireFerry 只接收用户明确公开的主页和岗位。",
                    "HireFerry receives only profiles and roles people explicitly publish."
                ),
                t(
                    "公共 MCP 无需登录、只读，只返回明确公开内容。",
                    "Public MCP needs no login, is read-only and returns only explicit public content."
                ),
                t("查看公开目录", "View public directory"),
                t("创建公开主页", "Create a public profile")
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
            p if is_public_directory_detail_path(p) && public_detail.is_some() => {
                let detail = public_detail.expect("checked above");
                if p.starts_with("/discover/jobs/") {
                    format!(
                        "<h1>{title}</h1><p>{description}</p><article class=\"markdown\"><h2>公开岗位</h2>{}</article>",
                        detail["jd_html"].as_str().unwrap_or_default()
                    )
                } else {
                    format!(
                        "<h1>{title}</h1><p>{description}</p><article class=\"markdown\"><h2>公开简历</h2>{}</article>",
                        detail["resume_html"].as_str().unwrap_or_default()
                    )
                }
            }
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
