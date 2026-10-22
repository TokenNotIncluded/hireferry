use crate::{open_external, policy, require_launcher};
use std::sync::RwLock;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_deep_link::DeepLinkExt;
use url::Url;

#[derive(Default)]
struct Connection(RwLock<Option<Url>>);

#[tauri::command]
async fn connect(window: WebviewWindow, app: AppHandle, server: String) -> Result<(), String> {
    require_launcher(&window)?;
    let url = policy::server_origin(&server).map_err(str::to_owned)?;
    // Production Android/iOS transport policy requires TLS. A debug build can
    // use loopback for `adb reverse`, without allowing cleartext LAN servers.
    if !cfg!(debug_assertions) && url.scheme() != "https" {
        return Err("invalid_server".into());
    }
    *app.state::<Connection>()
        .0
        .write()
        .map_err(|_| "window_failed".to_owned())? = Some(url.clone());
    window.navigate(url).map_err(|_| "window_failed".into())
}

fn launcher_url() -> Url {
    #[cfg(target_os = "android")]
    let url = "https://tauri.localhost/index.html";
    #[cfg(target_os = "ios")]
    let url = "tauri://localhost/index.html";
    Url::parse(url).expect("static local URL")
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .plugin(
            tauri_plugin_opener::Builder::new()
                // Website links use our navigation policy. The plugin's
                // automatic JS interceptor would require remote IPC permission.
                .open_js_links_on_click(false)
                .build(),
        )
        .manage(Connection::default())
        .invoke_handler(tauri::generate_handler![connect, crate::open_browser])
        .setup(|app| {
            let return_app = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                let urls = event.urls();
                if urls.len() != 1 || !policy::login_return(&urls[0]) {
                    return;
                }
                // The OS foregrounds the existing app. A fixed, payload-free
                // event only asks its already-open login page to poll sooner.
                // Never import a URL/session or navigate on a cold start.
                let Some(window) = return_app.get_webview_window("launcher") else {
                    return;
                };
                if window.url().is_ok_and(|current| {
                    current.path() == "/auth/app"
                        && return_app
                            .state::<Connection>()
                            .0
                            .read()
                            .is_ok_and(|selected| {
                                selected.as_ref().is_some_and(|origin| {
                                    policy::service_navigation(origin, &current)
                                })
                            })
                }) {
                    let _ = window.eval("window.dispatchEvent(new Event('hireferry-app-return'))");
                }
            });
            let navigation_app = app.handle().clone();
            WebviewWindowBuilder::new(app, "launcher", WebviewUrl::App("index.html".into()))
                .title("HireFerry · 聘渡")
                .use_https_scheme(true)
                .initialization_script(
                    include_str!("mobile-bootstrap.js")
                        .replace("__HIREFERRY_LAUNCHER_URL__", launcher_url().as_str()),
                )
                .on_navigation(move |next| {
                    if policy::launcher_url(next) {
                        return true;
                    }
                    let selected = navigation_app
                        .state::<Connection>()
                        .0
                        .read()
                        .ok()
                        .and_then(|selected| selected.clone());
                    if let Some(origin) = selected {
                        if let Some(landing) = policy::login_landing(&origin, next) {
                            let app = navigation_app.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Some(window) = app.get_webview_window("launcher") {
                                    let _ = window.navigate(landing);
                                }
                            });
                            return false;
                        }
                        if policy::service_navigation(&origin, next)
                            && !policy::login_authorization(&origin, next)
                        {
                            return true;
                        }
                    }
                    if policy::web_url(next) {
                        let app = navigation_app.clone();
                        let url = next.clone();
                        // The browser holds its own Google CSRF/session cookies
                        // and explicitly approves the short-lived app request.
                        // Only the original WebView can redeem its HttpOnly proof.
                        std::thread::spawn(move || {
                            let _ = open_external(&app, &url);
                        });
                    }
                    false
                })
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run HireFerry mobile");
}
