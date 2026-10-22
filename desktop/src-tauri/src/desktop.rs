use crate::{open_external, policy, require_launcher};

use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{
    AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    menu::{MenuBuilder, SubmenuBuilder},
    webview::{NewWindowResponse, PageLoadEvent},
};
use url::Url;

static NEXT_WINDOW: AtomicU64 = AtomicU64::new(1);

fn show_launcher(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("launcher") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub async fn connect(window: WebviewWindow, app: AppHandle, server: String) -> Result<(), String> {
    require_launcher(&window)?;
    let url = policy::server_origin(&server).map_err(str::to_owned)?;
    let selected = url.clone();
    let popup_selected = url.clone();
    let navigation_app = app.clone();
    let popup_app = app.clone();
    let home = url.clone();
    let label = format!("workbench-{}", NEXT_WINDOW.fetch_add(1, Ordering::Relaxed));
    let navigation_label = label.clone();
    let popup_label = label.clone();
    let old_windows: Vec<_> = app
        .webview_windows()
        .into_values()
        .filter(|window| window.label().starts_with("workbench-"))
        .collect();
    let workbench = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url.clone()))
        .title(format!(
            "HireFerry · {}",
            url.origin().ascii_serialization()
        ))
        .inner_size(1280.0, 860.0)
        .min_inner_size(680.0, 540.0)
        .center()
        // No remote capability is assigned to workbench-* windows. The remote
        // service keeps its own CSP; it can never invoke our native commands.
        .on_navigation(move |next| {
            if let Some(landing) = policy::login_landing(&selected, next) {
                navigate_later(&navigation_app, &navigation_label, landing);
                return false;
            }
            if policy::service_navigation(&selected, next)
                && !policy::login_authorization(&selected, next)
            {
                return true;
            }
            // Approval belongs to the system browser. The original WebView's
            // HttpOnly proof alone can redeem the server's one-time grant.
            let app = navigation_app.clone();
            let next = next.clone();
            std::thread::spawn(move || {
                let _ = open_external(&app, &next);
            });
            false
        })
        .on_new_window(move |url, _| {
            if let Some(landing) = policy::login_landing(&popup_selected, &url) {
                navigate_later(&popup_app, &popup_label, landing);
            } else {
                let app = popup_app.clone();
                std::thread::spawn(move || {
                    let _ = open_external(&app, &url);
                });
            }
            NewWindowResponse::Deny
        })
        .build()
        .map_err(|_| "window_failed".to_owned())?;

    let handle = app.clone();
    let closed_label = label.clone();
    let focus_origin = url;
    workbench.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Focused(true)) {
            let app = handle.clone();
            let label = closed_label.clone();
            let origin = focus_origin.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(window) = app.get_webview_window(&label)
                    && window.url().is_ok_and(|current| {
                        policy::service_navigation(&origin, &current)
                            && current.path() == "/auth/app"
                    })
                {
                    let _ = window.eval("window.dispatchEvent(new Event('hireferry-app-return'))");
                }
            });
        }
        if matches!(event, tauri::WindowEvent::Destroyed)
            && handle
                .webview_windows()
                .keys()
                .all(|label| !label.starts_with("workbench-") || *label == closed_label)
        {
            show_launcher(&handle);
        }
    });
    workbench.on_menu_event(move |window, event| {
        let Some(webview) = window.get_webview_window(window.label()) else {
            return;
        };
        match event.id().as_ref() {
            "home" => {
                let _ = webview.navigate(home.clone());
            }
            "reload" => {
                let _ = webview.reload();
            }
            "settings" => show_launcher(window.app_handle()),
            _ => {}
        }
    });
    for old in old_windows {
        let _ = old.destroy();
    }
    window.hide().map_err(|_| "window_failed".to_owned())?;
    Ok(())
}

fn navigate_later(app: &AppHandle, label: &str, url: Url) {
    let app = app.clone();
    let label = label.to_owned();
    tauri::async_runtime::spawn(async move {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.navigate(url);
        }
    });
}

pub fn run() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("hireferry-desktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let smoke_test = args.iter().any(|arg| arg == "--smoke-test");
    tauri::Builder::default()
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .invoke_handler(tauri::generate_handler![connect, crate::open_browser])
        .setup(move |app| {
            let app_menu = SubmenuBuilder::new(app, "HireFerry")
                .text("settings", "Connection / 连接设置")
                .separator()
                .quit()
                .build()?;
            let edit_menu = SubmenuBuilder::new(app, "Edit / 编辑")
                .undo()
                .redo()
                .separator()
                .cut()
                .copy()
                .paste()
                .select_all()
                .build()?;
            let view_menu = SubmenuBuilder::new(app, "View / 视图")
                .text("home", "Home / 首页")
                .text("reload", "Reload / 刷新")
                .separator()
                .fullscreen()
                .build()?;
            let menu = MenuBuilder::new(app)
                .items(&[&app_menu, &edit_menu, &view_menu])
                .build()?;
            app.set_menu(menu)?;
            let handle = app.handle().clone();
            let launcher =
                WebviewWindowBuilder::new(app, "launcher", WebviewUrl::App("index.html".into()))
                    .title("HireFerry · 聘渡")
                    .inner_size(920.0, 700.0)
                    .min_inner_size(640.0, 580.0)
                    .center()
                    .on_navigation(policy::launcher_url)
                    .on_new_window(|_, _| NewWindowResponse::Deny)
                    .on_page_load(move |_, event| {
                        if smoke_test && matches!(event.event(), PageLoadEvent::Finished) {
                            println!("HireFerry desktop smoke test: launcher loaded");
                            handle.exit(0);
                        }
                    })
                    .build()?;
            let connection_handle = app.handle().clone();
            launcher.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event
                    && connection_handle
                        .webview_windows()
                        .keys()
                        .any(|label| label.starts_with("workbench-"))
                {
                    // Keep connection settings available from the menu while
                    // the user continues working in the remote window.
                    api.prevent_close();
                    if let Some(window) = connection_handle.get_webview_window("launcher") {
                        let _ = window.hide();
                    }
                }
            });
            launcher.on_menu_event(|window, event| {
                if event.id().as_ref() == "settings" {
                    show_launcher(window.app_handle());
                }
            });
            if smoke_test {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(20));
                    eprintln!("desktop smoke test timed out before the launcher loaded");
                    handle.exit(1);
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run HireFerry desktop");
}
