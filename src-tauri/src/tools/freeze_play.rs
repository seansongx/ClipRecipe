//! `tool.freeze-play` 定格播放：在指定位置暂停一段时间再继续播放；暂停期间可在画面中央叠加暂停图标，声音静音。
//! 一进一出，重编码，视频变长 `位置数 × 暂停时长`。
//!
//! - **位置** `positions`：一个或多个时间点，用逗号、分号、空格或换行分隔；每个可以是秒数（`12.5`）、
//!   `分:秒` / `时:分:秒`（`1:05`、`0:01:05.5`）或百分比（`50%`）。超出时长的位置忽略并提示；落在同一帧上的合并。
//!   暂停的画面是该时间点（或之后）的第一帧，暂停结束后从下一帧继续。
//! - **暂停时长** `pauseSeconds`（0.01～60 秒，按帧取整，至少 1 帧）。
//! - **暂停图标** `showIcon`：`icon` 为空时用内置图标（半透明深色方块 + 两条白色竖条，用 `drawbox` 画）；
//!   否则用图片（png / jpg / webp / bmp / gif；目录模式按素材规则随机或依次取一张，gif 循环播放），
//!   按 `iconSize`（画面短边的百分比，5～80）等比缩放后居中，只在暂停期间显示。
//! - 声音：暂停期间插入静音（与 `tool.freeze-anim` 的“插入停顿”相同），音画同步。
//!
//! 实现与 `tool.freeze-anim` 共用 [`gap_video_filter`] / [`gap_audio_filter`]（[`crate::tools::common::filter_job`]）；
//! 位置解析 [`parse_positions`] / [`parse_time`] 在 [`crate::tools::common::positions`]。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::model::AssetSource;
use crate::pipeline::modules::text::escape_value;
use crate::tools::common::filter_job::{
    check_duration, display_size, even_crop, gap_audio_filter, gap_video_filter, run_job, AudioOut,
    FilterJob, Rate,
};
use crate::tools::freeze_anim::MAX_HOLDS;
use crate::tools::{Tool, ToolContext, ToolError};

pub const ID: &str = "tool.freeze-play";

// 共用的位置解析与图片输入已移到 `tools::common`，这里保留原名的再导出。
pub use crate::tools::common::filter_job::{image_input_args, IMAGE_EXTENSIONS};
pub use crate::tools::common::positions::{parse_positions, parse_time, position_frames, windows_expr};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FreezePlayParams {
    /// 暂停位置（秒、分:秒、百分比，逗号 / 空格 / 换行分隔）
    pub positions: String,
    /// 每次暂停的时长（秒）
    pub pause_seconds: f64,
    /// 暂停期间显示图标
    pub show_icon: bool,
    /// 图标图片；为空时用内置图标
    pub icon: AssetSource,
    /// 图标大小：画面短边的百分比
    pub icon_size: f64,
}

impl Default for FreezePlayParams {
    fn default() -> Self {
        Self {
            positions: "50%".into(),
            pause_seconds: 1.0,
            show_icon: true,
            icon: AssetSource::default(),
            icon_size: 20.0,
        }
    }
}


impl FreezePlayParams {
    fn validate(&self) -> Result<(), String> {
        if !(self.pause_seconds.is_finite() && (0.01..=60.0).contains(&self.pause_seconds)) {
            return Err("暂停时长必须在 0.01～60 秒之间".into());
        }
        if self.show_icon && !(self.icon_size.is_finite() && (5.0..=80.0).contains(&self.icon_size))
        {
            return Err("图标大小必须在 5%～80% 之间".into());
        }
        Ok(())
    }
}

/// 内置暂停图标：居中的半透明深色方块 + 两条白色竖条（`drawbox`，只在 `enable` 为真时绘制）。
pub fn builtin_icon(w: u32, h: u32, size_percent: f64, enable: &str) -> String {
    let side = ((f64::from(w.min(h)) * size_percent / 100.0).round() as u32).max(8);
    let (cx, cy) = (w / 2, h / 2);
    let bar_w = ((f64::from(side) * 0.16).round() as u32).max(1);
    let bar_h = ((f64::from(side) * 0.5).round() as u32).max(2);
    let gap = ((f64::from(side) * 0.14).round() as u32).max(1);
    let enable = escape_value(enable);
    let boxes = [
        (
            cx.saturating_sub(side / 2),
            cy.saturating_sub(side / 2),
            side,
            side,
            "black@0.5",
        ),
        (
            cx.saturating_sub(gap / 2 + bar_w),
            cy.saturating_sub(bar_h / 2),
            bar_w,
            bar_h,
            "white@0.9",
        ),
        (
            cx + gap - gap / 2,
            cy.saturating_sub(bar_h / 2),
            bar_w,
            bar_h,
            "white@0.9",
        ),
    ];
    boxes
        .iter()
        .map(|(x, y, bw, bh, color)| {
            format!("drawbox=x={x}:y={y}:w={bw}:h={bh}:color={color}:t=fill:enable={enable}")
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// 生成命令；`icon` 为要叠加的图标图片（None 时按 `show_icon` 用内置图标或不显示）。
pub fn build(
    params: &FreezePlayParams,
    info: &MediaInfo,
    input: &str,
    icon: Option<&str>,
) -> Result<FilterJob, String> {
    params.validate()?;
    check_duration(info.duration)?;
    let (w, h) = display_size(info)?;
    let rate = Rate::of(info)?;
    let (times, mut warnings) = parse_positions(&params.positions, info.duration)?;
    let frames = position_frames(&times, &rate, info, &mut warnings);
    if frames.len() > MAX_HOLDS {
        return Err(format!("暂停位置太多（最多 {MAX_HOLDS} 个）"));
    }
    let gap_frames = rate.frames_for(params.pause_seconds);
    let gap = rate.seconds(f64::from(gap_frames));

    let mut job = FilterJob::new(input);
    job.warnings = warnings;
    job.duration = info.duration + gap * frames.len() as f64;
    let (ew, eh) = (w & !1, h & !1);
    let windows: Vec<(f64, f64)> = frames
        .iter()
        .enumerate()
        .map(|(k, f)| {
            let start = rate.seconds((f + 1) as f64) + gap * k as f64;
            (start, start + gap)
        })
        .collect();
    let enable = windows_expr(&windows, &rate);
    let held = format!(
        "[0:v:0]{}{}",
        gap_video_filter(&frames, gap_frames, &rate),
        even_crop(w, h)
    );
    job.graph = match (params.show_icon, icon) {
        (false, _) => format!("{held}[v]"),
        (true, None) => format!(
            "{held},{}[v]",
            builtin_icon(ew, eh, params.icon_size, &enable)
        ),
        (true, Some(path)) => {
            let index = job.add_input(image_input_args(path), path);
            let side = ((f64::from(ew.min(eh)) * params.icon_size / 100.0).round() as u32).max(8);
            format!(
                "{held}[vh];[{index}:v:0]scale={side}:{side}:force_original_aspect_ratio=decrease[icon];\
                 [vh][icon]overlay=x=(W-w)/2:y=(H-h)/2:shortest=1:enable={}[v]",
                escape_value(&enable)
            )
        }
    };
    if let Some(audio) = info.audio.first() {
        let cuts: Vec<f64> = frames
            .iter()
            .map(|f| rate.seconds((f + 1) as f64))
            .collect();
        job.graph.push(';');
        job.graph
            .push_str(&gap_audio_filter("[0:a:0]", &cuts, gap, audio, "[a]"));
        job.audio = AudioOut::Filtered {
            channels: audio.channels,
        };
    }
    Ok(job)
}

impl Tool for FreezePlayParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn supports_preview(&self) -> bool {
        true
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let info = ctx.info(0)?;
        let input = path_arg(ctx.input_path(0)?)?;
        let icon = if self.show_icon && !self.icon.is_empty() {
            let index = ctx.number().saturating_sub(1);
            let picked = self.icon.pick(IMAGE_EXTENSIONS, index, &mut ctx.rng)?;
            Some(path_arg(&picked)?)
        } else {
            None
        };
        let job = build(self, &info, &input, icon.as_deref())?;
        run_job(ctx, "定格播放", &job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::sample_info;

    #[test]
    fn times_in_all_formats() {
        assert_eq!(parse_time("12.5", 100.0).unwrap(), 12.5);
        assert_eq!(parse_time("1:05", 100.0).unwrap(), 65.0);
        assert_eq!(parse_time("0:01:05.5", 100.0).unwrap(), 65.5);
        assert_eq!(parse_time("25%", 100.0).unwrap(), 25.0);
        assert_eq!(parse_time(" 50％ ", 10.0).unwrap(), 5.0);
        assert_eq!(parse_time("1：30", 100.0).unwrap(), 90.0);
        for bad in ["abc", "120%", "-1", "1:2:3:4", "1:-2", ""] {
            assert!(parse_time(bad, 100.0).is_err(), "{bad}");
        }
    }

    #[test]
    fn position_lists() {
        let (times, warnings) = parse_positions("8, 2；50%\n100%  20", 10.0).unwrap();
        assert_eq!(times, [2.0, 5.0, 8.0]);
        assert_eq!(
            warnings,
            [
                "位置 100% 超出视频时长，已忽略",
                "位置 20 超出视频时长，已忽略"
            ]
        );
        assert!(parse_positions("  ", 10.0).is_err());
        assert!(parse_positions("30", 10.0).is_err());
        assert!(parse_positions("1, x", 10.0).is_err());
    }

    #[test]
    fn frames_are_merged() {
        let rate = Rate::of(&sample_info()).unwrap();
        let mut warnings = Vec::new();
        let frames = position_frames(
            &[1.0, 1.01, 2.0, 9.999],
            &rate,
            &sample_info(),
            &mut warnings,
        );
        assert_eq!(frames, [25, 26, 50, 249]);
        let frames = position_frames(&[1.0, 1.0], &rate, &sample_info(), &mut warnings);
        assert_eq!(frames, [25]);
        assert_eq!(warnings, ["有 1 个位置落在同一帧上，已合并"]);
    }

    #[test]
    fn builtin_icon_command() {
        let p = FreezePlayParams {
            positions: "2, 50%".into(),
            pause_seconds: 1.0,
            ..FreezePlayParams::default()
        };
        let job = build(&p, &sample_info(), "in.mp4", None).unwrap();
        // 第 50、125 帧之后各停 25 帧；暂停窗口 [2.04, 3.04)、[6.04, 7.04)
        let enable = "gt(t\\,2.02)*lt(t\\,3.02)+gt(t\\,6.02)*lt(t\\,7.02)";
        assert_eq!(
            job.graph,
            format!(
                "[0:v:0]fps=25,setpts=PTS+(gt(N\\,50)+gt(N\\,125))*25*1/25/TB,fps=25:start_time=0,\
                 drawbox=x=568:y=288:w=144:h=144:color=black@0.5:t=fill:enable={enable},\
                 drawbox=x=607:y=324:w=23:h=72:color=white@0.9:t=fill:enable={enable},\
                 drawbox=x=650:y=324:w=23:h=72:color=white@0.9:t=fill:enable={enable}[v];\
                 [0:a:0]asplit=3[ga0][ga1][ga2];\
                 [ga0]atrim=end=2.04,asetpts=PTS-STARTPTS[gs0];\
                 [ga1]atrim=start=2.04:end=5.04,asetpts=PTS-STARTPTS[gs1];\
                 [ga2]atrim=start=5.04,asetpts=PTS-STARTPTS[gs2];\
                 anullsrc=r=48000:cl=stereo,atrim=duration=1[gz0];\
                 anullsrc=r=48000:cl=stereo,atrim=duration=1[gz1];\
                 [gs0][gz0][gs1][gz1][gs2]concat=n=5:v=0:a=1[a]"
            )
        );
        assert_eq!(job.duration, 12.0);
    }

    #[test]
    fn custom_icon_and_no_icon() {
        let p = FreezePlayParams {
            positions: "1".into(),
            pause_seconds: 0.5,
            ..FreezePlayParams::default()
        };
        let mut info = sample_info();
        info.audio.clear();
        let job = build(&p, &info, "in.mp4", Some("C:/i/pause.gif")).unwrap();
        assert_eq!(
            job.inputs,
            ["-i", "in.mp4", "-stream_loop", "-1", "-i", "C:/i/pause.gif"]
        );
        assert_eq!(
            job.graph,
            "[0:v:0]fps=25,setpts=PTS+(gt(N\\,25))*13*1/25/TB,fps=25:start_time=0[vh];\
             [1:v:0]scale=144:144:force_original_aspect_ratio=decrease[icon];\
             [vh][icon]overlay=x=(W-w)/2:y=(H-h)/2:shortest=1:enable=gt(t\\,1.02)*lt(t\\,1.54)[v]"
        );
        assert_eq!(job.audio, AudioOut::None);
        let hidden = FreezePlayParams {
            show_icon: false,
            icon_size: 0.0,
            ..p
        };
        let job = build(&hidden, &info, "in.mp4", None).unwrap();
        assert_eq!(
            job.graph,
            "[0:v:0]fps=25,setpts=PTS+(gt(N\\,25))*13*1/25/TB,fps=25:start_time=0[v]"
        );
        assert_eq!(image_input_args("a.PNG"), ["-loop", "1"]);
    }

    #[test]
    fn invalid_params() {
        for p in [
            FreezePlayParams {
                pause_seconds: 0.0,
                ..FreezePlayParams::default()
            },
            FreezePlayParams {
                icon_size: 100.0,
                ..FreezePlayParams::default()
            },
            FreezePlayParams {
                positions: "".into(),
                ..FreezePlayParams::default()
            },
            FreezePlayParams {
                positions: "11".into(),
                ..FreezePlayParams::default()
            },
        ] {
            assert!(build(&p, &sample_info(), "in.mp4", None).is_err(), "{p:?}");
        }
    }
}
