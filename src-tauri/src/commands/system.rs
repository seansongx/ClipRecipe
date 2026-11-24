//! 系统相关命令：全部完成后关机 / 退出（`set.after-all`）、打开文件夹与项目页面、检查更新与自动更新、
//! 托盘（`set.tray`）、提示音文件（`set.sound`）。
//!
//! 事件：`shutdown-countdown`：`{ remaining }`（剩余秒数；0 表示正在关机）；自动更新的事件见 [`install_update`]。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};

use super::{blocking, config_dir};
use crate::desktop::show_main;
use crate::fsutil::has_extension;
use crate::system::{
    self, Countdown, DownloadProgress, UpdateFlow, UpdateInfo, SHUTDOWN_COUNTDOWN_SECS,
};
use tauri_plugin_updater::{Update, UpdaterExt};

const TRAY_ID: &str = "main-tray";
const MAIN_WINDOW: &str = "main";

/// 系统相关的全局状态（单独托管，不放进 `AppState`）。
#[derive(Default)]
pub struct SystemState {
    countdown: Mutex<Option<Countdown>>,
    /// 设置 `set.tray`：关闭窗口时隐藏到托盘
    close_to_tray: AtomicBool,
}

#[derive(Clone, Serialize)]
struct CountdownTick {
    remaining: u32,
}

/// 开始关机倒计时（30 秒，可用 [`cancel_shutdown`] 取消）。已有倒计时时重新开始。
#[tauri::command]
pub fn schedule_shutdown(app: AppHandle, state: State<'_, SystemState>) -> Result<(), String> {
    let mut slot = state.countdown.lock().map_err(|e| e.to_string())?;
    if let Some(old) = slot.take() {
        old.cancel();
    }
    let tick_app = app.clone();
    let fire_app = app.clone();
    *slot = Some(Countdown::start(
        SHUTDOWN_COUNTDOWN_SECS,
        Duration::from_secs(1),
        move |remaining| {
            let _ = tick_app.emit("shutdown-countdown", CountdownTick { remaining });
        },
        move || {
            if let Err(e) = system::shutdown_now() {
                let _ = fire_app.emit("shutdown-failed", e);
            }
        },
    ));
    Ok(())
}

/// 取消关机倒计时。
#[tauri::command]
pub fn cancel_shutdown(state: State<'_, SystemState>) -> Result<(), String> {
    if let Some(countdown) = state.countdown.lock().map_err(|e| e.to_string())?.take() {
        countdown.cancel();
    }
    Ok(())
}

/// 退出程序（`set.after-all` 为“退出程序”，或托盘菜单“退出”）。
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    crate::desktop::quit(&app);
}

/// 要打开的文件夹：文件夹本身，或文件所在的文件夹。
fn folder_to_open(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        return Some(path.to_path_buf());
    }
    path.parent().filter(|p| p.is_dir()).map(Path::to_path_buf)
}

/// 用系统文件管理器打开文件夹（传入文件时打开其所在文件夹）。
#[tauri::command]
pub fn open_folder(path: String) -> Result<(), String> {
    let dir =
        folder_to_open(Path::new(path.trim())).ok_or_else(|| format!("文件夹不存在：{path}"))?;
    tauri_plugin_opener::open_path(&dir, None::<&str>).map_err(|e| format!("无法打开文件夹：{e}"))
}

/// 打开数据目录（配置、日志）。
#[tauri::command]
pub fn open_data_dir(app: AppHandle) -> Result<(), String> {
    let dir = config_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建数据目录：{e}"))?;
    tauri_plugin_opener::open_path(&dir, None::<&str>).map_err(|e| format!("无法打开数据目录：{e}"))
}

/// 在浏览器中打开本项目的页面（只接受 `system::REPO_URL` 下的地址）。
#[tauri::command]
pub fn open_project_page(url: Option<String>) -> Result<(), String> {
    let url = url.unwrap_or_else(|| system::REPO_URL.to_owned());
    if !system::is_project_url(&url) {
        return Err("只能打开本项目的页面".into());
    }
    tauri_plugin_opener::open_url(&url, None::<&str>).map_err(|e| format!("无法打开浏览器：{e}"))
}

/// 自动更新的状态（单独托管）：状态流转、检查到的待装更新、正在进行的下载。
#[derive(Default)]
pub struct UpdateState {
    flow: Mutex<UpdateFlow>,
    pending: Mutex<Option<Update>>,
    download: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

#[derive(Clone, Serialize)]
struct UpdateProgress {
    downloaded: u64,
    total: Option<u64>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// 本程序 `tauri.conf.json` 里的更新公钥（`plugins.updater.pubkey`）。
fn configured_pubkey(app: &AppHandle) -> Option<String> {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|v| v.get("pubkey"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
}

/// 程序退出安装新版本前（Windows 上更新插件直接结束进程）：保存窗口位置、清理预览临时文件。
fn before_update_exit(app: &AppHandle) {
    use tauri_plugin_window_state::AppHandleExt;
    let _ = app.save_window_state(crate::desktop::window_state_flags());
    super::preview::cleanup(app);
}

/// 用更新插件检查 `latest.json`（见 `tauri.conf.json` 的 `plugins.updater.endpoints`）。
async fn check_with_updater(app: &AppHandle) -> Result<Option<Update>, String> {
    let exit_app = app.clone();
    app.updater_builder()
        .on_before_exit(move || before_update_exit(&exit_app))
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())
}

/// 检查更新（离线时返回错误，前端静默或提示）。
///
/// 能自动更新时（配置了公钥，且是用 NSIS 安装包或 AppImage 安装的）读 Release 里的 `latest.json`，记下待装的更新；
/// 否则、或读取失败时，改为读 GitHub 公开 API 的最新发布，只提示并打开发布页（`auto` 为 false，`manualReason` 说明原因）。
#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let pubkey = configured_pubkey(&app);
    let mut reason = system::manual_update_reason(
        system::InstallKind::current(),
        system::pubkey_configured(pubkey.as_deref()),
    )
    .map(str::to_owned);

    if reason.is_none() {
        match check_with_updater(&app).await {
            Ok(found) => {
                let info = match &found {
                    Some(update) => UpdateInfo {
                        current: current.clone(),
                        latest: update.version.clone(),
                        newer: true,
                        url: system::release_page(&update.version),
                        notes: system::clean_notes(update.body.as_deref()),
                        auto: true,
                        manual_reason: None,
                    },
                    None => UpdateInfo {
                        latest: current.clone(),
                        newer: false,
                        url: system::release_page(&current),
                        notes: None,
                        auto: true,
                        manual_reason: None,
                        current,
                    },
                };
                if lock(&state.flow).checked(found.is_some()) {
                    *lock(&state.pending) = found;
                }
                return Ok(info);
            }
            Err(e) => reason = Some(format!("无法读取自动更新信息（{e}），请到发布页下载")),
        }
    }

    let version = current.clone();
    let mut info = blocking(move || system::check_update(&version, system::http_get)).await??;
    if lock(&state.flow).checked(false) {
        *lock(&state.pending) = None;
    }
    info.manual_reason = reason;
    Ok(info)
}

/// 视频处理或工具正在运行时不安装（安装会结束程序）。
fn busy(app: &AppHandle) -> Result<bool, String> {
    let state = app.state::<super::AppState>();
    Ok(super::jobs::jobs_running(app.state())? || super::tools::is_running(&state)?)
}

/// 下载并安装检查到的更新，然后重启程序。立即返回；进度与结果通过事件通知：
/// `update-progress`：`{ downloaded, total }`（字节；total 未知时为 null）；`update-installing`（下载并校验完成，
/// 开始安装，之后程序会退出并重启）；`update-failed`：错误说明（下载、校验或安装失败，可以重试或打开发布页）。
#[tauri::command]
pub fn install_update(app: AppHandle, state: State<'_, UpdateState>) -> Result<(), String> {
    if busy(&app)? {
        return Err("正在处理视频，请等全部任务结束后再更新".into());
    }
    let update = lock(&state.pending)
        .clone()
        .ok_or("没有可以安装的更新，请先检查更新")?;
    lock(&state.flow).start_download()?;

    let task_app = app.clone();
    let handle = tauri::async_runtime::spawn(async move {
        let app = task_app;
        let state = app.state::<UpdateState>();
        let mut progress = DownloadProgress::default();
        let progress_app = app.clone();
        let result = update
            .download(
                |chunk, total| {
                    if progress.add(chunk, total) {
                        let _ = progress_app.emit(
                            "update-progress",
                            UpdateProgress {
                                downloaded: progress.downloaded,
                                total: progress.total,
                            },
                        );
                    }
                },
                || {},
            )
            .await;
        // 已被取消：不再安装
        if !lock(&state.flow).download_finished(result.is_ok()) {
            return;
        }
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(e) => {
                let _ = app.emit("update-failed", format!("下载更新失败：{e}"));
                return;
            }
        };
        let _ = app.emit("update-installing", ());
        // Windows：启动安装程序后由插件结束本进程，安装完成后安装程序重新打开 ClipRecipe
        match update.install(bytes) {
            Ok(()) => {
                before_update_exit(&app);
                app.restart();
            }
            Err(e) => {
                lock(&state.flow).install_failed();
                let _ = app.emit("update-failed", format!("安装更新失败：{e}"));
            }
        }
    });
    *lock(&state.download) = Some(handle);
    Ok(())
}

/// 取消正在进行的下载（安装开始后不能取消），返回是否取消了。
#[tauri::command]
pub fn cancel_update(state: State<'_, UpdateState>) -> bool {
    if !lock(&state.flow).cancel() {
        return false;
    }
    if let Some(handle) = lock(&state.download).take() {
        handle.abort();
    }
    true
}

/// 允许 WebView 读取自定义提示音（只接受存在的 wav 文件），返回原路径。
#[tauri::command]
pub fn allow_sound_file(app: AppHandle, path: String) -> Result<String, String> {
    let file = PathBuf::from(&path);
    if !file.is_file() {
        return Err(format!("提示音文件不存在：{path}"));
    }
    if !has_extension(&file, &["wav"]) {
        return Err("提示音只支持 wav 文件".into());
    }
    app.asset_protocol_scope()
        .allow_file(&file)
        .map_err(|e| format!("无法访问提示音文件：{e}"))?;
    Ok(path)
}

/// 关闭主窗口时是否改为隐藏到托盘：开启了 `set.tray` 且托盘图标确实存在（否则窗口隐藏后找不回来）。
pub fn should_hide_on_close(close_to_tray: bool, tray_exists: bool) -> bool {
    close_to_tray && tray_exists
}

/// 主窗口的关闭请求：需要隐藏到托盘时返回 true（调用方据此阻止关闭）。
pub fn hide_instead_of_close(app: &AppHandle) -> bool {
    let enabled = app
        .try_state::<SystemState>()
        .is_some_and(|s| s.close_to_tray.load(Ordering::SeqCst));
    if !should_hide_on_close(enabled, app.tray_by_id(TRAY_ID).is_some()) {
        return false;
    }
    app.get_webview_window(MAIN_WINDOW)
        .is_some_and(|w| w.hide().is_ok())
}

/// 按设置创建或移除托盘图标（菜单：显示主窗口 / 退出）。
pub fn sync_tray(app: &AppHandle, enabled: bool) -> Result<(), String> {
    if let Some(state) = app.try_state::<SystemState>() {
        state.close_to_tray.store(enabled, Ordering::SeqCst);
    }
    let exists = app.tray_by_id(TRAY_ID).is_some();
    if enabled == exists {
        return Ok(());
    }
    if !enabled {
        app.remove_tray_by_id(TRAY_ID);
        return Ok(());
    }
    let build = || -> tauri::Result<()> {
        let show = MenuItem::with_id(app, "tray-show", "显示主窗口", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "tray-quit", "退出", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&show, &quit])?;
        let mut builder = TrayIconBuilder::with_id(TRAY_ID)
            .tooltip("ClipRecipe")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id.as_ref() {
                "tray-show" => show_main(app),
                "tray-quit" => crate::desktop::quit(app),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_main(tray.app_handle());
                }
            });
        if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
        builder.build(app)?;
        Ok(())
    };
    build().map_err(|e| format!("无法创建托盘图标：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_to_tray_needs_setting_and_icon() {
        assert!(should_hide_on_close(true, true));
        assert!(!should_hide_on_close(true, false));
        assert!(!should_hide_on_close(false, true));
    }

    #[test]
    fn folder_for_file_is_its_parent() {
        let dir = std::env::temp_dir();
        assert_eq!(folder_to_open(&dir), Some(dir.clone()));
        let file = dir.join(format!("cr-open-{}.txt", std::process::id()));
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(folder_to_open(&file), Some(dir.clone()));
        std::fs::remove_file(&file).unwrap();
        assert_eq!(
            folder_to_open(Path::new("Z:/definitely/missing/x.mp4")),
            None
        );
    }
}
