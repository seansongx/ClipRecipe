//! 设置与配方。

use tauri::AppHandle;

use super::config_dir;
use crate::ffmpeg::encoders;
use crate::model::settings::SETTINGS_FILE;
use crate::model::{Recipe, Settings};

/// 读取设置；文件不存在时返回默认值，损坏时返回错误（前端会改用默认值并提示）。
/// “全部完成后”每次启动都恢复为“无操作”（`Settings::load_for_startup`）。
#[tauri::command]
pub fn load_settings(app: AppHandle) -> Result<Settings, String> {
    let settings = Settings::load_for_startup(&config_dir(&app)?.join(SETTINGS_FILE))?;
    apply_runtime(&app, &settings);
    Ok(settings)
}

#[tauri::command]
pub fn save_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    apply_runtime(&app, &settings);
    settings.save(&config_dir(&app)?.join(SETTINGS_FILE))
}

/// 让需要在后端即时生效的设置生效：核显优先（独立工具也据此选编码器）、托盘。
fn apply_runtime(app: &AppHandle, settings: &Settings) {
    encoders::set_prefer_integrated(settings.prefer_integrated_gpu);
    if let Err(e) = super::system::sync_tray(app, settings.tray) {
        eprintln!("{e}");
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
