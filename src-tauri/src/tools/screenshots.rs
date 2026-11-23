//! `tool.screenshots` 批量截图：按规则给队列中每个视频截若干张图，放进以视频名命名的文件夹
//! （`<输出目录>/<名称>/<名称>_01.jpg`…）。
//!
//! **截图时间** `mode`：
//! - `interval`（默认）每隔 `intervalSeconds` 秒（默认 10，至少 0.1）一张，从 0 秒开始；
//! - `count` 平均截 `count` 张（默认 9，1～1000）：把视频等分成 N 段，取每段的正中间；
//! - `timestamps` 指定时间：`timestamps` 文本，逗号 / 空格 / 换行分隔，每项为秒数、`分:秒` 或 `时:分:秒`
//!   （如 `1.5, 1:30, 0:02:03.5`），按书写顺序截图；超出视频时长的时间点跳过并提示；
//! - `single` 一张：在时长的 `position`%（默认 50）处。
//!
//! 每个时间点取**最接近的一帧**（`-ss` 放在输入前，提前半帧，精确到帧）；时间点不晚于最后一帧。截图数上限 1000。
//!
//! **格式** `format`：`jpg`（默认，`quality` 1～100，默认 90，映射到 `-q:v` 31～2）或 `png`（无损）。
//! `width` 缩放宽度（默认 0 = 原尺寸；16～7680，高度按比例取偶）。
//!
//! **绘制文件名** `drawName`（默认关闭）：把视频文件名（不含扩展名，原名，不做文件名去除）画在图片上，支持中文
//! （与流水线“文本”模块相同的系统中文字体与转义）。样式：`nameSize` 字号（图片高度的 %，默认 5）、`nameColor`
//! 文字颜色（默认白色）、`nameOutline` 描边宽度（字号的 %，默认 8，0 为不描边）、`nameOutlineColor` 描边颜色（默认黑色）、
//! `namePosition` 位置（九宫格中的 7 个：左上、上、右上、居中、左下、下（默认）、右下；边距为图片高度的 3%）。
//! 文字比图片宽时自动缩小字号。
//!
//! **命名** `naming`：`index`（默认，`_01`、`_02`…，位数随张数，至少两位）或 `time`（截图时间，`_00-01-05.500`）。
//!
//! 每张截图运行一次 ffmpeg（输入端跳转，长视频也很快），进度每张一步。不用视频编码器，不受“码率”设置影响。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::pipeline::escape_filter_path;
use crate::pipeline::fmt_num;
use crate::pipeline::modules::text::{default_font, escape_value, normalize_text, widest_line_em};
use crate::tools::common::frame::{color_arg, even};
use crate::tools::media::secs;
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.screenshots";

/// 截图数上限。
pub const MAX_SHOTS: usize = 1000;
/// 绘制文件名的最小字号（像素）。
const MIN_FONT_PX: u32 = 8;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShotMode {
    /// 每隔 N 秒
    #[default]
    Interval,
    /// 平均 N 张
    Count,
    /// 指定时间
    Timestamps,
    /// 一张（按位置）
    Single,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShotFormat {
    #[default]
    Jpg,
    Png,
}

impl ShotFormat {
    pub fn ext(self) -> &'static str {
        match self {
            ShotFormat::Jpg => "jpg",
            ShotFormat::Png => "png",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShotNaming {
    /// 序号
    #[default]
    Index,
    /// 截图时间
    Time,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NamePosition {
    TopLeft,
    Top,
    TopRight,
    Center,
    BottomLeft,
    #[default]
    Bottom,
    BottomRight,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScreenshotsParams {
    pub mode: ShotMode,
    /// 每隔多少秒
    pub interval_seconds: f64,
    /// 平均截几张
    pub count: u32,
    /// 指定时间（逗号 / 空格 / 换行分隔）
    pub timestamps: String,
    /// 一张：位置（时长的百分比）
    pub position: f64,
    pub format: ShotFormat,
    /// jpg 质量 1～100
    pub quality: u32,
    /// 缩放宽度，0 = 原尺寸
    pub width: u32,
    pub naming: ShotNaming,
    /// 把文件名画在图片上
    pub draw_name: bool,
    /// 字号（图片高度的百分比）
    pub name_size: f64,
    pub name_color: String,
    /// 描边宽度（字号的百分比）
    pub name_outline: f64,
    pub name_outline_color: String,
    pub name_position: NamePosition,
}

impl Default for ScreenshotsParams {
    fn default() -> Self {
        Self {
            mode: ShotMode::Interval,
            interval_seconds: 10.0,
            count: 9,
            timestamps: String::new(),
            position: 50.0,
            format: ShotFormat::Jpg,
            quality: 90,
            width: 0,
            naming: ShotNaming::Index,
            draw_name: false,
            name_size: 5.0,
            name_color: "#ffffff".into(),
            name_outline: 8.0,
            name_outline_color: "#000000".into(),
            name_position: NamePosition::Bottom,
        }
    }
}

fn in_range(value: f64, (min, max): (f64, f64)) -> bool {
    value.is_finite() && (min..=max).contains(&value)
}

/// 解析一个时间：秒数、`分:秒` 或 `时:分:秒`。
pub fn parse_time(text: &str) -> Result<f64, String> {
    let invalid = || format!("无法识别的时间：{text}");
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() > 3 || parts.iter().any(|p| p.trim().is_empty()) {
        return Err(invalid());
    }
    let mut total = 0.0;
    for (i, part) in parts.iter().enumerate() {
        let value: f64 = part.trim().parse().map_err(|_| invalid())?;
        let last = i + 1 == parts.len();
        if !value.is_finite() || value < 0.0 || (!last && value.fract() != 0.0) {
            return Err(invalid());
        }
        if i > 0 && value >= 60.0 {
            return Err(invalid());
        }
        total = total * 60.0 + value;
    }
    Ok(total)
}

/// 解析“指定时间”文本。
pub fn parse_timestamps(text: &str) -> Result<Vec<f64>, String> {
    text.split(|c: char| c == ',' || c == '，' || c == '、' || c == ';' || c.is_whitespace())
        .filter(|s| !s.trim().is_empty())
        .map(parse_time)
        .collect()
}

/// 截图时间表：要截的时间点（秒，按截图顺序）与被跳过的时间点（超出时长）。
#[derive(Debug, Clone, PartialEq)]
pub struct ShotTimes {
    pub times: Vec<f64>,
    pub skipped: Vec<f64>,
}

impl ScreenshotsParams {
    fn validate(&self) -> Result<(), String> {
        match self.mode {
            ShotMode::Interval if !in_range(self.interval_seconds, (0.1, 86_400.0)) => {
                return Err("截图间隔必须在 0.1～86400 秒之间".into());
            }
            ShotMode::Count if !(1..=MAX_SHOTS as u32).contains(&self.count) => {
                return Err(format!("截图张数必须在 1～{MAX_SHOTS} 之间"));
            }
            ShotMode::Single if !in_range(self.position, (0.0, 100.0)) => {
                return Err("截图位置必须在 0～100% 之间".into());
            }
            ShotMode::Timestamps if parse_timestamps(&self.timestamps)?.is_empty() => {
                return Err("请填写截图时间".into());
            }
            _ => {}
        }
        if !(1..=100).contains(&self.quality) {
            return Err("图片质量必须在 1～100 之间".into());
        }
        if self.width != 0 && !(16..=7680).contains(&self.width) {
            return Err("缩放宽度必须在 16～7680 像素之间（0 表示原尺寸）".into());
        }
        if self.draw_name {
            if !in_range(self.name_size, (1.0, 30.0)) {
                return Err("文件名字号必须在 1～30% 之间".into());
            }
            if !in_range(self.name_outline, (0.0, 25.0)) {
                return Err("描边宽度必须在 0～25% 之间".into());
            }
            color_arg(&self.name_color, "文字颜色")?;
            color_arg(&self.name_outline_color, "描边颜色")?;
        }
        Ok(())
    }

    /// 按规则计算截图时间。`fps` 用于把时间点限制在最后一帧之前。
    pub fn shot_times(&self, duration: f64, fps: f64) -> Result<ShotTimes, String> {
        self.validate()?;
        if !(duration.is_finite() && duration > 0.0) {
            return Err("无法读取视频时长".into());
        }
        let frame = if fps.is_finite() && fps > 0.0 {
            1.0 / fps
        } else {
            0.04
        };
        let last = (duration - frame).max(0.0);
        let mut skipped = Vec::new();
        let times: Vec<f64> = match self.mode {
            ShotMode::Interval => {
                let step = self.interval_seconds;
                if (duration / step).ceil() as usize > MAX_SHOTS {
                    return Err(format!("截图太多（超过 {MAX_SHOTS} 张），请加大截图间隔"));
                }
                let mut times = Vec::new();
                let mut k = 0.0;
                while step * k < duration - 1e-6 {
                    times.push((step * k).min(last));
                    k += 1.0;
                }
                times
            }
            ShotMode::Count => {
                let n = f64::from(self.count);
                (0..self.count)
                    .map(|i| (duration * (f64::from(i) + 0.5) / n).min(last))
                    .collect()
            }
            ShotMode::Single => vec![(duration * self.position / 100.0).min(last)],
            ShotMode::Timestamps => {
                let mut times = Vec::new();
                for t in parse_timestamps(&self.timestamps)? {
                    if t < duration {
                        times.push(t.min(last));
                    } else {
                        skipped.push(t);
                    }
                }
                if times.len() > MAX_SHOTS {
                    return Err(format!("截图太多（超过 {MAX_SHOTS} 张）"));
                }
                times
            }
        };
        if times.is_empty() {
            return Err("所有指定时间都超出了视频时长".into());
        }
        Ok(ShotTimes { times, skipped })
    }

    /// jpg 质量 → `-q:v`（100 → 2，1 → 31）。
    pub fn jpeg_q(&self) -> u32 {
        let q = 2.0 + f64::from(100 - self.quality.clamp(1, 100)) * 29.0 / 99.0;
        q.round() as u32
    }
}

/// 输出文件名的后缀：序号或时间。
pub fn shot_suffix(naming: ShotNaming, index: usize, total: usize, time: f64) -> String {
    match naming {
        ShotNaming::Index => {
            let width = total.max(1).to_string().len().max(2);
            format!("_{:0width$}", index + 1)
        }
        ShotNaming::Time => {
            let ms = (time * 1000.0).round().max(0.0) as u64;
            format!(
                "_{:02}-{:02}-{:02}.{:03}",
                ms / 3_600_000,
                ms / 60_000 % 60,
                ms / 1000 % 60,
                ms % 1000
            )
        }
    }
}

/// 截图的滤镜（缩放、绘制文件名）。`size`：源视频显示尺寸；`name`：要画的文字；`font`：字体文件。
/// 返回滤镜串（可能为空）。
pub fn shot_filter(
    params: &ScreenshotsParams,
    (sw, sh): (u32, u32),
    name: &str,
    font: Option<&str>,
) -> Result<String, String> {
    let mut filters = Vec::new();
    let (w, h) = if params.width > 0 && sw > 0 {
        let h = even(((f64::from(sh) * f64::from(params.width) / f64::from(sw)).round()) as u32);
        filters.push(format!("scale={}:{h}", params.width));
        (params.width, h)
    } else {
        (sw, sh)
    };
    let text = normalize_text(name);
    if params.draw_name && !text.is_empty() {
        let color = color_arg(&params.name_color, "文字颜色")?;
        let outline_color = color_arg(&params.name_outline_color, "描边颜色")?;
        let mut size = ((f64::from(h) * params.name_size / 100.0).round() as u32).max(MIN_FONT_PX);
        let em = widest_line_em(&text);
        let available = f64::from(w) * 0.94;
        if em > 0.0 && f64::from(size) * em > available {
            size = ((available / em).floor() as u32).max(MIN_FONT_PX);
        }
        let margin = (f64::from(h) * 0.03).round() as u32;
        use NamePosition::*;
        let x = match params.name_position {
            TopLeft | BottomLeft => margin.to_string(),
            Top | Center | Bottom => "(w-tw)/2".to_owned(),
            TopRight | BottomRight => format!("w-tw-{margin}"),
        };
        let y = match params.name_position {
            TopLeft | Top | TopRight => margin.to_string(),
            Center => "(h-th)/2".to_owned(),
            _ => format!("h-th-{margin}"),
        };
        let mut opts = Vec::new();
        if let Some(font) = font {
            opts.push(format!("fontfile={}", escape_filter_path(font)));
        }
        opts.push("expansion=none".to_owned());
        opts.push(format!("text={}", escape_value(&text)));
        opts.push(format!("fontsize={size}"));
        opts.push(format!("fontcolor={color}"));
        opts.push(format!("x={}", escape_value(&x)));
        opts.push(format!("y={}", escape_value(&y)));
        let border = (f64::from(size) * params.name_outline / 100.0).round() as u32;
        if border > 0 {
            opts.push(format!("borderw={border}:bordercolor={outline_color}"));
        }
        filters.push(format!("drawtext={}", opts.join(":")));
    }
    // FFmpeg 9.1 的 JPEG 编码器只接受全范围（full range）YUV
    if params.format == ShotFormat::Jpg {
        filters.push("format=yuvj420p".to_owned());
    }
    Ok(filters.join(","))
}

/// 一张截图的参数。`time` 为截图时间，提前半帧跳转以取最接近的一帧。
pub fn shot_args(
    params: &ScreenshotsParams,
    input: &str,
    time: f64,
    fps: f64,
    filter: &str,
    output: &str,
) -> Vec<String> {
    let half = if fps.is_finite() && fps > 0.0 {
        0.5 / fps
    } else {
        0.02
    };
    let seek = (time - half).max(0.0);
    let mut args = Vec::new();
    if seek > 0.0 {
        args.extend(["-ss".to_owned(), secs(seek)]);
    }
    args.extend([
        "-i".into(),
        input.into(),
        "-map".into(),
        "0:V:0".into(),
        "-frames:v".into(),
        "1".into(),
    ]);
    if !filter.is_empty() {
        args.extend(["-vf".into(), filter.into()]);
    }
    if params.format == ShotFormat::Jpg {
        args.extend(["-q:v".into(), params.jpeg_q().to_string()]);
    }
    args.extend([
        "-an".into(),
        "-sn".into(),
        "-dn".into(),
        "-update".into(),
        "1".into(),
        output.into(),
    ]);
    args
}

fn source_fps(info: &MediaInfo) -> f64 {
    info.video.as_ref().map_or(0.0, |v| v.fps)
}

impl Tool for ScreenshotsParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let info = ctx.info(0)?;
        let size = info
            .video
            .as_ref()
            .ok_or("文件中没有视频流")?
            .display_size();
        let fps = source_fps(&info);
        let plan = self.shot_times(info.duration, fps)?;
        if !plan.skipped.is_empty() {
            let list: Vec<String> = plan.skipped.iter().map(|t| fmt_num(*t)).collect();
            ctx.warn(format!(
                "以下时间超出了视频时长（{} 秒），已跳过：{}",
                fmt_num(info.duration),
                list.join("、")
            ));
        }
        let input_path = ctx.input_path(0)?.to_path_buf();
        let input = path_arg(&input_path)?;
        let name = input_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let font = if self.draw_name {
            match default_font() {
                Some(path) => Some(path.to_string_lossy().into_owned()),
                None => {
                    ctx.warn("没有找到支持中文的系统字体，改用 FFmpeg 默认字体，中文可能无法显示");
                    None
                }
            }
        } else {
            None
        };
        let filter = shot_filter(self, size, &name, font.as_deref())?;
        let total = plan.times.len();
        ctx.set_steps(&vec![1.0; total]);
        for (i, time) in plan.times.iter().enumerate() {
            ctx.check_cancelled()?;
            let spec = OutputSpec::file(self.format.ext())
                .suffix(shot_suffix(self.naming, i, total, *time))
                .in_stem_folder();
            let output = path_arg(&ctx.output(&spec)?)?;
            ctx.run_ffmpeg(
                "截图",
                shot_args(self, &input, *time, fps, &filter, &output),
                0.0,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(mode: ShotMode) -> ScreenshotsParams {
        ScreenshotsParams {
            mode,
            ..ScreenshotsParams::default()
        }
    }

    #[test]
    fn times_by_rule() {
        let p = ScreenshotsParams {
            interval_seconds: 3.0,
            ..with(ShotMode::Interval)
        };
        assert_eq!(
            p.shot_times(10.0, 25.0).unwrap().times,
            [0.0, 3.0, 6.0, 9.0]
        );
        assert_eq!(p.shot_times(9.0, 25.0).unwrap().times, [0.0, 3.0, 6.0]);
        let p = ScreenshotsParams {
            count: 4,
            ..with(ShotMode::Count)
        };
        assert_eq!(p.shot_times(8.0, 25.0).unwrap().times, [1.0, 3.0, 5.0, 7.0]);
        let p = ScreenshotsParams {
            position: 100.0,
            ..with(ShotMode::Single)
        };
        assert_eq!(
            p.shot_times(4.0, 25.0).unwrap().times,
            [3.96],
            "不晚于最后一帧"
        );
        let p = ScreenshotsParams {
            timestamps: "1.5， 0:02\n1:00 3".into(),
            ..with(ShotMode::Timestamps)
        };
        let t = p.shot_times(10.0, 25.0).unwrap();
        assert_eq!(t.times, [1.5, 2.0, 3.0]);
        assert_eq!(t.skipped, [60.0]);
        let late = ScreenshotsParams {
            timestamps: "20".into(),
            ..with(ShotMode::Timestamps)
        };
        assert!(late.shot_times(10.0, 25.0).is_err());
    }

    #[test]
    fn parse_times() {
        assert_eq!(parse_time("1:02:03.5").unwrap(), 3723.5);
        assert_eq!(parse_time(" 90 ").unwrap(), 90.0);
        for bad in ["", "a", "1:60", "1.5:00", "-1", "1:2:3:4", "1::2"] {
            assert!(parse_time(bad).is_err(), "{bad}");
        }
        assert!(parse_timestamps("1, x").is_err());
    }

    #[test]
    fn invalid_params() {
        for p in [
            ScreenshotsParams {
                interval_seconds: 0.0,
                ..with(ShotMode::Interval)
            },
            ScreenshotsParams {
                count: 0,
                ..with(ShotMode::Count)
            },
            ScreenshotsParams {
                position: 101.0,
                ..with(ShotMode::Single)
            },
            with(ShotMode::Timestamps),
            ScreenshotsParams {
                quality: 0,
                ..ScreenshotsParams::default()
            },
            ScreenshotsParams {
                width: 4,
                ..ScreenshotsParams::default()
            },
            ScreenshotsParams {
                draw_name: true,
                name_color: "white".into(),
                ..ScreenshotsParams::default()
            },
        ] {
            assert!(p.shot_times(10.0, 25.0).is_err(), "{p:?}");
        }
        let many = ScreenshotsParams {
            interval_seconds: 0.1,
            ..with(ShotMode::Interval)
        };
        assert!(many.shot_times(1000.0, 25.0).is_err());
        assert!(ScreenshotsParams::default().shot_times(0.0, 25.0).is_err());
    }

    #[test]
    fn names_and_quality() {
        assert_eq!(shot_suffix(ShotNaming::Index, 0, 9, 0.0), "_01");
        assert_eq!(shot_suffix(ShotNaming::Index, 4, 120, 0.0), "_005");
        assert_eq!(shot_suffix(ShotNaming::Time, 0, 1, 3725.5), "_01-02-05.500");
        let q = |quality| {
            ScreenshotsParams {
                quality,
                ..ScreenshotsParams::default()
            }
            .jpeg_q()
        };
        assert_eq!((q(100), q(90), q(1)), (2, 5, 31));
    }

    #[test]
    fn filter_and_args() {
        let plain = ScreenshotsParams::default();
        let filter = shot_filter(&plain, (1280, 720), "片名", None).unwrap();
        assert_eq!(filter, "format=yuvj420p");
        let args = shot_args(&plain, "in.mp4", 2.0, 25.0, &filter, "o.jpg").join(" ");
        assert_eq!(
            args,
            "-ss 1.98 -i in.mp4 -map 0:V:0 -frames:v 1 -vf format=yuvj420p -q:v 5 -an -sn -dn \
             -update 1 o.jpg"
        );
        let png = ScreenshotsParams {
            format: ShotFormat::Png,
            ..ScreenshotsParams::default()
        };
        assert_eq!(shot_filter(&png, (1280, 720), "片名", None).unwrap(), "");
        let small = ScreenshotsParams {
            width: 320,
            ..ScreenshotsParams::default()
        };
        assert_eq!(
            shot_filter(&small, (1281, 721), "片名", None).unwrap(),
            "scale=320:180,format=yuvj420p"
        );
        let first = shot_args(&plain, "in.mp4", 0.0, 25.0, "", "o.jpg").join(" ");
        assert!(first.starts_with("-i in.mp4"), "{first}");

        let named = ScreenshotsParams {
            width: 640,
            draw_name: true,
            format: ShotFormat::Png,
            name_position: NamePosition::TopRight,
            ..ScreenshotsParams::default()
        };
        let filter = shot_filter(&named, (1280, 720), "第1集: 开场", Some("C:/f.ttc")).unwrap();
        assert_eq!(
            filter,
            "scale=640:360,drawtext=fontfile=C\\\\:/f.ttc:expansion=none:text=第1集\\\\: 开场:\
             fontsize=18:fontcolor=0xffffff:x=w-tw-11:y=11:borderw=1:bordercolor=0x000000"
        );
        let args = shot_args(&named, "in.mp4", 2.0, 25.0, &filter, "o.png").join(" ");
        assert!(
            args.contains("-frames:v 1 -vf scale=640:360,drawtext="),
            "{args}"
        );
        assert!(!args.contains("-q:v"), "{args}");
    }

    #[test]
    fn long_names_shrink() {
        let p = ScreenshotsParams {
            draw_name: true,
            name_size: 20.0,
            ..ScreenshotsParams::default()
        };
        let filter = shot_filter(&p, (200, 400), "一二三四五六七八九十", None).unwrap();
        // 10 个汉字，可用宽度 188 → 字号 18
        assert!(filter.contains("fontsize=18:"), "{filter}");
    }
}
