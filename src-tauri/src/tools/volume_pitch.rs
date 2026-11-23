//! `tool.volume-pitch` 音量音调：调整每个视频第一条音轨的音量、音调，或把响度统一到目标值；
//! 视频流直接复制（不重编码），音频重编码。视频没有音轨时失败并说明原因。
//!
//! **参数**（括号内为默认值）
//! - `volumeMode` 音量（`normalize` 统一响度）：
//!   - `keep` 不改音量；
//!   - `gain` 增益：`gainUnit` 为 `db`（`gainDb`，0，-60～30 dB）或 `percent`（`gainPercent`，100，0～1000%）；
//!   - `normalize` 统一响度（EBU R128）：`targetLufs` 目标响度（-16 LUFS，-70～-5）、`truePeak` 真峰值上限
//!     （-1.5 dBTP，-9～0）、`loudnessRange` 响度范围（11 LU，1～50）。
//! - `pitchSemitones` 音调（0，-12～12 半音，可为小数），速度与时长不变。
//!
//! 什么都不改（音量不变且音调为 0、或增益为 0 dB / 100%）时报错。
//!
//! **实现**
//! - 音调：`asetrate=采样率×2^(n/12),aresample=采样率,atempo=1/比例`——先整体变速变调，再用 atempo 把速度
//!   还原，时长不变。不依赖可选的 librubberband，任何 FFmpeg 构建都能用。
//! - 统一响度：loudnorm **两遍**。第一遍（分析）在音调处理之后测量，`print_format=json:stats_file=临时文件`
//!   把测量结果写进工作项临时目录；第二遍把 `measured_I/TP/LRA/thresh`、`offset` 传回 loudnorm 并 `linear=true`
//!   （能线性调整时整体增益、不改变动态，否则 loudnorm 自动改用动态模式），因此一批视频最终响度一致。
//!   loudnorm 内部会升采样到 192 kHz，最后 `aresample` 回源采样率。静音的音轨无法统一响度，报错。
//! - 只处理第一条音轨；其余音轨、字幕不保留（会提示）。音频编码为 AAC（码率按声道数，见 `pipeline::aac_kbps`），
//!   webm 容器改用 Opus。容器见 `media::copy_container`（按设置“目标格式”，装不下时 mp4 / mkv）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::pipeline::{aac_kbps, escape_filter_path, fmt_num};
use crate::tools::media::copy_container;
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.volume-pitch";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VolumeMode {
    /// 不改音量
    Keep,
    /// 增益（dB 或百分比）
    Gain,
    /// 统一响度（loudnorm 两遍）
    #[default]
    Normalize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GainUnit {
    #[default]
    Db,
    Percent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VolumePitchParams {
    pub volume_mode: VolumeMode,
    pub gain_unit: GainUnit,
    /// 增益（dB）
    pub gain_db: f64,
    /// 增益（百分比，100 为不变）
    pub gain_percent: f64,
    /// 目标响度（LUFS）
    pub target_lufs: f64,
    /// 真峰值上限（dBTP）
    pub true_peak: f64,
    /// 目标响度范围（LU）
    pub loudness_range: f64,
    /// 音调（半音），速度不变
    pub pitch_semitones: f64,
}

impl Default for VolumePitchParams {
    fn default() -> Self {
        Self {
            volume_mode: VolumeMode::Normalize,
            gain_unit: GainUnit::Db,
            gain_db: 0.0,
            gain_percent: 100.0,
            target_lufs: -16.0,
            true_peak: -1.5,
            loudness_range: 11.0,
            pitch_semitones: 0.0,
        }
    }
}

fn check_range(value: f64, (min, max): (f64, f64), what: &str) -> Result<(), String> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!("{what}必须在 {min}～{max} 之间"))
    }
}

impl VolumePitchParams {
    pub fn validate(&self) -> Result<(), String> {
        check_range(self.pitch_semitones, (-12.0, 12.0), "音调（半音）")?;
        let gain_changes = match self.volume_mode {
            VolumeMode::Keep => false,
            VolumeMode::Normalize => {
                check_range(self.target_lufs, (-70.0, -5.0), "目标响度（LUFS）")?;
                check_range(self.true_peak, (-9.0, 0.0), "真峰值上限（dBTP）")?;
                check_range(self.loudness_range, (1.0, 50.0), "响度范围（LU）")?;
                true
            }
            VolumeMode::Gain => match self.gain_unit {
                GainUnit::Db => {
                    check_range(self.gain_db, (-60.0, 30.0), "增益（dB）")?;
                    self.gain_db != 0.0
                }
                GainUnit::Percent => {
                    check_range(self.gain_percent, (0.0, 1000.0), "音量百分比")?;
                    self.gain_percent != 100.0
                }
            },
        };
        if !gain_changes && self.pitch_semitones == 0.0 {
            return Err("参数没有任何改变：请调整音量、统一响度或音调".into());
        }
        Ok(())
    }
}

/// loudnorm 第一遍的测量结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loudness {
    pub input_i: f64,
    pub input_tp: f64,
    pub input_lra: f64,
    pub input_thresh: f64,
    pub target_offset: f64,
}

/// 解析 loudnorm `print_format=json` 的结果（`stats_file` 的内容，或日志中 `{ … }` 一段）。
pub fn parse_loudnorm_stats(text: &str) -> Result<Loudness, String> {
    let start = text.find('{').ok_or("响度测量没有输出结果")?;
    let end = text
        .rfind('}')
        .filter(|e| *e > start)
        .ok_or("响度测量结果不完整")?;
    let json: Value = serde_json::from_str(&text[start..=end])
        .map_err(|e| format!("响度测量结果无法解析：{e}"))?;
    let field = |key: &str| -> Result<f64, String> {
        let raw = json[key]
            .as_str()
            .map(str::to_owned)
            .or_else(|| json[key].as_f64().map(|v| v.to_string()))
            .ok_or_else(|| format!("响度测量结果缺少 {key}"))?;
        raw.trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| "音轨是静音（响度为 -∞），无法统一响度".to_owned())
    };
    Ok(Loudness {
        input_i: field("input_i")?,
        input_tp: field("input_tp")?,
        input_lra: field("input_lra")?,
        input_thresh: field("input_thresh")?,
        target_offset: field("target_offset")?,
    })
}

/// 音调滤镜（半音为 0 时为空）。
pub fn pitch_filter(semitones: f64, sample_rate: u32) -> Option<String> {
    if semitones == 0.0 {
        return None;
    }
    let ratio = 2f64.powf(semitones / 12.0);
    let shifted = (f64::from(sample_rate) * ratio).round().max(1.0);
    let tempo = f64::from(sample_rate) / shifted;
    Some(format!(
        "asetrate={},aresample={sample_rate},atempo={}",
        fmt_num(shifted),
        fmt_num(tempo)
    ))
}

fn loudnorm_targets(params: &VolumePitchParams) -> String {
    format!(
        "loudnorm=I={}:TP={}:LRA={}",
        fmt_num(params.target_lufs),
        fmt_num(params.true_peak),
        fmt_num(params.loudness_range)
    )
}

/// 第一遍（测量）的音频滤镜：音调处理之后测量响度，结果写进 `stats_file`。
pub fn measure_filter(params: &VolumePitchParams, sample_rate: u32, stats_file: &str) -> String {
    let mut chain: Vec<String> = pitch_filter(params.pitch_semitones, sample_rate)
        .into_iter()
        .collect();
    chain.push(format!(
        "{}:print_format=json:stats_file={}",
        loudnorm_targets(params),
        escape_filter_path(stats_file)
    ));
    chain.join(",")
}

/// 输出用的音频滤镜。统一响度时必须给出第一遍的测量结果。
pub fn apply_filter(
    params: &VolumePitchParams,
    sample_rate: u32,
    measured: Option<&Loudness>,
) -> Result<String, String> {
    params.validate()?;
    if sample_rate == 0 {
        return Err("读不到音频采样率".into());
    }
    let mut chain: Vec<String> = pitch_filter(params.pitch_semitones, sample_rate)
        .into_iter()
        .collect();
    match params.volume_mode {
        VolumeMode::Keep => {}
        VolumeMode::Gain => chain.push(match params.gain_unit {
            GainUnit::Db => format!("volume={}dB", fmt_num(params.gain_db)),
            GainUnit::Percent => format!("volume={}", fmt_num(params.gain_percent / 100.0)),
        }),
        VolumeMode::Normalize => {
            let m = measured.ok_or("统一响度需要先测量响度")?;
            let clamp = |v: f64, lo: f64, hi: f64| fmt_num(v.clamp(lo, hi));
            chain.push(format!(
                "{}:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true",
                loudnorm_targets(params),
                clamp(m.input_i, -99.0, 0.0),
                clamp(m.input_tp, -99.0, 99.0),
                clamp(m.input_lra, 0.0, 99.0),
                clamp(m.input_thresh, -99.0, 0.0),
                clamp(m.target_offset, -99.0, 99.0),
            ));
            chain.push(format!("aresample={sample_rate}"));
        }
    }
    Ok(chain.join(","))
}

/// 输出容器与音频编码参数。
pub fn output_plan(
    info: &MediaInfo,
    container: Option<&str>,
) -> Result<(String, Vec<String>), String> {
    let audio = info.audio.first().ok_or("视频没有音轨，无法调整音量音调")?;
    let ext = container.unwrap_or("m4a").to_owned();
    let codec = if ext == "webm" {
        vec![
            "-c:a".into(),
            "libopus".into(),
            "-b:a".into(),
            "128k".into(),
        ]
    } else {
        vec![
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            format!("{}k", aac_kbps(audio.channels)),
        ]
    };
    Ok((ext, codec))
}

/// 第二遍（输出）的完整参数（不含开头的通用参数，输出路径在最后）。
pub fn apply_args(
    input: &str,
    filter: &str,
    has_video: bool,
    codec: &[String],
    sample_rate: u32,
    ext: &str,
    output: &str,
) -> Vec<String> {
    let mut args: Vec<String> = vec!["-i".into(), input.into()];
    if has_video {
        args.extend(["-map".into(), "0:V:0".into(), "-c:v".into(), "copy".into()]);
    }
    args.extend(["-map".into(), "0:a:0".into(), "-af".into(), filter.into()]);
    args.extend(codec.iter().cloned());
    args.extend(["-ar".into(), sample_rate.to_string()]);
    args.extend(["-sn".into(), "-dn".into()]);
    if matches!(ext, "mp4" | "mov" | "m4v" | "m4a") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

impl Tool for VolumePitchParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let info = ctx.info(0)?;
        let input = ctx.input_path(0)?.to_path_buf();
        let audio = info
            .audio
            .first()
            .ok_or("视频没有音轨，无法调整音量音调")?
            .clone();
        let sample_rate = audio.sample_rate;
        if sample_rate == 0 {
            return Err("读不到音频采样率".into());
        }
        if info.audio.len() > 1 {
            ctx.warn("只处理第一条音轨，其余音轨不保留");
        }
        if !info.subtitles.is_empty() {
            ctx.warn("字幕轨不保留");
        }
        let container = info
            .video
            .as_ref()
            .map(|v| copy_container(ctx.settings(), &input, &v.codec));
        let (ext, codec) = output_plan(&info, container.as_deref())?;
        let input_arg = path_arg(&input)?;

        let measured = if self.volume_mode == VolumeMode::Normalize {
            ctx.set_steps(&[1.0, 1.0]);
            let stats = ctx.temp_file("loudnorm.json")?;
            let filter = measure_filter(self, sample_rate, &path_arg(&stats)?);
            let args = vec![
                "-i".into(),
                input_arg.clone(),
                "-map".into(),
                "0:a:0".into(),
                "-vn".into(),
                "-sn".into(),
                "-dn".into(),
                "-af".into(),
                filter,
                "-f".into(),
                "null".into(),
                "-".into(),
            ];
            ctx.run_ffmpeg("测量响度", args, info.duration)?;
            let text =
                std::fs::read_to_string(&stats).map_err(|e| format!("读不到响度测量结果：{e}"))?;
            let loudness = parse_loudnorm_stats(&text)?;
            ctx.note(format!(
                "测得响度 {} LUFS（真峰值 {} dBTP），目标 {} LUFS",
                fmt_num(loudness.input_i),
                fmt_num(loudness.input_tp),
                fmt_num(self.target_lufs)
            ));
            Some(loudness)
        } else {
            None
        };

        let filter = apply_filter(self, sample_rate, measured.as_ref())?;
        let output = ctx.output(&OutputSpec::file(&ext))?;
        let args = apply_args(
            &input_arg,
            &filter,
            info.video.is_some(),
            &codec,
            sample_rate,
            &ext,
            &path_arg(&output)?,
        );
        ctx.run_ffmpeg("音量音调", args, info.duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::sample_info;

    const STATS: &str = "[Parsed_loudnorm_0 @ 0000] \n{\n\t\"input_i\" : \"-27.61\",\n\t\"input_tp\" : \"-4.47\",\n\t\"input_lra\" : \"18.06\",\n\t\"input_thresh\" : \"-39.20\",\n\t\"output_i\" : \"-16.58\",\n\t\"output_tp\" : \"-1.50\",\n\t\"output_lra\" : \"14.78\",\n\t\"output_thresh\" : \"-27.71\",\n\t\"normalization_type\" : \"dynamic\",\n\t\"target_offset\" : \"0.58\"\n}\n[out#0/null @ 0] video:0KiB";

    fn gain_db(db: f64) -> VolumePitchParams {
        VolumePitchParams {
            volume_mode: VolumeMode::Gain,
            gain_db: db,
            ..VolumePitchParams::default()
        }
    }

    #[test]
    fn parses_loudnorm_json_from_log_or_stats_file() {
        let m = parse_loudnorm_stats(STATS).unwrap();
        assert_eq!(
            m,
            Loudness {
                input_i: -27.61,
                input_tp: -4.47,
                input_lra: 18.06,
                input_thresh: -39.2,
                target_offset: 0.58
            }
        );
        let silent = "{\"input_i\" : \"-inf\", \"input_tp\" : \"-inf\", \"input_lra\" : \"0.00\", \"input_thresh\" : \"-70.00\", \"target_offset\" : \"inf\"}";
        assert!(parse_loudnorm_stats(silent).unwrap_err().contains("静音"));
        assert!(parse_loudnorm_stats("no json here").is_err());
        assert!(parse_loudnorm_stats("{\"input_i\" : \"-20\"}").is_err());
    }

    #[test]
    fn pitch_keeps_tempo() {
        assert_eq!(pitch_filter(0.0, 48_000), None);
        assert_eq!(
            pitch_filter(12.0, 44_100).unwrap(),
            "asetrate=88200,aresample=44100,atempo=0.5"
        );
        assert_eq!(
            pitch_filter(-12.0, 48_000).unwrap(),
            "asetrate=24000,aresample=48000,atempo=2"
        );
        // 3 半音：48000 × 2^(3/12) = 57082.4 → 57082
        assert_eq!(
            pitch_filter(3.0, 48_000).unwrap(),
            "asetrate=57082,aresample=48000,atempo=0.8409"
        );
    }

    #[test]
    fn gain_and_percent_filters() {
        assert_eq!(
            apply_filter(&gain_db(6.0), 44_100, None).unwrap(),
            "volume=6dB"
        );
        assert_eq!(
            apply_filter(&gain_db(-3.5), 44_100, None).unwrap(),
            "volume=-3.5dB"
        );
        let percent = VolumePitchParams {
            volume_mode: VolumeMode::Gain,
            gain_unit: GainUnit::Percent,
            gain_percent: 150.0,
            pitch_semitones: 12.0,
            ..VolumePitchParams::default()
        };
        assert_eq!(
            apply_filter(&percent, 44_100, None).unwrap(),
            "asetrate=88200,aresample=44100,atempo=0.5,volume=1.5"
        );
        let pitch_only = VolumePitchParams {
            volume_mode: VolumeMode::Keep,
            pitch_semitones: -12.0,
            ..VolumePitchParams::default()
        };
        assert_eq!(
            apply_filter(&pitch_only, 48_000, None).unwrap(),
            "asetrate=24000,aresample=48000,atempo=2"
        );
    }

    #[test]
    fn normalize_uses_measured_values_linearly() {
        let params = VolumePitchParams::default();
        let m = parse_loudnorm_stats(STATS).unwrap();
        assert_eq!(
            apply_filter(&params, 48_000, Some(&m)).unwrap(),
            "loudnorm=I=-16:TP=-1.5:LRA=11:measured_I=-27.61:measured_TP=-4.47:measured_LRA=18.06:measured_thresh=-39.2:offset=0.58:linear=true,aresample=48000"
        );
        assert!(apply_filter(&params, 48_000, None).is_err(), "必须先测量");
        assert_eq!(
            measure_filter(&params, 48_000, r"C:\t\l.json"),
            r"loudnorm=I=-16:TP=-1.5:LRA=11:print_format=json:stats_file=C\\:/t/l.json"
        );
        let shifted = VolumePitchParams {
            pitch_semitones: 12.0,
            ..params
        };
        assert_eq!(
            measure_filter(&shifted, 44_100, "l.json"),
            "asetrate=88200,aresample=44100,atempo=0.5,loudnorm=I=-16:TP=-1.5:LRA=11:print_format=json:stats_file=l.json"
        );
    }

    #[test]
    fn invalid_or_neutral_params_are_rejected() {
        assert!(gain_db(0.0).validate().is_err(), "0 dB 且不改音调");
        assert!(gain_db(31.0).validate().is_err());
        assert!(gain_db(f64::NAN).validate().is_err());
        let keep = VolumePitchParams {
            volume_mode: VolumeMode::Keep,
            ..VolumePitchParams::default()
        };
        assert!(keep.validate().is_err());
        let percent = VolumePitchParams {
            volume_mode: VolumeMode::Gain,
            gain_unit: GainUnit::Percent,
            ..VolumePitchParams::default()
        };
        assert!(percent.validate().is_err(), "100% 不改变");
        for bad in [
            VolumePitchParams {
                target_lufs: -4.0,
                ..VolumePitchParams::default()
            },
            VolumePitchParams {
                true_peak: 1.0,
                ..VolumePitchParams::default()
            },
            VolumePitchParams {
                loudness_range: 0.5,
                ..VolumePitchParams::default()
            },
            VolumePitchParams {
                pitch_semitones: 13.0,
                ..VolumePitchParams::default()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        assert!(VolumePitchParams::default().validate().is_ok());
        assert!(apply_filter(&gain_db(6.0), 0, None).is_err(), "采样率未知");
    }

    #[test]
    fn output_args_copy_video_and_encode_first_track() {
        let info = sample_info();
        let (ext, codec) = output_plan(&info, Some("mp4")).unwrap();
        assert_eq!(ext, "mp4");
        let args = apply_args(
            "in.mp4",
            "volume=6dB",
            true,
            &codec,
            44_100,
            &ext,
            "out.mp4",
        );
        assert_eq!(
            args.join(" "),
            "-i in.mp4 -map 0:V:0 -c:v copy -map 0:a:0 -af volume=6dB -c:a aac -b:a 192k -ar 44100 -sn -dn -movflags +faststart out.mp4"
        );
        let (webm, opus) = output_plan(&info, Some("webm")).unwrap();
        assert_eq!(
            (webm.as_str(), opus.join(" ").as_str()),
            ("webm", "-c:a libopus -b:a 128k")
        );
        let mut silent = sample_info();
        silent.audio.clear();
        assert!(output_plan(&silent, Some("mp4"))
            .unwrap_err()
            .contains("没有音轨"));
    }
}
