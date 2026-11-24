//! 设置与配方。

use tauri::AppHandle;

use super::{blocking, config_dir};
use crate::ffmpeg::encoders;
use crate::model::settings::SETTINGS_FILE;
use crate::model::{Recipe, Settings};

/// 读取设置；文件不存在时返回默认值，损坏时返回错误（前端会改用默认值并提示）。
/// “全部完成后”每次启动都恢复为“无操作”（`Settings::load_for_startup`）。
#[tauri::command]
pub async fn load_settings(app: AppHandle) -> Result<Settings, String> {
    let path = config_dir(&app)?.join(SETTINGS_FILE);
    let settings = blocking(move || Settings::load_for_startup(&path)).await??;
    apply_runtime(&app, &settings).await;
    Ok(settings)
}

/// 保存设置：先让设置生效，再写入文件。
#[tauri::command]
pub async fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    apply_runtime(&app, &settings).await;
    let path = config_dir(&app)?.join(SETTINGS_FILE);
    blocking(move || settings.save(&path)).await?
}

/// 让需要在后端即时生效的设置生效：核显优先（独立工具也据此选编码器）、托盘。
///
/// 在主线程执行并等它完成：Tauri 创建托盘时会自己转到主线程，但移除托盘只是丢弃图标对象，
/// 在 Windows 上会销毁托盘的隐藏窗口，只能在创建它的主线程里做。
async fn apply_runtime(app: &AppHandle, settings: &Settings) {
    let (tx, rx) = std::sync::mpsc::channel();
    let main_app = app.clone();
    let prefer_integrated = settings.prefer_integrated_gpu;
    let tray = settings.tray;
    let dispatched = app.run_on_main_thread(move || {
        encoders::set_prefer_integrated(prefer_integrated);
        if let Err(e) = super::system::sync_tray(&main_app, tray) {
            eprintln!("{e}");
        }
        let _ = tx.send(());
    });
    match dispatched {
        Ok(()) => {
            let _ = blocking(move || rx.recv()).await;
        }
        Err(e) => eprintln!("无法应用设置：{e}"),
    }
}

/// 默认配方（“参数重置”使用）。
#[tauri::command]
pub fn default_recipe() -> Recipe {
    Recipe::default()
}

/// 数据目录（配置、日志）。
#[tauri::command]
pub fn data_dir(app: AppHandle) -> Result<String, String> {
    Ok(config_dir(&app)?.to_string_lossy().into_owned())
}
