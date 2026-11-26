//! `tool.hls` M3U8 切片：把每个队列视频无损切成 HLS（一个文件夹：`index.m3u8` + `index_0000.ts`…）。
//!
//! 参数与取值约定（docs/specs/tools.md 未规定的部分）：
//! - `segmentSeconds`：目标分片时长（1～600 秒，默认 6）。视频直接复制，分片只能在关键帧处切开，
//!   所以实际分片时长会有误差（关键帧间隔越大误差越大）；播放列表里写的是每片的实际时长。
//! - 输出：`<输出目录>/<视频名>/`（文件夹已存在时加 “ (n)”，不覆盖），点播类型（`EXT-X-PLAYLIST-TYPE:VOD`），
//!   列表包含全部分片，分片名为相对路径，整个文件夹可以直接移动或上传。
//! - 视频编码必须能放进 MPEG-TS（H.264、H.265、MPEG-2…），否则失败并说明原因；音频编码 TS 不支持时
//!   转为 AAC（提示），否则直接复制。只保留第一条视频流与第一条音频流。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::pipeline::fmt_num;
use crate::tools::context::Encoding;
use crate::tools::media::{container_accepts, secs, StreamKind};
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.hls";
pub const PLAYLIST: &str = "index.m3u8";
pub const SEGMENT_PATTERN: &str = "index_%04d.ts";
pub const MIN_SEGMENT: f64 = 1.0;
pub const MAX_SEGMENT: f64 = 600.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HlsParams {
    /// 目标分片时长（秒）
    pub segment_seconds: f64,
}

impl Default for HlsParams {
    fn default() -> Self {
        Self {
            segment_seconds: 6.0,
        }
    }
}

impl HlsParams {
    fn validate(&self) -> Result<(), String> {
        if !(MIN_SEGMENT..=MAX_SEGMENT).contains(&self.segment_seconds) {
            return Err(format!(
                "分片时长必须在 {}～{} 秒之间",
                fmt_num(MIN_SEGMENT),
                fmt_num(MAX_SEGMENT)
            ));
        }
        Ok(())
    }
}

/// 音频的处理方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HlsAudio {
    None,
    Copy,
    /// 转为 AAC（声道数）
    Aac(u32),
}

/// 检查编码能否放进 TS，决定音频处理方式。
pub fn hls_audio(info: &MediaInfo) -> Result<HlsAudio, String> {
    let video = info.video.as_ref().ok_or("没有视频流，无法切片")?;
    if container_accepts("ts", StreamKind::Video, &video.codec) == Some(false) {
        return Err(format!(
            "视频编码 {} 不能无损切成 HLS（TS 分片只支持 H.264、H.265、MPEG-2 等），请先转码",
            video.codec
        ));
    }
    Ok(match info.audio.first() {
        None => HlsAudio::None,
        Some(a) if container_accepts("ts", StreamKind::Audio, &a.codec) == Some(false) => {
            HlsAudio::Aac(a.channels)
        }
        Some(_) => HlsAudio::Copy,
    })
}

/// 切片参数（不含通用开头参数）。`folder` 为输出文件夹（路径参数）。
pub fn hls_args(input: &str, folder: &str, segment_seconds: f64, audio: HlsAudio) -> Vec<String> {
    let sep = if folder.ends_with(['/', '\\']) {
        ""
    } else {
        "/"
    };
    let mut args = vec!["-i".into(), input.into(), "-map".into(), "0:V:0".into()];
    if audio != HlsAudio::None {
        args.extend(["-map".into(), "0:a:0".into()]);
    }
    args.extend(["-c:v".into(), "copy".into()]);
    match audio {
        HlsAudio::None => {}
        HlsAudio::Copy => args.extend(["-c:a".into(), "copy".into()]),
        HlsAudio::Aac(channels) => args.extend(Encoding::audio_args(channels)),
    }
    args.extend([
        "-sn".into(),
        "-dn".into(),
        "-f".into(),
        "hls".into(),
        "-hls_time".into(),
        secs(segment_seconds),
        "-hls_playlist_type".into(),
        "vod".into(),
        "-hls_list_size".into(),
        "0".into(),
        "-hls_segment_filename".into(),
        format!("{folder}{sep}{SEGMENT_PATTERN}"),
        format!("{folder}{sep}{PLAYLIST}"),
    ]);
    args
}

impl Tool for HlsParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let info = ctx.info(0)?;
        let audio = hls_audio(&info)?;
        if let HlsAudio::Aac(_) = audio {
            let codec = info
                .audio
                .first()
                .map(|a| a.codec.clone())
                .unwrap_or_default();
            ctx.note(format!("音频编码 {codec} 不能放进 TS 分片，转为 AAC"));
        }
        let input = path_arg(ctx.input_path(0)?)?;
        let folder = path_arg(&ctx.output(&OutputSpec::folder())?)?;
        ctx.run_ffmpeg(
            "HLS 切片",
            hls_args(&input, &folder, self.segment_seconds, audio),
            info.duration,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::sample_info;

    #[test]
    fn args_copy_streams_into_vod_playlist() {
        assert_eq!(
            hls_args("in.mp4", "D:/out/clip", 6.0, HlsAudio::Copy).join(" "),
            "-i in.mp4 -map 0:V:0 -map 0:a:0 -c:v copy -c:a copy -sn -dn -f hls -hls_time 6 \
             -hls_playlist_type vod -hls_list_size 0 -hls_segment_filename D:/out/clip/index_%04d.ts \
             D:/out/clip/index.m3u8"
        );
        let silent = hls_args("in.mp4", "o/", 2.5, HlsAudio::None).join(" ");
        assert!(silent.contains("-map 0:V:0 -c:v copy -sn"), "{silent}");
        assert!(silent.ends_with("o/index.m3u8"), "{silent}");
        let aac = hls_args("in.mkv", "o", 6.0, HlsAudio::Aac(2)).join(" ");
        assert!(aac.contains("-c:v copy -c:a aac -b:a 192k"), "{aac}");
    }

    #[test]
    fn codecs_decide_copy_or_fail() {
        let info = sample_info();
        assert_eq!(hls_audio(&info), Ok(HlsAudio::Copy));
        let mut vorbis = sample_info();
        vorbis.audio[0].codec = "vorbis".into();
        assert_eq!(hls_audio(&vorbis), Ok(HlsAudio::Aac(2)));
        let mut silent = sample_info();
        silent.audio.clear();
        assert_eq!(hls_audio(&silent), Ok(HlsAudio::None));
        let mut vp9 = sample_info();
        vp9.video.as_mut().unwrap().codec = "vp9".into();
        assert!(hls_audio(&vp9).unwrap_err().contains("vp9"));
        assert!(HlsParams {
            segment_seconds: 0.5
        }
        .validate()
        .is_err());
        assert!(HlsParams::default().validate().is_ok());
    }
}
