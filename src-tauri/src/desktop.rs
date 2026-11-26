//! 桌面窗口体验：主窗口的创建（隐藏创建、首帧画好后再显示，避免白屏）、标题栏配色、
//! 任务栏进度、单实例、窗口位置与大小的记忆。前端接口见 `commands::window`，约定见 docs/architecture/frontend.md“桌面应用体验约定”。
//!
//! 应用只有深色主题：窗口主题固定为深色（系统标题栏、WebView 的 prefers-color-scheme 都是深色，不随系统切换）。
//! 标题栏：保留系统窗口框架（贴靠布局、缩放边框、阴影、圆角、系统菜单、双击最大化都由系统提供），
//! 只把颜色改成应用的颜色。Windows 11（22000+）用 DWM 的 `DWMWA_CAPTION_COLOR` / `DWMWA_TEXT_COLOR` /
//! `DWMWA_BORDER_COLOR`；Windows 10 与其它平台只有系统的深色标题栏。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use tauri::window::{Color, ProgressBarState, ProgressBarStatus};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, Runtime, WebviewWindow};

pub const MAIN_WINDOW: &str = "main";

/// 前端没有在这段时间内报告“首帧已画好”（脚本出错、调试时断点）时也显示窗口，免得窗口永远不出现。
const SHOW_FALLBACK: Duration = Duration::from_secs(6);

/// 窗口显示前与调整大小时露出的底色：`src/index.css` 的 `--background`（oklch）换算成的 sRGB（单元测试核对）。
/// 前端加载后会用变量的实际值再设置一次（`set_window_chrome`）。
pub const BOOT_BACKGROUND: &str = "#0f1012";

/// 第二次启动时转交给已运行实例的文件（事件 `open-paths`，内容为路径数组）。
pub const OPEN_PATHS_EVENT: &str = "open-paths";

/// 桌面窗口的状态（Tauri 托管）。
#[derive(Default)]
pub struct DesktopState {
    /// 主窗口是否已经显示过（之后页面重新加载不再重复显示，托盘隐藏的窗口保持隐藏）
    shown: AtomicBool,
    /// 首次启动时命令行带的文件（前端启动完成后取走一次）
    launch_paths: Mutex<Vec<String>>,
}

/// `#rrggbb` / `#rgb` → (r, g, b)。
pub fn parse_hex(color: &str) -> Option<(u8, u8, u8)> {
    let hex = color.trim().strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => {
            let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
            Some((byte(0)?, byte(2)?, byte(4)?))
        }
        3 => {
            let nibble = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok().map(|v| v * 17);
            Some((nibble(0)?, nibble(1)?, nibble(2)?))
        }
        _ => None,
    }
}

fn color_of(hex: &str) -> Option<Color> {
    parse_hex(hex).map(|(r, g, b)| Color(r, g, b, 255))
}

/// 命令行参数里的文件 / 文件夹（相对路径按 `cwd` 解析，只保留存在的；以 `-` 开头的参数忽略）。
pub fn launch_paths(args: &[String], cwd: &Path) -> Vec<String> {
    args.iter()
        .filter(|a| !a.is_empty() && !a.starts_with('-'))
        .map(|a| {
            let p = PathBuf::from(a);
            if p.is_absolute() {
                p
            } else {
                cwd.join(p)
            }
        })
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// 创建主窗口：先隐藏，设为深色窗口主题与深色底色，恢复上次的位置大小（window-state 插件在创建时完成），
/// 再确认标题栏在某个显示器上。前端画好第一帧后调用 `window_ready` 显示。
pub fn create_main_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == MAIN_WINDOW)
        .cloned()
        .unwrap_or_default();
    let window = tauri::WebviewWindowBuilder::from_config(app, &config)?
        .visible(false)
        .theme(Some(tauri::Theme::Dark))
        .build()?;

    if let Some(color) = color_of(BOOT_BACKGROUND) {
        let _ = window.set_background_color(Some(color));
    }
    #[cfg(windows)]
    windows_chrome::set_dark(&window);
    keep_title_bar_reachable(&window);
    configure_webview(&window);

    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_FALLBACK);
        show_first_time(&handle);
    });
    Ok(window)
}

/// 保存的位置所在的显示器已拔掉、或标题栏落在屏幕外时，把窗口移回主显示器中央。
fn keep_title_bar_reachable<R: Runtime>(window: &WebviewWindow<R>) {
    if window.is_maximized().unwrap_or(false) || window.is_fullscreen().unwrap_or(false) {
        return;
    }
    let (Ok(pos), Ok(size), Ok(monitors)) = (
        window.outer_position(),
        window.outer_size(),
        window.available_monitors(),
    ) else {
        return;
    };
    let reachable = monitors.iter().any(|m| {
        let mp = m.position();
        let ms = m.size();
        title_bar_visible(
            (pos.x, pos.y, size.width as i32),
            (mp.x, mp.y, ms.width as i32, ms.height as i32),
        )
    });
    if !reachable {
        let _ = window.center();
        if let Ok(p) = window.outer_position() {
            if p.y < 0 {
                let _ = window.set_position(PhysicalPosition::new(p.x, 0));
            }
        }
    }
}

/// 标题栏（窗口顶部一条，至少 120 像素宽、32 像素高）是否落在显示器 `(x, y, w, h)` 内。
/// 顶边允许超出 16 像素（Windows 的窗口外框包含不可见的缩放边框）。
pub fn title_bar_visible(window: (i32, i32, i32), monitor: (i32, i32, i32, i32)) -> bool {
    let (wx, wy, ww) = window;
    let (mx, my, mw, mh) = monitor;
    let overlap = (wx + ww).min(mx + mw) - wx.max(mx);
    let top_inside = wy >= my - 16 && wy + 32 <= my + mh;
    overlap >= 120.min(ww) && top_inside
}

/// WebView2 的浏览器行为：关闭表单自动填充与密码保存；正式版再关闭浏览器快捷键（刷新、查找、打印、
/// 光标浏览、开发者工具等，WebView2 的 `AreBrowserAcceleratorKeysEnabled`）。调试版保留快捷键（F12 开发者工具），
/// 刷新等由前端 `lib/desktop.ts` 拦截。缩放（Ctrl + 滚轮、Ctrl +/-、双指）与滑动前进后退 Tauri 默认已关闭。
fn configure_webview<R: Runtime>(window: &WebviewWindow<R>) {
    #[cfg(windows)]
    {
        let _ = window.with_webview(|webview| unsafe {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                ICoreWebView2Settings3, ICoreWebView2Settings4,
            };
            use windows::core::Interface;
            let Ok(core) = webview.controller().CoreWebView2() else {
                return;
            };
            let Ok(settings) = core.Settings() else {
                return;
            };
            if let Ok(s4) = settings.cast::<ICoreWebView2Settings4>() {
                let _ = s4.SetIsGeneralAutofillEnabled(false);
                let _ = s4.SetIsPasswordAutosaveEnabled(false);
            }
            if !cfg!(debug_assertions) {
                if let Ok(s3) = settings.cast::<ICoreWebView2Settings3>() {
                    let _ = s3.SetAreBrowserAcceleratorKeysEnabled(false);
                }
            }
        });
    }
    #[cfg(not(windows))]
    let _ = window;
}

/// 第一次显示主窗口（之后的调用不做任何事）。
pub fn show_first_time(app: &AppHandle) {
    let Some(state) = app.try_state::<DesktopState>() else {
        return;
    };
    if state.shown.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 显示并激活主窗口（托盘、第二次启动）：从最小化、托盘隐藏中恢复。
pub fn show_main(app: &AppHandle) {
    if let Some(state) = app.try_state::<DesktopState>() {
        state.shown.store(true, Ordering::SeqCst);
    }
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 记住的窗口状态：位置、大小、是否最大化（不含“是否可见”：窗口总是先隐藏创建，首帧画好后才显示）。
pub fn window_state_flags() -> tauri_plugin_window_state::StateFlags {
    use tauri_plugin_window_state::StateFlags;
    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED
}

/// 退出程序（托盘“退出”、`quit_app`）：先按窗口的当前状态保存位置与大小，再退出。
/// 插件在退出事件里也会保存，但那时窗口可能已经销毁，只能用移动事件里缓存的值。
pub fn quit(app: &AppHandle) {
    use tauri_plugin_window_state::AppHandleExt;
    if let Err(e) = app.save_window_state(window_state_flags()) {
        eprintln!("保存窗口位置失败：{e}");
    }
    app.exit(0);
}

/// 记下首次启动时命令行带的文件。
pub fn remember_launch_paths(app: &AppHandle) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_default();
    let paths = launch_paths(&args, &cwd);
    if let Some(state) = app.try_state::<DesktopState>() {
        if let Ok(mut slot) = state.launch_paths.lock() {
            *slot = paths;
        }
    }
}

/// 取走首次启动时的文件（只返回一次）。
pub fn take_launch_paths(app: &AppHandle) -> Vec<String> {
    app.try_state::<DesktopState>()
        .and_then(|s| {
            s.launch_paths
                .lock()
                .ok()
                .map(|mut v| std::mem::take(&mut *v))
        })
        .unwrap_or_default()
}

/// 单实例：再次启动程序时激活已运行的窗口，并把命令行里的文件交给它导入。
pub fn on_second_instance(app: &AppHandle, argv: Vec<String>, cwd: String) {
    show_main(app);
    let paths = launch_paths(argv.get(1..).unwrap_or_default(), Path::new(&cwd));
    if !paths.is_empty() {
        let _ = app.emit(OPEN_PATHS_EVENT, paths);
    }
}

/// 前端传来的窗口配色（`src/index.css` 颜色变量的实际值，`#rrggbb`）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowChrome {
    /// 标题栏底色（`--titlebar`）；标题栏、标题文字、边框三个颜色为空串时恢复系统默认（前端只在 Windows 平台着色）
    pub titlebar: String,
    /// 标题文字（`--foreground`）
    pub text: String,
    /// 窗口边框（`--border`）
    pub border: String,
    /// 窗口底色（`--background`）
    pub background: String,
}

/// 按应用配色设置窗口：底色，Windows 11 上再设置标题栏、标题文字与边框颜色（窗口主题创建时已固定为深色）。
pub fn apply_chrome<R: Runtime>(
    window: &WebviewWindow<R>,
    chrome: &WindowChrome,
) -> Result<(), String> {
    if let Some(color) = color_of(&chrome.background) {
        window
            .set_background_color(Some(color))
            .map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    windows_chrome::apply(window, chrome);
    Ok(())
}

/// 任务栏进度：`none` 清除、`normal` 普通（0～100）、`indeterminate` 不确定、`paused` 暂停、`error` 出错。
pub fn set_taskbar_progress<R: Runtime>(
    window: &WebviewWindow<R>,
    state: &str,
    progress: Option<u64>,
) -> Result<(), String> {
    let status = match state {
        "none" => ProgressBarStatus::None,
        "normal" => ProgressBarStatus::Normal,
        "indeterminate" => ProgressBarStatus::Indeterminate,
        "paused" => ProgressBarStatus::Paused,
        "error" => ProgressBarStatus::Error,
        other => return Err(format!("未知的任务栏进度状态：{other}")),
    };
    window
        .set_progress_bar(ProgressBarState {
            status: Some(status),
            progress: progress.map(|p| p.min(100)),
        })
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
mod windows_chrome {
    use super::{parse_hex, WindowChrome};
    use tauri::{Runtime, WebviewWindow};
    use windows::core::BOOL;
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
        DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE,
    };

    fn colorref(hex: &str) -> Option<COLORREF> {
        // COLORREF 是 0x00BBGGRR
        parse_hex(hex)
            .map(|(r, g, b)| COLORREF(u32::from(r) | u32::from(g) << 8 | u32::from(b) << 16))
    }

    fn set<T>(
        window_hwnd: windows::Win32::Foundation::HWND,
        attribute: DWMWINDOWATTRIBUTE,
        value: &T,
    ) {
        // Windows 10 不认识标题栏颜色属性，调用失败时什么也不做（系统标题栏仍是深色）
        let _ = unsafe {
            DwmSetWindowAttribute(
                window_hwnd,
                attribute,
                value as *const T as *const core::ffi::c_void,
                std::mem::size_of::<T>() as u32,
            )
        };
    }

    /// 深色标题栏（沉浸式深色模式）。tao 设置窗口主题时也会做，这里在窗口创建后立即做一次，首次显示就正确。
    pub fn set_dark<R: Runtime>(window: &WebviewWindow<R>) {
        if let Ok(hwnd) = window.hwnd() {
            set(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &BOOL::from(true));
        }
    }

    pub fn apply<R: Runtime>(window: &WebviewWindow<R>, chrome: &WindowChrome) {
        let Ok(hwnd) = window.hwnd() else {
            return;
        };
        set(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &color_or_default(&chrome.titlebar),
        );
        set(hwnd, DWMWA_TEXT_COLOR, &color_or_default(&chrome.text));
        set(hwnd, DWMWA_BORDER_COLOR, &color_or_default(&chrome.border));
    }

    /// `DWMWA_COLOR_DEFAULT`：交还给系统的默认颜色。
    const DWM_COLOR_DEFAULT: COLORREF = COLORREF(0xFFFF_FFFF);

    /// 颜色为空（前端不着色，例如开发环境模拟非 Windows 平台）或无法解析时恢复系统默认，而不是保留上一次的颜色。
    fn color_or_default(hex: &str) -> COLORREF {
        colorref(hex).unwrap_or(DWM_COLOR_DEFAULT)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn colorref_is_bgr() {
            assert_eq!(super::colorref("#00c1cd").map(|c| c.0), Some(0x00cd_c100));
            assert_eq!(super::colorref("bad"), None);
        }

        #[test]
        fn empty_color_resets_to_system_default() {
            assert_eq!(super::color_or_default("").0, 0xFFFF_FFFF);
            assert_eq!(super::color_or_default("#0a0a0a").0, 0x000a_0a0a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors_parse() {
        assert_eq!(parse_hex("#141416"), Some((0x14, 0x14, 0x16)));
        assert_eq!(parse_hex(" #F4F5F6 "), Some((0xf4, 0xf5, 0xf6)));
        assert_eq!(parse_hex("#fff"), Some((255, 255, 255)));
        assert_eq!(parse_hex("#ffff"), None);
        assert_eq!(parse_hex("141416"), None);
        assert_eq!(parse_hex("#14141g"), None);
    }

    /// CSS 的 `oklch(L C H)`（不带透明度）换算成 #rrggbb：OKLab → 线性 sRGB → sRGB（与浏览器的换算相同，超出色域时截断）。
    fn oklch_to_hex(value: &str) -> String {
        let args = value
            .trim()
            .strip_prefix("oklch(")
            .and_then(|v| v.strip_suffix(')'))
            .unwrap_or_else(|| panic!("不是 oklch() 颜色：{value}"));
        let n: Vec<f64> = args
            .split_whitespace()
            .map(|v| v.parse().unwrap_or_else(|_| panic!("无法解析 {value}")))
            .collect();
        let [l, c, h] = n[..] else {
            panic!("oklch() 需要三个分量：{value}");
        };
        let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
        let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
        let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
        let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
        let linear = [
            4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_,
            -1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_,
            -0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701 * s_,
        ];
        let byte = |x: f64| {
            let x = x.clamp(0.0, 1.0);
            let v = if x <= 0.003_130_8 {
                12.92 * x
            } else {
                1.055 * x.powf(1.0 / 2.4) - 0.055
            };
            (v * 255.0).round() as u8
        };
        format!(
            "#{:02x}{:02x}{:02x}",
            byte(linear[0]),
            byte(linear[1]),
            byte(linear[2])
        )
    }

    #[test]
    fn oklch_converts_like_css() {
        assert_eq!(oklch_to_hex("oklch(1 0 0)"), "#ffffff");
        assert_eq!(oklch_to_hex("oklch(0 0 0)"), "#000000");
        // Tailwind 的 neutral-950 / neutral-500 / red-600
        assert_eq!(oklch_to_hex("oklch(0.145 0 0)"), "#0a0a0a");
        assert_eq!(oklch_to_hex("oklch(0.556 0 0)"), "#737373");
        assert_eq!(oklch_to_hex("oklch(0.577 0.245 27.325)"), "#e7000b");
    }

    /// 启动底色必须与 index.css 的 --background 一致（全文件只有一处定义），否则窗口显示瞬间或调整大小时露出另一种颜色。
    #[test]
    fn boot_background_matches_theme_token() {
        let css =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/index.css"))
                .unwrap();
        let values: Vec<&str> = css
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("--background:"))
            .map(|v| v.trim().trim_end_matches(';'))
            .collect();
        assert_eq!(values.len(), 1, "index.css 应只定义一次 --background");
        assert_eq!(oklch_to_hex(values[0]), BOOT_BACKGROUND);
    }

    #[test]
    fn launch_paths_keep_existing_files_only() {
        let dir = std::env::temp_dir().join(format!("cr-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.mp4");
        std::fs::write(&file, b"x").unwrap();
        let args = vec![
            "--flag".to_owned(),
            "a.mp4".to_owned(),
            file.to_string_lossy().into_owned(),
            "missing.mp4".to_owned(),
            String::new(),
        ];
        let got = launch_paths(&args, &dir);
        let expect = file.to_string_lossy().into_owned();
        assert_eq!(got, vec![expect.clone(), expect]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn title_bar_must_be_on_a_monitor() {
        let monitor = (0, 0, 1920, 1080);
        assert!(title_bar_visible((100, 100, 1280), monitor));
        assert!(title_bar_visible((1700, 0, 1280), monitor)); // 右侧超出，但还有 220 像素可拖动
        assert!(!title_bar_visible((1850, 100, 1280), monitor)); // 只剩 70 像素
        assert!(!title_bar_visible((100, -40, 1280), monitor)); // 标题栏在屏幕上方
        assert!(!title_bar_visible((2000, 100, 1280), monitor)); // 在拔掉的第二块屏上
        assert!(title_bar_visible((2000, 100, 1280), (1920, 0, 2560, 1440)));
    }
}
