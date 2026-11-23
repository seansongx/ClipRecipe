//! `tool.extract-av` 音视频提取：导出音频（可全部音轨）和 / 或纯视频（去掉音频）。
//!
//! 参考实现 · **一进多出**：一条 ffmpeg 命令带多个输出（每个输出一组 `-map`/编码参数 + 一个
//! `ctx.output(..)` 路径），只读一遍源文件。规划是纯函数 [`plan_outputs`]。
//!
//! - 能不重编码就不重编码：原格式导出时音频直接复制（aac → m4a、mp3 → mp3、PCM → wav…）；
//!   选 mp3 / m4a / wav 时源编码相同也直接复制，否则转码（mp3 码率可选）。
//! - 纯视频：视频流直接复制，去掉音频、字幕、数据流；容器见 [`copy_container`]。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::model::Settings;
use crate::pipeline::aac_kbps;
use crate::tools::media::{audio_copy_extension, copy_container};
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};
use std::path::Path;

pub const ID: &str = "tool.extract-av";

/// 提取什么。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtractMode {
    #[default]
    Audio,
    Video,
    Both,
}

/// 导出哪些音轨。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioTracks {
    First,
    #[default]
    All,
}

/// 音频格式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioFormat {
    /// 保持原编码（不重编码）
    #[default]
    Original,
    Mp3,
    M4a,
    Wav,
}

/// 可选的 mp3 码率（kbit/s）。
pub const MP3_KBPS: [u32; 5] = [128, 160, 192, 256, 320];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExtractAvParams {
    pub mode: ExtractMode,
    pub tracks: AudioTracks,
    pub audio_format: AudioFormat,
    /// 转为 mp3 时的码率（kbit/s），取值见 [`MP3_KBPS`]
    pub mp3_kbps: u32,
}

impl Default for ExtractAvParams {
    fn default() -> Self {
        Self {
            mode: ExtractMode::Audio,
            tracks: AudioTracks::All,
            audio_format: AudioFormat::Original,
            mp3_kbps: 192,
        }
    }
}

/// 一个输出：命名方式 + 输出路径之前的参数。
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedOutput {
    pub spec: OutputSpec,
    pub args: Vec<String>,
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// 规划全部输出。`input` 用于决定纯视频的容器（设置“保持源格式”时）。
pub fn plan_outputs(
    params: &ExtractAvParams,
    info: &MediaInfo,
    input: &Path,
    settings: &Settings,
) -> Result<(Vec<PlannedOutput>, Vec<String>), String> {
    if !MP3_KBPS.contains(&params.mp3_kbps) {
        return Err(format!("mp3 码率只能是 {MP3_KBPS:?} 之一"));
    }
    let mut outputs = Vec::new();
    let mut warnings = Vec::new();
    let want_audio = matches!(params.mode, ExtractMode::Audio | ExtractMode::Both);
    let want_video = matches!(params.mode, ExtractMode::Video | ExtractMode::Both);

    if want_audio {
        if info.audio.is_empty() {
            if params.mode == ExtractMode::Audio {
                return Err("文件中没有音频流".into());
            }
            warnings.push("文件中没有音频流，只导出视频".into());
        }
        let tracks = match params.tracks {
            AudioTracks::First => info.audio.len().min(1),
            AudioTracks::All => info.audio.len(),
        };
        for (i, audio) in info.audio.iter().take(tracks).enumerate() {
            let codec = audio.codec.to_ascii_lowercase();
            let (ext, codec_args) = match params.audio_format {
                AudioFormat::Original => (audio_copy_extension(&codec), strings(&["-c:a", "copy"])),
                AudioFormat::Mp3 if codec == "mp3" => ("mp3", strings(&["-c:a", "copy"])),
                AudioFormat::Mp3 => (
                    "mp3",
                    vec![
                        "-c:a".into(),
                        "libmp3lame".into(),
                        "-b:a".into(),
                        format!("{}k", params.mp3_kbps),
                    ],
                ),
                AudioFormat::M4a if codec == "aac" => ("m4a", strings(&["-c:a", "copy"])),
                AudioFormat::M4a => (
                    "m4a",
                    vec![
                        "-c:a".into(),
                        "aac".into(),
                        "-b:a".into(),
                        format!("{}k", aac_kbps(audio.channels)),
                    ],
                ),
                AudioFormat::Wav if codec == "pcm_s16le" => ("wav", strings(&["-c:a", "copy"])),
                AudioFormat::Wav => ("wav", strings(&["-c:a", "pcm_s16le"])),
            };
            let suffix = if tracks > 1 {
                format!("_audio_{}", i + 1)
            } else {
                "_audio".to_owned()
            };
            let mut args = vec!["-map".into(), format!("0:a:{i}"), "-vn".into(), "-sn".into(), "-dn".into()];
            args.extend(codec_args);
            outputs.push(PlannedOutput {
                spec: OutputSpec::file(ext).suffix(suffix),
                args,
            });
        }
    }

    if want_video {
        let video = info.video.as_ref().ok_or("文件中没有视频流")?;
        let ext = copy_container(settings, input, &video.codec);
        let mut args = strings(&["-map", "0:V:0", "-an", "-sn", "-dn", "-c:v", "copy"]);
        if matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
            args.extend(strings(&["-movflags", "+faststart"]));
        }
        outputs.push(PlannedOutput {
            spec: OutputSpec::file(&ext).suffix("_video"),
            args,
        });
    }
    Ok((outputs, warnings))
}

impl Tool for ExtractAvParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let info = ctx.info(0)?;
        let input = ctx.input_path(0)?.to_path_buf();
        let (outputs, warnings) = plan_outputs(self, &info, &input, ctx.settings())?;
        for warning in warnings {
            ctx.warn(warning);
        }
        let mut args = vec!["-i".to_owned(), path_arg(&input)?];
        for planned in outputs {
            let path = ctx.output(&planned.spec)?;
            args.extend(planned.args);
            args.push(path_arg(&path)?);
        }
        ctx.run_ffmpeg("音视频提取", args, info.duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::AudioStream;
    use crate::pipeline::testing::sample_info;

    fn two_tracks() -> MediaInfo {
        let mut info = sample_info();
        info.audio.push(AudioStream {
            index: 2,
            codec: "pcm_s16le".into(),
            channels: 1,
            sample_rate: 48_000,
            bit_rate: None,
            language: None,
            title: None,
        });
        info
    }

    fn plan(params: &ExtractAvParams, info: &MediaInfo) -> Vec<(String, String)> {
        let (outputs, _) = plan_outputs(params, info, Path::new("in.mkv"), &Settings::default()).unwrap();
        outputs
            .into_iter()
            .map(|o| (format!("{}.{}", o.spec.suffix, o.spec.ext), o.args.join(" ")))
            .collect()
    }

    #[test]
    fn original_format_copies_every_track() {
        let outputs = plan(&ExtractAvParams::default(), &two_tracks());
        assert_eq!(
            outputs,
            [
                ("_audio_1.m4a".into(), "-map 0:a:0 -vn -sn -dn -c:a copy".into()),
                ("_audio_2.wav".into(), "-map 0:a:1 -vn -sn -dn -c:a copy".into()),
            ]
        );
    }

    #[test]
    fn mp3_converts_unless_already_mp3_and_first_track_only() {
        let params = ExtractAvParams {
            audio_format: AudioFormat::Mp3,
            mp3_kbps: 320,
            tracks: AudioTracks::First,
            ..ExtractAvParams::default()
        };
        let outputs = plan(&params, &two_tracks());
        assert_eq!(
            outputs,
            [("_audio.mp3".into(), "-map 0:a:0 -vn -sn -dn -c:a libmp3lame -b:a 320k".into())]
        );
        let mut mp3 = sample_info();
        mp3.audio[0].codec = "mp3".into();
        assert_eq!(plan(&params, &mp3)[0].1, "-map 0:a:0 -vn -sn -dn -c:a copy");
        let bad = ExtractAvParams {
            mp3_kbps: 100,
            ..ExtractAvParams::default()
        };
        assert!(plan_outputs(&bad, &sample_info(), Path::new("a.mp4"), &Settings::default()).is_err());
    }

    #[test]
    fn m4a_and_wav_copy_when_possible() {
        let m4a = ExtractAvParams {
            audio_format: AudioFormat::M4a,
            ..ExtractAvParams::default()
        };
        let outputs = plan(&m4a, &two_tracks());
        assert_eq!(outputs[0].1, "-map 0:a:0 -vn -sn -dn -c:a copy");
        assert_eq!(outputs[1].1, "-map 0:a:1 -vn -sn -dn -c:a aac -b:a 96k");
        let wav = ExtractAvParams {
            audio_format: AudioFormat::Wav,
            ..ExtractAvParams::default()
        };
        let outputs = plan(&wav, &two_tracks());
        assert_eq!(outputs[0].1, "-map 0:a:0 -vn -sn -dn -c:a pcm_s16le");
        assert_eq!(outputs[1].1, "-map 0:a:1 -vn -sn -dn -c:a copy");
    }

    #[test]
    fn video_only_and_both() {
        let video = ExtractAvParams {
            mode: ExtractMode::Video,
            ..ExtractAvParams::default()
        };
        assert_eq!(
            plan(&video, &sample_info()),
            [(
                "_video.mp4".into(),
                "-map 0:V:0 -an -sn -dn -c:v copy -movflags +faststart".into()
            )]
        );
        let both = ExtractAvParams {
            mode: ExtractMode::Both,
            ..ExtractAvParams::default()
        };
        let mut silent = sample_info();
        silent.audio.clear();
        let (outputs, warnings) =
            plan_outputs(&both, &silent, Path::new("a.mp4"), &Settings::default()).unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert!(plan_outputs(&ExtractAvParams::default(), &silent, Path::new("a.mp4"), &Settings::default()).is_err());
    }
}
