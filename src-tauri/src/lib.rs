//! ClipRecipe 后端。目录约定见 docs/architecture.md。

pub mod commands;
pub mod desktop;
pub mod ffmpeg;
pub mod fsutil;
pub mod jobs;
pub mod model;
pub mod net;
pub mod pipeline;
pub mod preview;
pub mod system;
pub mod tools;

/// 窗口位置与大小保存在配置目录（设置了 `CLIPRECIPE_CONFIG_DIR` 时在它里面，端到端测试因此互不干扰）。
const WINDOW_STATE_FILE: &str = "window-state.json";

/// 端到端测试（只在 `e2e-hooks` 调试构建中存在）：设置后即使配置目录被替换也启用单实例，并改用专用的应用标识，
/// 不会与用户正在运行的 ClipRecipe 互相激活。
#[cfg(feature = "e2e-hooks")]
const E2E_INSTANCE_ENV: &str = "CLIPRECIPE_E2E_SINGLE_INSTANCE";

fn config_dir_override() -> Option<std::path::PathBuf> {
    std::env::var_os(commands::CONFIG_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
}

/// 是否启用单实例。设置了 `CLIPRECIPE_CONFIG_DIR`（独立的一套配置，例如端到端测试）时视为另一个实例，不启用。
fn single_instance<R: tauri::Runtime>(context: &mut tauri::Context<R>) -> bool {
    #[cfg(feature = "e2e-hooks")]
    if std::env::var_os(E2E_INSTANCE_ENV).is_some_and(|v| v == "1") {
        let id = format!("{}.e2e", context.config().identifier);
        context.config_mut().identifier = id;
        return true;
    }
    let _ = context;
    config_dir_override().is_none()
}

fn window_state_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    // 插件把文件名接在系统配置目录后面；给绝对路径时就是这个路径
    let file = match config_dir_override() {
        Some(dir) => {
            // 插件只创建系统配置目录，替换的配置目录要先建好，否则保存失败
            let _ = std::fs::create_dir_all(&dir);
            dir.join(WINDOW_STATE_FILE).to_string_lossy().into_owned()
        }
        None => WINDOW_STATE_FILE.to_owned(),
    };
    tauri_plugin_window_state::Builder::default()
        .with_state_flags(desktop::window_state_flags())
        .with_filename(file)
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut context = tauri::generate_context!();
    let mut builder = tauri::Builder::default();
    // 单实例插件必须最先注册
    if single_instance(&mut context) {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            desktop::on_second_instance(app, argv, cwd)
        }));
    }
    builder
        .plugin(window_state_plugin())
        .plugin(tauri_plugin_dialog::init())
        // 自动更新（只在 Rust 侧调用，没有授权给前端；见 commands::system::check_update）
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(commands::AppState::default())
        .manage(commands::system::SystemState::default())
        .manage(commands::system::UpdateState::default())
        .manage(desktop::DesktopState::default())
        .setup(|app| {
            if let Err(e) = commands::preview::setup(app.handle()) {
                eprintln!("预览目录初始化失败：{e}");
            }
            // 应用内下载的 FFmpeg 放在本地数据目录（CLIPRECIPE_CONFIG_DIR 优先，见 ffmpeg::managed::root）
            match tauri::Manager::path(app).app_local_data_dir() {
                Ok(dir) => ffmpeg::managed::set_default_root(dir.join("ffmpeg")),
                Err(e) => eprintln!("无法确定本地数据目录，不能下载 FFmpeg：{e}"),
            }
            desktop::remember_launch_paths(app.handle());
            // 主窗口不由配置自动创建（tauri.conf.json 中 create: false）：先隐藏创建，首帧画好后再显示
            desktop::create_main_window(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // set.tray：关闭主窗口时隐藏到托盘；正在处理时先让用户确认
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if commands::system::on_close_requested(tauri::Manager::app_handle(window)) {
                    api.prevent_close();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::app::ffmpeg_status,
            commands::app::encoder_info,
            commands::app::encoder_info_cached,
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
            commands::system::close_prompt_shown,
            commands::system::close_prompt_dismissed,
            commands::system::open_folder,
            commands::system::open_data_dir,
            commands::system::open_project_page,
            commands::system::check_update,
            commands::system::install_update,
            commands::system::cancel_update,
            commands::system::allow_sound_file,
            commands::window::window_ready,
            commands::window::set_window_chrome,
            commands::window::set_taskbar_progress,
            commands::window::take_launch_paths,
        ])
        .build(context)
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // 先结束 ffmpeg / whisper-cli 子进程，再删预览目录（预览的 ffmpeg 可能还在写）
                commands::stop_all_work(app);
                commands::preview::cleanup(app);
            }
        });
}
