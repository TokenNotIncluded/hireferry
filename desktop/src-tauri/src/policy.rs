use url::{Host, Url};

fn loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain("localhost")) => true,
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    }
}

pub fn web_url(url: &Url) -> bool {
    (url.scheme() == "https" || (url.scheme() == "http" && loopback(url)))
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.port() != Some(0)
}

pub fn server_origin(input: &str) -> Result<Url, &'static str> {
    let url = Url::parse(input.trim()).map_err(|_| "invalid_server")?;
    if !web_url(&url) || url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err("invalid_server");
    }
    Ok(url)
}

pub fn launcher_url(url: &Url) -> bool {
    ((url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost")))
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(url.path(), "" | "/" | "/index.html")
        && url.query().is_none()
        && url.fragment().is_none()
}

#[cfg(any(mobile, test))]
pub fn login_return(url: &Url) -> bool {
    url.scheme() == "hireferry"
        && url.host_str() == Some("login-return")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(url.path(), "" | "/")
        && url.query().is_none()
        && url.fragment().is_none()
}

pub fn service_navigation(origin: &Url, next: &Url) -> bool {
    web_url(next) && next.origin() == origin.origin()
}

pub fn login_landing(origin: &Url, next: &Url) -> Option<Url> {
    if !service_navigation(origin, next) || next.path() != "/api/auth/google/start" {
        return None;
    }
    let mut landing = origin.clone();
    landing.set_path("/auth/app");
    landing.set_query(None);
    landing.set_fragment(None);
    Some(landing)
}

pub fn login_authorization(origin: &Url, next: &Url) -> bool {
    service_navigation(origin, next) && next.path() == "/auth/app/authorize"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https_and_actual_loopback_origins() {
        for input in [
            "https://hireferry.lmm.best",
            "https://selfhost.example:8443/",
            "http://localhost:3000",
            "http://127.0.0.1:3000",
            "http://[::1]:3000",
        ] {
            assert!(server_origin(input).is_ok(), "{input}");
        }
    }

    #[test]
    fn rejects_insecure_remote_origins_credentials_and_paths() {
        for input in [
            "http://hireferry.lmm.best",
            "http://localhost.example/",
            "http://192.168.1.2/",
            "https://user:password@hireferry.lmm.best/",
            "https://hireferry.lmm.best/login",
            "https://hireferry.lmm.best/?token=secret",
            "https://hireferry.lmm.best/#settings",
            "https://hireferry.lmm.best:0/",
            "file:///etc/passwd",
            "javascript:alert(1)",
        ] {
            assert!(server_origin(input).is_err(), "{input}");
        }
    }

    #[test]
    fn launcher_identity_does_not_accept_lookalike_remote_origins() {
        for input in [
            "tauri://localhost/index.html",
            "http://tauri.localhost/index.html",
            "https://tauri.localhost/",
        ] {
            assert!(launcher_url(&Url::parse(input).unwrap()), "{input}");
        }
        for input in [
            "https://tauri.localhost.example/",
            "https://tauri.localhost:444/",
            "https://user@tauri.localhost/",
            "https://localhost/",
            "tauri://remote/",
            "tauri://localhost/index.html?server=https://evil.example",
            "tauri://localhost/index.html#token=secret",
            "https://tauri.localhost/remote.html",
        ] {
            assert!(!launcher_url(&Url::parse(input).unwrap()), "{input}");
        }
    }

    #[test]
    fn mobile_service_navigation_keeps_the_exact_selected_origin() {
        let selected = server_origin("https://selfhost.example:8443").unwrap();
        assert!(service_navigation(
            &selected,
            &Url::parse("https://selfhost.example:8443/login").unwrap()
        ));
        for input in [
            "https://selfhost.example/login",
            "https://selfhost.example.evil.test:8443/",
            "https://user@selfhost.example:8443/",
            "http://selfhost.example:8443/",
            "javascript:alert(1)",
            "file:///etc/passwd",
        ] {
            assert!(
                !service_navigation(&selected, &Url::parse(input).unwrap()),
                "{input}"
            );
        }
    }

    #[test]
    fn login_return_only_wakes_the_app_without_a_payload() {
        for input in ["hireferry://login-return", "hireferry://login-return/"] {
            assert!(login_return(&Url::parse(input).unwrap()));
        }
        for input in [
            "hireferry://login-return?code=untrusted",
            "hireferry://login-return?server=https://evil.example",
            "hireferry://login-return#token=untrusted",
            "hireferry://user@login-return",
            "hireferry://login-return:123",
            "hireferry://login-return/anything",
            "hireferry://login-return.evil.example",
            "https://login-return",
        ] {
            assert!(!login_return(&Url::parse(input).unwrap()), "{input}");
        }
    }

    #[test]
    fn native_login_landing_cannot_import_a_redirect_or_external_origin() {
        let selected = server_origin("https://selfhost.example:8443").unwrap();
        let google = Url::parse("https://selfhost.example:8443/api/auth/google/start?redirect=https://evil.test&app_request=untrusted#token").unwrap();
        assert_eq!(
            login_landing(&selected, &google).unwrap().as_str(),
            "https://selfhost.example:8443/auth/app"
        );
        for input in [
            "https://selfhost.example/api/auth/google/start",
            "https://selfhost.example.evil.test:8443/api/auth/google/start",
            "https://user@selfhost.example:8443/api/auth/google/start",
            "https://selfhost.example:8443/api/auth/google/start/extra",
            "https://selfhost.example:8443/auth/app/authorize?request=opaque",
        ] {
            assert!(login_landing(&selected, &Url::parse(input).unwrap()).is_none());
        }
    }

    #[test]
    fn login_approval_is_the_exact_selected_service_browser_endpoint() {
        let selected = server_origin("https://selfhost.example:8443").unwrap();
        assert!(login_authorization(
            &selected,
            &Url::parse("https://selfhost.example:8443/auth/app/authorize?request=opaque").unwrap()
        ));
        for input in [
            "https://selfhost.example/auth/app/authorize?request=opaque",
            "https://selfhost.example.evil.test:8443/auth/app/authorize",
            "https://user@selfhost.example:8443/auth/app/authorize",
            "https://selfhost.example:8443/auth/app/authorize/extra",
            "https://selfhost.example:8443/auth/app",
        ] {
            assert!(!login_authorization(&selected, &Url::parse(input).unwrap()));
        }
    }
}
