//! `tool.cover` 封面处理：给视频嵌入封面图，或从视频截取封面图。视频、音频一律直接复制，从不重编码。
//!
//! **参数**（括号内为默认值）
//! - `mode`（`embed`）：
//!   - `embed` 嵌入封面：封面来自 `source`；
//!   - `capture` 截取封面：在 `position` 处截一帧，按 `namePattern` 命名保存在输出位置；`alsoEmbed` 时同时嵌入视频。
//! - `source`（`frame`）嵌入时封面的来源：`image` 指定图片（`image` 路径）；`sameName` 视频旁同名图片
//!   （`<视频名>.jpg/.jpeg/.png/.webp/.bmp`，找不到时失败）；`frame` 截取视频 `position` 处的一帧。
//! - `position`（`10%`）截取位置：秒数（`12.5`）、`分:秒`、`时:分:秒` 或百分比（`10%`）；超出时长时取最后一帧附近。
//! - `namePattern`（`{name}_cover`）截图文件名（不含扩展名）：`{name}` 视频名（已应用“文件名去除”）、
//!   `{index}` 队列序号；不能为空。仍按设置加队列序号前缀，重名时按“目标已存在时”处理。
//! - `imageFormat`（`jpg`）截图格式：`jpg`（质量 2）或 `png`。
//! - `alsoEmbed`（false）截图后同时把它嵌入视频。
//!
//! **嵌入方式**
//! - mp4 / m4v / mov：封面作为一条 `attached_pic` 视频流（jpg / png 直接复制，其它图片格式转为 jpg）；
//!   只保留第一条视频流、全部音轨、mov_text 字幕（其它字幕放不进 mp4，丢弃并提示）。
//! - mkv：封面作为附件（`-attach`，文件名 `cover.jpg` / `cover.png`，带 mimetype），FFmpeg 读取时会把它识别为
//!   `attached_pic`；原有附件（字体等）保留，原有封面被替换。
//! - 输出容器按设置“目标格式”（mp4 或保持源格式）；源格式不能带封面（如 avi、ts、flv）或流放不进 mp4 时改用 mkv。
//! - 嵌入后用 ffprobe 检查输出确实带封面（`attached_pic`），否则失败。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::pipeline::fmt_num;
use crate::tools::media::{container_accepts, StreamKind};
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.cover";

/// 同名图片 / 指定图片可用的扩展名。
pub const IMAGE_EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "webp", "bmp"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverMode {
    #[default]
    Embed,
    Capture,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverSource {
    Image,
    SameName,
    #[default]
    Frame,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageFormat {
    #[default]
    Jpg,
    Png,
}

impl ImageFormat {
    fn ext(self) -> &'static str {
        match self {
            ImageFormat::Jpg => "jpg",
            ImageFormat::Png => "png",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CoverParams {
    pub mode: CoverMode,
    pub source: CoverSource,
    /// 指定的封面图片
    pub image: String,
    /// 截取位置：秒、分:秒、时:分:秒或百分比
    pub position: String,
    /// 截图文件名样式
    pub name_pattern: String,
    pub image_format: ImageFormat,
    /// 截图后同时嵌入
    pub also_embed: bool,
}

impl Default for CoverParams {
    fn default() -> Self {
        Self {
            mode: CoverMode::Embed,
            source: CoverSource::Frame,
            image: String::new(),
            position: "10%".into(),
            name_pattern: "{name}_cover".into(),
            image_format: ImageFormat::Jpg,
            also_embed: false,
        }
    }
}

/// 解析截取位置（见模块说明），返回秒数（未限制到时长内）。
pub fn parse_position(text: &str, duration: f64) -> Result<f64, String> {
    let text = text.trim();
    let invalid = || format!("无法识别的截取位置“{text}”：请写秒数、分:秒或百分比（如 10%）");
    if text.is_empty() {
        return Err("请填写截取位置".into());
    }
    if let Some(percent) = text.strip_suffix(['%', '％']) {
        let p: f64 = percent.trim().parse().map_err(|_| invalid())?;
        if !(0.0..=100.0).contains(&p) {
            return Err("截取位置的百分比必须在 0～100 之间".into());
        }
        return Ok(duration.max(0.0) * p / 100.0);
    }
    let parts: Vec<&str> = text.split([':', '：']).collect();
    if parts.len() > 3 {
        return Err(invalid());
    }
    let mut seconds = 0.0;
    for (i, part) in parts.iter().enumerate() {
        let value: f64 = part.trim().parse().map_err(|_| invalid())?;
        let last = i + 1 == parts.len();
        if !value.is_finite() || value < 0.0 || (!last && value.fract() != 0.0) {
            return Err(invalid());
        }
        seconds = seconds * 60.0 + value;
    }
    Ok(seconds)
}

/// 截取时间：限制在视频之内（最后一帧之前一点），超出时给出提示。
pub fn capture_time(position: &str, info: &MediaInfo) -> Result<(f64, Option<String>), String> {
    let wanted = parse_position(position, info.duration)?;
    if info.duration <= 0.0 {
        return Ok((wanted, None));
    }
    let frame = info
        .video
        .as_ref()
        .map(|v| if v.fps > 0.0 { 1.0 / v.fps } else { 0.04 })
        .unwrap_or(0.04);
    let last = (info.duration - frame * 2.0).max(0.0);
    if wanted > last {
        return Ok((
            last,
            Some(format!(
                "截取位置 {} 秒超出视频时长，改为 {} 秒",
                fmt_num(wanted),
                fmt_num(last)
            )),
        ));
    }
    Ok((wanted, None))
}

/// 截图文件名：替换 `{name}`、`{index}`。
pub fn render_name(pattern: &str, name: &str, index: usize) -> Result<String, String> {
    let rendered = pattern
        .replace("{name}", name)
        .replace("{index}", &index.to_string());
    let rendered = rendered.trim();
    if rendered.is_empty() {
        return Err("截图文件名样式不能为空".into());
    }
    if rendered.contains(['{', '}']) {
        return Err(format!(
            "截图文件名样式只支持 {{name}} 和 {{index}}：{pattern}"
        ));
    }
    Ok(rendered.to_owned())
}

/// 截一帧的参数（输出路径在最后）。
pub fn capture_args(input: &str, time: f64, format: ImageFormat, output: &str) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-ss".into(),
        fmt_num(time),
        "-i".into(),
        input.into(),
        "-map".into(),
        "0:V:0".into(),
        "-frames:v".into(),
        "1".into(),
    ];
    match format {
        ImageFormat::Jpg => args.extend(["-c:v".into(), "mjpeg".into(), "-q:v".into(), "2".into()]),
        ImageFormat::Png => args.extend(["-c:v".into(), "png".into()]),
    }
    args.extend([
        "-update".into(),
        "1".into(),
        "-f".into(),
        "image2".into(),
        output.into(),
    ]);
    args
}

/// 图片文件的类型（按扩展名）：jpg / png 可以直接复制进视频，其它需要转换。
fn image_kind(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        _ => None,
    }
}

/// 嵌入封面的容器：`wanted` 为按设置得到的扩展名。
pub fn embed_container(wanted: &str, info: &MediaInfo) -> (String, Option<String>) {
    let wanted = wanted.to_ascii_lowercase();
    let fits_mp4 = |ext: &str| {
        let video_ok = info.video.as_ref().map_or(true, |v| {
            container_accepts(ext, StreamKind::Video, &v.codec) != Some(false)
        });
        let audio_ok = info
            .audio
            .iter()
            .all(|a| container_accepts(ext, StreamKind::Audio, &a.codec) != Some(false));
        video_ok && audio_ok
    };
    match wanted.as_str() {
        "mkv" => ("mkv".into(), None),
        "mp4" | "m4v" | "mov" if fits_mp4(&wanted) => (wanted, None),
        "mp4" | "m4v" | "mov" => (
            "mkv".into(),
            Some(format!("音视频编码放不进 {wanted}，改为输出 mkv")),
        ),
        other if fits_mp4("mp4") => (
            "mp4".into(),
            Some(format!("{other} 格式不能带封面，改为输出 mp4")),
        ),
        other => (
            "mkv".into(),
            Some(format!("{other} 格式不能带封面，改为输出 mkv")),
        ),
    }
}

/// 嵌入封面的参数（输出路径在最后）与提示。`image` 为 jpg / png 文件（mkv 时文件名应为 `cover.jpg` / `cover.png`）。
pub fn embed_args(
    input: &str,
    image: &Path,
    info: &MediaInfo,
    container: &str,
    output: &str,
) -> Result<(Vec<String>, Vec<String>), String> {
    if info.video.is_none() {
        return Err("文件中没有视频流".into());
    }
    let image_arg = path_arg(image)?;
    let mut warnings = Vec::new();
    let mut args: Vec<String> = vec!["-i".into(), input.into()];
    if container == "mkv" {
        let kind = image_kind(image).ok_or("mkv 封面必须是 jpg 或 png 图片")?;
        // ffmpeg 取 `-attach` 路径最后一个 `/` 之后的部分作为附件的 filename，Windows 路径要换成正斜杠
        let attach = image_arg.replace('\\', "/");
        let name = attach.rsplit('/').next().unwrap_or_default().to_owned();
        let mime = if kind == "png" {
            "image/png"
        } else {
            "image/jpeg"
        };
        args.extend([
            "-attach".into(),
            attach,
            "-map".into(),
            "0:V:0".into(),
            "-map".into(),
            "0:a?".into(),
            "-map".into(),
            "0:s?".into(),
            "-map".into(),
            "0:t?".into(),
            "-c".into(),
            "copy".into(),
            format!("-metadata:s:t:m:filename:{name}"),
            format!("mimetype={mime}"),
            output.into(),
        ]);
        return Ok((args, warnings));
    }
    args.extend(["-i".into(), image_arg]);
    args.extend(["-map".into(), "0:V:0".into(), "-map".into(), "0:a?".into()]);
    let text_subs = info
        .subtitles
        .iter()
        .all(|s| s.codec.eq_ignore_ascii_case("mov_text"));
    if !info.subtitles.is_empty() {
        if text_subs {
            args.extend(["-map".into(), "0:s?".into()]);
        } else {
            warnings.push(format!("字幕轨放不进 {container}，已丢弃"));
        }
    }
    args.extend(["-map".into(), "1:v:0".into(), "-c".into(), "copy".into()]);
    if image_kind(image).is_none() {
        args.extend(["-c:v:1".into(), "mjpeg".into(), "-q:v:1".into(), "2".into()]);
    }
    args.extend([
        "-disposition:v:0".into(),
        "default".into(),
        "-disposition:v:1".into(),
        "attached_pic".into(),
        "-movflags".into(),
        "+faststart".into(),
        output.into(),
    ]);
    Ok((args, warnings))
}

/// 视频旁的同名图片。
pub fn same_name_image(video: &Path, exists: impl Fn(&Path) -> bool) -> Result<PathBuf, String> {
    let stem = video
        .file_stem()
        .ok_or_else(|| format!("无效的文件名：{}", video.display()))?;
    let dir = video.parent().unwrap_or(Path::new(""));
    for ext in IMAGE_EXTENSIONS {
        let mut name = stem.to_os_string();
        name.push(".");
        name.push(ext);
        let candidate = dir.join(name);
        if exists(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "没有找到同名封面图片（{}.jpg / .png 等）",
        stem.to_string_lossy()
    ))
}

impl CoverParams {
    fn validate(&self) -> Result<(), String> {
        parse_position(&self.position, 100.0)?;
        if self.mode == CoverMode::Capture {
            render_name(&self.name_pattern, "name", 1)?;
        }
        if self.mode == CoverMode::Embed && self.source == CoverSource::Image {
            let path = self.image.trim();
            if path.is_empty() {
                return Err("请选择封面图片".into());
            }
            let ext = Path::new(path)
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
                return Err(format!(
                    "封面图片只支持 {}：{path}",
                    IMAGE_EXTENSIONS.join(" / ")
                ));
            }
        }
        Ok(())
    }

    fn embeds(&self) -> bool {
        self.mode == CoverMode::Embed || self.also_embed
    }
}

/// 把封面准备成 mkv 能用的 `cover.jpg` / `cover.png`（jpg / png 复制，其它格式转为 jpg）。
fn prepare_mkv_image(ctx: &mut ToolContext, image: &Path) -> Result<PathBuf, ToolError> {
    let dir = ctx.temp_dir()?;
    match image_kind(image) {
        Some(kind) => {
            let target = dir.join(format!("cover.{kind}"));
            std::fs::copy(image, &target).map_err(|e| format!("无法复制封面图片：{e}"))?;
            Ok(target)
        }
        None => {
            let target = dir.join("cover.jpg");
            let args = vec![
                "-i".into(),
                path_arg(image)?,
                "-frames:v".into(),
                "1".into(),
                "-c:v".into(),
                "mjpeg".into(),
                "-q:v".into(),
                "2".into(),
                "-update".into(),
                "1".into(),
                "-f".into(),
                "image2".into(),
                path_arg(&target)?,
            ];
            ctx.run_ffmpeg("转换封面图片", args, 0.0)?;
            Ok(target)
        }
    }
}

fn embed(
    ctx: &mut ToolContext,
    info: &MediaInfo,
    input: &Path,
    image: &Path,
) -> Result<(), ToolError> {
    let wanted = ctx.video_extension(0)?;
    let (container, note) = embed_container(&wanted, info);
    if let Some(note) = note {
        ctx.warn(note);
    }
    let image = if container == "mkv" {
        prepare_mkv_image(ctx, image)?
    } else {
        image.to_path_buf()
    };
    let output = ctx.output(&OutputSpec::file(&container).suffix("_cover"))?;
    let (args, warnings) = embed_args(
        &path_arg(input)?,
        &image,
        info,
        &container,
        &path_arg(&output)?,
    )?;
    for warning in warnings {
        ctx.warn(warning);
    }
    ctx.run_ffmpeg("嵌入封面", args, info.duration)?;
    let written = ctx.probe(&output)?;
    if !written.has_cover {
        return Err("封面没有写入成功：ffprobe 未在输出中识别到封面".into());
    }
    Ok(())
}

impl Tool for CoverParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let info = ctx.info(0)?;
        let input = ctx.input_path(0)?.to_path_buf();
        if info.video.is_none() {
            return Err("文件中没有视频流".into());
        }
        let needs_frame = self.mode == CoverMode::Capture || self.source == CoverSource::Frame;
        let mut steps = Vec::new();
        if needs_frame {
            steps.push(1.0);
        }
        if self.embeds() {
            steps.push(2.0);
        }
        ctx.set_steps(&steps);

        let image = if needs_frame {
            let (time, note) = capture_time(&self.position, &info)?;
            if let Some(note) = note {
                ctx.warn(note);
            }
            let path = if self.mode == CoverMode::Capture {
                let name = render_name(&self.name_pattern, &ctx.stem(0)?, ctx.number())?;
                ctx.output(&OutputSpec::file(self.image_format.ext()).stem(name))?
            } else {
                ctx.temp_file(&format!("frame.{}", self.image_format.ext()))?
            };
            let args = capture_args(
                &path_arg(&input)?,
                time,
                self.image_format,
                &path_arg(&path)?,
            );
            ctx.run_ffmpeg("截取封面", args, 0.0)?;
            if !path.is_file() {
                return Err(format!("没有截取到画面（{} 秒）", fmt_num(time)).into());
            }
            path
        } else if self.source == CoverSource::SameName {
            let found = same_name_image(&input, Path::is_file)?;
            ctx.note(format!("使用同名封面：{}", found.display()));
            found
        } else {
            let path = PathBuf::from(self.image.trim());
            if !path.is_file() {
                return Err(format!("封面图片不存在：{}", path.display()).into());
            }
            path
        };

        if self.embeds() {
            embed(ctx, &info, &input, &image)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::SubtitleStream;
    use crate::pipeline::testing::sample_info;

    #[test]
    fn positions() {
        assert_eq!(parse_position("12.5", 100.0), Ok(12.5));
        assert_eq!(parse_position("1:30", 100.0), Ok(90.0));
        assert_eq!(parse_position("1:00:02.5", 0.0), Ok(3602.5));
        assert_eq!(parse_position("10%", 40.0), Ok(4.0));
        assert_eq!(parse_position(" 50 ％", 10.0), Ok(5.0));
        for bad in ["", "abc", "-1", "101%", "1:2:3:4", "1.5:20", "1::2"] {
            assert!(parse_position(bad, 10.0).is_err(), "{bad:?}");
        }
        let info = sample_info(); // 10 秒 25fps
        assert_eq!(capture_time("2", &info).unwrap(), (2.0, None));
        let (t, note) = capture_time("100%", &info).unwrap();
        assert_eq!(t, 9.92);
        assert!(note.is_some());
    }

    #[test]
    fn name_patterns() {
        assert_eq!(
            render_name("{name}_cover", "clip", 3).unwrap(),
            "clip_cover"
        );
        assert_eq!(render_name("{index}", "clip", 3).unwrap(), "3");
        assert_eq!(render_name("{name}", "clip", 3).unwrap(), "clip");
        assert_eq!(
            render_name("封面-{index}-{name}", "a b", 12).unwrap(),
            "封面-12-a b"
        );
        assert!(render_name("  ", "clip", 1).is_err());
        assert!(render_name("{title}", "clip", 1).is_err());
    }

    #[test]
    fn capture_command() {
        assert_eq!(
            capture_args("in.mp4", 1.5, ImageFormat::Jpg, "out.jpg").join(" "),
            "-ss 1.5 -i in.mp4 -map 0:V:0 -frames:v 1 -c:v mjpeg -q:v 2 -update 1 -f image2 out.jpg"
        );
        assert_eq!(
            capture_args("in.mp4", 0.0, ImageFormat::Png, "o.png").join(" "),
            "-ss 0 -i in.mp4 -map 0:V:0 -frames:v 1 -c:v png -update 1 -f image2 o.png"
        );
    }

    #[test]
    fn mp4_embed_uses_attached_pic_stream() {
        let info = sample_info();
        let (args, warnings) =
            embed_args("in.mp4", Path::new("c.png"), &info, "mp4", "out.mp4").unwrap();
        assert_eq!(
            args.join(" "),
            "-i in.mp4 -i c.png -map 0:V:0 -map 0:a? -map 1:v:0 -c copy -disposition:v:0 default -disposition:v:1 attached_pic -movflags +faststart out.mp4"
        );
        assert!(warnings.is_empty());
        let mut subs = sample_info();
        subs.subtitles.push(SubtitleStream {
            index: 2,
            codec: "subrip".into(),
            language: None,
            title: None,
        });
        let (args, warnings) =
            embed_args("in.mkv", Path::new("c.webp"), &subs, "mov", "out.mov").unwrap();
        assert_eq!(
            args.join(" "),
            "-i in.mkv -i c.webp -map 0:V:0 -map 0:a? -map 1:v:0 -c copy -c:v:1 mjpeg -q:v:1 2 -disposition:v:0 default -disposition:v:1 attached_pic -movflags +faststart out.mov"
        );
        assert_eq!(warnings.len(), 1, "srt 字幕放不进 mov");
        subs.subtitles[0].codec = "mov_text".into();
        let (args, _) = embed_args("in.mp4", Path::new("c.jpg"), &subs, "mp4", "o.mp4").unwrap();
        assert!(args.join(" ").contains("-map 0:s? -map 1:v:0"));
    }

    #[test]
    fn mkv_embed_uses_attachment() {
        let (args, _) = embed_args(
            "in.mkv",
            Path::new("cover.jpg"),
            &sample_info(),
            "mkv",
            "out.mkv",
        )
        .unwrap();
        assert_eq!(
            args.join(" "),
            "-i in.mkv -attach cover.jpg -map 0:V:0 -map 0:a? -map 0:s? -map 0:t? -c copy -metadata:s:t:m:filename:cover.jpg mimetype=image/jpeg out.mkv"
        );
        let (args, _) = embed_args(
            "in.mkv",
            Path::new(r"C:\tmp\cover.png"),
            &sample_info(),
            "mkv",
            "o.mkv",
        )
        .unwrap();
        let line = args.join(" ");
        assert!(line.contains("-attach C:/tmp/cover.png "), "{line}");
        assert!(
            line.contains("-metadata:s:t:m:filename:cover.png mimetype=image/png"),
            "{line}"
        );
        assert!(embed_args("in.mkv", Path::new("c.bmp"), &sample_info(), "mkv", "o.mkv").is_err());
        let mut no_video = sample_info();
        no_video.video = None;
        assert!(embed_args("a.m4a", Path::new("c.jpg"), &no_video, "mp4", "o.mp4").is_err());
    }

    #[test]
    fn containers_that_can_hold_covers() {
        let info = sample_info(); // h264 + aac
        assert_eq!(embed_container("mp4", &info), ("mp4".into(), None));
        assert_eq!(embed_container("MOV", &info), ("mov".into(), None));
        assert_eq!(embed_container("mkv", &info), ("mkv".into(), None));
        assert_eq!(embed_container("avi", &info).0, "mp4");
        assert!(embed_container("ts", &info).1.is_some());
        let mut vorbis = sample_info();
        vorbis.audio[0].codec = "vorbis".into();
        assert_eq!(embed_container("mp4", &vorbis).0, "mkv");
        assert_eq!(embed_container("webm", &vorbis).0, "mkv");
    }

    #[test]
    fn same_name_lookup() {
        let video = Path::new("dir/clip.mp4");
        let found = same_name_image(video, |p| p == Path::new("dir/clip.png")).unwrap();
        assert_eq!(found, Path::new("dir/clip.png"));
        assert!(same_name_image(video, |_| false).is_err());
    }

    #[test]
    fn validation() {
        assert!(CoverParams::default().validate().is_ok());
        let image = CoverParams {
            source: CoverSource::Image,
            ..CoverParams::default()
        };
        assert!(image.validate().is_err(), "没有选择图片");
        let gif = CoverParams {
            source: CoverSource::Image,
            image: "a.gif".into(),
            ..CoverParams::default()
        };
        assert!(gif.validate().is_err());
        let capture = CoverParams {
            mode: CoverMode::Capture,
            name_pattern: String::new(),
            ..CoverParams::default()
        };
        assert!(capture.validate().is_err());
        let bad_position = CoverParams {
            position: "x".into(),
            ..CoverParams::default()
        };
        assert!(bad_position.validate().is_err());
    }
}
