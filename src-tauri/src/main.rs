#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{path::PathBuf, sync::Arc};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, State,
};
use tauri_plugin_dialog::DialogExt;
use xproxy::shutdown::{ExitDecision, ShutdownGate};
use xproxy::{
    model::{Config, Status},
    supervisor::{Controller, Timing},
};
type AppController = Arc<Controller>;

#[tauri::command]
async fn get_config(c: State<'_, AppController>) -> Result<Config, String> {
    Ok(c.config().await)
}
#[tauri::command]
async fn get_status(c: State<'_, AppController>) -> Result<Status, String> {
    Ok(c.status().await)
}
#[tauri::command]
async fn save_config(c: State<'_, AppController>, config: Config) -> Result<Config, String> {
    c.save(config).await.map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn set_enabled(c: State<'_, AppController>, enabled: bool) -> Result<(), String> {
    c.set_enabled(enabled).await.map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn retry_now(c: State<'_, AppController>) -> Result<(), String> {
    c.retry_now().await.map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn launch_browser(
    c: State<'_, AppController>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if !matches!(c.status().await.phase, xproxy::model::Phase::Connected) {
        return Err("请等待 SSH 隧道连接成功".into());
    }
    let config = c.config().await;
    let dir = data_dir(&app).map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || xproxy::browser::launch(&dir, config.socks_port))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}
fn data_dir(app: &tauri::AppHandle) -> anyhow::Result<PathBuf> {
    // Local test runs can be isolated from real user configuration.
    if let Some(dir) = std::env::var_os("XPROXY_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    Ok(app.path().app_data_dir()?)
}
fn main() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--ssh-guardian")) {
        let Some(program) = args.next() else {
            std::process::exit(2)
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("guardian runtime");
        let code = runtime.block_on(xproxy::ssh::guardian(program, args.collect()));
        // Tokio stdin may have an outstanding blocking read. Terminate without waiting on it.
        std::process::exit(code);
    }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_config,
            get_status,
            save_config,
            set_enabled,
            retry_now,
            launch_browser
        ])
        .setup(|app| {
            let dir = data_dir(app.handle())?;
            let driver = Arc::new(xproxy::ssh::SshDriver { dir: dir.clone() });
            let proxy = Arc::new(xproxy::system::SystemProxy::new(dir.clone()));
            let controller = Arc::new(Controller::new_with(dir, driver, proxy, Timing::default())?);
            app.manage(controller);
            let show = MenuItem::with_id(app, "show", "打开 XProxy Tunnel", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new().menu(&menu).tooltip("XProxy Tunnel");
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            app.manage(
                tray.on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?,
            );
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("Cannot initialize XProxy Tunnel");
    let exiting = Arc::new(ShutdownGate::default());
    app.run(move |handle, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            match exiting.request() {
                ExitDecision::Allow => return,
                ExitDecision::Wait => {
                    api.prevent_exit();
                    return;
                }
                ExitDecision::Begin => api.prevent_exit(),
            }
            let handle = handle.clone();
            let exiting = exiting.clone();
            tauri::async_runtime::spawn(async move {
                let controller = handle.state::<AppController>();
                if let Err(e) = controller.set_enabled(false).await {
                    if controller.pending_recovery().await {
                        exiting.retry();
                        handle
                            .dialog()
                            .message(format!(
                                "系统代理尚未恢复，应用将保持打开。请处理授权后再次退出。\n\n{e:#}"
                            ))
                            .title("代理恢复失败")
                            .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                            .show(|_| {});
                        return;
                    }
                }
                exiting.cleaned();
                handle.exit(0);
            });
        }
    });
}
