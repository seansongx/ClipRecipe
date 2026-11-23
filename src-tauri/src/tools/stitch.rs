//! `tool.stitch` 画面拼接：把文件夹里的视频按顺序每 N 个一组（N 由布局决定）拼成一个分屏视频。
//!
//! **布局** `layout`：`sideBySide` 左右 2 屏（默认）、`stacked` 上下 2 屏、`row3` 横排 3 屏、`column3` 竖排 3 屏、
//! `grid4` 四宫格（2×2，按 左上、右上、左下、右下 排列）。文件按名称自然排序后依次分组；
//! 最后一组不足 N 个时，空位填充背景色。文件夹里至少要有 2 个视频。
//!
//! **画面尺寸** `size`：`auto`（默认）每格为第一个视频的显示尺寸，整幅画面最长边超过 1920 时等比缩小；
//! `custom` 整幅画面为 `width` × `height`（默认 1920×1080），每格均分。每格宽高取偶数。
//! 每个视频按 `cellFit` 放进格子（默认 `fill` 铺满居中裁切；`fit` 完整放入、空白为背景色 `background`；`stretch` 拉伸），
//! 统一为 `fps` 帧率（默认 30）。
//!
//! **播放方式** `playback`：
//! - `together`（默认）同时播放：总时长 = 最长的视频，先播完的格子停在最后一帧；
//! - `sequential` 依次播放：第 1 个播完第 2 个才开始……总时长 = 各视频时长之和；还没轮到的格子停在第一帧，
//!   播完的停在最后一帧；
//! - `fillTurns` 依次填充播放（只用于四宫格）：先显示 `gridSeconds` 秒（默认 1 秒，0～10）静止的四宫格
//!   （各格显示第一帧），然后第 1 个视频放大到整个画面播放（按 `cellFit` 放进整幅画面），播完回到四宫格
//!   （该格停在最后一帧）再显示 `gridSeconds` 秒，接着第 2 个视频放大播放……最后一个播完后再显示一次四宫格结束。
//!   总时长 = 各视频时长之和 + (个数 + 1) × `gridSeconds`。
//!
//! **音频** `audio`（统一为 48 kHz 立体声 AAC）：
//! - `mix`（默认）所有视频的音频从头开始同时播放、混合（`amix`，音量按路数平均）；
//! - `first` 只用第一个视频的音频；
//! - `current` 当前播放的那个视频的音频（只用于依次播放 / 依次填充播放）：每段时间只有正在播放的视频发声，
//!   四宫格停留期间静音；没有音频的视频那一段为静音。
//!
//! 音频比画面短时补静音，长于总时长时截断。没有任何可用音频时输出不含音轨。
//! 输出名为每组第一个视频的名字加 `_拼接`。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::fsutil::VIDEO_EXTENSIONS;
use crate::pipeline::fmt_num;
use crate::pipeline::modules::text::escape_value;
use crate::tools::common::frame::{color_arg, even, fit_scale, target_size, Fit};
use crate::tools::context::Encoding;
use crate::tools::{
    FolderSlot, ItemInput, OutputSpec, PlanRequest, Tool, ToolContext, ToolError, ToolInputs,
    WorkItem,
};

pub const ID: &str = "tool.stitch";

/// 文件夹输入的键。
pub const SOURCE: &str = "source";

/// 自动尺寸时整幅画面最长边的上限。
pub const AUTO_MAX_SIDE: u32 = 1920;

/// 统一的音频格式。
const AUDIO_FORMAT: &str = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";
const SILENCE: &str = "anullsrc=r=48000:cl=stereo";

/// 宽高（像素）或（列数，行数）。
pub type Size = (u32, u32);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StitchLayout {
    /// 左右 2 屏
    #[default]
    SideBySide,
    /// 上下 2 屏
    Stacked,
    /// 横排 3 屏
    Row3,
    /// 竖排 3 屏
    Column3,
    /// 四宫格
    Grid4,
}

impl StitchLayout {
    /// （列数，行数）
    pub fn grid(self) -> Size {
        match self {
            StitchLayout::SideBySide => (2, 1),
            StitchLayout::Stacked => (1, 2),
            StitchLayout::Row3 => (3, 1),
            StitchLayout::Column3 => (1, 3),
            StitchLayout::Grid4 => (2, 2),
        }
    }

    pub fn cells(self) -> usize {
        let (c, r) = self.grid();
        (c * r) as usize
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StitchPlayback {
    /// 同时播放
    #[default]
    Together,
    /// 依次播放
    Sequential,
    /// 依次填充播放（四宫格）
    FillTurns,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StitchAudio {
    /// 全部混合
    #[default]
    Mix,
    /// 只用第一个
    First,
    /// 当前播放的
    Current,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StitchSize {
    #[default]
    Auto,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StitchParams {
    pub layout: StitchLayout,
    pub playback: StitchPlayback,
    /// 依次填充播放：每次回到四宫格停留的秒数
    pub grid_seconds: f64,
    pub audio: StitchAudio,
    pub size: StitchSize,
    /// 自定义：整幅画面宽高
    pub width: u32,
    pub height: u32,
    pub cell_fit: Fit,
    /// 背景色（#rrggbb）
    pub background: String,
    pub fps: f64,
}

impl Default for StitchParams {
    fn default() -> Self {
        Self {
            layout: StitchLayout::SideBySide,
            playback: StitchPlayback::Together,
            grid_seconds: 1.0,
            audio: StitchAudio::Mix,
            size: StitchSize::Auto,
            width: 1920,
            height: 1080,
            cell_fit: Fit::Fill,
            background: "#000000".into(),
            fps: 30.0,
        }
    }
}

impl StitchParams {
    fn validate(&self) -> Result<(), String> {
        if self.playback == StitchPlayback::FillTurns && self.layout != StitchLayout::Grid4 {
            return Err("“依次填充播放”只能用于四宫格布局".into());
        }
        if self.audio == StitchAudio::Current && self.playback == StitchPlayback::Together {
            return Err("“当前播放的音频”只能用于依次播放".into());
        }
        if !(self.grid_seconds.is_finite() && (0.0..=10.0).contains(&self.grid_seconds)) {
            return Err("四宫格停留时间必须在 0～10 秒之间".into());
        }
        if !(self.fps.is_finite() && (1.0..=120.0).contains(&self.fps)) {
            return Err("帧率必须在 1～120 之间".into());
        }
        color_arg(&self.background, "背景色")?;
        if self.size == StitchSize::Custom {
            target_size(self.width, self.height)?;
        }
        Ok(())
    }

    /// （每格宽高，整幅画面宽高）。`first`：第一个视频的显示尺寸。
    pub fn sizes(&self, first: Size) -> Result<(Size, Size), String> {
        let (cols, rows) = self.layout.grid();
        let (cw, ch) = match self.size {
            StitchSize::Custom => {
                let (w, h) = target_size(self.width, self.height)?;
                (even(w / cols), even(h / rows))
            }
            StitchSize::Auto => {
                let (w, h) = first;
                if w < 2 || h < 2 {
                    return Err("无法读取视频分辨率".into());
                }
                let longest = (w * cols).max(h * rows);
                if longest > AUTO_MAX_SIDE {
                    let k = f64::from(AUTO_MAX_SIDE) / f64::from(longest);
                    (
                        even((f64::from(w) * k) as u32),
                        even((f64::from(h) * k) as u32),
                    )
                } else {
                    (even(w), even(h))
                }
            }
        };
        Ok(((cw, ch), (cw * cols, ch * rows)))
    }
}

/// 时间轴：每个视频开始播放的时间与总时长。
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    pub offsets: Vec<f64>,
    pub total: f64,
}

pub fn timeline(playback: StitchPlayback, durations: &[f64], grid: f64) -> Timeline {
    match playback {
        StitchPlayback::Together => Timeline {
            offsets: vec![0.0; durations.len()],
            total: durations.iter().copied().fold(0.0, f64::max),
        },
        StitchPlayback::Sequential | StitchPlayback::FillTurns => {
            let gap = if playback == StitchPlayback::FillTurns {
                grid
            } else {
                0.0
            };
            let mut at = 0.0;
            let offsets = durations
                .iter()
                .map(|d| {
                    at += gap;
                    let start = at;
                    at += d;
                    start
                })
                .collect();
            Timeline {
                offsets,
                total: at + gap,
            }
        }
    }
}

/// 一组视频的拼接命令（不含编码参数与输出路径）。
#[derive(Debug, Clone, PartialEq)]
pub struct StitchJob {
    pub graph: String,
    /// 是否有音频输出 `[a]`
    pub audio: bool,
    pub total: f64,
    pub canvas: (u32, u32),
    pub warnings: Vec<String>,
}

/// 放进 `w × h` 框（`Fit` 时补背景色）、统一 SAR 与像素格式。
fn conform(fit: Fit, (w, h): (u32, u32), bg: &str) -> String {
    let pad = if fit == Fit::Fit {
        format!(",pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color={bg}")
    } else {
        String::new()
    };
    format!("{}{pad},setsar=1,format=yuv420p", fit_scale(fit, w, h))
}

/// 构建拼接滤镜图。`infos`：本组的视频（按顺序）。
pub fn build(params: &StitchParams, infos: &[MediaInfo]) -> Result<StitchJob, String> {
    params.validate()?;
    let n = infos.len();
    let cells = params.layout.cells();
    if n == 0 || n > cells {
        return Err(format!("每组需要 1～{cells} 个视频"));
    }
    let mut durations = Vec::with_capacity(n);
    for info in infos {
        if info.video.is_none() {
            return Err(format!("文件中没有视频流：{}", info.path));
        }
        if !(info.duration.is_finite() && info.duration > 0.0) {
            return Err(format!("无法读取视频时长：{}", info.path));
        }
        durations.push(info.duration);
    }
    let first = infos[0].video.as_ref().map(|v| v.display_size()).unwrap();
    let ((cw, ch), (w, h)) = params.sizes(first)?;
    let bg = color_arg(&params.background, "背景色")?;
    let fps = fmt_num(params.fps);
    let tl = timeline(params.playback, &durations, params.grid_seconds);
    let fill_turns = params.playback == StitchPlayback::FillTurns;
    let mut parts: Vec<String> = Vec::new();

    // 每格：统一帧率与尺寸，前后用第一帧 / 最后一帧补齐时间轴（多补 1 秒，最后由 -t 截断）
    for c in 0..cells {
        if c >= n {
            parts.push(format!(
                "color=c={bg}:s={cw}x{ch}:r={fps}:d={}[c{c}]",
                fmt_num(tl.total + 1.0)
            ));
            continue;
        }
        let (start, d) = (tl.offsets[c], durations[c]);
        let mut pad = vec![
            "stop_mode=clone".to_owned(),
            format!("stop_duration={}", fmt_num(tl.total - start - d + 1.0)),
        ];
        if start > 0.0 {
            pad.splice(
                0..0,
                [
                    "start_mode=clone".to_owned(),
                    format!("start_duration={}", fmt_num(start)),
                ],
            );
        }
        // 依次填充播放时，另一路放大到整幅画面（见下）
        let source = if fill_turns {
            parts.push(format!("[{c}:v:0]fps={fps},split=2[s{c}][t{c}]"));
            format!("[s{c}]")
        } else {
            format!("[{c}:v:0]fps={fps},")
        };
        parts.push(format!(
            "{source}{},tpad={}[c{c}]",
            conform(params.cell_fit, (cw, ch), &bg),
            pad.join(":")
        ));
    }
    let (cols, _) = params.layout.grid();
    let layout: Vec<String> = (0..cells as u32)
        .map(|i| format!("{}_{}", (i % cols) * cw, (i / cols) * ch))
        .collect();
    let labels: String = (0..cells).map(|c| format!("[c{c}]")).collect();
    let mut last = "grid".to_owned();
    parts.push(format!(
        "{labels}xstack=inputs={cells}:layout={}[{last}]",
        layout.join("|")
    ));

    if fill_turns {
        for (i, d) in durations.iter().enumerate() {
            let start = tl.offsets[i];
            parts.push(format!(
                "[t{i}]{},tpad=start_mode=clone:start_duration={}[f{i}]",
                conform(params.cell_fit, (w, h), &bg),
                fmt_num(start)
            ));
            let enable = escape_value(&format!(
                "gte(t,{})*lt(t,{})",
                fmt_num(start),
                fmt_num(start + d)
            ));
            let next = format!("o{i}");
            parts.push(format!(
                "[{last}][f{i}]overlay=x=0:y=0:eof_action=pass:enable={enable}[{next}]"
            ));
            last = next;
        }
    }
    parts.push(format!("[{last}]format=yuv420p[v]"));

    let mut warnings = Vec::new();
    let has_audio: Vec<bool> = infos.iter().map(|i| !i.audio.is_empty()).collect();
    let total = fmt_num(tl.total);
    let audio = match params.audio {
        StitchAudio::Mix => {
            let with: Vec<usize> = (0..n).filter(|i| has_audio[*i]).collect();
            for i in &with {
                parts.push(format!("[{i}:a:0]{AUDIO_FORMAT}[a{i}]"));
            }
            match with.len() {
                0 => false,
                1 => {
                    parts.push(format!("[a{}]apad=whole_dur={total}[a]", with[0]));
                    true
                }
                k => {
                    let labels: String = with.iter().map(|i| format!("[a{i}]")).collect();
                    parts.push(format!(
                        "{labels}amix=inputs={k}:duration=longest,apad=whole_dur={total}[a]"
                    ));
                    true
                }
            }
        }
        StitchAudio::First => {
            if has_audio[0] {
                parts.push(format!("[0:a:0]{AUDIO_FORMAT},apad=whole_dur={total}[a]"));
                true
            } else {
                warnings.push("第一个视频没有音频，输出不含音轨".to_owned());
                false
            }
        }
        StitchAudio::Current => {
            if has_audio.iter().any(|a| *a) {
                let gap = if fill_turns { params.grid_seconds } else { 0.0 };
                let silence = |label: &str, seconds: f64| {
                    format!(
                        "{SILENCE},{AUDIO_FORMAT},atrim=duration={}[{label}]",
                        fmt_num(seconds)
                    )
                };
                let mut order: Vec<String> = Vec::new();
                for (i, d) in durations.iter().enumerate() {
                    if gap > 0.0 {
                        parts.push(silence(&format!("g{i}"), gap));
                        order.push(format!("[g{i}]"));
                    }
                    if has_audio[i] {
                        parts.push(format!(
                            "[{i}:a:0]{AUDIO_FORMAT},apad,atrim=duration={}[p{i}]",
                            fmt_num(*d)
                        ));
                    } else {
                        parts.push(silence(&format!("p{i}"), *d));
                    }
                    order.push(format!("[p{i}]"));
                }
                if gap > 0.0 {
                    parts.push(silence("gend", gap));
                    order.push("[gend]".to_owned());
                }
                parts.push(format!(
                    "{}concat=n={}:v=0:a=1[a]",
                    order.concat(),
                    order.len()
                ));
                true
            } else {
                false
            }
        }
    };
    if !audio && params.audio != StitchAudio::First {
        warnings.push("这一组视频都没有音频，输出不含音轨".to_owned());
    }
    Ok(StitchJob {
        graph: parts.join(";"),
        audio,
        total: tl.total,
        canvas: (w, h),
        warnings,
    })
}

/// 完整参数（不含开头的通用参数）。
pub fn job_args(
    job: &StitchJob,
    inputs: &[String],
    encoding: &Encoding,
    container: &str,
    output: &str,
) -> Vec<String> {
    let mut args = Vec::new();
    for input in inputs {
        args.extend(["-i".to_owned(), input.clone()]);
    }
    args.extend([
        "-filter_complex".into(),
        job.graph.clone(),
        "-map".into(),
        "[v]".into(),
    ]);
    if job.audio {
        args.extend(["-map".into(), "[a]".into()]);
    }
    args.extend(encoding.video_args(container));
    if job.audio {
        args.extend(Encoding::audio_args(2));
    } else {
        args.push("-an".into());
    }
    args.extend(["-t".into(), fmt_num(job.total), "-sn".into(), "-dn".into()]);
    if matches!(container, "mp4" | "mov" | "m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

/// 把文件按布局每 N 个分成一组。
pub fn groups(files: &[std::path::PathBuf], per: usize) -> Vec<&[std::path::PathBuf]> {
    files.chunks(per.max(1)).collect()
}

impl Tool for StitchParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn inputs(&self) -> ToolInputs {
        const FOLDERS: &[FolderSlot] = &[FolderSlot {
            key: SOURCE,
            label: "视频文件夹",
        }];
        ToolInputs::folders(FOLDERS)
    }

    fn plan(&self, req: &PlanRequest) -> Result<Vec<WorkItem>, String> {
        self.validate()?;
        let root = req.folder(SOURCE)?.to_path_buf();
        let files = req.folder_files(SOURCE, VIDEO_EXTENSIONS)?;
        if files.len() < 2 {
            return Err("文件夹里至少要有 2 个视频才能拼接".into());
        }
        Ok(groups(&files, self.layout.cells())
            .into_iter()
            .enumerate()
            .map(|(k, group)| {
                let label = group
                    .iter()
                    .map(|p| {
                        p.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
                    .join(" + ");
                WorkItem::new(
                    format!("{ID}#{}", k + 1),
                    label,
                    k + 1,
                    group
                        .iter()
                        .map(|path| ItemInput {
                            path: path.clone(),
                            info: None,
                            source_root: Some(root.clone()),
                        })
                        .collect(),
                )
            })
            .collect())
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let count = ctx.inputs().len();
        let mut infos = Vec::with_capacity(count);
        let mut inputs = Vec::with_capacity(count);
        for i in 0..count {
            infos.push(ctx.info(i)?);
            inputs.push(path_arg(ctx.input_path(i)?)?);
        }
        let job = build(self, &infos)?;
        for warning in &job.warnings {
            ctx.warn(warning.clone());
        }
        if count < self.layout.cells() {
            ctx.warn(format!("最后一组只有 {count} 个视频，空位填充背景色"));
        }
        let container = ctx.video_extension(0)?;
        let output = path_arg(&ctx.output(&OutputSpec::file(&container).suffix("_拼接"))?)?;
        ctx.run_encode("画面拼接", job.total, |enc| {
            Ok(job_args(&job, &inputs, enc, &container, &output))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{AudioStream, VideoStream};

    fn info(path: &str, (w, h): (u32, u32), duration: f64, audio: bool) -> MediaInfo {
        MediaInfo {
            path: path.into(),
            duration,
            video: Some(VideoStream {
                width: w,
                height: h,
                fps: 25.0,
                fps_text: "25".into(),
                ..VideoStream::default()
            }),
            audio: if audio {
                vec![AudioStream {
                    codec: "aac".into(),
                    channels: 2,
                    sample_rate: 44100,
                    ..AudioStream::default()
                }]
            } else {
                vec![]
            },
            ..MediaInfo::default()
        }
    }

    #[test]
    fn timelines() {
        let d = [2.0, 3.0, 1.0];
        assert_eq!(
            timeline(StitchPlayback::Together, &d, 1.0),
            Timeline {
                offsets: vec![0.0; 3],
                total: 3.0
            }
        );
        assert_eq!(
            timeline(StitchPlayback::Sequential, &d, 1.0),
            Timeline {
                offsets: vec![0.0, 2.0, 5.0],
                total: 6.0
            }
        );
        assert_eq!(
            timeline(StitchPlayback::FillTurns, &d, 0.5),
            Timeline {
                offsets: vec![0.5, 3.0, 6.5],
                total: 8.0
            }
        );
    }

    #[test]
    fn sizes_auto_and_custom() {
        let p = StitchParams::default();
        assert_eq!(p.sizes((640, 360)).unwrap(), ((640, 360), (1280, 360)));
        // 1920×1080 左右两屏 → 3840 宽，缩到 1920
        assert_eq!(p.sizes((1920, 1080)).unwrap(), ((960, 540), (1920, 540)));
        let grid = StitchParams {
            layout: StitchLayout::Grid4,
            size: StitchSize::Custom,
            width: 1081,
            height: 1921,
            ..StitchParams::default()
        };
        assert_eq!(grid.sizes((1, 1)).unwrap(), ((540, 960), (1080, 1920)));
        let column = StitchParams {
            layout: StitchLayout::Column3,
            ..StitchParams::default()
        };
        assert_eq!(column.sizes((321, 241)).unwrap(), ((320, 240), (320, 720)));
    }

    #[test]
    fn together_side_by_side_graph() {
        let infos = [
            info("a.mp4", (320, 240), 2.0, true),
            info("b.mp4", (320, 240), 3.0, true),
        ];
        let job = build(&StitchParams::default(), &infos).unwrap();
        assert_eq!(job.canvas, (640, 240));
        assert_eq!(job.total, 3.0);
        assert_eq!(
            job.graph,
            "[0:v:0]fps=30,scale=320:240:force_original_aspect_ratio=increase,crop=320:240,setsar=1,format=yuv420p,\
             tpad=stop_mode=clone:stop_duration=2[c0];\
             [1:v:0]fps=30,scale=320:240:force_original_aspect_ratio=increase,crop=320:240,setsar=1,format=yuv420p,\
             tpad=stop_mode=clone:stop_duration=1[c1];\
             [c0][c1]xstack=inputs=2:layout=0_0|320_0[grid];[grid]format=yuv420p[v];\
             [0:a:0]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo[a0];\
             [1:a:0]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo[a1];\
             [a0][a1]amix=inputs=2:duration=longest,apad=whole_dur=3[a]"
        );
        assert!(job.audio);
        let enc = Encoding {
            encoder: crate::ffmpeg::VideoEncoder::software(crate::ffmpeg::VideoCodec::H264),
            video_kbps: Some(1000),
        };
        let args = job_args(
            &job,
            &["a.mp4".into(), "b.mp4".into()],
            &enc,
            "mp4",
            "o.mp4",
        );
        let line = args.join(" ");
        assert!(
            line.starts_with("-i a.mp4 -i b.mp4 -filter_complex"),
            "{line}"
        );
        assert!(
            line.ends_with(
                "-map [v] -map [a] -c:v libx264 -preset medium -pix_fmt yuv420p -b:v 1000k -maxrate 1500k \
                 -bufsize 2000k -c:a aac -b:a 192k -t 3 -sn -dn -movflags +faststart o.mp4"
            ),
            "{line}"
        );
    }

    #[test]
    fn sequential_with_current_audio_and_empty_cell() {
        let p = StitchParams {
            layout: StitchLayout::Row3,
            playback: StitchPlayback::Sequential,
            audio: StitchAudio::Current,
            cell_fit: Fit::Fit,
            fps: 25.0,
            ..StitchParams::default()
        };
        let infos = [
            info("a.mp4", (320, 240), 2.0, true),
            info("b.mp4", (320, 240), 1.5, false),
        ];
        let job = build(&p, &infos).unwrap();
        assert_eq!(job.total, 3.5);
        let g = &job.graph;
        assert!(
            g.contains(
                "[0:v:0]fps=25,scale=320:240:force_original_aspect_ratio=decrease:force_divisible_by=2,\
                 pad=320:240:(ow-iw)/2:(oh-ih)/2:color=0x000000,setsar=1,format=yuv420p,\
                 tpad=stop_mode=clone:stop_duration=2.5[c0]"
            ),
            "{g}"
        );
        assert!(
            g.contains(
                "tpad=start_mode=clone:start_duration=2:stop_mode=clone:stop_duration=1[c1]"
            ),
            "{g}"
        );
        assert!(
            g.contains("color=c=0x000000:s=320x240:r=25:d=4.5[c2]"),
            "{g}"
        );
        assert!(
            g.contains("xstack=inputs=3:layout=0_0|320_0|640_0[grid]"),
            "{g}"
        );
        assert!(
            g.ends_with(
                "[0:a:0]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,apad,atrim=duration=2[p0];\
                 anullsrc=r=48000:cl=stereo,aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,\
                 atrim=duration=1.5[p1];[p0][p1]concat=n=2:v=0:a=1[a]"
            ),
            "{g}"
        );
    }

    #[test]
    fn fill_turns_overlays_each_video_in_its_window() {
        let p = StitchParams {
            layout: StitchLayout::Grid4,
            playback: StitchPlayback::FillTurns,
            grid_seconds: 0.5,
            audio: StitchAudio::First,
            fps: 25.0,
            ..StitchParams::default()
        };
        let infos: Vec<MediaInfo> = (0..4)
            .map(|i| info(&format!("{i}.mp4"), (160, 120), 1.0, i != 1))
            .collect();
        let job = build(&p, &infos).unwrap();
        assert_eq!(job.total, 6.5);
        assert_eq!(job.canvas, (320, 240));
        let g = &job.graph;
        assert!(
            g.contains("[0:v:0]fps=25,split=2[s0][t0];[s0]scale=160:120"),
            "{g}"
        );
        assert!(
            g.contains("xstack=inputs=4:layout=0_0|160_0|0_120|160_120[grid]"),
            "{g}"
        );
        assert!(
            g.contains(
                "[t1]scale=320:240:force_original_aspect_ratio=increase,crop=320:240,setsar=1,format=yuv420p,\
                 tpad=start_mode=clone:start_duration=2[f1];\
                 [o0][f1]overlay=x=0:y=0:eof_action=pass:enable=gte(t\\,2)*lt(t\\,3)[o1]"
            ),
            "{g}"
        );
        assert!(g.contains("[o3]format=yuv420p[v]"), "{g}");
        assert!(g.ends_with(
            "[0:a:0]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,apad=whole_dur=6.5[a]"
        ));
    }

    #[test]
    fn audio_fallbacks() {
        let silent = [
            info("a.mp4", (320, 240), 2.0, false),
            info("b.mp4", (320, 240), 2.0, false),
        ];
        let job = build(&StitchParams::default(), &silent).unwrap();
        assert!(!job.audio);
        assert_eq!(job.warnings.len(), 1);
        let first = StitchParams {
            audio: StitchAudio::First,
            ..StitchParams::default()
        };
        let job = build(&first, &silent).unwrap();
        assert!(!job.audio);
        assert_eq!(job.warnings, ["第一个视频没有音频，输出不含音轨"]);
    }

    #[test]
    fn invalid_params() {
        let infos = [
            info("a.mp4", (320, 240), 2.0, true),
            info("b.mp4", (320, 240), 2.0, true),
        ];
        for p in [
            StitchParams {
                playback: StitchPlayback::FillTurns,
                ..StitchParams::default()
            },
            StitchParams {
                audio: StitchAudio::Current,
                ..StitchParams::default()
            },
            StitchParams {
                fps: 0.0,
                ..StitchParams::default()
            },
            StitchParams {
                background: "zz".into(),
                ..StitchParams::default()
            },
            StitchParams {
                size: StitchSize::Custom,
                width: 4,
                ..StitchParams::default()
            },
        ] {
            assert!(build(&p, &infos).is_err(), "{p:?}");
        }
        let three = [infos[0].clone(), infos[0].clone(), infos[0].clone()];
        assert!(build(&StitchParams::default(), &three).is_err(), "超过一组");
        let mut zero = infos.clone();
        zero[1].duration = 0.0;
        assert!(build(&StitchParams::default(), &zero).is_err());
        let files: Vec<std::path::PathBuf> = (0..5).map(|i| format!("{i}.mp4").into()).collect();
        let g = groups(&files, 2);
        assert_eq!(g.len(), 3);
        assert_eq!(g[2].len(), 1);
    }
}
