use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use include_dir::{Dir, File, include_dir};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::OnceLock};

static FILES: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/static");
static BUNDLE: OnceLock<Bundle> = OnceLock::new();

struct Asset {
    bytes: Bytes,
    etag: HeaderValue,
    mime: &'static str,
}

struct Bundle {
    revision: String,
    assets: BTreeMap<&'static str, Asset>,
}

fn files(dir: &'static Dir<'static>, collected: &mut Vec<&'static File<'static>>) {
    collected.extend(dir.files());
    for child in dir.dirs() {
        files(child, collected);
    }
}

fn bundle() -> &'static Bundle {
    BUNDLE.get_or_init(|| {
        let mut all = Vec::new();
        files(&FILES, &mut all);
        all.sort_by_key(|file| file.path());
        let mut hash = Sha256::new();
        for file in &all {
            let path = file.path().to_str().expect("UTF-8 asset path");
            hash.update((path.len() as u64).to_be_bytes());
            hash.update(path.as_bytes());
            hash.update((file.contents().len() as u64).to_be_bytes());
            hash.update(file.contents());
        }
        let revision = hex::encode(hash.finalize());
        let prefix = format!("/static/{revision}/");
        let assets = all
            .into_iter()
            .map(|file| {
                let name = file.path().to_str().expect("UTF-8 asset path");
                let mime = match name.rsplit('.').next().unwrap_or("") {
                    "html" => "text/html; charset=utf-8",
                    "css" => "text/css; charset=utf-8",
                    "js" => "text/javascript; charset=utf-8",
                    "json" => "application/json; charset=utf-8",
                    "svg" => "image/svg+xml",
                    "png" => "image/png",
                    "woff2" => "font/woff2",
                    _ => "text/plain; charset=utf-8",
                };
                // Relative module imports inherit this directory, so an entire
                // module graph uses one release without rewriting JavaScript.
                let bytes = if name.ends_with(".html") {
                    Bytes::from(
                        file.contents_utf8()
                            .expect("UTF-8 HTML asset")
                            .replace("/static/", &prefix),
                    )
                } else {
                    Bytes::from_static(file.contents())
                };
                // Weak validators remain correct for gzip and identity bodies.
                let etag = HeaderValue::from_str(&format!(
                    "W/\"{}\"",
                    hex::encode(Sha256::digest(&bytes))
                ))
                .expect("hexadecimal asset ETag");
                (name, Asset { bytes, etag, mime })
            })
            .collect();
        Bundle { revision, assets }
    })
}

pub(crate) async fn serve(
    State(state): State<crate::AppState>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path == "api"
        || path.starts_with("api/")
        || path == "v1"
        || path.starts_with("v1/")
        || path == "mcp"
    {
        return crate::AppError::not_found("Route not found").into_response();
    }
    let bundle = bundle();
    let page = path.is_empty() || !path.rsplit('/').next().unwrap_or("").contains('.');
    let explicit_asset = path.strip_prefix("static/");
    let (name, versioned) = match explicit_asset {
        Some(path) => match path.split_once('/') {
            Some((revision, name)) if revision == bundle.revision => (name, true),
            _ => (path, false),
        },
        None if page => ("index.html", false),
        None => (path, false),
    };
    // Explicit asset requests, including obsolete release URLs, never fall
    // back to HTML or receive a newer file under an immutable older URL.
    let generated =
        if explicit_asset.is_none() && (page || matches!(path, "robots.txt" | "sitemap.xml")) {
            let (text, mime) = match path {
                "robots.txt" => (crate::seo::robots(&state.cfg), "text/plain; charset=utf-8"),
                "sitemap.xml" => (
                    crate::seo::sitemap(&state.cfg),
                    "application/xml; charset=utf-8",
                ),
                _ => {
                    let template = std::str::from_utf8(&bundle.assets["index.html"].bytes)
                        .expect("UTF-8 HTML template");
                    let html = match crate::seo::page(template, uri.path(), &state.cfg) {
                        Ok(html) => html,
                        Err(error) => return error.into_response(),
                    };
                    (html, "text/html; charset=utf-8")
                }
            };
            let etag = HeaderValue::from_str(&format!(
                "W/\"{}\"",
                hex::encode(Sha256::digest(text.as_bytes()))
            ))
            .expect("hexadecimal page ETag");
            Some(Asset {
                bytes: Bytes::from(text),
                etag,
                mime,
            })
        } else {
            None
        };
    let Some(asset) = generated.as_ref().or_else(|| bundle.assets.get(name)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let missing_page = explicit_asset.is_none() && page && !crate::seo::known_page(uri.path());
    let not_modified = !missing_page
        && headers.get_all(header::IF_NONE_MATCH).iter().any(|value| {
            value.to_str().is_ok_and(|value| {
                value.split(',').any(|candidate| {
                    let candidate = candidate.trim();
                    candidate == "*"
                        || candidate.strip_prefix("W/").unwrap_or(candidate)
                            == asset.etag.to_str().unwrap().trim_start_matches("W/")
                })
            })
        });
    let cache = if versioned && !name.ends_with(".html") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    // Axum's GET route strips HEAD bodies after applying compression metadata.
    let mut response = Response::new(if not_modified {
        Body::empty()
    } else {
        Body::from(asset.bytes.clone())
    });
    *response.status_mut() = if missing_page {
        StatusCode::NOT_FOUND
    } else if not_modified {
        StatusCode::NOT_MODIFIED
    } else {
        StatusCode::OK
    };
    let result = response.headers_mut();
    result.insert(header::CONTENT_TYPE, HeaderValue::from_static(asset.mime));
    result.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    result.insert(header::ETAG, asset.etag.clone());
    // A 304 must carry the same representation selection as the original 200.
    result.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    if !not_modified {
        result.insert(header::CONTENT_LENGTH, HeaderValue::from(asset.bytes.len()));
    }
    response
}
