//! 集成测试共用工具：定位 FFmpeg、用 lavfi 现场生成测试视频、收集任务事件。
//! 不提交任何媒体文件；所有文件都写在系统临时目录里，测试结束删除。

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths, MediaInfo};
use cliprecipe_lib::jobs::{EventSink, JobEvent};

/// 与应用相同的方式定位 FFmpeg；不可用时给出明确的失败信息。
pub fn ffmpeg_paths() -> FfmpegPaths {
    let status = ffmpeg::status();
    assert!(
        status.available,
        "集成测试需要 FFmpeg：{}。在仓库根目录运行 npm install（自动使用 node_modules/@seankit 里的 FFmpeg），或把 ffmpeg 和 ffprobe 加入 PATH，或设置环境变量 {} 指向 ffmpeg 可执行文件（ffprobe 需在同一目录）。",
        status.error.unwrap_or_default(),
        ffmpeg::FFMPEG_ENV
    );
    ffmpeg::locate()
}

/// 测试用临时目录，离开作用域时删除。
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("cliprecipe-it-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, p: &str) -> PathBuf {
        self.0.join(p)
    }

    /// 目录下的全部文件名（不递归），排序后返回。
    pub fn file_names(&self, sub: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.0.join(sub))
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().is_file())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 测试视频参数。
pub struct Clip {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub seconds: f64,
    pub audio: bool,
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            width: 320,
            height: 240,
            fps: 25,
            seconds: 2.0,
            audio: true,
        }
    }
}

/// 用 lavfi（testsrc2 + sine）生成测试视频。
///
/// 宽或高为奇数时生成真正的奇数尺寸视频：改用 `testsrc`（testsrc2 会把尺寸取偶）
/// 与 yuv444p（yuv420p 不允许奇数尺寸），仍是 H.264。
pub fn make_clip(paths: &FfmpegPaths, out: &Path, clip: &Clip) {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    let odd = clip.width % 2 == 1 || clip.height % 2 == 1;
    let (source, pix_fmt) = if odd {
        ("testsrc", "yuv444p")
    } else {
        ("testsrc2", "yuv420p")
    };
    let mut cmd = ffmpeg::command(&paths.ffmpeg);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
    ])
    .arg(format!(
        "{source}=size={}x{}:rate={}:duration={}",
        clip.width, clip.height, clip.fps, clip.seconds
    ));
    if clip.audio {
        cmd.args(["-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=440:duration={}", clip.seconds));
    }
    cmd.args([
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        pix_fmt,
    ]);
    if clip.audio {
        cmd.args(["-c:a", "aac", "-shortest"]);
    }
    cmd.arg(out);
    let output = cmd.output().expect("无法启动 ffmpeg 生成测试视频");
    assert!(
        output.status.success(),
        "生成测试视频失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn probe(paths: &FfmpegPaths, file: &Path) -> MediaInfo {
    ffmpeg::probe(&paths.ffprobe, file)
        .unwrap_or_else(|e| panic!("探测 {} 失败：{e}", file.display()))
}

/// 生成 1 秒、25fps、无音频的奇数尺寸视频（见 [`make_clip`]）。
pub fn make_odd_clip(paths: &FfmpegPaths, out: &Path, width: u32, height: u32) {
    make_clip(
        paths,
        out,
        &Clip {
            width,
            height,
            fps: 25,
            seconds: 1.0,
            audio: false,
        },
    );
}

/// 各条流的时长与帧数（ffprobe 直接读取，不经过 MediaInfo 的取整）。
#[derive(Debug, Clone, PartialEq)]
pub struct Streams {
    /// 容器时长
    pub format: f64,
    /// 第一条视频流的时长与帧数、帧率
    pub video: f64,
    pub frames: u64,
    pub fps: f64,
    /// 第一条音频流的时长与采样率（没有音频为 None）
    pub audio: Option<f64>,
    pub sample_rate: Option<u32>,
}

pub fn streams(paths: &FfmpegPaths, file: &Path) -> Streams {
    let output = ffmpeg::command(&paths.ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,duration,nb_frames,sample_rate,avg_frame_rate:format=duration",
            "-of",
            "json",
        ])
        .arg(file)
        .output()
        .expect("无法启动 ffprobe");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("ffprobe 输出无法解析（{}）：{e}", file.display()));
    let num = |v: &serde_json::Value| v.as_str().and_then(|s| s.parse::<f64>().ok());
    let list = json["streams"].as_array().expect("没有 streams");
    let video = list
        .iter()
        .find(|s| s["codec_type"] == "video")
        .expect("没有视频流");
    let audio = list.iter().find(|s| s["codec_type"] == "audio");
    let fps = video["avg_frame_rate"]
        .as_str()
        .and_then(|r| {
            let (n, d) = r.split_once('/')?;
            let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
            (d > 0.0).then(|| n / d)
        })
        .unwrap_or(0.0);
    Streams {
        format: num(&json["format"]["duration"]).expect("没有容器时长"),
        video: num(&video["duration"]).expect("没有视频时长"),
        frames: num(&video["nb_frames"]).expect("没有帧数") as u64,
        fps,
        audio: audio.map(|a| num(&a["duration"]).expect("没有音频时长")),
        sample_rate: audio.and_then(|a| num(&a["sample_rate"]).map(|r| r as u32)),
    }
}

/// 某条流（如 `v:0`、`a:0`）的时长。
pub fn stream_duration(paths: &FfmpegPaths, file: &Path, stream: &str) -> f64 {
    let s = streams(paths, file);
    match stream {
        "a:0" | "a" => s.audio.expect("没有音频流"),
        _ => s.video,
    }
}

pub fn assert_close(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}：实际 {actual}，期望 {expected}±{tolerance}"
    );
}

/// 第 `time` 秒那一帧中区域 (x, y, w, h) 的平均颜色（RGB）。
pub fn avg_rgb(
    paths: &FfmpegPaths,
    file: &Path,
    time: f64,
    (x, y, w, h): (u32, u32, u32, u32),
) -> [i32; 3] {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &time.to_string(),
            "-i",
        ])
        .arg(file)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!(
            "crop={w}:{h}:{x}:{y},scale=1:1:flags=area,format=rgb24"
        ))
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len(), 3);
    [0, 1, 2].map(|i| i32::from(output.stdout[i]))
}

/// 把事件同时记到列表、发到通道，便于测试等待某个事件。
pub struct Collector {
    pub events: Arc<Mutex<Vec<JobEvent>>>,
    pub rx: Receiver<JobEvent>,
    pub sink: Arc<dyn EventSink>,
}

pub fn collector() -> Collector {
    let events: Arc<Mutex<Vec<JobEvent>>> = Arc::default();
    let (tx, rx) = mpsc::channel();
    let store = Arc::clone(&events);
    let tx = Mutex::new(tx);
    let sink = move |event: JobEvent| {
        store.lock().unwrap().push(event.clone());
        let _ = tx.lock().unwrap().send(event);
    };
    Collector {
        events,
        rx,
        sink: Arc::new(sink),
    }
}

impl Collector {
    /// 等到满足条件的事件出现（最多 `secs` 秒）。
    pub fn wait_for(&self, secs: u64, mut pred: impl FnMut(&JobEvent) -> bool) -> JobEvent {
        let deadline = std::time::Instant::now() + Duration::from_secs(secs);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(event) if pred(&event) => return event,
                Ok(_) => {}
                Err(_) => panic!("等待事件超时；已收到：{:#?}", self.events.lock().unwrap()),
            }
        }
    }

    pub fn snapshot(&self) -> Vec<JobEvent> {
        self.events.lock().unwrap().clone()
    }

    /// 某个任务的最终状态事件。
    pub fn final_state(&self, task_id: &str) -> Option<JobEvent> {
        self.snapshot().into_iter().rev().find(|e| {
            matches!(e, JobEvent::TaskState { task_id: id, state, .. }
                if id == task_id && !matches!(state, cliprecipe_lib::jobs::TaskState::Waiting | cliprecipe_lib::jobs::TaskState::Running))
        })
    }
}
