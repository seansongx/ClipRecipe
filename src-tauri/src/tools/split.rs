//! `tool.split` 视频分割：按时长、按段数或按场景切换点分割；重编码（默认，切点准确）或无损（只能在关键帧处切开）。
//!
//! 参考实现 · **一进多出 + 先分析后处理**：
//! - 按场景分割时先 `ctx.analyze_input(0, &SceneDetect{..})` 得到切点，再用 `ctx.split_next_step` 把剩下的进度
//!   按片段时长分成若干步；
//! - 重编码：每段一条 `ctx.run_encode`（`-ss`/`-t` 放在输入前，转码时切点精确到帧），输出
//!   `<名称>/<名称>_01.mp4`…；
//! - 无损：一条 segment 命令把片段写进 `ctx.temp_dir()`，再逐个 `ctx.keep_file` 成为输出（临时目录自动删除）。
//!
//! 切点规划是纯函数 [`plan_cuts`] / [`apply_min_length`]。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::fsutil;
use crate::tools::analysis::{segments_from_cuts, Interval, SceneDetect};
use crate::tools::context::Encoding;
use crate::tools::media::{copy_container, secs};
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.split";

/// 段数上限（避免误填导致生成成千上万个文件）。
pub const MAX_PIECES: usize = 1000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SplitMode {
    /// 每段固定时长
    #[default]
    Duration,
    /// 平均分成 N 段
    Count,
    /// 在场景切换点切开
    Scene,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SplitParams {
    pub mode: SplitMode,
    /// 按时长：每段秒数
    pub segment_seconds: f64,
    /// 按段数：段数
    pub count: u32,
    /// 无损（不重编码，切点会落在关键帧上，时长不精确）
    pub lossless: bool,
    /// 按场景：场景切换阈值（0～1，越小越敏感）
    pub scene_threshold: f64,
    /// 最短片段（秒）：按时长时末段短于它就并入上一段；按场景时相邻切点至少间隔这么久
    pub min_segment_seconds: f64,
}

impl Default for SplitParams {
    fn default() -> Self {
        Self {
            mode: SplitMode::Duration,
            segment_seconds: 60.0,
            count: 2,
            lossless: false,
            scene_threshold: 0.3,
            min_segment_seconds: 1.0,
        }
    }
}

impl SplitParams {
    fn validate(&self) -> Result<(), String> {
        if !(self.min_segment_seconds.is_finite() && self.min_segment_seconds >= 0.0) {
            return Err("最短片段不能为负数".into());
        }
        match self.mode {
            SplitMode::Duration
                if !(self.segment_seconds.is_finite() && self.segment_seconds >= 0.1) =>
            {
                Err("每段时长至少 0.1 秒".into())
            }
            SplitMode::Count if self.count < 2 || self.count as usize > MAX_PIECES => {
                Err(format!("段数必须在 2～{MAX_PIECES} 之间"))
            }
            SplitMode::Scene if !(0.0..=1.0).contains(&self.scene_threshold) => {
                Err("场景切换阈值必须在 0～1 之间".into())
            }
            _ => Ok(()),
        }
    }

    fn scene_detect(&self) -> SceneDetect {
        SceneDetect {
            threshold: self.scene_threshold,
            min_gap: self.min_segment_seconds,
        }
    }
}

/// 按时长 / 段数的切点（不含 0 与结尾）。按场景时返回空（切点来自检测）。
pub fn plan_cuts(params: &SplitParams, duration: f64) -> Result<Vec<f64>, String> {
    params.validate()?;
    if !(duration.is_finite() && duration > 0.0) {
        return Err("无法读取视频时长".into());
    }
    let cuts = match params.mode {
        SplitMode::Duration => {
            let step = params.segment_seconds;
            if (duration / step).ceil() as usize > MAX_PIECES {
                return Err(format!("片段太多（超过 {MAX_PIECES} 段），请加大每段时长"));
            }
            let mut cuts = Vec::new();
            let mut k = 1.0;
            while step * k < duration - 1e-6 {
                cuts.push(step * k);
                k += 1.0;
            }
            apply_min_length(cuts, duration, params.min_segment_seconds)
        }
        SplitMode::Count => {
            let n = params.count as usize;
            if duration / (n as f64) < 0.1 {
                return Err("视频太短，无法分成这么多段".into());
            }
            (1..n).map(|k| duration * k as f64 / n as f64).collect()
        }
        SplitMode::Scene => Vec::new(),
    };
    Ok(cuts)
}

/// 末段短于 `min_len` 时去掉最后一个切点（并入上一段）。
pub fn apply_min_length(mut cuts: Vec<f64>, duration: f64, min_len: f64) -> Vec<f64> {
    if let Some(&last) = cuts.last() {
        if duration - last < min_len {
            cuts.pop();
        }
    }
    cuts
}

/// 片段文件名的后缀：`_01`、`_02`…（位数随段数，至少两位）。
pub fn piece_suffix(index: usize, total: usize) -> String {
    let width = total.max(1).to_string().len().max(2);
    format!("_{:0width$}", index + 1)
}

/// 重编码一段的参数（不含开头的通用参数）。
pub fn encode_piece_args(
    input: &str,
    output: &str,
    piece: Interval,
    info: &MediaInfo,
    encoding: &Encoding,
    container: &str,
) -> Vec<String> {
    let mut args = vec![
        "-ss".into(),
        secs(piece.start),
        "-t".into(),
        secs(piece.duration()),
        "-i".into(),
        input.into(),
        "-map".into(),
        "0:V:0".into(),
    ];
    if let Some(audio) = info.audio.first() {
        args.extend(["-map".into(), "0:a:0".into()]);
        args.extend(encoding.video_args(container));
        args.extend(Encoding::audio_args(audio.channels));
    } else {
        args.extend(encoding.video_args(container));
    }
    args.extend(["-sn".into(), "-dn".into()]);
    if matches!(container, "mp4" | "mov" | "m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

/// 无损分割（segment 分离器）的参数：片段写成 `pattern`（含 `%04d`）。
pub fn lossless_args(input: &str, pattern: &str, cuts: &[f64], container: &str) -> Vec<String> {
    let mut args = vec![
        "-i".into(),
        input.into(),
        "-map".into(),
        "0:V:0".into(),
        "-map".into(),
        "0:a?".into(),
        "-c".into(),
        "copy".into(),
        "-sn".into(),
        "-dn".into(),
        "-f".into(),
        "segment".into(),
        "-reset_timestamps".into(),
        "1".into(),
    ];
    if cuts.is_empty() {
        // 没有切点：整段作为一个片段
        args.extend(["-segment_time".into(), "86400000".into()]);
    } else {
        let times: Vec<String> = cuts.iter().map(|t| secs(*t)).collect();
        args.extend(["-segment_times".into(), times.join(",")]);
    }
    if matches!(container, "mp4" | "mov" | "m4v") {
        args.extend([
            "-segment_format_options".into(),
            "movflags=+faststart".into(),
        ]);
    }
    args.push(pattern.into());
    args
}

impl Tool for SplitParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let info = ctx.info(0)?;
        let input_path = ctx.input_path(0)?.to_path_buf();
        let input = path_arg(&input_path)?;
        let mut cuts = plan_cuts(self, info.duration)?;

        if self.mode == SplitMode::Scene {
            // 第 1 步分析（约占 1/4），剩下的在得知片段后再细分
            ctx.set_steps(&[1.0, 3.0]);
            cuts = ctx.analyze_input(0, &self.scene_detect())?;
            if cuts.is_empty() {
                ctx.warn("没有检测到场景切换，输出整段（可调低场景切换阈值）");
            } else {
                ctx.note(format!("检测到 {} 个场景切换点", cuts.len()));
            }
        }
        let pieces = segments_from_cuts(&cuts, info.duration);

        if self.lossless {
            let video_codec = info
                .video
                .as_ref()
                .map(|v| v.codec.clone())
                .unwrap_or_default();
            let container = copy_container(ctx.settings(), &input_path, &video_codec);
            let dir = ctx.temp_dir()?;
            let pattern = dir.join(format!("piece_%04d.{container}"));
            ctx.run_ffmpeg(
                "无损分割",
                lossless_args(&input, &path_arg(&pattern)?, &cuts, &container),
                info.duration,
            )?;
            let produced = fsutil::list_files(&dir, false, &[container.as_str()])
                .map_err(|e| format!("无法读取分割结果：{e}"))?;
            if produced.len() != pieces.len() {
                ctx.warn(format!(
                    "无损分割只能在关键帧处切开，实际得到 {} 段（计划 {} 段）",
                    produced.len(),
                    pieces.len()
                ));
            }
            for (i, file) in produced.iter().enumerate() {
                let spec = OutputSpec::file(&container)
                    .suffix(piece_suffix(i, produced.len()))
                    .in_stem_folder();
                ctx.keep_file(file, &spec)?;
            }
            return Ok(());
        }

        let container = ctx.video_extension(0)?;
        let weights: Vec<f64> = pieces.iter().map(Interval::duration).collect();
        if self.mode == SplitMode::Scene {
            ctx.split_next_step(&weights);
        } else {
            ctx.set_steps(&weights);
        }
        for (i, piece) in pieces.iter().enumerate() {
            ctx.check_cancelled()?;
            let spec = OutputSpec::file(&container)
                .suffix(piece_suffix(i, pieces.len()))
                .in_stem_folder();
            let output = path_arg(&ctx.output(&spec)?)?;
            ctx.run_encode("分割", piece.duration(), |enc| {
                Ok(encode_piece_args(
                    &input, &output, *piece, &info, enc, &container,
                ))
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{VideoCodec, VideoEncoder};
    use crate::pipeline::testing::sample_info;

    fn params(mode: SplitMode) -> SplitParams {
        SplitParams {
            mode,
            ..SplitParams::default()
        }
    }

    #[test]
    fn duration_cuts_merge_short_tail() {
        let p = SplitParams {
            segment_seconds: 2.0,
            min_segment_seconds: 0.5,
            ..params(SplitMode::Duration)
        };
        assert_eq!(plan_cuts(&p, 5.0).unwrap(), [2.0, 4.0]);
        assert_eq!(plan_cuts(&p, 4.3).unwrap(), [2.0], "末段 0.3 秒并入上一段");
        assert_eq!(plan_cuts(&p, 4.0).unwrap(), [2.0]);
        assert!(plan_cuts(&p, 1.0).unwrap().is_empty());
        let tiny = SplitParams {
            segment_seconds: 0.01,
            ..p.clone()
        };
        assert!(plan_cuts(&tiny, 10.0).is_err());
        assert!(plan_cuts(&p, 0.0).is_err());
    }

    #[test]
    fn count_cuts_are_even() {
        let p = SplitParams {
            count: 3,
            ..params(SplitMode::Count)
        };
        assert_eq!(plan_cuts(&p, 6.0).unwrap(), [2.0, 4.0]);
        let bad = SplitParams {
            count: 1,
            ..params(SplitMode::Count)
        };
        assert!(plan_cuts(&bad, 6.0).is_err());
        assert!(plan_cuts(&params(SplitMode::Scene), 6.0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn piece_names() {
        assert_eq!(piece_suffix(0, 3), "_01");
        assert_eq!(piece_suffix(99, 120), "_100");
    }

    #[test]
    fn encode_args_seek_before_input() {
        let encoding = Encoding {
            encoder: VideoEncoder::software(VideoCodec::H264),
            video_kbps: Some(1000),
        };
        let args = encode_piece_args(
            "in.mp4",
            "out.mp4",
            Interval::new(2.0, 4.5),
            &sample_info(),
            &encoding,
            "mp4",
        );
        assert_eq!(
            args.join(" "),
            "-ss 2 -t 2.5 -i in.mp4 -map 0:V:0 -map 0:a:0 -c:v libx264 -preset medium -pix_fmt yuv420p \
             -b:v 1000k -maxrate 1500k -bufsize 2000k -c:a aac -b:a 192k -sn -dn -movflags +faststart out.mp4"
        );
    }

    #[test]
    fn lossless_args_use_segment_times() {
        let args = lossless_args("in.mp4", "p_%04d.mp4", &[2.0, 4.5], "mp4").join(" ");
        assert!(args.contains("-c copy"), "{args}");
        assert!(
            args.contains("-f segment -reset_timestamps 1 -segment_times 2,4.5"),
            "{args}"
        );
        assert!(
            args.ends_with("-segment_format_options movflags=+faststart p_%04d.mp4"),
            "{args}"
        );
        let whole = lossless_args("in.mkv", "p_%04d.mkv", &[], "mkv").join(" ");
        assert!(whole.contains("-segment_time 86400000 p_%04d.mkv"), "{whole}");
    }

    #[test]
    fn scene_detect_uses_threshold_and_min_length() {
        let p = SplitParams {
            scene_threshold: 0.5,
            min_segment_seconds: 2.0,
            ..params(SplitMode::Scene)
        };
        assert_eq!(
            p.scene_detect(),
            SceneDetect {
                threshold: 0.5,
                min_gap: 2.0
            }
        );
        let bad = SplitParams {
            scene_threshold: 2.0,
            ..p
        };
        assert!(plan_cuts(&bad, 5.0).is_err());
    }
}
