//! 工具常用的小函数：参数列表、时间格式、容器与编码的兼容性、concat 列表。

use std::path::{Path, PathBuf};

use crate::model::Settings;
use crate::pipeline::{fmt_num, output_extension};

/// `&["-c", "copy"]` → `Vec<String>`。
pub fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// 秒数写进命令行（`-ss`、`-t`、滤镜参数），最多 4 位小数：`12.5` → `12.5`、`3.0` → `3`。
pub fn secs(value: f64) -> String {
    fmt_num(value.max(0.0))
}

/// 流的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
}

/// 能否把某个编码的流**不重编码**地放进某个容器。`None` 表示不确定（交给 ffmpeg 试）。
pub fn container_accepts(container: &str, kind: StreamKind, codec: &str) -> Option<bool> {
    use StreamKind::*;
    let codec = codec.to_ascii_lowercase();
    let c = codec.as_str();
    let pcm = c.starts_with("pcm_");
    let accepts = match (container.to_ascii_lowercase().as_str(), kind) {
        ("mkv" | "mka", _) => true,
        ("mp4" | "m4v" | "m4a", Video) => matches!(
            c,
            "h264"
                | "hevc"
                | "av1"
                | "vp9"
                | "mpeg4"
                | "mpeg2video"
                | "mpeg1video"
                | "mjpeg"
                | "vvc"
        ),
        ("mp4" | "m4v" | "m4a", Audio) => matches!(
            c,
            "aac" | "mp3" | "ac3" | "eac3" | "opus" | "flac" | "alac" | "mp2"
        ),
        ("mp4" | "m4v" | "mov", Subtitle) => c == "mov_text",
        ("mov", Video) => matches!(
            c,
            "h264" | "hevc" | "av1" | "vp9" | "mpeg4" | "mpeg2video" | "mjpeg" | "prores" | "dnxhd"
        ),
        ("mov", Audio) => {
            pcm || matches!(c, "aac" | "mp3" | "ac3" | "eac3" | "alac" | "flac" | "opus")
        }
        ("ts", Video) => matches!(c, "h264" | "hevc" | "mpeg2video" | "mpeg1video" | "av1"),
        ("ts", Audio) => matches!(c, "aac" | "mp3" | "mp2" | "ac3" | "eac3" | "opus"),
        ("ts" | "flv" | "avi", Subtitle) => false,
        ("flv", Video) => matches!(c, "h264" | "hevc" | "av1" | "flv1"),
        ("flv", Audio) => matches!(c, "aac" | "mp3"),
        ("webm", Video) => matches!(c, "vp8" | "vp9" | "av1"),
        ("webm", Audio) => matches!(c, "opus" | "vorbis"),
        ("webm", Subtitle) => c == "webvtt",
        ("avi", Video) => !matches!(c, "hevc" | "av1" | "vp9"),
        ("avi", Audio) => pcm || matches!(c, "mp3" | "ac3" | "aac" | "mp2"),
        _ => return None,
    };
    Some(accepts)
}

/// 不重编码地输出视频时用的容器：设置为“保持源格式”且源容器装得下时用源扩展名，
/// 否则 mp4（装得下时），再否则 mkv。
pub fn copy_container(settings: &Settings, input: &Path, video_codec: &str) -> String {
    let wanted = output_extension(settings, input);
    for candidate in [wanted.as_str(), "mp4", "mkv"] {
        if container_accepts(candidate, StreamKind::Video, video_codec) != Some(false) {
            return candidate.to_owned();
        }
    }
    "mkv".to_owned()
}

/// 文本字幕（可以转成 mov_text 放进 mp4）。
pub fn is_text_subtitle(codec: &str) -> bool {
    matches!(
        codec.to_ascii_lowercase().as_str(),
        "subrip" | "ass" | "ssa" | "mov_text" | "webvtt" | "text"
    )
}

/// 不重编码导出一条音频流时用的扩展名（编码 → 最常见的独立音频格式）；未知编码用 mka。
pub fn audio_copy_extension(codec: &str) -> &'static str {
    let codec = codec.to_ascii_lowercase();
    match codec.as_str() {
        "aac" | "alac" => "m4a",
        "mp3" => "mp3",
        "mp2" => "mp2",
        "opus" => "opus",
        "vorbis" => "ogg",
        "flac" => "flac",
        "ac3" => "ac3",
        "eac3" => "eac3",
        c if c.starts_with("pcm_") => "wav",
        _ => "mka",
    }
}

/// concat 分离器（`-f concat -safe 0 -i list.txt`）的列表内容。
pub fn concat_list(files: &[PathBuf]) -> String {
    let mut text = String::from("ffconcat version 1.0\n");
    for file in files {
        let path = file
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "'\\''");
        text.push_str(&format!("file '{path}'\n"));
    }
    text
}

/// 写 concat 列表文件（UTF-8）。
pub fn write_concat_list(list: &Path, files: &[PathBuf]) -> Result<(), String> {
    std::fs::write(list, concat_list(files)).map_err(|e| format!("无法写入拼接列表：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TargetFormat;

    #[test]
    fn compatibility_table() {
        use StreamKind::*;
        assert_eq!(container_accepts("mkv", Audio, "pcm_s16le"), Some(true));
        assert_eq!(container_accepts("mp4", Video, "h264"), Some(true));
        assert_eq!(container_accepts("MP4", Audio, "vorbis"), Some(false));
        assert_eq!(container_accepts("mp4", Subtitle, "subrip"), Some(false));
        assert_eq!(container_accepts("mov", Audio, "pcm_s24le"), Some(true));
        assert_eq!(container_accepts("flv", Video, "vp9"), Some(false));
        assert_eq!(container_accepts("xyz", Video, "h264"), None);
        assert!(is_text_subtitle("ASS"));
        assert!(!is_text_subtitle("hdmv_pgs_subtitle"));
    }

    #[test]
    fn copy_container_prefers_settings_then_mp4_then_mkv() {
        let mp4 = Settings::default();
        let source = Settings {
            target_format: TargetFormat::Source,
            ..Settings::default()
        };
        assert_eq!(copy_container(&mp4, Path::new("a.mkv"), "h264"), "mp4");
        assert_eq!(copy_container(&source, Path::new("a.mkv"), "h264"), "mkv");
        assert_eq!(copy_container(&source, Path::new("a.flv"), "vp9"), "mp4");
        assert_eq!(copy_container(&mp4, Path::new("a.avi"), "wmv3"), "mkv");
    }

    #[test]
    fn audio_extensions() {
        assert_eq!(audio_copy_extension("aac"), "m4a");
        assert_eq!(audio_copy_extension("pcm_s16le"), "wav");
        assert_eq!(audio_copy_extension("dts"), "mka");
    }

    #[test]
    fn concat_list_escapes_quotes_and_backslashes() {
        let list = concat_list(&[PathBuf::from(r"C:\a\it's.mp4"), PathBuf::from("/b c.mp4")]);
        assert_eq!(
            list,
            "ffconcat version 1.0\nfile 'C:/a/it'\\''s.mp4'\nfile '/b c.mp4'\n"
        );
    }

    #[test]
    fn helpers() {
        assert_eq!(args(&["-c", "copy"]), ["-c", "copy"]);
        assert_eq!(secs(3.0), "3");
        assert_eq!(secs(1.23456), "1.2346");
        assert_eq!(secs(-1.0), "0");
    }
}
