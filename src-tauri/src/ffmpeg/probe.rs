//! 用 ffprobe 读取媒体信息。

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::command;

/// 一个媒体文件的基本信息。字段与前端 `MediaInfo` 类型一致。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MediaInfo {
    pub path: String,
    /// 容器格式名，如 `mov,mp4,m4a,3gp,3g2,mj2`
    pub format_name: String,
    /// 时长（秒）；未知为 0
    pub duration: f64,
    /// 文件大小（字节）
    pub size: u64,
    /// 容器总码率（bit/s）
    pub bit_rate: Option<u64>,
    /// 第一条非封面视频流
    pub video: Option<VideoStream>,
    pub audio: Vec<AudioStream>,
    pub subtitles: Vec<SubtitleStream>,
    /// 是否带封面图（attached_pic）
    pub has_cover: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoStream {
    /// 在文件中的流序号
    pub index: u32,
    pub codec: String,
    /// 编码宽高（未考虑旋转）
    pub width: u32,
    pub height: u32,
    /// 平均帧率；读不到时退回 r_frame_rate
    pub fps: f64,
    /// 帧率原文，如 `30000/1001`
    pub fps_text: String,
    pub bit_rate: Option<u64>,
    pub pix_fmt: String,
    /// 播放时需顺时针旋转的角度：0 / 90 / 180 / 270
    pub rotation: u32,
    pub frames: Option<u64>,
    /// 色彩传输特性（`color_transfer`），如 `bt709`、`smpte2084`（PQ）、`arib-std-b67`（HLG）；未知为空
    pub color_transfer: String,
    /// 色域（`color_primaries`），如 `bt709`、`bt2020`
    pub color_primaries: String,
    /// 矩阵（`color_space`），如 `bt709`、`bt2020nc`
    pub color_space: String,
    /// 像素宽高比（`sample_aspect_ratio`），如 `1:1`、`4:3`（变形 / 非方形像素）；未知为空
    pub sar: String,
}

impl VideoStream {
    /// 是否为 HDR（PQ / HLG 传输特性）。
    pub fn is_hdr(&self) -> bool {
        matches!(self.color_transfer.as_str(), "smpte2084" | "arib-std-b67")
    }

    /// 非方形像素时的像素宽高比（分子, 分母）；方形（1:1）或未知时 None。
    pub fn sar_ratio(&self) -> Option<(u32, u32)> {
        let (n, d) = self.sar.split_once(':')?;
        let (n, d): (u32, u32) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
        (n > 0 && d > 0 && n != d).then_some((n, d))
    }

    /// 按方形像素显示时的宽高（已考虑旋转与 SAR）：宽度乘以 SAR 后取偶，与滤镜
    /// `scale=trunc(iw*sar/2)*2:ih` 一致。方形像素时等于 [`Self::display_size`]。
    pub fn square_display_size(&self) -> (u32, u32) {
        let width = match self.sar_ratio() {
            Some((n, d)) => {
                let w = (u64::from(self.width) * u64::from(n) / u64::from(d)) as u32;
                (w / 2 * 2).max(2)
            }
            None => self.width,
        };
        if self.rotation % 180 == 90 {
            (self.height, width)
        } else {
            (width, self.height)
        }
    }

    /// 旋转后实际显示的宽高（FFmpeg 解码时默认自动旋转）。
    pub fn display_size(&self) -> (u32, u32) {
        if self.rotation % 180 == 90 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioStream {
    pub index: u32,
    pub codec: String,
    pub channels: u32,
    pub sample_rate: u32,
    pub bit_rate: Option<u64>,
    pub language: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleStream {
    pub index: u32,
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
}

impl MediaInfo {
    /// 视频流码率；流上没有时用 容器码率 − 音频码率 估算。
    pub fn video_bit_rate(&self) -> Option<u64> {
        if let Some(rate) = self.video.as_ref().and_then(|v| v.bit_rate) {
            return Some(rate);
        }
        let total = self.bit_rate.or_else(|| {
            (self.duration > 0.0 && self.size > 0)
                .then(|| (self.size as f64 * 8.0 / self.duration) as u64)
        })?;
        let audio: u64 = self.audio.iter().filter_map(|a| a.bit_rate).sum();
        total.checked_sub(audio).filter(|r| *r > 0)
    }
}

/// 运行 ffprobe 读取媒体信息。
pub fn probe(ffprobe: &Path, file: &Path) -> Result<MediaInfo, String> {
    let output = command(ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(file)
        .output()
        .map_err(|e| format!("无法启动 ffprobe（{}）：{e}", ffprobe.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .last()
            .unwrap_or("未知错误")
            .trim()
            .to_owned();
        return Err(format!("无法读取媒体信息：{reason}"));
    }
    let json = String::from_utf8_lossy(&output.stdout);
    let mut info = parse_probe_json(&json)?;
    info.path = file.to_string_lossy().into_owned();
    Ok(info)
}

fn num<T: std::str::FromStr>(v: &Value) -> Option<T> {
    match v {
        Value::String(s) => s.trim().parse().ok(),
        Value::Number(n) => n.to_string().parse().ok(),
        _ => None,
    }
}

fn text(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned).filter(|s| !s.is_empty())
}

/// 解析 `num/den` 形式的帧率；0/0 等无效值返回 None。
pub fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/').unwrap_or((s, "1"));
    let n: f64 = n.trim().parse().ok()?;
    let d: f64 = d.trim().parse().ok()?;
    (n > 0.0 && d > 0.0).then_some(n / d)
}

/// 把 ffprobe 的 rotation（逆时针为正）转为“需顺时针旋转的角度”。
fn normalize_rotation(raw: f64) -> u32 {
    let cw = (-raw).round() as i64;
    let r = cw.rem_euclid(360);
    // 只认 90 的倍数
    (((r + 45) / 90 * 90) % 360) as u32
}

fn stream_rotation(stream: &Value) -> u32 {
    if let Some(list) = stream["side_data_list"].as_array() {
        for side in list {
            if let Some(rot) = side.get("rotation").and_then(num::<f64>) {
                return normalize_rotation(rot);
            }
        }
    }
    // 旧版 ffprobe：tags.rotate 为顺时针角度
    stream["tags"]["rotate"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|cw| normalize_rotation(-cw))
        .unwrap_or(0)
}

/// 解析 `ffprobe -print_format json -show_format -show_streams` 的输出。
pub fn parse_probe_json(json: &str) -> Result<MediaInfo, String> {
    let root: Value =
        serde_json::from_str(json).map_err(|e| format!("ffprobe 输出无法解析：{e}"))?;
    let format = &root["format"];
    let mut info = MediaInfo {
        path: text(&format["filename"]).unwrap_or_default(),
        format_name: text(&format["format_name"]).unwrap_or_default(),
        duration: num(&format["duration"]).unwrap_or(0.0),
        size: num(&format["size"]).unwrap_or(0),
        bit_rate: num(&format["bit_rate"]),
        ..MediaInfo::default()
    };

    let empty = Vec::new();
    for stream in root["streams"].as_array().unwrap_or(&empty) {
        let index = num(&stream["index"]).unwrap_or(0);
        let codec = text(&stream["codec_name"]).unwrap_or_default();
        let language = text(&stream["tags"]["language"]);
        let title = text(&stream["tags"]["title"]);
        match stream["codec_type"].as_str() {
            Some("video") => {
                let attached = stream["disposition"]["attached_pic"].as_i64() == Some(1);
                if attached {
                    info.has_cover = true;
                    continue;
                }
                if info.video.is_some() {
                    continue;
                }
                let avg = text(&stream["avg_frame_rate"]).filter(|s| parse_rate(s).is_some());
                let fps_text = avg
                    .or_else(|| text(&stream["r_frame_rate"]))
                    .unwrap_or_default();
                info.video = Some(VideoStream {
                    index,
                    codec,
                    width: num(&stream["width"]).unwrap_or(0),
                    height: num(&stream["height"]).unwrap_or(0),
                    fps: parse_rate(&fps_text).unwrap_or(0.0),
                    fps_text,
                    bit_rate: num(&stream["bit_rate"]),
                    pix_fmt: text(&stream["pix_fmt"]).unwrap_or_default(),
                    rotation: stream_rotation(stream),
                    frames: num(&stream["nb_frames"]),
                    color_transfer: text(&stream["color_transfer"]).unwrap_or_default(),
                    color_primaries: text(&stream["color_primaries"]).unwrap_or_default(),
                    color_space: text(&stream["color_space"]).unwrap_or_default(),
                    sar: text(&stream["sample_aspect_ratio"]).unwrap_or_default(),
                });
            }
            Some("audio") => info.audio.push(AudioStream {
                index,
                codec,
                channels: num(&stream["channels"]).unwrap_or(0),
                sample_rate: num(&stream["sample_rate"]).unwrap_or(0),
                bit_rate: num(&stream["bit_rate"]),
                language,
                title,
            }),
            Some("subtitle") => info.subtitles.push(SubtitleStream {
                index,
                codec,
                language,
                title,
            }),
            _ => {}
        }
    }

    if info.duration <= 0.0 {
        // 有些容器只有流时长
        info.duration = root["streams"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .filter_map(|s| num::<f64>(&s["duration"]))
            .fold(0.0, f64::max);
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "streams": [
        {"index":0,"codec_name":"h264","codec_type":"video","width":1920,"height":1080,
         "pix_fmt":"yuv420p","r_frame_rate":"30000/1001","avg_frame_rate":"30000/1001",
         "bit_rate":"4000000","nb_frames":"300","color_transfer":"smpte2084","color_primaries":"bt2020","color_space":"bt2020nc",
         "side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}]},
        {"index":1,"codec_name":"aac","codec_type":"audio","channels":2,"sample_rate":"48000",
         "bit_rate":"128000","tags":{"language":"eng"}},
        {"index":2,"codec_name":"mov_text","codec_type":"subtitle","tags":{"language":"chi","title":"中文"}},
        {"index":3,"codec_name":"mjpeg","codec_type":"video","width":300,"height":300,
         "disposition":{"attached_pic":1}}
      ],
      "format": {"filename":"a.mp4","format_name":"mov,mp4,m4a,3gp,3g2,mj2",
                 "duration":"10.010000","size":"5000000","bit_rate":"4200000"}
    }"#;

    #[test]
    fn parses_streams_and_format() {
        let info = parse_probe_json(SAMPLE).unwrap();
        assert_eq!(info.duration, 10.01);
        assert_eq!(info.size, 5_000_000);
        assert_eq!(info.bit_rate, Some(4_200_000));
        let v = info.video.as_ref().unwrap();
        assert_eq!((v.width, v.height), (1920, 1080));
        assert!((v.fps - 29.97).abs() < 0.01);
        assert_eq!(v.fps_text, "30000/1001");
        assert_eq!(v.rotation, 90);
        assert_eq!(v.display_size(), (1080, 1920));
        assert_eq!(v.frames, Some(300));
        assert!(v.is_hdr());
        assert_eq!(v.color_primaries, "bt2020");
        assert_eq!(v.color_space, "bt2020nc");
        assert_eq!(info.audio.len(), 1);
        assert_eq!(info.audio[0].language.as_deref(), Some("eng"));
        assert_eq!(info.subtitles.len(), 1);
        assert_eq!(info.subtitles[0].title.as_deref(), Some("中文"));
        assert!(info.has_cover);
        assert_eq!(info.video_bit_rate(), Some(4_000_000));
    }

    #[test]
    fn rotation_normalization() {
        assert_eq!(normalize_rotation(-90.0), 90);
        assert_eq!(normalize_rotation(90.0), 270);
        assert_eq!(normalize_rotation(180.0), 180);
        assert_eq!(normalize_rotation(-180.0), 180);
        assert_eq!(normalize_rotation(0.0), 0);
    }

    #[test]
    fn video_bit_rate_falls_back_to_container_minus_audio() {
        let mut info = parse_probe_json(SAMPLE).unwrap();
        info.video.as_mut().unwrap().bit_rate = None;
        assert_eq!(info.video_bit_rate(), Some(4_200_000 - 128_000));
    }

    #[test]
    fn rate_parsing() {
        assert_eq!(parse_rate("25/1"), Some(25.0));
        assert_eq!(parse_rate("0/0"), None);
        assert_eq!(parse_rate("24"), Some(24.0));
    }

    #[test]
    fn non_square_pixels() {
        let mut v = VideoStream {
            width: 720,
            height: 576,
            sar: "16:15".into(),
            ..VideoStream::default()
        };
        assert_eq!(v.sar_ratio(), Some((16, 15)));
        assert_eq!(v.square_display_size(), (768, 576));
        v.rotation = 90;
        assert_eq!(v.square_display_size(), (576, 768));
        for square in ["1:1", "0:1", "", "N/A"] {
            v.sar = square.into();
            assert_eq!(v.sar_ratio(), None, "{square}");
            assert_eq!(v.square_display_size(), v.display_size());
        }
    }
}
