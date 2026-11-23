//! `tool.freeze-anim` 定帧动画：让画面停在一系列“定帧点”的帧上。一进一出，重编码。
//!
//! **定帧点**（`spacing`）：
//! - `count`：把视频平均分成 `count` 段，每段的开头是一个定帧点（`k × 时长 / count`，k = 0..count-1）；
//! - `interval`：从 0 秒起每隔 `intervalSeconds` 秒一个定帧点。
//!
//! **两种效果**（`mode`）：
//! - `stopMotion` 定格动画（默认）：画面变成一串静止帧——每个定帧点到下一个定帧点之间的整段都显示该段的
//!   第一帧（时长不变，音频原样保留：容器装得下时直接复制）。`holdSeconds` 不使用。
//!   实现：`select` 只留下每段的第一帧，`fps=源帧率` 重复它填满整段。
//! - `insertHolds` 插入停顿：播放到定帧点那一帧时停住 `holdSeconds` 秒（取整到整帧，至少 1 帧），然后继续播放；
//!   视频变长 `定帧点数 × 停顿时长`。`holdAudio`：`silence`（默认）在停顿处插入同样长的静音，音画始终同步；
//!   `continue` 音频不停、照常播放，结尾补静音到与视频等长（停顿之后音画不再对齐）。
//!   实现：`setpts` 把定帧点之后的帧整体后移，`fps=源帧率` 用定帧点那一帧填满空档；静音由
//!   `asplit` + `atrim` 切开、与 `anullsrc` 的静音片段 `concat` 起来。最多 [`MAX_HOLDS`] 个停顿。
//!
//! 定帧点按帧对齐（取该时间点或之后的第一帧）；可变帧率的视频先按平均帧率转成恒定帧率。奇数尺寸裁成偶数。
//!
//! 帧率 [`Rate`]、插入空档的滤镜、编码命令 [`FilterJob`] 等共用小函数在 [`crate::tools::common::filter_job`]。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::pipeline::modules::text::escape_value;
use crate::tools::media::secs;
use crate::tools::{Tool, ToolContext, ToolError};

pub const ID: &str = "tool.freeze-anim";

/// 插入停顿模式下最多的停顿个数（音频要切成这么多段）。
pub const MAX_HOLDS: usize = 500;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FreezeAnimMode {
    /// 定格动画：每段显示该段第一帧，时长不变
    #[default]
    StopMotion,
    /// 插入停顿：在定帧点停住一段时间再继续，时长变长
    InsertHolds,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FreezeSpacing {
    /// 固定间隔
    #[default]
    Interval,
    /// 平均分成 N 段
    Count,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HoldAudio {
    /// 停顿处插入静音（音画同步）
    #[default]
    Silence,
    /// 音频照常播放，结尾补静音
    Continue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FreezeAnimParams {
    pub mode: FreezeAnimMode,
    pub spacing: FreezeSpacing,
    /// 按数量：定帧点个数（1～1000）
    pub count: u32,
    /// 按间隔：相邻定帧点的间隔（秒，≥ 0.05）
    pub interval_seconds: f64,
    /// 插入停顿：每次停顿的时长（秒，0.01～60，取整到整帧）
    pub hold_seconds: f64,
    /// 插入停顿：停顿期间的音频
    pub hold_audio: HoldAudio,
}

impl Default for FreezeAnimParams {
    fn default() -> Self {
        Self {
            mode: FreezeAnimMode::StopMotion,
            spacing: FreezeSpacing::Interval,
            count: 10,
            interval_seconds: 0.5,
            hold_seconds: 1.0,
            hold_audio: HoldAudio::Silence,
        }
    }
}

impl FreezeAnimParams {
    fn validate(&self) -> Result<(), String> {
        match self.spacing {
            FreezeSpacing::Count if !(1..=1000).contains(&self.count) => {
                return Err("定帧点数量必须在 1～1000 之间".into())
            }
            FreezeSpacing::Interval
                if !(self.interval_seconds.is_finite() && self.interval_seconds >= 0.05) =>
            {
                return Err("定帧间隔至少 0.05 秒".into())
            }
            _ => {}
        }
        if self.mode == FreezeAnimMode::InsertHolds
            && !(self.hold_seconds.is_finite() && (0.01..=60.0).contains(&self.hold_seconds))
        {
            return Err("停顿时长必须在 0.01～60 秒之间".into());
        }
        Ok(())
    }

    /// 每段的长度（秒）。
    fn step(&self, duration: f64) -> f64 {
        match self.spacing {
            FreezeSpacing::Count => duration / f64::from(self.count),
            FreezeSpacing::Interval => self.interval_seconds,
        }
    }
}

/// 定帧点（秒，升序，都在 `[0, duration)` 内）。
pub fn freeze_points(params: &FreezeAnimParams, duration: f64) -> Result<Vec<f64>, String> {
    params.validate()?;
    check_duration(duration)?;
    let step = params.step(duration);
    let mut points = Vec::new();
    let mut k = 0.0;
    while step * k < duration - 1e-6 {
        points.push(step * k);
        k += 1.0;
        if points.len() > 1_000_000 {
            break;
        }
    }
    Ok(points)
}

/// 定格动画的视频滤镜：每段只留第一帧（`select`），再按源帧率重复填满。
pub fn stop_motion_filter(params: &FreezeAnimParams, duration: f64, rate: &Rate) -> String {
    // 段号 floor(t / step)；加一点余量避免浮点误差把正好落在段首的帧算进上一段
    let step = params.step(duration);
    let seg = |var: &str| format!("floor({var}/{}+0.000001)", float_text(step));
    let expr = format!("isnan(prev_t)+gte({},{}+1)", seg("t"), seg("prev_t"));
    format!(
        "select={},fps={}:start_time=0",
        escape_value(&expr),
        rate.text
    )
}

/// 生成命令（输入、滤镜图、音频处理、预计时长）。
pub fn build(
    params: &FreezeAnimParams,
    info: &MediaInfo,
    input: &str,
) -> Result<FilterJob, String> {
    params.validate()?;
    let (w, h) = display_size(info)?;
    let rate = Rate::of(info)?;
    check_duration(info.duration)?;
    let mut job = FilterJob::new(input);
    let even = even_crop(w, h);
    match params.mode {
        FreezeAnimMode::StopMotion => {
            job.graph = format!(
                "[0:v:0]{}{even}[v]",
                stop_motion_filter(params, info.duration, &rate)
            );
            job.audio = source_audio(info);
            job.duration = info.duration;
        }
        FreezeAnimMode::InsertHolds => {
            let points = freeze_points(params, info.duration)?;
            let total = rate.total_frames(info);
            let mut frames: Vec<i64> = points
                .iter()
                .map(|t| rate.frame_at(*t))
                .filter(|f| *f < total)
                .collect();
            frames.dedup();
            if frames.is_empty() {
                return Err("视频太短，没有可用的定帧点".into());
            }
            if frames.len() > MAX_HOLDS {
                return Err(format!(
                    "停顿太多（{} 个，最多 {MAX_HOLDS} 个），请加大间隔或减少数量",
                    frames.len()
                ));
            }
            let gap_frames = rate.frames_for(params.hold_seconds);
            let gap = rate.seconds(f64::from(gap_frames));
            job.graph = format!(
                "[0:v:0]{}{even}[v]",
                gap_video_filter(&frames, gap_frames, &rate)
            );
            job.duration = info.duration + gap * frames.len() as f64;
            if let Some(audio) = info.audio.first() {
                let graph = match params.hold_audio {
                    HoldAudio::Silence => {
                        let cuts: Vec<f64> = frames
                            .iter()
                            .map(|f| rate.seconds((f + 1) as f64))
                            .collect();
                        gap_audio_filter("[0:a:0]", &cuts, gap, audio, "[a]")
                    }
                    HoldAudio::Continue => {
                        format!("[0:a:0]apad=whole_dur={}[a]", secs(job.duration))
                    }
                };
                job.graph.push(';');
                job.graph.push_str(&graph);
                job.audio = AudioOut::Filtered {
                    channels: audio.channels,
                };
            }
        }
    }
    Ok(job)
}

impl Tool for FreezeAnimParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn supports_preview(&self) -> bool {
        true
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let info = ctx.info(0)?;
        let input = path_arg(ctx.input_path(0)?)?;
        let job = build(self, &info, &input)?;
        run_job(ctx, "定帧动画", &job)
    }
}

// 本组工具共用的小函数已移到 `tools::common::filter_job`，这里保留原名的再导出。
pub use crate::tools::common::filter_job::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{AudioStream, VideoCodec, VideoEncoder};
    use crate::tools::Encoding;
    use crate::pipeline::testing::sample_info;

    fn enc() -> Encoding {
        Encoding {
            encoder: VideoEncoder::software(VideoCodec::H264),
            video_kbps: Some(1000),
        }
    }

    fn params(mode: FreezeAnimMode, spacing: FreezeSpacing) -> FreezeAnimParams {
        FreezeAnimParams {
            mode,
            spacing,
            ..FreezeAnimParams::default()
        }
    }

    #[test]
    fn rate_parsing() {
        let mut info = sample_info();
        assert_eq!(Rate::of(&info).unwrap().text, "25");
        info.video.as_mut().unwrap().fps_text = "30000/1001".into();
        let r = Rate::of(&info).unwrap();
        assert_eq!((r.text.as_str(), r.num, r.den), ("30000/1001", 30000, 1001));
        info.video.as_mut().unwrap().fps_text = "0/0".into();
        info.video.as_mut().unwrap().fps = 24.0;
        assert_eq!(Rate::of(&info).unwrap().text, "24");
        info.video.as_mut().unwrap().fps = 0.0;
        assert!(Rate::of(&info).is_err());
        let r = Rate {
            text: "25".into(),
            num: 25,
            den: 1,
        };
        assert_eq!(r.frame_at(1.0), 25);
        assert_eq!(r.frame_at(1.01), 26);
        assert_eq!(r.frames_for(0.5), 12 + 1); // 12.5 → 13
        assert_eq!(r.frames_for(0.001), 1);
        assert_eq!(r.total_frames(&sample_info()), 250);
    }

    #[test]
    fn points_by_count_and_interval() {
        let count = FreezeAnimParams {
            count: 4,
            ..params(FreezeAnimMode::StopMotion, FreezeSpacing::Count)
        };
        assert_eq!(freeze_points(&count, 10.0).unwrap(), [0.0, 2.5, 5.0, 7.5]);
        let interval = FreezeAnimParams {
            interval_seconds: 3.0,
            ..params(FreezeAnimMode::StopMotion, FreezeSpacing::Interval)
        };
        assert_eq!(freeze_points(&interval, 9.0).unwrap(), [0.0, 3.0, 6.0]);
        assert!(freeze_points(&interval, 0.0).is_err());
    }

    #[test]
    fn invalid_params_are_rejected() {
        let bad = [
            FreezeAnimParams {
                count: 0,
                ..params(FreezeAnimMode::StopMotion, FreezeSpacing::Count)
            },
            FreezeAnimParams {
                interval_seconds: 0.01,
                ..FreezeAnimParams::default()
            },
            FreezeAnimParams {
                hold_seconds: 0.0,
                ..params(FreezeAnimMode::InsertHolds, FreezeSpacing::Interval)
            },
            FreezeAnimParams {
                hold_seconds: f64::NAN,
                ..params(FreezeAnimMode::InsertHolds, FreezeSpacing::Interval)
            },
        ];
        for p in bad {
            assert!(build(&p, &sample_info(), "in.mp4").is_err(), "{p:?}");
        }
        // 定格动画不使用停顿时长
        let ok = FreezeAnimParams {
            hold_seconds: 0.0,
            ..FreezeAnimParams::default()
        };
        assert!(build(&ok, &sample_info(), "in.mp4").is_ok());
        let too_many = FreezeAnimParams {
            interval_seconds: 0.05,
            ..params(FreezeAnimMode::InsertHolds, FreezeSpacing::Interval)
        };
        let mut long = sample_info();
        long.duration = 100.0;
        long.video.as_mut().unwrap().frames = Some(2500);
        assert!(build(&too_many, &long, "in.mp4")
            .unwrap_err()
            .contains("停顿太多"));
    }

    #[test]
    fn stop_motion_command() {
        let p = FreezeAnimParams {
            count: 5,
            ..params(FreezeAnimMode::StopMotion, FreezeSpacing::Count)
        };
        let job = build(&p, &sample_info(), "in.mp4").unwrap();
        assert_eq!(
            job.graph,
            "[0:v:0]select=isnan(prev_t)+gte(floor(t/2+0.000001)\\,floor(prev_t/2+0.000001)+1),\
             fps=25:start_time=0[v]"
        );
        assert_eq!(job.duration, 10.0);
        let mut job = job;
        job.audio_codec = "aac".into();
        assert_eq!(
            job.args(&enc(), "mp4", "out.mp4").join(" "),
            format!(
                "-i in.mp4 -filter_complex {} -map [v] -map 0:a:0 -c:v libx264 -preset medium -pix_fmt yuv420p \
                 -b:v 1000k -maxrate 1500k -bufsize 2000k -c:a copy -sn -dn -movflags +faststart out.mp4",
                job.graph
            )
        );
        // 音频编码装不进容器时转 AAC
        job.audio_codec = "pcm_s16le".into();
        let args = job.args(&enc(), "mp4", "out.mp4").join(" ");
        assert!(args.contains("-c:a aac -b:a 192k"), "{args}");
    }

    #[test]
    fn insert_holds_with_silence() {
        let p = FreezeAnimParams {
            count: 2,
            hold_seconds: 0.4,
            ..params(FreezeAnimMode::InsertHolds, FreezeSpacing::Count)
        };
        let job = build(&p, &sample_info(), "in.mp4").unwrap();
        // 定帧点 0 秒（第 0 帧）与 5 秒（第 125 帧），各停 10 帧
        assert_eq!(
            job.graph,
            "[0:v:0]fps=25,setpts=PTS+(gt(N\\,0)+gt(N\\,125))*10*1/25/TB,fps=25:start_time=0[v];\
             [0:a:0]asplit=3[ga0][ga1][ga2];\
             [ga0]atrim=end=0.04,asetpts=PTS-STARTPTS[gs0];\
             [ga1]atrim=start=0.04:end=5.04,asetpts=PTS-STARTPTS[gs1];\
             [ga2]atrim=start=5.04,asetpts=PTS-STARTPTS[gs2];\
             anullsrc=r=48000:cl=stereo,atrim=duration=0.4[gz0];\
             anullsrc=r=48000:cl=stereo,atrim=duration=0.4[gz1];\
             [gs0][gz0][gs1][gz1][gs2]concat=n=5:v=0:a=1[a]"
        );
        assert!((job.duration - 10.8).abs() < 1e-9);
        assert_eq!(job.audio, AudioOut::Filtered { channels: 2 });
        let args = job.args(&enc(), "mkv", "out.mkv").join(" ");
        assert!(
            args.ends_with(
                "-map [v] -map [a] -c:v libx264 -preset medium -pix_fmt yuv420p -b:v 1000k \
                 -maxrate 1500k -bufsize 2000k -c:a aac -b:a 192k -sn -dn out.mkv"
            ),
            "{args}"
        );
    }

    #[test]
    fn insert_holds_audio_continues_and_odd_size_is_cropped() {
        let p = FreezeAnimParams {
            interval_seconds: 4.0,
            hold_seconds: 1.0,
            hold_audio: HoldAudio::Continue,
            ..params(FreezeAnimMode::InsertHolds, FreezeSpacing::Interval)
        };
        let mut info = sample_info();
        info.video.as_mut().unwrap().width = 641;
        let job = build(&p, &info, "in.mp4").unwrap();
        assert_eq!(
            job.graph,
            "[0:v:0]fps=25,setpts=PTS+(gt(N\\,0)+gt(N\\,100)+gt(N\\,200))*25*1/25/TB,\
             fps=25:start_time=0,crop=640:720:0:0[v];[0:a:0]apad=whole_dur=13[a]"
        );
        let mut silent = sample_info();
        silent.audio.clear();
        let job = build(&p, &silent, "in.mp4").unwrap();
        assert_eq!(job.audio, AudioOut::None);
        assert!(job.args(&enc(), "mp4", "o.mp4").contains(&"-an".to_owned()));
    }

    #[test]
    fn gap_audio_skips_empty_pieces() {
        let audio = AudioStream {
            channels: 1,
            sample_rate: 44_100,
            ..AudioStream::default()
        };
        assert_eq!(
            gap_audio_filter("[0:a:0]", &[0.0], 0.5, &audio, "[a]"),
            "[0:a:0]asplit=1[ga0];[ga0]atrim=start=0,asetpts=PTS-STARTPTS[gs0];\
             anullsrc=r=44100:cl=mono,atrim=duration=0.5[gz0];[gz0][gs0]concat=n=2:v=0:a=1[a]"
        );
        assert_eq!(
            gap_audio_filter("[x]", &[], 0.5, &audio, "[a]"),
            "[x]anull[a]"
        );
    }

    #[test]
    fn colors_and_helpers() {
        assert_eq!(color_hex("#FFaa00", "颜色").unwrap(), "0xffaa00");
        assert_eq!(color_hex("#fa0", "颜色").unwrap(), "0xffaa00");
        assert!(color_hex("red", "颜色").is_err());
        assert_eq!(even_crop(1280, 720), "");
        assert_eq!(even_crop(321, 241), ",crop=320:240:0:0");
        assert_eq!(float_text(2.0), "2");
        assert_eq!(float_text(0.25), "0.25");
    }
}
