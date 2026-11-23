//! `tool.progress-bar` 进度条：在画面顶部或底部画一条随播放推进的进度条；可按配置分成带标题的章节，
//! 可让一个图片 / 动图指示器跟着进度移动。一进一出，重编码，时长与音频不变（音频容器装得下时直接复制）。
//!
//! **外观**（尺寸都按画面高度的百分比，随分辨率缩放）：
//! - `position` 顶部 / 底部，`margin` 与画面边缘的距离（0～45%），`height` 条的粗细（0.2～20%，至少 2 像素）；
//! - `barColor` / `barOpacity` 已播放部分，`backgroundColor` / `backgroundOpacity` 未播放部分（不透明度 0～100%）；
//! - `rounded` 圆角：整条进度条两端为半圆（已播放部分的前端仍是直边）。
//!
//! **章节**（`chapters`，可选）：每行一个章节 `时间,标题`，时间可写秒数、`分:秒` 或百分比，逗号可用中文逗号或
//! 制表符；空行与 `#` 开头的行忽略。例如：
//!
//! ```text
//! 0,开场
//! 1:30,第一部分
//! 75%,总结
//! ```
//!
//! 每个章节从它的时间开始、到下一个章节（或结尾）为止：章节分界处画一条 2 像素的分隔线（背景色、不透明），
//! 标题居中写在该段上方（底部进度条）或下方（顶部进度条），字号 `labelSize`（画面高度的 1～20%），
//! 颜色 `labelColor`，带黑色描边。字体自动选择支持中文的系统字体（与 `pipe.text` 相同，见
//! `pipeline::modules::text::default_font`）；文字转义也复用 `pipe.text` 的公开函数。
//!
//! **指示器**（`showIndicator`，可选）：图片或 GIF 动图（gif 循环播放），高度为画面高度的 `indicatorSize`%
//! （1～50%，等比缩放），中心跟随进度条前端、垂直居中于进度条。
//!
//! 实现：未播放部分用 `drawbox`；已播放部分是一条整宽色条（`color` 源），按 `x = W·t/时长 − w` 逐帧叠加；
//! 圆角时改为一块 `geq` 逐像素计算的画布（按时间 T 决定已播放区域、按圆角矩形决定透明度）。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::model::AssetSource;
use crate::pipeline::escape_filter_path;
use crate::pipeline::modules::text::{default_font, escape_value, normalize_text};
use crate::tools::common::filter_job::{
    check_duration, color_hex, display_size, even_crop, float_text, run_job, source_audio,
    FilterJob, Rate,
};
use crate::tools::common::filter_job::{image_input_args, IMAGE_EXTENSIONS};
use crate::tools::common::positions::parse_time;
use crate::tools::{Tool, ToolContext, ToolError};

pub const ID: &str = "tool.progress-bar";

/// 章节数上限。
pub const MAX_CHAPTERS: usize = 100;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BarPosition {
    Top,
    #[default]
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProgressBarParams {
    pub position: BarPosition,
    /// 与画面边缘的距离（画面高度的百分比）
    pub margin: f64,
    /// 粗细（画面高度的百分比）
    pub height: f64,
    pub bar_color: String,
    /// 已播放部分的不透明度（0～100）
    pub bar_opacity: f64,
    pub background_color: String,
    /// 未播放部分的不透明度（0～100）
    pub background_opacity: f64,
    /// 两端圆角
    pub rounded: bool,
    /// 章节配置：每行 `时间,标题`
    pub chapters: String,
    /// 章节标题字号（画面高度的百分比）
    pub label_size: f64,
    pub label_color: String,
    /// 显示进度指示器
    pub show_indicator: bool,
    pub indicator: AssetSource,
    /// 指示器高度（画面高度的百分比）
    pub indicator_size: f64,
}

impl Default for ProgressBarParams {
    fn default() -> Self {
        Self {
            position: BarPosition::Bottom,
            margin: 0.0,
            height: 1.5,
            bar_color: "#f4b740".into(),
            bar_opacity: 100.0,
            background_color: "#000000".into(),
            background_opacity: 40.0,
            rounded: false,
            chapters: String::new(),
            label_size: 3.5,
            label_color: "#ffffff".into(),
            show_indicator: false,
            indicator: AssetSource::default(),
            indicator_size: 6.0,
        }
    }
}

/// 一个章节：开始时间与标题。
#[derive(Debug, Clone, PartialEq)]
pub struct Chapter {
    pub start: f64,
    pub label: String,
}

/// 解析章节配置（见模块说明）。返回按时间排序的章节与提示（超出时长而忽略的行）。
pub fn parse_chapters(text: &str, duration: f64) -> Result<(Vec<Chapter>, Vec<String>), String> {
    let mut chapters = Vec::new();
    let mut warnings = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (time, label) = line.split_once([',', '，', '\t']).unwrap_or((line, ""));
        let start =
            parse_time(time, duration).map_err(|e| format!("章节配置第 {} 行：{e}", i + 1))?;
        let label = normalize_text(label).replace('\n', " ");
        if label.chars().count() > 100 {
            return Err(format!(
                "章节配置第 {} 行：标题太长（最多 100 个字）",
                i + 1
            ));
        }
        if start >= duration {
            warnings.push(format!("章节配置第 {} 行超出视频时长，已忽略", i + 1));
            continue;
        }
        chapters.push(Chapter { start, label });
    }
    if chapters.len() > MAX_CHAPTERS {
        return Err(format!("章节太多（最多 {MAX_CHAPTERS} 个）"));
    }
    chapters.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok((chapters, warnings))
}

fn check_percent(value: f64, (min, max): (f64, f64), what: &str) -> Result<(), String> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!("{what}必须在 {min}～{max} 之间"))
    }
}

/// 颜色 + 不透明度 → `0xrrggbb@a`。
fn color_alpha(color: &str, opacity: f64, what: &str) -> Result<String, String> {
    Ok(format!(
        "{}@{}",
        color_hex(color, what)?,
        float_text((opacity / 100.0 * 1000.0).round() / 1000.0)
    ))
}

impl ProgressBarParams {
    fn validate(&self) -> Result<(), String> {
        check_percent(self.margin, (0.0, 45.0), "边距")?;
        check_percent(self.height, (0.2, 20.0), "进度条粗细")?;
        check_percent(self.bar_opacity, (0.0, 100.0), "进度条不透明度")?;
        check_percent(self.background_opacity, (0.0, 100.0), "背景不透明度")?;
        check_percent(self.label_size, (1.0, 20.0), "标题字号")?;
        if self.show_indicator {
            check_percent(self.indicator_size, (1.0, 50.0), "指示器大小")?;
        }
        Ok(())
    }
}

/// 进度条在画面中的几何位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarGeometry {
    pub width: u32,
    pub height: u32,
    /// 进度条顶边的 y
    pub y: u32,
    /// 进度条粗细
    pub thickness: u32,
}

pub fn geometry(params: &ProgressBarParams, (w, h): (u32, u32)) -> BarGeometry {
    let fh = f64::from(h);
    let thickness = ((fh * params.height / 100.0).round() as u32).clamp(2, h);
    let margin = ((fh * params.margin / 100.0).round() as u32).min(h - thickness);
    let y = match params.position {
        BarPosition::Top => margin,
        BarPosition::Bottom => h - thickness - margin,
    };
    BarGeometry {
        width: w,
        height: h,
        y,
        thickness,
    }
}

/// 圆角进度条的 `geq` 画布表达式（r、g、b、a）。
fn rounded_geq(
    g: &BarGeometry,
    duration: f64,
    bar: (u8, u8, u8, f64),
    back: (u8, u8, u8, f64),
) -> String {
    let (w, t) = (g.width, g.thickness);
    let radius = float_text(f64::from(t) / 2.0);
    let filled = format!("lt(X,{w}*T/{})", float_text(duration));
    let inside = format!(
        "lte(pow(max(0,max({radius}-X-0.5,X+0.5-({w}-{radius}))),2)+pow(max(0,max({radius}-Y-0.5,Y+0.5-({t}-{radius}))),2),{radius}*{radius})"
    );
    let pick = |a: u8, b: u8| format!("if({filled},{a},{b})");
    let alpha = format!(
        "{inside}*if({filled},{},{})",
        (bar.3 * 255.0).round(),
        (back.3 * 255.0).round()
    );
    format!(
        "geq=r={}:g={}:b={}:a={}",
        escape_value(&pick(bar.0, back.0)),
        escape_value(&pick(bar.1, back.1)),
        escape_value(&pick(bar.2, back.2)),
        escape_value(&alpha)
    )
}

fn rgb(color: &str, what: &str) -> Result<(u8, u8, u8), String> {
    let hex = color_hex(color, what)?;
    let v = u32::from_str_radix(&hex[2..], 16).map_err(|_| format!("{what}不是有效的颜色"))?;
    Ok(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// 生成命令；`indicator` 为已选定的指示器图片，`font` 为章节标题用的字体文件。
pub fn build(
    params: &ProgressBarParams,
    info: &MediaInfo,
    input: &str,
    indicator: Option<&str>,
    font: Option<&str>,
) -> Result<FilterJob, String> {
    params.validate()?;
    check_duration(info.duration)?;
    let (sw, sh) = display_size(info)?;
    let rate = Rate::of(info)?;
    let (w, h) = (sw & !1, sh & !1);
    let g = geometry(params, (w, h));
    let d = float_text(info.duration);
    let (chapters, warnings) = parse_chapters(&params.chapters, info.duration)?;
    let mut job = FilterJob::new(input);
    job.warnings = warnings;
    job.audio = source_audio(info);
    job.duration = info.duration;

    let mut parts = Vec::new();
    let mut chain = format!("[0:v:0]null{}", even_crop(sw, sh));
    let bar_color = color_alpha(&params.bar_color, params.bar_opacity, "进度条颜色")?;
    let back_color = color_alpha(
        &params.background_color,
        params.background_opacity,
        "背景颜色",
    )?;
    if params.rounded {
        let (br, bg, bb) = rgb(&params.bar_color, "进度条颜色")?;
        let (kr, kg, kb) = rgb(&params.background_color, "背景颜色")?;
        parts.push(format!(
            "color=c=black@0:s={w}x{}:r={},format=rgba,{}[bar]",
            g.thickness,
            rate.text,
            rounded_geq(
                &g,
                info.duration,
                (br, bg, bb, params.bar_opacity / 100.0),
                (kr, kg, kb, params.background_opacity / 100.0)
            )
        ));
        parts.push(format!("{chain}[base]"));
        chain = format!("[base][bar]overlay=x=0:y={}:shortest=1", g.y);
    } else {
        if params.background_opacity > 0.0 {
            chain.push_str(&format!(
                ",drawbox=x=0:y={}:w={w}:h={}:color={back_color}:t=fill",
                g.y, g.thickness
            ));
        }
        parts.push(format!(
            "color=c={bar_color}:s={w}x{}:r={},format=rgba[fill]",
            g.thickness, rate.text
        ));
        parts.push(format!("{chain}[base]"));
        chain = format!("[base][fill]overlay=x=W*t/{d}-w:y={}:shortest=1", g.y);
    }

    // 章节：分隔线与标题
    let separator = color_hex(&params.background_color, "背景颜色")?;
    let label_color = color_hex(&params.label_color, "标题颜色")?;
    let font_size = ((f64::from(h) * params.label_size / 100.0).round() as u32).max(10);
    let border = (font_size / 12).max(1);
    let gap = (g.thickness / 2).max(2);
    for (i, chapter) in chapters.iter().enumerate() {
        let x = (f64::from(w) * chapter.start / info.duration).round() as u32;
        if x > 0 {
            chain.push_str(&format!(
                ",drawbox=x={}:y={}:w=2:h={}:color={separator}@1:t=fill",
                x.saturating_sub(1),
                g.y,
                g.thickness
            ));
        }
        if chapter.label.is_empty() {
            continue;
        }
        let end = chapters.get(i + 1).map_or(info.duration, |next| next.start);
        let centre = (f64::from(w) * (chapter.start + end) / 2.0 / info.duration).round();
        let x = format!("max(0,min(w-tw,{}-tw/2))", float_text(centre));
        let y = match params.position {
            BarPosition::Bottom => format!("{}-th", i64::from(g.y) - i64::from(gap)),
            BarPosition::Top => (g.y + g.thickness + gap).to_string(),
        };
        let mut opts = Vec::new();
        if let Some(font) = font {
            opts.push(format!("fontfile={}", escape_filter_path(font)));
        }
        opts.push("expansion=none".into());
        opts.push(format!("text={}", escape_value(&chapter.label)));
        opts.push(format!("fontsize={font_size}"));
        opts.push(format!("fontcolor={label_color}"));
        opts.push(format!("borderw={border}:bordercolor=0x000000@0.7"));
        opts.push(format!("x={}", escape_value(&x)));
        opts.push(format!("y={}", escape_value(&y)));
        chain.push_str(&format!(",drawtext={}", opts.join(":")));
    }

    if let Some(path) = indicator.filter(|_| params.show_indicator) {
        let index = job.add_input(image_input_args(path), path);
        let size = (((f64::from(h) * params.indicator_size / 100.0).round() as u32) & !1).max(4);
        let centre = g.y * 2 + g.thickness; // 进度条中线 × 2，避免小数
        parts.push(format!(
            "[{index}:v:0]scale=-2:{size},format=rgba[indicator]"
        ));
        parts.push(format!("{chain}[marked]"));
        chain = format!("[marked][indicator]overlay=x=W*t/{d}-w/2:y=({centre}-h)/2:shortest=1");
    }
    parts.push(format!("{chain}[v]"));
    job.graph = parts.join(";");
    Ok(job)
}

impl Tool for ProgressBarParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn supports_preview(&self) -> bool {
        true
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let info = ctx.info(0)?;
        let input = path_arg(ctx.input_path(0)?)?;
        let indicator = if self.show_indicator {
            if self.indicator.is_empty() {
                return Err("请选择进度指示器图片".into());
            }
            let index = ctx.number().saturating_sub(1);
            Some(path_arg(&self.indicator.pick(
                IMAGE_EXTENSIONS,
                index,
                &mut ctx.rng,
            )?)?)
        } else {
            None
        };
        let has_labels = parse_chapters(&self.chapters, info.duration)
            .map(|(c, _)| c.iter().any(|c| !c.label.is_empty()))
            .unwrap_or(false);
        let font = if has_labels {
            let font = default_font().map(|f| f.to_string_lossy().into_owned());
            if font.is_none() {
                ctx.warn("找不到支持中文的系统字体，章节标题可能无法正常显示");
            }
            font
        } else {
            None
        };
        let job = build(self, &info, &input, indicator.as_deref(), font.as_deref())?;
        run_job(ctx, "进度条", &job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::sample_info;

    #[test]
    fn chapter_config() {
        let text = "# 注释\n\n1:00,第二章\n0，开场\n50%\t中间, 含逗号\n20,超出\n5\n";
        let (chapters, warnings) = parse_chapters(text, 10.0).unwrap();
        assert_eq!(
            chapters,
            [
                Chapter {
                    start: 0.0,
                    label: "开场".into()
                },
                Chapter {
                    start: 5.0,
                    label: "中间, 含逗号".into()
                },
                Chapter {
                    start: 5.0,
                    label: String::new()
                },
            ]
        );
        assert_eq!(
            warnings,
            [
                "章节配置第 3 行超出视频时长，已忽略",
                "章节配置第 6 行超出视频时长，已忽略"
            ]
        );
        assert!(parse_chapters("abc,x", 10.0)
            .unwrap_err()
            .contains("第 1 行"));
        assert_eq!(parse_chapters("", 10.0).unwrap().0, []);
    }

    #[test]
    fn geometry_follows_position_and_margin() {
        let p = ProgressBarParams::default();
        assert_eq!(
            geometry(&p, (1280, 720)),
            BarGeometry {
                width: 1280,
                height: 720,
                y: 709,
                thickness: 11
            }
        );
        let top = ProgressBarParams {
            position: BarPosition::Top,
            margin: 5.0,
            height: 0.2,
            ..p
        };
        assert_eq!(geometry(&top, (1280, 720)).y, 36);
        assert_eq!(geometry(&top, (1280, 720)).thickness, 2);
    }

    #[test]
    fn plain_bar_command() {
        let job = build(
            &ProgressBarParams::default(),
            &sample_info(),
            "in.mp4",
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            job.graph,
            "color=c=0xf4b740@1:s=1280x11:r=25,format=rgba[fill];\
             [0:v:0]null,drawbox=x=0:y=709:w=1280:h=11:color=0x000000@0.4:t=fill[base];\
             [base][fill]overlay=x=W*t/10-w:y=709:shortest=1[v]"
        );
        assert_eq!(job.duration, 10.0);
    }

    #[test]
    fn chapters_labels_and_indicator() {
        let p = ProgressBarParams {
            chapters: "0,开场\n50%,第二章".into(),
            background_opacity: 0.0,
            show_indicator: true,
            ..ProgressBarParams::default()
        };
        let job = build(
            &p,
            &sample_info(),
            "in.mp4",
            Some("dot.gif"),
            Some("C:/f/msyh.ttc"),
        )
        .unwrap();
        assert_eq!(
            job.inputs,
            ["-i", "in.mp4", "-stream_loop", "-1", "-i", "dot.gif"]
        );
        let text = |label: &str, centre: u32| {
            format!(
                "drawtext=fontfile=C\\\\:/f/msyh.ttc:expansion=none:text={label}:fontsize=25:fontcolor=0xffffff:\
                 borderw=2:bordercolor=0x000000@0.7:x=max(0\\,min(w-tw\\,{centre}-tw/2)):y=704-th"
            )
        };
        assert_eq!(
            job.graph,
            format!(
                "color=c=0xf4b740@1:s=1280x11:r=25,format=rgba[fill];\
                 [0:v:0]null[base];\
                 [1:v:0]scale=-2:42,format=rgba[indicator];\
                 [base][fill]overlay=x=W*t/10-w:y=709:shortest=1,{},\
                 drawbox=x=639:y=709:w=2:h=11:color=0x000000@1:t=fill,{}[marked];\
                 [marked][indicator]overlay=x=W*t/10-w/2:y=(1429-h)/2:shortest=1[v]",
                text("开场", 320),
                text("第二章", 960)
            )
        );
    }

    #[test]
    fn rounded_bar_uses_geq_canvas() {
        let p = ProgressBarParams {
            rounded: true,
            position: BarPosition::Top,
            bar_color: "#ff0000".into(),
            background_color: "#0000ff".into(),
            background_opacity: 50.0,
            height: 2.0,
            ..ProgressBarParams::default()
        };
        let job = build(&p, &sample_info(), "in.mp4", None, None).unwrap();
        let filled = "if(lt(X\\,1280*T/10)";
        assert_eq!(
            job.graph,
            format!(
                "color=c=black@0:s=1280x14:r=25,format=rgba,geq=r={filled}\\,255\\,0):g={filled}\\,0\\,0):\
                 b={filled}\\,0\\,255):a=lte(pow(max(0\\,max(7-X-0.5\\,X+0.5-(1280-7)))\\,2)+\
                 pow(max(0\\,max(7-Y-0.5\\,Y+0.5-(14-7)))\\,2)\\,7*7)*{filled}\\,255\\,128)[bar];\
                 [0:v:0]null[base];[base][bar]overlay=x=0:y=0:shortest=1[v]"
            )
        );
    }

    #[test]
    fn invalid_params() {
        for p in [
            ProgressBarParams {
                height: 0.0,
                ..ProgressBarParams::default()
            },
            ProgressBarParams {
                bar_color: "orange".into(),
                ..ProgressBarParams::default()
            },
            ProgressBarParams {
                bar_opacity: 120.0,
                ..ProgressBarParams::default()
            },
            ProgressBarParams {
                chapters: "x,y".into(),
                ..ProgressBarParams::default()
            },
            ProgressBarParams {
                show_indicator: true,
                indicator_size: 0.0,
                ..ProgressBarParams::default()
            },
        ] {
            assert!(
                build(&p, &sample_info(), "in.mp4", None, None).is_err(),
                "{p:?}"
            );
        }
    }
}
