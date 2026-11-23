//! `tool.remux` 无损转换：只换封装（mp4 ↔ mkv 等），视频不重编码。
//!
//! 参考实现 · **一进一出、不重编码**：每个队列视频一项（默认 `plan`），一个输出
//! （`ctx.output`），一条 `ctx.run_ffmpeg`。命令拼装是纯函数 [`build_args`]，单元测试直接断言参数。
//!
//! - 视频流全部复制；目标容器装不下该视频编码时报错（建议改用 mkv）。
//! - 音频流复制；装不下时（如 mkv 里的 PCM / Vorbis → mp4）按设置转为 AAC 或报错。
//! - 字幕：mkv 全部复制；mp4 / mov 里文本字幕转为 mov_text，图形字幕丢弃；ts / flv 不保留字幕。
//! - 封面图（attached pic）、数据流不保留。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::pipeline::aac_kbps;
use crate::tools::media::{container_accepts, is_text_subtitle, StreamKind};
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.remux";

/// 目标容器。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemuxContainer {
    #[default]
    Mp4,
    Mkv,
    Mov,
    Ts,
    Flv,
}

impl RemuxContainer {
    pub fn ext(self) -> &'static str {
        match self {
            RemuxContainer::Mp4 => "mp4",
            RemuxContainer::Mkv => "mkv",
            RemuxContainer::Mov => "mov",
            RemuxContainer::Ts => "ts",
            RemuxContainer::Flv => "flv",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemuxParams {
    pub container: RemuxContainer,
    /// 音频编码与目标容器不兼容时转为 AAC（视频仍不重编码）；关闭时直接报错
    pub convert_incompatible_audio: bool,
    /// 保留字幕轨
    pub keep_subtitles: bool,
}

impl Default for RemuxParams {
    fn default() -> Self {
        Self {
            container: RemuxContainer::Mp4,
            convert_incompatible_audio: true,
            keep_subtitles: true,
        }
    }
}

/// 拼好的命令（不含开头的通用参数）与提示。
#[derive(Debug, Clone, PartialEq)]
pub struct RemuxCommand {
    pub args: Vec<String>,
    pub warnings: Vec<String>,
}

/// 生成换封装命令。
pub fn build_args(
    params: &RemuxParams,
    info: &MediaInfo,
    input: &str,
    output: &str,
) -> Result<RemuxCommand, String> {
    let ext = params.container.ext();
    let video = info.video.as_ref().ok_or("文件中没有视频流")?;
    if container_accepts(ext, StreamKind::Video, &video.codec) == Some(false) {
        return Err(format!(
            "视频编码 {} 无法不重编码地放进 {ext}，请改用 mkv",
            video.codec
        ));
    }
    let mut warnings = Vec::new();
    let mut args: Vec<String> = vec!["-i".into(), input.into(), "-map".into(), "0:V".into()];
    let mut codecs: Vec<String> = vec!["-c:v".into(), "copy".into()];

    for (i, audio) in info.audio.iter().enumerate() {
        args.extend(["-map".into(), format!("0:a:{i}")]);
        if container_accepts(ext, StreamKind::Audio, &audio.codec) == Some(false) {
            if !params.convert_incompatible_audio {
                return Err(format!(
                    "音频编码 {} 无法不重编码地放进 {ext}（可开启“音频不兼容时转为 AAC”，或改用 mkv）",
                    audio.codec
                ));
            }
            warnings.push(format!(
                "第 {} 条音轨（{}）无法直接放进 {ext}，已转为 AAC",
                i + 1,
                audio.codec
            ));
            codecs.extend([
                format!("-c:a:{i}"),
                "aac".into(),
                format!("-b:a:{i}"),
                format!("{}k", aac_kbps(audio.channels)),
            ]);
        } else {
            codecs.extend([format!("-c:a:{i}"), "copy".into()]);
        }
    }

    let mut kept = 0;
    let mut dropped = 0;
    if params.keep_subtitles {
        for (j, sub) in info.subtitles.iter().enumerate() {
            let target = match container_accepts(ext, StreamKind::Subtitle, &sub.codec) {
                Some(true) | None => "copy",
                Some(false) if matches!(ext, "mp4" | "mov") && is_text_subtitle(&sub.codec) => "mov_text",
                Some(false) => {
                    dropped += 1;
                    continue;
                }
            };
            args.extend(["-map".into(), format!("0:s:{j}")]);
            codecs.extend([format!("-c:s:{kept}"), target.into()]);
            kept += 1;
        }
    } else if !info.subtitles.is_empty() {
        warnings.push("已按设置去掉字幕轨".into());
    }
    if dropped > 0 {
        warnings.push(format!("{dropped} 条字幕轨无法放进 {ext}，已丢弃"));
    }
    if info.has_cover {
        warnings.push("封面图不会保留".into());
    }

    args.extend(codecs);
    args.extend(["-map_metadata".into(), "0".into()]);
    if matches!(ext, "mp4" | "mov") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    Ok(RemuxCommand { args, warnings })
}

impl Tool for RemuxParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let info = ctx.info(0)?;
        let input = path_arg(ctx.input_path(0)?)?;
        let output = ctx.output(&OutputSpec::file(self.container.ext()))?;
        let command = build_args(self, &info, &input, &path_arg(&output)?)?;
        for warning in command.warnings {
            ctx.warn(warning);
        }
        ctx.run_ffmpeg("无损转换", command.args, info.duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{AudioStream, SubtitleStream};
    use crate::pipeline::testing::sample_info;

    fn build(params: &RemuxParams, info: &MediaInfo) -> Result<RemuxCommand, String> {
        build_args(params, info, "in.mp4", "out.x")
    }

    #[test]
    fn mp4_to_mkv_copies_everything() {
        let params = RemuxParams {
            container: RemuxContainer::Mkv,
            ..RemuxParams::default()
        };
        let cmd = build(&params, &sample_info()).unwrap();
        assert_eq!(
            cmd.args.join(" "),
            "-i in.mp4 -map 0:V -map 0:a:0 -c:v copy -c:a:0 copy -map_metadata 0 out.x"
        );
        assert!(cmd.warnings.is_empty());
    }

    #[test]
    fn incompatible_audio_is_converted_or_rejected() {
        let mut info = sample_info();
        info.audio.push(AudioStream {
            index: 2,
            codec: "vorbis".into(),
            channels: 1,
            sample_rate: 44_100,
            bit_rate: None,
            language: None,
            title: None,
        });
        let cmd = build(&RemuxParams::default(), &info).unwrap();
        let line = cmd.args.join(" ");
        assert!(
            line.contains("-c:v copy -c:a:0 copy -c:a:1 aac -b:a:1 96k"),
            "{line}"
        );
        assert!(line.ends_with("-movflags +faststart out.x"), "{line}");
        assert_eq!(cmd.warnings.len(), 1);
        let strict = RemuxParams {
            convert_incompatible_audio: false,
            ..RemuxParams::default()
        };
        assert!(build(&strict, &info).is_err());
    }

    #[test]
    fn subtitles_follow_the_container() {
        let mut info = sample_info();
        info.subtitles = vec![
            SubtitleStream {
                index: 2,
                codec: "subrip".into(),
                language: None,
                title: None,
            },
            SubtitleStream {
                index: 3,
                codec: "hdmv_pgs_subtitle".into(),
                language: None,
                title: None,
            },
        ];
        let mp4 = build(&RemuxParams::default(), &info).unwrap();
        let line = mp4.args.join(" ");
        assert!(line.contains("-map 0:s:0 -c:v copy"), "{line}");
        assert!(line.contains("-c:s:0 mov_text"), "{line}");
        assert!(!line.contains("0:s:1"), "图形字幕不能放进 mp4：{line}");
        assert!(mp4.warnings.iter().any(|w| w.contains("1 条字幕轨")));

        let mkv = build(
            &RemuxParams {
                container: RemuxContainer::Mkv,
                ..RemuxParams::default()
            },
            &info,
        )
        .unwrap();
        let line = mkv.args.join(" ");
        assert!(line.contains("-map 0:s:0 -map 0:s:1"), "{line}");
        assert!(line.contains("-c:s:0 copy -c:s:1 copy"), "{line}");

        let none = build(
            &RemuxParams {
                keep_subtitles: false,
                ..RemuxParams::default()
            },
            &info,
        )
        .unwrap();
        assert!(!none.args.join(" ").contains("0:s"));
    }

    #[test]
    fn incompatible_video_is_an_error() {
        let mut info = sample_info();
        info.video.as_mut().unwrap().codec = "vp9".into();
        let flv = RemuxParams {
            container: RemuxContainer::Flv,
            ..RemuxParams::default()
        };
        let err = build(&flv, &info).unwrap_err();
        assert!(err.contains("mkv"), "{err}");
        info.video = None;
        assert!(build(&RemuxParams::default(), &info).is_err());
    }
}
