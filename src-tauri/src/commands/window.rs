//! 主窗口：首帧后显示、标题栏配色、任务栏进度、启动时带的文件（实现见 `crate::desktop`）。
//!
//! 事件：`open-paths`：再次启动程序时命令行里的文件（路径数组），前端按拖入窗口处理。

use tauri::{AppHandle, WebviewWindow};

use crate::desktop::{self, WindowChrome};

/// 前端已画好第一帧：第一次调用时显示主窗口（窗口隐藏创建，避免白屏与主题闪烁）。
#[tauri::command]
pub fn window_ready(app: AppHandle) {
    desktop::show_first_time(&app);
}

/// 按应用主题设置窗口主题、底色与标题栏颜色（启动时与主题变化时调用）。
#[tauri::command]
pub fn set_window_chrome(window: WebviewWindow, chrome: WindowChrome) -> Result<(), String> {
    desktop::apply_chrome(&window, &chrome)
}

/// 任务栏按钮上的进度（Windows）：`state` 为 none / normal / indeterminate / paused / error，`progress` 为 0～100。
#[tauri::command]
pub fn set_taskbar_progress(
    window: WebviewWindow,
    state: String,
    progress: Option<u64>,
) -> Result<(), String> {
    desktop::set_taskbar_progress(&window, &state, progress)
}

/// 首次启动时命令行带的文件（“打开方式”、拖到程序图标上）；只返回一次。
#[tauri::command]
pub fn take_launch_paths(app: AppHandle) -> Vec<String> {
    desktop::take_launch_paths(&app)
}
