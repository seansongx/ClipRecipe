//! `tool.simple-mix` 简单混剪：队列中的每个视频作为主视频，叠加副视频片段（铺满或居中）、背景音频、字幕文件。
//! 输出时长跟随主视频，重编码（码率、编码器沿用框架设置）。
//!
//! 参数与取值约定（FEATURES 未规定的部分）：
//! - 副视频 `clipsEnabled` + `clips`：单个文件或目录（视频）。像加水印的“分段依次显示”：时间轴上先显示主视频
//!   `gapSeconds` 秒（0～600），再叠加一个副视频 `showSeconds` 秒（0.5～600），如此循环直到主视频结束；
//!   每段从副视频开头播放（比段长短时循环）。目录模式按“随机取”（按种子打乱）或“依次取”（从队列序号开始）
//!   轮流使用目录里的视频。最多 [`MAX_WINDOWS`] 段（更长的视频后面只显示主视频，警告）。
//!   - `layout`：`fill` 铺满（按比例放大后居中裁切到主视频尺寸）；`center` 居中（默认），宽度为主视频的
//!     `centerSize`%（10～100），高度按比例。副视频的声音不使用。
//! - 背景音频 `bgmEnabled` + `bgm`（单个文件或目录，音频）：循环到主视频结束，`bgmVolume` 与主视频原声
//!   `originalVolume` 为音量百分比（0～400）后混合；主视频没有声音时只用背景音频。统一为 48kHz 立体声 AAC。
//! - 字幕 `subtitleEnabled` + `subtitle`（单个文件或目录，srt / ass / ssa）：用 `subtitles` 滤镜（libass）烧录到画面，
//!   在副视频之上；中文字体由 libass 按系统字体自动匹配。
//! - 三项至少开启一项。主视频宽或高为奇数时裁掉最后一行 / 列（yuv420p 要求偶数）。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::path_arg;
use crate::fsutil::VIDEO_EXTENSIONS;
use crate::model::{AssetSource, PickOrder};
use crate::pipeline::{escape_filter_path, fmt_num};
use crate::tools::analysis::Interval;
use crate::tools::av_mux::AUDIO_EXTENSIONS;
use crate::tools::context::Encoding;
use crate::tools::media::secs;
use crate::tools::subtitle_asr::MACOS_CJK_FONT;
use crate::tools::common::merge_engine::shuffle;
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.simple-mix";
pub const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa"];
/// 副视频最多叠加的段数。
pub const MAX_WINDOWS: usize = 120;
pub const MAX_VOLUME: f64 = 400.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MixLayout {
    /// 铺满画面
    Fill,
    /// 居中
    #[default]
    Center,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SimpleMixParams {
    pub clips_enabled: bool,
    pub clips: AssetSource,
    pub layout: MixLayout,
    /// 居中时副视频宽度占主视频的百分比
    pub center_size: f64,
    /// 每段副视频显示的秒数
    pub show_seconds: f64,
    /// 两段副视频之间只显示主视频的秒数（也是开头的间隔）
    pub gap_seconds: f64,
    pub bgm_enabled: bool,
    pub bgm: AssetSource,
    pub bgm_volume: f64,
    pub original_volume: f64,
    pub subtitle_enabled: bool,
    pub subtitle: AssetSource,
}

impl Default for SimpleMixParams {
    fn default() -> Self {
        Self {
            clips_enabled: true,
            clips: AssetSource::default(),
            layout: MixLayout::Center,
            center_size: 50.0,
            show_seconds: 3.0,
            gap_seconds: 3.0,
            bgm_enabled: false,
            bgm: AssetSource::default(),
            bgm_volume: 100.0,
            original_volume: 100.0,
            subtitle_enabled: false,
            subtitle: AssetSource::default(),
        }
    }
}

impl SimpleMixParams {
    fn validate(&self) -> Result<(), String> {
        if !(self.clips_enabled || self.bgm_enabled || self.subtitle_enabled) {
            return Err("请至少开启副视频、背景音频、字幕中的一项".into());
        }
        if self.clips_enabled {
            if self.clips.is_empty() {
                return Err("已开启副视频，但没有选择文件或目录".into());
            }
            if !(0.5..=600.0).contains(&self.show_seconds) {
                return Err("副视频每段时长必须在 0.5～600 秒之间".into());
            }
            if !(0.0..=600.0).contains(&self.gap_seconds) {
                return Err("间隔时长必须在 0～600 秒之间".into());
            }
            if !(10.0..=100.0).contains(&self.center_size) {
                return Err("居中大小必须在 10%～100% 之间".into());
            }
        }
        if self.bgm_enabled {
            if self.bgm.is_empty() {
                return Err("已开启背景音频，但没有选择文件或目录".into());
            }
            for v in [self.bgm_volume, self.original_volume] {
                if !(0.0..=MAX_VOLUME).contains(&v) {
                    return Err(format!("音量必须在 0～{}% 之间", fmt_num(MAX_VOLUME)));
                }
            }
        }
        if self.subtitle_enabled && self.subtitle.is_empty() {
            return Err("已开启字幕，但没有选择字幕文件或目录".into());
        }
        Ok(())
    }
}

/// 副视频的显示时段：先间隔 `gap`，再显示 `show`，循环到 `duration`。返回（时段，是否因上限被截断）。
pub fn windows(duration: f64, show: f64, gap: f64) -> (Vec<Interval>, bool) {
    let mut out = Vec::new();
    let period = show + gap;
    if !(duration > 0.0 && period > 0.0) {
        return (out, false);
    }
    let mut start = gap;
    while start < duration - 0.05 {
        if out.len() == MAX_WINDOWS {
            return (out, true);
        }
        out.push(Interval::new(start, (start + show).min(duration)));
        start += period;
    }
    (out, false)
}

/// 混剪滤镜图需要的信息。
#[derive(Debug, Clone, PartialEq)]
pub struct MixGraph {
    /// 主视频显示尺寸
    pub width: u32,
    pub height: u32,
    /// 副视频时段（第 k 段是输入 k + 1）
    pub windows: Vec<Interval>,
    pub layout: MixLayout,
    pub center_size: f64,
    /// 背景音频的输入序号（与音量）
    pub bgm: Option<(usize, f64)>,
    /// 主视频有声音时的原声音量
    pub original: Option<f64>,
    /// 已转义的字幕路径
    pub subtitle: Option<String>,
}

/// 字幕滤镜的字体参数：只在 macOS 上显式指定中文字体（否则中文画成方框，见 [`MACOS_CJK_FONT`]）；
/// 其它平台不指定，沿用 libass 的默认字体与回退。
fn subtitle_font_style() -> String {
    if cfg!(target_os = "macos") {
        format!(":force_style='FontName={MACOS_CJK_FONT}'")
    } else {
        String::new()
    }
}

impl MixGraph {
    /// 滤镜图：输出 `[vout]`；有背景音频时还有 `[aout]`。
    pub fn graph(&self) -> String {
        let (w, h) = (self.width, self.height);
        let mut chains = Vec::new();
        let mut base = "0:V:0".to_owned();
        for (k, win) in self.windows.iter().enumerate() {
            let input = k + 1;
            let (scale, position) = match self.layout {
                MixLayout::Fill => (
                    format!(
                        "scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setsar=1"
                    ),
                    "0:0".to_owned(),
                ),
                MixLayout::Center => {
                    let cw =
                        ((f64::from(w) * self.center_size / 100.0).round() as u32 / 2 * 2).max(2);
                    (
                        format!("scale={cw}:-2,setsar=1"),
                        "(W-w)/2:(H-h)/2".to_owned(),
                    )
                }
            };
            let start = secs(win.start);
            chains.push(format!(
                "[{input}:V:0]{scale},setpts=PTS-STARTPTS+{start}/TB[o{k}]"
            ));
            let next = format!("b{k}");
            chains.push(format!(
                "[{base}][o{k}]overlay={position}:eof_action=pass:enable='between(t,{start},{})'[{next}]",
                secs(win.end)
            ));
            base = next;
        }
        let mut tail = Vec::new();
        if let Some(sub) = &self.subtitle {
            tail.push(format!("subtitles={sub}{}", subtitle_font_style()));
        }
        let (ew, eh) = (w - w % 2, h - h % 2);
        if (ew, eh) != (w, h) {
            tail.push(format!("crop={ew}:{eh}:0:0"));
        }
        tail.push("format=yuv420p".into());
        chains.push(format!("[{base}]{}[vout]", tail.join(",")));
        if let Some((input, volume)) = self.bgm {
            let conform = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";
            let bgm_volume = fmt_num(volume / 100.0);
            match self.original {
                Some(original) => chains.push(format!(
                    "[0:a:0]{conform},volume={}[ma];[{input}:a:0]{conform},volume={bgm_volume}[ba];\
                     [ma][ba]amix=inputs=2:duration=longest:dropout_transition=0:normalize=0[aout]",
                    fmt_num(original / 100.0)
                )),
                None => chains.push(format!("[{input}:a:0]{conform},volume={bgm_volume}[aout]")),
            }
        }
        chains.join(";")
    }
}

/// 混剪参数（不含通用开头参数）。`clips[k]` 是第 k 段的副视频；滤镜图从 `script` 文件读取。
#[allow(clippy::too_many_arguments)]
pub fn mix_args(
    main: &str,
    clips: &[(String, f64)],
    bgm: Option<&str>,
    script: &str,
    audio: MixAudio,
    encoding: &Encoding,
    container: &str,
    duration: f64,
    output: &str,
) -> Vec<String> {
    let mut args = vec!["-i".into(), main.into()];
    for (clip, len) in clips {
        args.extend([
            "-stream_loop".into(),
            "-1".into(),
            "-t".into(),
            secs(*len),
            "-i".into(),
            clip.clone(),
        ]);
    }
    if let Some(bgm) = bgm {
        args.extend(["-stream_loop".into(), "-1".into(), "-i".into(), bgm.into()]);
    }
    args.extend([
        "-/filter_complex".into(),
        script.into(),
        "-map".into(),
        "[vout]".into(),
    ]);
    match audio {
        MixAudio::None => {}
        MixAudio::Main(_) => args.extend(["-map".into(), "0:a:0".into()]),
        MixAudio::Mixed => args.extend(["-map".into(), "[aout]".into()]),
    }
    args.extend(encoding.video_args(container));
    match audio {
        MixAudio::None => {}
        MixAudio::Main(channels) => args.extend(Encoding::audio_args(channels)),
        MixAudio::Mixed => args.extend(Encoding::audio_args(2)),
    }
    args.extend(["-t".into(), secs(duration), "-sn".into(), "-dn".into()]);
    if matches!(container, "mp4" | "mov" | "m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

/// 输出音频的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixAudio {
    None,
    /// 主视频原声（声道数）
    Main(u32),
    /// 滤镜图里的 `[aout]`
    Mixed,
}

impl Tool for SimpleMixParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let info = ctx.info(0)?;
        let video = info
            .video
            .as_ref()
            .ok_or_else(|| format!("{} 没有视频流", info.path))?;
        let (width, height) = video.display_size();
        if width < 2 || height < 2 || info.duration <= 0.0 {
            return Err("无法读取主视频的尺寸或时长".into());
        }
        let task_index = ctx.number().saturating_sub(1);

        // 副视频
        let mut slots = Vec::new();
        let mut clip_args = Vec::new();
        if self.clips_enabled {
            let mut files = self.clips.candidates(VIDEO_EXTENSIONS)?;
            match &self.clips {
                AssetSource::Directory {
                    order: PickOrder::Random,
                    ..
                } => shuffle(&mut files, &mut ctx.rng),
                _ => {
                    let start = task_index % files.len();
                    files.rotate_left(start);
                }
            }
            let (list, capped) = windows(info.duration, self.show_seconds, self.gap_seconds);
            if capped {
                ctx.warn(format!("副视频最多叠加 {MAX_WINDOWS} 段，之后只显示主视频"));
            }
            if list.is_empty() {
                ctx.warn("主视频太短，没有叠加副视频");
            }
            for (k, win) in list.iter().enumerate() {
                clip_args.push((path_arg(&files[k % files.len()])?, win.duration()));
            }
            slots = list;
        }

        // 背景音频
        let bgm_path = if self.bgm_enabled {
            Some(self.bgm.pick(AUDIO_EXTENSIONS, task_index, &mut ctx.rng)?)
        } else {
            None
        };
        let has_main_audio = !info.audio.is_empty();
        let audio = match (&bgm_path, info.audio.first()) {
            (Some(_), _) => MixAudio::Mixed,
            (None, Some(a)) => MixAudio::Main(a.channels),
            (None, None) => MixAudio::None,
        };

        // 字幕
        let subtitle = if self.subtitle_enabled {
            let path = self
                .subtitle
                .pick(SUBTITLE_EXTENSIONS, task_index, &mut ctx.rng)?;
            Some(escape_filter_path(&path_arg(&path)?))
        } else {
            None
        };

        let graph = MixGraph {
            width,
            height,
            windows: slots,
            layout: self.layout,
            center_size: self.center_size,
            bgm: bgm_path
                .as_ref()
                .map(|_| (clip_args.len() + 1, self.bgm_volume)),
            original: has_main_audio.then_some(self.original_volume),
            subtitle,
        }
        .graph();
        let script = ctx.temp_file("mix-filter.txt")?;
        std::fs::write(&script, graph).map_err(|e| format!("无法写入滤镜脚本：{e}"))?;
        let script = path_arg(&script)?;
        let main = path_arg(ctx.input_path(0)?)?;
        let bgm = bgm_path.as_deref().map(path_arg).transpose()?;
        let container = ctx.video_extension(0)?;
        let out = path_arg(&ctx.output(&OutputSpec::file(&container))?)?;
        let duration = info.duration;
        ctx.run_encode("简单混剪", duration, |enc| {
            Ok(mix_args(
                &main,
                &clip_args,
                bgm.as_deref(),
                &script,
                audio,
                enc,
                &container,
                duration,
                &out,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{VideoCodec, VideoEncoder};

    #[test]
    fn windows_alternate_gap_and_show() {
        let (list, capped) = windows(10.0, 2.0, 1.5);
        assert!(!capped);
        assert_eq!(
            list,
            [
                Interval::new(1.5, 3.5),
                Interval::new(5.0, 7.0),
                Interval::new(8.5, 10.0)
            ]
        );
        let (back_to_back, _) = windows(5.0, 2.0, 0.0);
        assert_eq!(back_to_back.len(), 3);
        assert_eq!(back_to_back[2], Interval::new(4.0, 5.0));
        let (many, capped) = windows(10_000.0, 1.0, 0.0);
        assert!(capped);
        assert_eq!(many.len(), MAX_WINDOWS);
        assert!(windows(0.0, 1.0, 1.0).0.is_empty());
    }

    #[test]
    fn graph_overlays_each_window_then_subtitles_and_mixes_bgm() {
        // macOS 上字幕滤镜带显式的中文字体，其它平台为空
        let font = subtitle_font_style();
        let graph = MixGraph {
            width: 1280,
            height: 720,
            windows: vec![Interval::new(1.0, 3.0), Interval::new(5.0, 7.5)],
            layout: MixLayout::Center,
            center_size: 50.0,
            bgm: Some((3, 80.0)),
            original: Some(100.0),
            subtitle: Some("C\\\\:/s.srt".into()),
        }
        .graph();
        let parts: Vec<&str> = graph.split(';').collect();
        assert_eq!(
            parts[..5],
            [
                "[1:V:0]scale=640:-2,setsar=1,setpts=PTS-STARTPTS+1/TB[o0]",
                "[0:V:0][o0]overlay=(W-w)/2:(H-h)/2:eof_action=pass:enable='between(t,1,3)'[b0]",
                "[2:V:0]scale=640:-2,setsar=1,setpts=PTS-STARTPTS+5/TB[o1]",
                "[b0][o1]overlay=(W-w)/2:(H-h)/2:eof_action=pass:enable='between(t,5,7.5)'[b1]",
                &format!("[b1]subtitles=C\\\\:/s.srt{font},format=yuv420p[vout]"),
            ]
        );
        let conform = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";
        assert_eq!(
            parts[5..].join(";"),
            format!(
                "[0:a:0]{conform},volume=1[ma];[3:a:0]{conform},volume=0.8[ba];\
                 [ma][ba]amix=inputs=2:duration=longest:dropout_transition=0:normalize=0[aout]"
            )
        );

        let fill = MixGraph {
            width: 321,
            height: 240,
            windows: vec![Interval::new(0.0, 2.0)],
            layout: MixLayout::Fill,
            center_size: 50.0,
            bgm: Some((2, 100.0)),
            original: None,
            subtitle: None,
        }
        .graph();
        assert!(fill
            .contains("scale=321:240:force_original_aspect_ratio=increase,crop=321:240,setsar=1"));
        assert!(fill.contains("overlay=0:0:"), "{fill}");
        assert!(
            fill.contains("[b0]crop=320:240:0:0,format=yuv420p[vout]"),
            "{fill}"
        );
        assert!(fill.ends_with(
            "[2:a:0]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,volume=1[aout]"
        ));

        let plain = MixGraph {
            width: 640,
            height: 360,
            windows: Vec::new(),
            layout: MixLayout::Center,
            center_size: 50.0,
            bgm: None,
            original: Some(100.0),
            subtitle: Some("s.ass".into()),
        }
        .graph();
        assert_eq!(
            plain,
            format!("[0:V:0]subtitles=s.ass{font},format=yuv420p[vout]")
        );
    }

    #[test]
    fn args_loop_each_clip_for_its_window() {
        let enc = Encoding {
            encoder: VideoEncoder::software(VideoCodec::H264),
            video_kbps: None,
        };
        let args = mix_args(
            "m.mp4",
            &[("c.mp4".into(), 2.0)],
            Some("b.mp3"),
            "g.txt",
            MixAudio::Mixed,
            &enc,
            "mp4",
            9.5,
            "o.mp4",
        )
        .join(" ");
        assert!(
            args.starts_with(
                "-i m.mp4 -stream_loop -1 -t 2 -i c.mp4 -stream_loop -1 -i b.mp3 \
             -/filter_complex g.txt -map [vout] -map [aout] -c:v libx264"
            ),
            "{args}"
        );
        assert!(
            args.ends_with("-c:a aac -b:a 192k -t 9.5 -sn -dn -movflags +faststart o.mp4"),
            "{args}"
        );
        let main_audio = mix_args(
            "m.mp4",
            &[],
            None,
            "g.txt",
            MixAudio::Main(1),
            &enc,
            "mkv",
            3.0,
            "o.mkv",
        )
        .join(" ");
        assert!(
            main_audio.contains("-map [vout] -map 0:a:0"),
            "{main_audio}"
        );
        assert!(main_audio.contains("-c:a aac -b:a 96k"), "{main_audio}");
        let silent = mix_args(
            "m.mp4",
            &[],
            None,
            "g.txt",
            MixAudio::None,
            &enc,
            "mp4",
            3.0,
            "o.mp4",
        )
        .join(" ");
        assert!(!silent.contains("-c:a"), "{silent}");
    }

    #[test]
    fn params_validate() {
        let mut p = SimpleMixParams::default();
        assert!(p.validate().is_err(), "默认没有选择副视频");
        p.clips = AssetSource::File {
            path: "c.mp4".into(),
        };
        assert!(p.validate().is_ok());
        p.show_seconds = 0.1;
        assert!(p.validate().is_err());
        let none = SimpleMixParams {
            clips_enabled: false,
            ..SimpleMixParams::default()
        };
        assert!(none.validate().is_err());
        let sub = SimpleMixParams {
            clips_enabled: false,
            subtitle_enabled: true,
            ..SimpleMixParams::default()
        };
        assert!(sub.validate().is_err(), "没有选择字幕");
    }
}
