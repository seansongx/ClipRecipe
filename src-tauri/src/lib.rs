//! ClipRecipe 后端。目录约定见 docs/PLAN.md。

pub mod commands;
pub mod ffmpeg;
pub mod fsutil;
pub mod jobs;
pub mod model;
pub mod net;
pub mod pipeline;
pub mod preview;
pub mod system;
pub mod tools;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::default())
        .manage(commands::system::SystemState::default())
        .setup(|app| {
            if let Err(e) = commands::preview::setup(app.handle()) {
                eprintln!("预览目录初始化失败：{e}");
            }
            // 应用内下载的 FFmpeg 放在本地数据目录（CLIPRECIPE_CONFIG_DIR 优先，见 ffmpeg::managed::root）
            match tauri::Manager::path(app).app_local_data_dir() {
                Ok(dir) => ffmpeg::managed::set_default_root(dir.join("ffmpeg")),
                Err(e) => eprintln!("无法确定本地数据目录，不能下载 FFmpeg：{e}"),
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // set.tray：关闭主窗口时隐藏到托盘
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if commands::system::hide_instead_of_close(tauri::Manager::app_handle(window)) {
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::app::ffmpeg_status,
            commands::app::encoder_info,
            commands::ffmpeg::download_ffmpeg,
            commands::ffmpeg::cancel_ffmpeg_download,
            commands::ffmpeg::remove_managed_ffmpeg,
            commands::ffmpeg::redetect_ffmpeg,
            commands::media::import_paths,
            commands::media::probe_media,
            commands::settings::load_settings,
            commands::settings::save_settings,
            commands::settings::default_recipe,
            commands::settings::data_dir,
            commands::recipe::save_recipe_file,
            commands::recipe::load_recipe_file,
            commands::jobs::start_jobs,
            commands::jobs::stop_jobs,
            commands::jobs::cancel_task,
            commands::jobs::prioritize_task,
            commands::jobs::jobs_running,
            commands::jobs::preview_command,
            commands::jobs::check_output_dir,
            commands::preview::render_preview,
            commands::preview::cancel_preview,
            commands::preview::extract_frame,
            commands::preview::release_preview_file,
            commands::preview::allow_media_file,
            commands::tools::start_tool,
            commands::tools::stop_tool,
            commands::tools::cancel_tool_item,
            commands::tools::tool_running,
            commands::tools::render_tool_preview,
            commands::fonts::list_system_fonts,
            commands::queue::save_queue,
            commands::queue::restore_queue,
            commands::queue::find_sidecars,
            commands::system::schedule_shutdown,
            commands::system::cancel_shutdown,
            commands::system::quit_app,
            commands::system::open_folder,
            commands::system::open_data_dir,
            commands::system::open_project_page,
            commands::system::check_update,
            commands::system::allow_sound_file,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                commands::preview::cleanup(app);
            }
        });
}
