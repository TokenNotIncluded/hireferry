mod policy;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

use tauri::{AppHandle, WebviewWindow};
use tauri_plugin_opener::OpenerExt;
use url::Url;

fn require_launcher(window: &WebviewWindow) -> Result<(), String> {
    // The local-only capability is the first boundary; keep an origin check as
    // defense in depth even though mobile reuses the same window for the site.
    if window.label() != "launcher" || !window.url().is_ok_and(|url| policy::launcher_url(&url)) {
        return Err("not_allowed".into());
    }
    Ok(())
}

fn open_external(app: &AppHandle, url: &Url) -> Result<(), String> {
    if !policy::web_url(url) {
        return Err("invalid_server".into());
    }
    // The registered opener plugin supplies Android/iOS implementations. No
    // opener command or remote capability is exposed to the website itself.
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| "browser_failed".into())
}

#[tauri::command]
async fn open_browser(window: WebviewWindow, app: AppHandle, server: String) -> Result<(), String> {
    require_launcher(&window)?;
    let url = policy::server_origin(&server).map_err(str::to_owned)?;
    open_external(&app, &url)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(desktop)]
    desktop::run();
    #[cfg(mobile)]
    mobile::run();
}
