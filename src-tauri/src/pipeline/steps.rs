//! 流水线级步骤（不属于任何 `pipe.*` 模块，由设置与队列项决定，在 [`super::build`] 中固定调用）：
//!
//! | 步骤 | 位置 | 来源 |
//! |---|---|---|
//! | HDR 转 SDR（`set.hdr`） | 所有模块之前 | 源视频的色彩信息（`VideoStream::is_hdr`） |
//! | 烧录字幕（`queue.sidecar` / `set.auto-subtitle`） | 所有模块之前（HDR 转换之后） | 队列项关联的字幕文件 + `Settings::subtitle_style` |
//! | 嵌入封面（`queue.sidecar` / `set.auto-cover`） | 所有模块之后（偶数尺寸裁剪之后），预览不做 | 队列项关联的图片 + `cover_frames` / `cover_stretch` |
//! | 版权信息（`set.copyright`） | 输出参数 | `Settings::copyright` |
//!
//! 字幕在模块之前烧录：字幕跟随源视频的画面与时间，之后的裁剪、缩放、变速等模块对字幕和画面一视同仁，
//! 片头片尾上不会出现字幕。去头尾（`pipe.trim`）用输入端 `-ss` 定位，滤镜看到的时间从 0 开始，
//! 所以烧录时先把时间戳加回起点（`setpts=PTS+起点/TB`），烧完再减回去；起点要等模块都应用完才知道，
//! 因此先写一个占位标记，构建结束时替换（[`finish_subtitle`]）。

use std::path::{Path, PathBuf};

use super::{escape_filter_path, fmt_num, BuildContext};
use crate::model::SubtitleStyle;

/// 队列项关联的附加文件（同名字幕、封面）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sidecar<'a> {
    /// 烧录进画面的字幕（srt / ass / ssa / vtt）
    pub subtitle: Option<&'a Path>,
    /// 嵌入的封面图片
    pub cover: Option<&'a Path>,
}

impl Sidecar<'_> {
    pub const NONE: Sidecar<'static> = Sidecar {
        subtitle: None,
        cover: None,
    };
}

/// 可作为字幕自动关联的扩展名（按优先顺序）。
pub const SUBTITLE_EXTENSIONS: [&str; 4] = ["srt", "ass", "ssa", "vtt"];
/// 可作为封面自动关联的扩展名（按优先顺序）。
pub const COVER_EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "webp", "bmp"];

/// 字幕滤镜在构建过程中的占位标记（见模块说明）。
const SUBTITLE_MARKER: &str = "@@cliprecipe-subtitles@@";

/// 写进滤镜参数的普通值：先做“选项值”一级转义（`\` `'` `:`），再做“滤镜图”一级转义。
pub fn escape_filter_value(value: &str) -> String {
    let mut option_level = String::new();
    for ch in value.chars() {
        if matches!(ch, '\\' | '\'' | ':') {
            option_level.push('\\');
        }
        option_level.push(ch);
    }
    let mut graph_level = String::new();
    for ch in option_level.chars() {
        if matches!(ch, '\\' | '\'' | '[' | ']' | ',' | ';') {
            graph_level.push('\\');
        }
        graph_level.push(ch);
    }
    graph_level
}

/// `#rrggbb` → ASS 颜色 `&H00BBGGRR`（不透明）。无效时返回 None。
pub fn ass_color(hex: &str) -> Option<String> {
    let hex = hex.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let (r, g, b) = (&hex[0..2], &hex[2..4], &hex[4..6]);
    Some(format!("&H00{b}{g}{r}").to_uppercase())
}

// ---------------------------------------------------------------- 方形像素

/// 变形（非方形像素，SAR ≠ 1:1）源：在所有模块之前缩放为方形像素（`ctx.stream` 已是换算后的尺寸，
/// 见 `VideoStream::square_display_size`），之后各模块的 `setsar=1`、按宽高计算的坐标都基于正确的显示比例。
pub(super) fn normalize_pixels(ctx: &mut BuildContext) {
    if ctx
        .info
        .video
        .as_ref()
        .and_then(|v| v.sar_ratio())
        .is_some()
    {
        let (w, h) = (ctx.stream.width, ctx.stream.height);
        ctx.graph.video(format!("scale={w}:{h},setsar=1"));
    }
}

// ---------------------------------------------------------------- HDR

/// HDR 源：转 SDR 时在所有模块之前做色调映射（预览总是转换，WebView 无法正确显示 HDR）。
/// 返回本次输出是否保留 HDR（需要 10 bit H.265，见 [`super::build`]）。
pub(super) fn apply_hdr(ctx: &mut BuildContext) -> bool {
    let Some(video) = ctx.info.video.as_ref().filter(|v| v.is_hdr()) else {
        return false;
    };
    if !ctx.settings.hdr_to_sdr && !ctx.is_preview() {
        return true;
    }
    let tin = if video.color_transfer.is_empty() {
        "smpte2084"
    } else {
        video.color_transfer.as_str()
    };
    let pin = known_or(&video.color_primaries, &["bt2020", "bt709"], "bt2020");
    let min = known_or(
        &video.color_space,
        &["bt2020nc", "bt2020c", "bt709"],
        "bt2020nc",
    );
    ctx.graph.video(format!(
        "zscale=tin={tin}:pin={pin}:min={min}:t=linear:npl=100,format=gbrpf32le,\
         zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p"
    ));
    ctx.warn("源视频为 HDR，已转换为 SDR（可在设置中改为保留 HDR）");
    false
}

fn known_or<'a>(value: &'a str, known: &[&str], fallback: &'a str) -> &'a str {
    if known.contains(&value) {
        value
    } else {
        fallback
    }
}

// ---------------------------------------------------------------- 字幕

/// 字幕文件是否自带样式（ass / ssa）：自带样式时只指定字体目录，不覆盖作者的样式。
fn has_own_style(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ass") || e.eq_ignore_ascii_case("ssa"))
}

/// 选用的字体文件：样式里指定的，否则自动选择支持中文的系统字体。
fn subtitle_font(style: &SubtitleStyle) -> Option<PathBuf> {
    let chosen = style.font.trim();
    if !chosen.is_empty() {
        return Some(PathBuf::from(chosen));
    }
    super::modules::text::default_font()
}

/// `force_style` 的内容（尺寸以 1080 高为基准：把 PlayRes 设为 `宽:1080`，libass 按实际画面等比缩放）。
pub fn force_style(style: &SubtitleStyle, family: Option<&str>, width: u32, height: u32) -> String {
    let play_x = if height > 0 {
        (1080.0 * f64::from(width) / f64::from(height)).round() as u32
    } else {
        1920
    };
    let mut parts = vec![format!("PlayResX={play_x}"), "PlayResY=1080".to_owned()];
    if let Some(family) = family.filter(|f| !f.is_empty()) {
        parts.push(format!("FontName={family}"));
    }
    parts.push(format!("FontSize={}", style.size.clamp(8, 400)));
    if let Some(color) = ass_color(&style.color) {
        parts.push(format!("PrimaryColour={color}"));
    }
    if let Some(color) = ass_color(&style.outline_color) {
        parts.push(format!("OutlineColour={color}"));
    }
    parts.extend([
        "BorderStyle=1".to_owned(),
        format!("Outline={}", fmt_num(style.outline.clamp(0.0, 20.0))),
        "Shadow=0".to_owned(),
        "Alignment=2".to_owned(),
        format!("MarginV={}", style.margin_bottom.min(1000)),
    ]);
    parts.join(",")
}

/// 字幕：在所有模块之前写入占位标记（真正的滤镜在 [`finish_subtitle`] 中生成）。
pub(super) fn begin_subtitle(ctx: &mut BuildContext, subtitle: &Path) -> Result<(), String> {
    if !subtitle.is_file() {
        return Err(format!("字幕文件不存在：{}", subtitle.display()));
    }
    let style = ctx.settings.subtitle_style.clone();
    let mut filter = format!(
        "subtitles=filename={}",
        escape_filter_path(&subtitle.to_string_lossy())
    );
    match subtitle_font(&style) {
        Some(font) if font.is_file() => {
            // 只把选中的字体放进单独的目录交给 libass（不扫描整个系统字体目录）
            let name = font
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "font.ttf".into());
            let dir = ctx.temp_file("subtitle-fonts")?;
            std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建字体临时目录：{e}"))?;
            let linked = dir.join(&name);
            if std::fs::hard_link(&font, &linked).is_err() {
                std::fs::copy(&font, &linked)
                    .map_err(|e| format!("无法读取字幕字体 {}：{e}", font.display()))?;
            }
            filter.push_str(&format!(
                ":fontsdir={}",
                escape_filter_path(&dir.to_string_lossy())
            ));
            if !has_own_style(subtitle) {
                let family = std::fs::File::open(&font)
                    .ok()
                    .and_then(|mut f| crate::commands::fonts::read_font_names(&mut f))
                    .map(|n| n.family);
                let style_text = force_style(
                    &style,
                    family.as_deref(),
                    ctx.stream.width,
                    ctx.stream.height,
                );
                filter.push_str(&format!(
                    ":force_style={}",
                    escape_filter_value(&style_text)
                ));
            }
        }
        Some(font) => return Err(format!("字幕字体不存在：{}", font.display())),
        None => {
            ctx.warn("没有找到支持中文的字体，字幕使用 libass 的默认字体");
            if !has_own_style(subtitle) {
                let style_text = force_style(&style, None, ctx.stream.width, ctx.stream.height);
                filter.push_str(&format!(
                    ":force_style={}",
                    escape_filter_value(&style_text)
                ));
            }
        }
    }
    ctx.subtitle_filter = Some(filter);
    ctx.graph.video(SUBTITLE_MARKER);
    Ok(())
}

/// 模块都应用完后，把占位标记换成真正的字幕滤镜（按输入端 `-ss` 起点平移时间戳）。
pub(super) fn finish_subtitle(ctx: &mut BuildContext) -> Result<(), String> {
    let Some(filter) = ctx.subtitle_filter.take() else {
        return Ok(());
    };
    let offset = input_start(&ctx.main_input_args);
    let replacement = if offset > 0.0 {
        let t = fmt_num(offset);
        format!("setpts=PTS+{t}/TB,{filter},setpts=PTS-{t}/TB")
    } else {
        filter
    };
    if ctx.graph.replace_marker(SUBTITLE_MARKER, &replacement) {
        Ok(())
    } else {
        Err("滤镜图错误：字幕滤镜丢失".into())
    }
}

/// 主输入前置参数里最后一个 `-ss` 的秒数。
fn input_start(args: &[String]) -> f64 {
    args.iter()
        .rposition(|a| a == "-ss")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------- 封面

/// 封面输出的附加参数（作为 mp4 的 attached_pic）。
pub(super) struct CoverAttachment {
    /// 封面输入的序号
    pub input: usize,
}

/// 嵌入封面：帧数不超过帧率时盖住开头 N 帧；超过帧率时用图片生成 N 帧的片段接在开头（音频相应延后）。
/// mp4 / mov / m4v 还会把图片作为封面图（attached_pic）写入，返回其输入序号。
pub(super) fn apply_cover(
    ctx: &mut BuildContext,
    cover: &Path,
    container: &str,
) -> Result<Option<CoverAttachment>, String> {
    if !cover.is_file() {
        return Err(format!("封面图片不存在：{}", cover.display()));
    }
    let frames = ctx.settings.cover_frames.clamp(1, 100_000);
    let (w, h) = (ctx.stream.width, ctx.stream.height);
    let fps = if ctx.stream.fps > 0.0 {
        ctx.stream.fps
    } else {
        25.0
    };
    let fit = if ctx.settings.cover_stretch {
        format!("scale={w}:{h},setsar=1")
    } else {
        format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:black,setsar=1"
        )
    };
    let main = ctx.graph.take_video();
    let [image, out] = ["cv", "cv"].map(|p| ctx.graph.label(p));
    if f64::from(frames) <= fps {
        let input = ctx.add_input(Vec::new(), cover)?;
        ctx.graph.chain(&[&format!("{input}:v")], &fit, &[&image]);
        ctx.graph.chain(
            &[&main, &image],
            &format!("overlay=0:0:eof_action=repeat:enable='lt(n,{frames})'"),
            &[&out],
        );
    } else {
        let rate = ctx.rate_text().unwrap_or_else(|| fmt_num(fps));
        let input = ctx.add_input(
            vec!["-loop".into(), "1".into(), "-framerate".into(), rate],
            cover,
        )?;
        ctx.graph.chain(
            &[&format!("{input}:v")],
            &format!("{fit},trim=end_frame={frames},setpts=PTS-STARTPTS"),
            &[&image],
        );
        ctx.graph
            .chain(&[&image, &main], "concat=n=2:v=1:a=0", &[&out]);
        let lead = f64::from(frames) / fps;
        let ms = (lead * 1000.0).round() as u64;
        ctx.graph.audio(format!("adelay=delays={ms}:all=1"));
        ctx.stream.duration += lead;
    }
    ctx.graph.set_video(out);

    if matches!(container, "mp4" | "mov" | "m4v") {
        let input = ctx.add_input(Vec::new(), cover)?;
        return Ok(Some(CoverAttachment { input }));
    }
    Ok(None)
}

/// 封面图（attached_pic）的输出参数：作为第 2 条视频流，用 mjpeg 编码。
pub(super) fn cover_output_args(attachment: &CoverAttachment) -> Vec<String> {
    [
        "-map",
        &format!("{}:v:0", attachment.input),
        "-c:v:1",
        "mjpeg",
        "-pix_fmt:v:1",
        "yuvj420p",
        "-q:v:1",
        "2",
        "-disposition:v:1",
        "attached_pic",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

// ---------------------------------------------------------------- 版权

/// `set.copyright`：写入容器元数据的参数；为空时不写。
pub fn copyright_args(copyright: &str) -> Vec<String> {
    let text = copyright.trim();
    if text.is_empty() {
        return Vec::new();
    }
    vec!["-metadata".into(), format!("copyright={text}")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_escaping() {
        assert_eq!(ass_color("#ff8000").as_deref(), Some("&H000080FF"));
        assert_eq!(ass_color("xyz"), None);
        assert_eq!(
            escape_filter_value("FontName=A B,Size=1:x"),
            r"FontName=A B\,Size=1\\:x"
        );
    }

    #[test]
    fn force_style_scales_from_1080() {
        let style = SubtitleStyle::default();
        let text = force_style(&style, Some("Microsoft YaHei"), 1280, 720);
        assert_eq!(
            text,
            "PlayResX=1920,PlayResY=1080,FontName=Microsoft YaHei,FontSize=48,\
             PrimaryColour=&H00FFFFFF,OutlineColour=&H00000000,BorderStyle=1,Outline=2,Shadow=0,\
             Alignment=2,MarginV=60"
        );
        let portrait = force_style(&style, None, 720, 1280);
        assert!(portrait.starts_with("PlayResX=608,PlayResY=1080,FontSize=48"));
    }

    #[test]
    fn input_start_reads_last_ss() {
        let args: Vec<String> = ["-ss", "1.5", "-t", "3"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(input_start(&args), 1.5);
        assert_eq!(input_start(&[]), 0.0);
    }

    #[test]
    fn copyright_metadata() {
        assert!(copyright_args("  ").is_empty());
        assert_eq!(copyright_args("© 某某"), ["-metadata", "copyright=© 某某"]);
    }
}
