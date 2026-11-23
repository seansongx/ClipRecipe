//! 独立工具集成测试共用工具（在 `tests/common` 之上）：运行工具、生成特殊的测试素材、读取输出。
//!
//! 用法（`tests/tool_<工具>.rs`）：
//! ```ignore
//! mod common;
//! mod tool_support;
//! use common::*;
//! use tool_support::*;
//! ```
//!
//! - 运行：`tool_config`（未启用码率模块，按源画质自动选码率）、`fixed_bitrate`、`registered`、`run_tool`、
//!   `start_tool`、`done_outputs`、`log_messages`
//! - 素材：`make_lavfi`（任意滤镜图）、`make_scene_clip`、`colour_scenes`（纯色场景硬切换）、
//!   `split_colour_scenes`（左右两色的场景）、`color_clip`（带噪点的纯色视频）、
//!   `make_luma_counter`（第 N 帧亮度 16 + 3N 的“亮度计数器”，配合 `counter_index` 还原帧序号）、`tone`（纯音频正弦）
//! - 画面：`frame_lumas` / `frame_colours`（逐帧区域平均亮度 / 颜色）、`frame_pixels` / `frame_row`（某一帧区域的全部像素）、
//!   `assert_colour`（容差 40）、颜色常量 `RED` … `BLACK`、`CENTRE`（320x240 画面中央 40x40）、`psnr`
//! - 声音：`audio_samples`（8 kHz 单声道样本）+ `rms`、`tone_db`（某频率附近的平均音量）
//! - 流与文件：`codecs`、`stream_count`、`video_duration`、`video_packets_md5`、`all_files`

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{self, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{JobEvent, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{Range, Settings};
use cliprecipe_lib::pipeline::modules::{BitrateMode, BitrateParams};
use cliprecipe_lib::tools::{self, runner, Tool, ToolRunConfig, ToolRunHandle};

use crate::common::{collector, Collector};

/// 队列任务（未探测，执行时再探测）。
pub fn video_task(id: &str, path: &Path) -> TaskSpec {
    TaskSpec {
        id: id.into(),
        path: path.to_string_lossy().into_owned(),
        source_root: None,
        info: None,
        number: 0,
    }
}

/// 测试用的运行配置：软件编码、固定种子、临时目录放在 `temp_root` 下（便于检查清理）。
pub fn tool_config(paths: &FfmpegPaths, out_dir: &Path, temp_root: &Path) -> ToolRunConfig {
    ToolRunConfig {
        paths: paths.clone(),
        encoders: EncoderCaps::software_only(),
        settings: Settings::default(),
        output_dir: Some(out_dir.to_path_buf()),
        bitrate: BitrateParams::default(),
        seed: 7,
        temp_root: Some(temp_root.to_path_buf()),
    }
}

/// 改用固定码率 `kbps`（kbit/s）编码，相当于主界面启用码率模块的“固定”模式。
///
/// 默认码率（未启用码率模块）已按源画质给出下限，纯色素材也不会糊；只有测试需要确定的码率时才用。
pub fn fixed_bitrate(mut config: ToolRunConfig, kbps: u64) -> ToolRunConfig {
    config.bitrate = BitrateParams {
        enabled: true,
        mode: BitrateMode::Fixed,
        kbps: Range::fixed(kbps),
        ..BitrateParams::default()
    };
    config
}

/// 按 ID 和参数 JSON 创建注册表里的工具。
pub fn registered(id: &str, params: serde_json::Value) -> Arc<dyn Tool> {
    tools::create(id, params).unwrap_or_else(|e| panic!("无法创建 {id}：{e}"))
}

/// 规划并开始运行（不等待）。
pub fn start_tool(
    config: ToolRunConfig,
    tool: Arc<dyn Tool>,
    videos: &[TaskSpec],
    folders: &BTreeMap<String, PathBuf>,
    c: &Collector,
) -> ToolRunHandle {
    let items = runner::plan_seeded(
        tool.as_ref(),
        videos,
        folders,
        &config.settings,
        config.seed,
    )
    .unwrap_or_else(|e| panic!("规划失败：{e}"));
    runner::start(config, tool, items, Arc::clone(&c.sink))
}

/// 运行到结束，返回汇总与事件收集器。
pub fn run_tool(
    config: ToolRunConfig,
    tool: Arc<dyn Tool>,
    videos: &[TaskSpec],
) -> (RunSummary, Collector) {
    let c = collector();
    let summary = start_tool(config, tool, videos, &BTreeMap::new(), &c).wait();
    (summary, c)
}

/// 某个工作项完成时的全部输出；未完成时 panic 并打印事件。
pub fn done_outputs(c: &Collector, id: &str) -> Vec<PathBuf> {
    match c.final_state(id) {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            outputs,
            ..
        }) => outputs.into_iter().map(PathBuf::from).collect(),
        other => panic!("{id} 应完成，实际：{other:?}\n事件：{:#?}", c.snapshot()),
    }
}

/// 运行过程中全部日志事件的文字（按发生顺序）。
pub fn log_messages(c: &Collector) -> Vec<String> {
    c.snapshot()
        .into_iter()
        .filter_map(|e| match e {
            JobEvent::Log { message, .. } => Some(message),
            _ => None,
        })
        .collect()
}

/// 用任意 lavfi 滤镜图生成测试视频：`graph` 输出一路视频（和可选的音频 `audio`），`extra` 为编码参数。
pub fn make_lavfi(
    paths: &FfmpegPaths,
    out: &Path,
    graph: &str,
    audio: Option<&str>,
    extra: &[&str],
) {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    let mut cmd = ffmpeg::command(&paths.ffmpeg);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        graph,
    ]);
    if let Some(audio) = audio {
        cmd.args(["-f", "lavfi", "-i", audio]);
    }
    cmd.args([
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]);
    if audio.is_some() {
        cmd.args(["-c:a", "aac", "-shortest"]);
    }
    cmd.args(extra);
    cmd.arg(out);
    let output = cmd.output().expect("无法启动 ffmpeg");
    assert!(
        output.status.success(),
        "生成测试视频失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 三段画面完全不同的视频（testsrc2 → smptebars → rgbtestsrc），每段 `each` 秒：
/// 场景切换点在 `each` 和 `2 * each` 处。带正弦音频。
pub fn make_scene_clip(paths: &FfmpegPaths, out: &Path, each: f64) {
    let graph = format!(
        "testsrc2=s=320x240:r=25:d={each},format=yuv420p[a];\
         smptebars=s=320x240:r=25:d={each},format=yuv420p[b];\
         rgbtestsrc=s=320x240:r=25:d={each},format=yuv420p[c];\
         [a][b][c]concat=n=3"
    );
    let audio = format!("sine=frequency=440:duration={}", each * 3.0);
    make_lavfi(paths, out, &graph, Some(&audio), &[]);
}

/// 纯色场景（320x240、25fps，每项为颜色与秒数），场景之间硬切换；可带 440 Hz 音频。
pub fn colour_scenes(paths: &FfmpegPaths, out: &Path, scenes: &[(&str, f64)], audio: bool) {
    let mut graph = String::new();
    for (i, (colour, secs)) in scenes.iter().enumerate() {
        graph.push_str(&format!(
            "color=c={colour}:s=320x240:r=25:d={secs},format=yuv420p[c{i}];"
        ));
    }
    for i in 0..scenes.len() {
        graph.push_str(&format!("[c{i}]"));
    }
    graph.push_str(&format!("concat=n={}", scenes.len()));
    let total: f64 = scenes.iter().map(|s| s.1).sum();
    let sine = format!("sine=frequency=440:duration={total}");
    make_lavfi(paths, out, &graph, audio.then_some(sine.as_str()), &[]);
}

/// 左右两半颜色不同的若干场景（320x240、25fps，每项为左、右颜色，每段 `each` 秒），场景之间硬切换；
/// 可带 440 Hz 音频。
pub fn split_colour_scenes(
    paths: &FfmpegPaths,
    out: &Path,
    scenes: &[(&str, &str)],
    each: f64,
    audio: bool,
) {
    let mut graph = String::new();
    for (i, (left, right)) in scenes.iter().enumerate() {
        graph.push_str(&format!(
            "color=c={left}:s=160x240:r=25:d={each}[l{i}];\
             color=c={right}:s=160x240:r=25:d={each}[r{i}];\
             [l{i}][r{i}]hstack,format=yuv420p[c{i}];"
        ));
    }
    for i in 0..scenes.len() {
        graph.push_str(&format!("[c{i}]"));
    }
    graph.push_str(&format!("concat=n={}", scenes.len()));
    let sine = format!("sine=frequency=440:duration={}", each * scenes.len() as f64);
    make_lavfi(paths, out, &graph, audio.then_some(sine.as_str()), &[]);
}

/// 带时域噪点的纯色视频（320x240、25fps），可带 440 Hz 音频。噪点让画面像真实素材一样有细节。
pub fn color_clip(paths: &FfmpegPaths, out: &Path, color: &str, seconds: f64, audio: bool) {
    let sine = format!("sine=frequency=440:duration={seconds}");
    make_lavfi(
        paths,
        out,
        &format!(
            "color=c={color}:s=320x240:r=25:d={seconds},noise=alls=20:allf=t+u,format=yuv420p"
        ),
        audio.then_some(sine.as_str()),
        &[],
    );
}

/// “亮度计数器”：灰色画面，第 N 帧整幅亮度为 16 + 3N（有限范围，无损编码），总帧数最多 66 帧，
/// 所以输出每一帧的平均亮度（[`frame_lumas`] + [`counter_index`]）就能还原它来自源视频的第几帧。
/// 宽或高为奇数时用 yuv444p；可带 440 Hz 音频。
pub fn make_luma_counter(
    paths: &FfmpegPaths,
    out: &Path,
    (w, h): (u32, u32),
    fps: u32,
    seconds: f64,
    audio: bool,
) {
    assert!(f64::from(fps) * seconds <= 66.0, "亮度计数器最多 66 帧");
    let pix = if w % 2 == 1 || h % 2 == 1 {
        "yuv444p"
    } else {
        "yuv420p"
    };
    let graph = format!(
        "color=c=black:s={w}x{h}:r={fps}:d={seconds},format={pix},geq=lum='16+3*N':cb=128:cr=128"
    );
    let sine = format!("sine=frequency=440:duration={seconds}");
    make_lavfi(
        paths,
        out,
        &graph,
        audio.then_some(sine.as_str()),
        &["-qp", "0", "-pix_fmt", pix],
    );
}

/// 亮度计数器某帧的全范围灰度 → 源视频帧序号（灰度 = 3N × 255 / 219）。
pub fn counter_index(gray: u8) -> i64 {
    (f64::from(gray) * 219.0 / 255.0 / 3.0).round() as i64
}

/// 纯音频文件（正弦，`freq` Hz、`seconds` 秒），格式由扩展名决定。
pub fn tone(paths: &FfmpegPaths, out: &Path, freq: u32, seconds: f64) {
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let output = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("sine=frequency={freq}:duration={seconds}"))
        .arg(out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 每条流的编码名（按流顺序），如 `["h264", "aac"]`。
pub fn codecs(paths: &FfmpegPaths, file: &Path) -> Vec<String> {
    probe_entries(paths, file, "all-stream=codec_name")
        .into_iter()
        .filter(|c| !c.is_empty())
        .collect()
}

/// 某类流（`v` / `a` / `s`）的数量。
pub fn stream_count(paths: &FfmpegPaths, file: &Path, kind: &str) -> usize {
    probe_json(paths, file, Some(kind), "stream=index")["streams"]
        .as_array()
        .map_or(0, Vec::len)
}

/// 视频流时长（秒，ffprobe 直接读取）。
pub fn video_duration(paths: &FfmpegPaths, file: &Path) -> f64 {
    probe_entries(paths, file, "stream=duration")
        .iter()
        .find_map(|l| l.trim().parse::<f64>().ok())
        .or_else(|| {
            probe_entries(paths, file, "format=duration")
                .first()
                .and_then(|d| d.trim().parse().ok())
        })
        .unwrap_or_else(|| panic!("读不到 {} 的时长", file.display()))
}

/// 第一条视频流全部数据包的 MD5（用来确认“直接复制”没有重编码）。
pub fn video_packets_md5(paths: &FfmpegPaths, file: &Path) -> String {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args([
            "-map", "0:v:0", "-c", "copy", "-f", "hash", "-hash", "md5", "-",
        ])
        .output()
        .expect("无法启动 ffmpeg");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// 画面区域 (x, y, w, h)。
pub type Rect = (u32, u32, u32, u32);

pub const RED: [i32; 3] = [255, 0, 0];
pub const LIME: [i32; 3] = [0, 255, 0];
pub const BLUE: [i32; 3] = [0, 0, 255];
pub const YELLOW: [i32; 3] = [255, 255, 0];
pub const WHITE: [i32; 3] = [255, 255, 255];
pub const BLACK: [i32; 3] = [0, 0, 0];
/// 320x240 画面中央 40x40 的区域。
pub const CENTRE: Rect = (140, 100, 40, 40);

/// 颜色（如 `avg_rgb` 的结果）每个分量都与期望相差不超过 40。
pub fn assert_colour(actual: [i32; 3], expected: [i32; 3], what: &str) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (a - e).abs() <= 40),
        "{what}：实际 {actual:?}，期望约 {expected:?}"
    );
}

/// 逐帧把区域（`None` 为整幅画面）缩成 1 个像素，按 `pix_fmt` 输出原始数据。
fn per_frame_average(
    paths: &FfmpegPaths,
    file: &Path,
    region: Option<Rect>,
    pix_fmt: &str,
) -> Vec<u8> {
    let crop = region
        .map(|(x, y, w, h)| format!("crop={w}:{h}:{x}:{y},"))
        .unwrap_or_default();
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .arg("-vf")
        .arg(format!("{crop}scale=1:1:flags=area,format={pix_fmt}"))
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// 每一帧中区域（`None` 为整幅画面）的平均亮度（全范围灰度），按帧顺序。
pub fn frame_lumas(paths: &FfmpegPaths, file: &Path, region: Option<Rect>) -> Vec<u8> {
    per_frame_average(paths, file, region, "gray")
}

/// 每一帧中区域（`None` 为整幅画面）的平均颜色（RGB），按帧顺序。
pub fn frame_colours(paths: &FfmpegPaths, file: &Path, region: Option<Rect>) -> Vec<[u8; 3]> {
    per_frame_average(paths, file, region, "rgb24")
        .chunks_exact(3)
        .map(|p| [p[0], p[1], p[2]])
        .collect()
}

/// 第 `t` 秒那一帧中区域 (x, y, w, h) 的全部像素（RGB，逐行）；`w` 为 0 时取整行宽度。
pub fn frame_pixels(paths: &FfmpegPaths, file: &Path, t: f64, (x, y, w, h): Rect) -> Vec<[u8; 3]> {
    let width = if w == 0 {
        "iw".to_owned()
    } else {
        w.to_string()
    };
    let output = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &t.to_string(),
            "-i",
        ])
        .arg(file)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("crop={width}:{h}:{x}:{y},format=rgb24"))
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .chunks_exact(3)
        .map(|p| [p[0], p[1], p[2]])
        .collect()
}

/// 第 `t` 秒那一帧第 `y` 行的全部像素（RGB）。
pub fn frame_row(paths: &FfmpegPaths, file: &Path, t: f64, y: u32) -> Vec<[u8; 3]> {
    frame_pixels(paths, file, t, (0, y, 0, 1))
}

/// 两个视频（尺寸相同）第一条视频流的平均 PSNR（dB，ffmpeg `psnr` 滤镜）；完全相同时为 100。
pub fn psnr(paths: &FfmpegPaths, a: &Path, b: &Path) -> f64 {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(a)
        .arg("-i")
        .arg(b)
        .args(["-lavfi", "[0:v:0][1:v:0]psnr", "-f", "null", "-"])
        .output()
        .expect("无法启动 ffmpeg");
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{text}");
    let average = text
        .lines()
        .rev()
        .find_map(|l| l.split("average:").nth(1))
        .and_then(|v| v.split_whitespace().next())
        .unwrap_or_else(|| panic!("ffmpeg 输出里没有 PSNR：{text}"));
    if average == "inf" {
        100.0
    } else {
        average
            .parse()
            .unwrap_or_else(|e| panic!("PSNR “{average}” 无法解析：{e}"))
    }
}

/// 采样率（Hz）：[`audio_samples`] 把音频统一成这个采样率。
const SAMPLE_RATE: f64 = 8000.0;

/// 第一条音轨的样本（混成单声道、8 kHz、浮点）。
pub fn audio_samples(paths: &FfmpegPaths, file: &Path) -> Vec<f32> {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args([
            "-map", "0:a:0", "-ac", "1", "-ar", "8000", "-f", "f32le", "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// [`audio_samples`] 在 `from`..`to` 秒之间的均方根（满幅正弦约 0.7，静音为 0）。
pub fn rms(samples: &[f32], from: f64, to: f64) -> f64 {
    let a = ((from * SAMPLE_RATE) as usize).min(samples.len());
    let b = ((to * SAMPLE_RATE) as usize).min(samples.len());
    let part = &samples[a..b];
    assert!(!part.is_empty(), "{from}..{to} 超出音频长度");
    (part.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / part.len() as f64).sqrt()
}

/// 第一条音轨在 `freq` Hz 附近的平均音量（dB）：带通滤波后 volumedetect；读不到时为负无穷。
pub fn tone_db(paths: &FfmpegPaths, file: &Path, freq: u32) -> f64 {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(file)
        .args(["-map", "0:a:0", "-af"])
        .arg(format!(
            "bandpass=f={freq}:width_type=h:width=60,volumedetect"
        ))
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .find_map(|l| l.split("mean_volume:").nth(1))
        .and_then(|v| v.trim().trim_end_matches("dB").trim().parse().ok())
        .unwrap_or(f64::NEG_INFINITY)
}

/// ffprobe `-show_entries` 的 JSON 输出。
fn probe_json(
    paths: &FfmpegPaths,
    file: &Path,
    select: Option<&str>,
    entries: &str,
) -> serde_json::Value {
    let mut cmd = ffmpeg::command(&paths.ffprobe);
    cmd.args(["-v", "error"]);
    if let Some(select) = select {
        cmd.args(["-select_streams", select]);
    }
    let output = cmd
        .args(["-show_entries", entries, "-of", "json"])
        .arg(file)
        .output()
        .expect("无法启动 ffprobe");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "ffprobe 输出无法解析（{}）：{e}\n{}",
            file.display(),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// ffprobe `-show_entries` 的取值（每条流一项）；`stream=键` 只看第一条视频流，`all-stream=键` 看全部流，
/// `format=键` 为容器的值。
///
/// 只读取顶层的 `streams` / `format`：ffprobe 9.0 对带节目（program）的容器（MPEG-TS、HLS 分片）在
/// `-show_entries stream=…` 时还会输出 `programs[].streams`，按 CSV 逐行读取会把每条流数两次。
fn probe_entries(paths: &FfmpegPaths, file: &Path, entries: &str) -> Vec<String> {
    let (select, entries) = match entries.strip_prefix("all-") {
        Some(all) => (None, all),
        None if entries.starts_with("stream=") => (Some("v:0"), entries),
        None => (None, entries),
    };
    let (section, key) = entries.split_once('=').expect("键=值");
    let json = probe_json(paths, file, select, entries);
    let text = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    };
    match section {
        "format" => vec![text(&json["format"][key])],
        _ => json["streams"]
            .as_array()
            .map(|list| list.iter().map(|s| text(&s[key])).collect())
            .unwrap_or_default(),
    }
}

/// 目录下（递归）的全部文件。
pub fn all_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(all_files(&path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}
