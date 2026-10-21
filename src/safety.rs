use crate::error::{AppError, AppResult};
pub fn bounded_text(value: &str, max: usize, label: &str) -> AppResult<()> {
    if value.len() > max || value.contains('\0') {
        Err(AppError::bad_request(format!(
            "{label} is too long or contains invalid characters"
        )))
    } else {
        Ok(())
    }
}
pub fn email(value: &str) -> AppResult<String> {
    let email = value.trim().to_ascii_lowercase();
    if email.len() > 254
        || email.contains(['\r', '\n', ' ', '<', '>'])
        || email.matches('@').count() != 1
        || email.starts_with('@')
        || !email.split('@').nth(1).unwrap_or("").contains('.')
    {
        return Err(AppError::bad_request("Enter a valid email address"));
    }
    Ok(email)
}
pub fn markdown(text: &str) -> String {
    let options = pulldown_cmark::Options::ENABLE_TABLES
        | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
        | pulldown_cmark::Options::ENABLE_TASKLISTS;
    let parser = pulldown_cmark::Parser::new_ext(text, options).filter(|event| {
        !matches!(
            event,
            pulldown_cmark::Event::Html(_) | pulldown_cmark::Event::InlineHtml(_)
        )
    });
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    ammonia::Builder::default()
        .rm_tags(&["img"])
        .link_rel(Some("noopener noreferrer nofollow"))
        .clean(&html)
        .to_string()
}
/// An SVG allowlist. No script, CSS, external references, animation or embedded HTML.
pub fn safe_svg(svg: &str) -> AppResult<String> {
    bounded_text(svg, 64_000, "SVG")?;
    let doc =
        roxmltree::Document::parse(svg).map_err(|_| AppError::bad_request("Invalid SVG XML"))?;
    if doc.descendants().any(|node| node.is_pi()) {
        return Err(AppError::bad_request(
            "SVG processing instructions are not allowed",
        ));
    }
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err(AppError::bad_request("Expected an SVG element"));
    }
    let tags = [
        "svg",
        "g",
        "path",
        "circle",
        "ellipse",
        "rect",
        "line",
        "polyline",
        "polygon",
        "title",
        "desc",
        "defs",
        "linearGradient",
        "radialGradient",
        "stop",
        "clipPath",
    ];
    let attrs = [
        "id",
        "xmlns",
        "viewBox",
        "width",
        "height",
        "x",
        "y",
        "x1",
        "y1",
        "x2",
        "y2",
        "cx",
        "cy",
        "r",
        "rx",
        "ry",
        "d",
        "points",
        "fill",
        "stroke",
        "stroke-width",
        "stroke-linecap",
        "stroke-linejoin",
        "stroke-miterlimit",
        "stroke-dasharray",
        "stroke-dashoffset",
        "fill-rule",
        "clip-rule",
        "opacity",
        "fill-opacity",
        "stroke-opacity",
        "transform",
        "gradientUnits",
        "gradientTransform",
        "offset",
        "stop-color",
        "stop-opacity",
        "clip-path",
        "preserveAspectRatio",
    ];
    for node in doc.descendants().filter(|n| n.is_element()) {
        if !tags.contains(&node.tag_name().name())
            || node
                .tag_name()
                .namespace()
                .is_some_and(|n| n != "http://www.w3.org/2000/svg")
        {
            return Err(AppError::bad_request("SVG contains an unsupported element"));
        }
        for attr in node.attributes() {
            if !attrs.contains(&attr.name()) || attr.namespace().is_some() {
                return Err(AppError::bad_request(
                    "SVG contains an unsupported attribute",
                ));
            }
            let v = attr.value().to_ascii_lowercase();
            if v.contains("url(")
                && !(v.starts_with("url(#")
                    && v.ends_with(')')
                    && v[5..v.len() - 1]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            {
                return Err(AppError::bad_request(
                    "External SVG references are not allowed",
                ));
            }
            if v.contains("javascript:")
                || v.contains("data:")
                || v.contains(['@', '<', '\\', ';', '{', '}'])
                || v.contains("/*")
            {
                return Err(AppError::bad_request("Unsafe SVG value"));
            }
        }
    }
    Ok(svg.to_string())
}
