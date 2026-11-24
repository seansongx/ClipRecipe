//! `pipe.text` 文本：最多 3 处文字，每处单独开关，各用一个 `drawtext` 绘制。
//!
//! 参数与取舍（FEATURES.md 未规定的部分）：
//! - **内容来源** `source`：
//!   - `fileName` 视频文件名（不含扩展名）：先删去 `strip` 中的每个子串（全部出现处），
//!     再在 `lineBreaks` 里的任一字符处分行（分隔符本身去掉），各行去掉首尾空白、丢弃空行；
//!   - `textFiles` 文本文件（`.txt`，单文件或目录随机 / 依次取，每个视频一个）：UTF-8（可带 BOM）或带 BOM 的 UTF-16，
//!     其它编码报错并提示另存为 UTF-8；文件不超过 1 MB；
//!   - `custom` 自定义多行文本。
//! - **文字整理**：三种来源都会统一换行符、制表符换成空格、去掉其它控制字符、去掉每行行尾空白与整段首尾空白；
//!   `maxLineChars`（0 = 不限）把超长的行按字符数硬折行（不按单词断开）。
//!   结果为空时跳过这一处并给出警告（自定义文本为空视为参数错误）。
//! - **字体** `fontFile`：字体文件路径（前端从 `list_system_fonts` 列出的系统字体中选，或指定任意字体文件）；
//!   为空时自动选择支持中文的系统字体（见 [`default_font_candidates`]，Windows 优先微软雅黑）。
//!   指定的文件不存在时退回默认字体并警告；找不到任何默认字体时不指定字体（由 FFmpeg 的 fontconfig 决定）并警告。
//!   `.ttc` 字体集合只使用其中第一个字体。
//! - **大小**：`fontSize` 为画面高度的百分比（1～30），随分辨率缩放；`fitWidth`（默认开）时若估算的行宽超出
//!   可用宽度（画面宽减去左右边距，至少一半），按估算把字号缩小到能放下（中日韩等全角字符按 1 个字号宽、
//!   其它按 0.6 个字号宽估算，是近似值）。
//! - **样式**：颜色 `color`；描边 `outlineWidth`（字号的百分比，0～25，0 为无描边）+ `outlineColor`；
//!   阴影 `shadow` + `shadowColor`（60% 不透明，偏移为字号的 6%，至少 1 像素）；
//!   背景框 `background` + `backgroundColor` + `backgroundOpacity`（0～1，内边距为字号的 30%）；
//!   行距 `lineSpacing`（字号的百分比，0～200）；多行对齐 `align`（左 / 中 / 右，`text_align`）。
//! - **位置** `position`：九宫格锚点，`marginX` / `marginY` 为距边缘的画面宽 / 高百分比（0～45，居中的方向上不使用）；
//!   或 `custom`：`point` 为文字块中心点，坐标是**源视频像素**（PointPicker 选取），按“源尺寸 → 当前画面”比例换算，
//!   并限制文字块不超出画面。本模块排在裁剪、分辨率等之后，坐标与字号都基于当前（最终）画面。
//! - **时间**：`start`（秒，延时出现）与 `duration`（秒，0 = 一直到结尾），用 `enable` 实现。
//!   时间是本模块所在位置的时间轴：去头尾 / 变速之后的视频时间，不含之后拼接的片头。
//!
//! 文本写进滤镜的方式（`expansion=none`，`%` 不做展开）：
//! - 一般长度：`text=` 直接写进滤镜，依次做“选项值”（`\ ' :`）和“滤镜图”（`\ ' [ ] , ;`）两级反斜杠转义，
//!   换行符原样保留（drawtext 按换行分行）。集成测试逐像素验证它与 `textfile=` 的渲染结果一致。
//!   这样构建过程不写任何文件，“查看命令”给出的命令行也能直接复制运行；
//! - 转义后超过 [`MAX_INLINE_TEXT`] 字节（长文本文件）：写进本次构建的临时文件（`ctx.temp_file`，
//!   任务 / 预览结束时自动删除），用 `textfile=`——避免整条命令超出 Windows 约 32K 字符的命令行上限。
//!
//! 不改变尺寸、帧率与时长。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::AssetSource;
use crate::pipeline::{escape_filter_path, fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.text";

/// 最多几处文本。
pub const MAX_ITEMS: usize = 3;
/// 文本文件的扩展名（目录模式下的过滤条件）。
pub const TEXT_EXTENSIONS: [&str; 1] = ["txt"];
/// 文本文件大小上限（字节）。
pub const MAX_TEXT_FILE_BYTES: u64 = 1024 * 1024;
/// 时间参数允许的最大值（秒）。
pub const MAX_SECONDS: f64 = 86_400.0;
/// 自动缩小后的最小字号（像素）。
const MIN_FONT_PX: u32 = 8;
/// 转义后的文字超过这么多字节时改用 `textfile=` 临时文件（三处都写满也远低于 Windows 命令行上限）。
pub const MAX_INLINE_TEXT: usize = 4096;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextSource {
    /// 视频文件名
    #[default]
    FileName,
    /// 文本文件（单文件或目录）
    TextFiles,
    /// 自定义文本
    Custom,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextPosition {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    #[default]
    Bottom,
    BottomRight,
    /// 自选位置（`point` 为文字块中心）
    Custom,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// 画面上的一点（源像素）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextPoint {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextItem {
    pub enabled: bool,
    pub source: TextSource,
    /// 文件名：要删去的子串
    pub strip: Vec<String>,
    /// 文件名：在这些字符处分行
    pub line_breaks: String,
    /// 文本文件：单文件或目录
    pub text_files: AssetSource,
    /// 自定义文本（可多行）
    pub custom_text: String,
    /// 每行最多字符数，0 = 不限
    pub max_line_chars: u32,
    /// 字体文件；空 = 自动选择中文字体
    pub font_file: String,
    /// 字号：画面高度的百分比
    pub font_size: f64,
    /// 超出宽度时自动缩小
    pub fit_width: bool,
    pub color: String,
    /// 描边宽度：字号的百分比
    pub outline_width: f64,
    pub outline_color: String,
    pub shadow: bool,
    pub shadow_color: String,
    pub background: bool,
    pub background_color: String,
    pub background_opacity: f64,
    /// 行距：字号的百分比
    pub line_spacing: f64,
    pub align: TextAlign,
    pub position: TextPosition,
    /// 左右边距：画面宽的百分比
    pub margin_x: f64,
    /// 上下边距：画面高的百分比
    pub margin_y: f64,
    /// 自选位置（源像素）
    pub point: Option<TextPoint>,
    /// 出现时间（秒）
    pub start: f64,
    /// 持续时间（秒），0 = 到结尾
    pub duration: f64,
}

impl Default for TextItem {
    fn default() -> Self {
        Self {
            enabled: false,
            source: TextSource::FileName,
            strip: Vec::new(),
            line_breaks: String::new(),
            text_files: AssetSource::default(),
            custom_text: String::new(),
            max_line_chars: 0,
            font_file: String::new(),
            font_size: 5.0,
            fit_width: true,
            color: "#ffffff".into(),
            outline_width: 8.0,
            outline_color: "#000000".into(),
            shadow: false,
            shadow_color: "#000000".into(),
            background: false,
            background_color: "#000000".into(),
            background_opacity: 0.5,
            line_spacing: 20.0,
            align: TextAlign::Center,
            position: TextPosition::Bottom,
            margin_x: 5.0,
            margin_y: 8.0,
            point: None,
            start: 0.0,
            duration: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextParams {
    pub enabled: bool,
    /// 最多 3 处
    pub items: Vec<TextItem>,
}

impl Default for TextParams {
    fn default() -> Self {
        Self {
            enabled: false,
            items: vec![
                TextItem {
                    enabled: true,
                    ..TextItem::default()
                },
                TextItem {
                    source: TextSource::Custom,
                    position: TextPosition::Top,
                    margin_y: 6.0,
                    ..TextItem::default()
                },
                TextItem {
                    source: TextSource::Custom,
                    position: TextPosition::Center,
                    ..TextItem::default()
                },
            ],
        }
    }
}

/// 把任意文本写成滤镜选项值：先做选项值级（`\ ' :`）再做滤镜图级（`\ ' [ ] , ;`）反斜杠转义。
pub fn escape_value(text: &str) -> String {
    let mut option = String::with_capacity(text.len() * 2);
    for ch in text.chars() {
        if matches!(ch, '\\' | '\'' | ':') {
            option.push('\\');
        }
        option.push(ch);
    }
    let mut graph = String::with_capacity(option.len() * 2);
    for ch in option.chars() {
        if matches!(ch, '\\' | '\'' | '[' | ']' | ',' | ';') {
            graph.push('\\');
        }
        graph.push(ch);
    }
    graph
}

/// 统一换行、去掉控制字符与多余空白（见模块说明）。
pub fn normalize_text(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let cleaned: String = text
        .chars()
        .filter_map(|c| match c {
            '\n' => Some('\n'),
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    let lines: Vec<&str> = cleaned.lines().map(str::trim_end).collect();
    lines.join("\n").trim().to_owned()
}

/// 把超过 `max` 个字符的行硬折行；`max` 为 0 时不处理。
pub fn wrap_lines(text: &str, max: u32) -> String {
    if max == 0 {
        return text.to_owned();
    }
    let max = max as usize;
    text.split('\n')
        .flat_map(|line| {
            let chars: Vec<char> = line.chars().collect();
            if chars.is_empty() {
                return vec![String::new()];
            }
            chars
                .chunks(max)
                .map(|c| c.iter().collect::<String>().trim().to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 由文件名得到文本：删子串、按分隔符分行。
pub fn file_name_text(stem: &str, strip: &[String], line_breaks: &str) -> String {
    let mut s = stem.to_owned();
    for pattern in strip.iter().filter(|p| !p.is_empty()) {
        s = s.replace(pattern.as_str(), "");
    }
    s.split(|c: char| line_breaks.contains(c))
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 读取文本文件：UTF-8（可带 BOM）或带 BOM 的 UTF-16。
pub fn read_text_file(path: &Path) -> Result<String, String> {
    let shown = path.display();
    let meta = std::fs::metadata(path).map_err(|e| format!("无法读取文本文件 {shown}：{e}"))?;
    if meta.len() > MAX_TEXT_FILE_BYTES {
        return Err(format!("文本文件太大（超过 1 MB）：{shown}"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("无法读取文本文件 {shown}：{e}"))?;
    let utf16 = |data: &[u8], le: bool| -> Result<String, String> {
        let units: Vec<u16> = data
            .chunks_exact(2)
            .map(|p| {
                if le {
                    u16::from_le_bytes([p[0], p[1]])
                } else {
                    u16::from_be_bytes([p[0], p[1]])
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|_| format!("文本文件编码无法识别：{shown}"))
    };
    match bytes.as_slice() {
        [0xEF, 0xBB, 0xBF, rest @ ..] => {
            String::from_utf8(rest.to_vec()).map_err(|_| format!("文本文件编码无法识别：{shown}"))
        }
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        _ => String::from_utf8(bytes).map_err(|_| {
            format!("文本文件不是 UTF-8 编码：{shown}（请用记事本“另存为”UTF-8 后再试）")
        }),
    }
}

/// 自动选择字体时依次尝试的字体文件（都支持中文）。
pub fn default_font_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        let root = std::env::var_os("WINDIR")
            .or_else(|| std::env::var_os("SystemRoot"))
            .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        let fonts = root.join("Fonts");
        [
            "msyh.ttc",
            "msyh.ttf",
            "Deng.ttf",
            "simhei.ttf",
            "simsun.ttc",
        ]
        .iter()
        .map(|f| fonts.join(f))
        .collect()
    } else if cfg!(target_os = "macos") {
        [
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Medium.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    } else {
        [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
            "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }
}

/// 第一个存在的默认字体。
pub fn default_font() -> Option<PathBuf> {
    default_font_candidates().into_iter().find(|p| p.is_file())
}

/// 字符的估算宽度（以字号为单位）。
fn char_em(c: char) -> f64 {
    let wide = matches!(c as u32,
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFD);
    if wide {
        1.0
    } else {
        0.6
    }
}

/// 最长一行的估算宽度（以字号为单位）。
pub fn widest_line_em(text: &str) -> f64 {
    text.split('\n')
        .map(|l| l.chars().map(char_em).sum::<f64>())
        .fold(0.0, f64::max)
}

/// 解析 `#rrggbb`，返回 `0xrrggbb`。
fn color_arg(text: &str, what: &str) -> Result<String, String> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(format!("0x{}", hex.to_ascii_lowercase()))
    } else {
        Err(format!("{what}格式不正确：{text}"))
    }
}

fn check_range(value: f64, (min, max): (f64, f64), what: &str) -> Result<(), String> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{what}需在 {}～{} 之间",
            fmt_num(min),
            fmt_num(max)
        ))
    }
}

fn px(value: f64) -> u32 {
    value.round().max(0.0) as u32
}

impl TextItem {
    fn validate(&self, label: &str) -> Result<(), String> {
        let check = |v, r, what: &str| check_range(v, r, &format!("{label}的{what}"));
        check(self.font_size, (1.0, 30.0), "字号（画面高度的百分比）")?;
        check(self.outline_width, (0.0, 25.0), "描边宽度（字号的百分比）")?;
        check(self.line_spacing, (0.0, 200.0), "行距（字号的百分比）")?;
        check(self.margin_x, (0.0, 45.0), "左右边距（%）")?;
        check(self.margin_y, (0.0, 45.0), "上下边距（%）")?;
        check(self.background_opacity, (0.0, 1.0), "背景不透明度")?;
        check(self.start, (0.0, MAX_SECONDS), "出现时间（秒）")?;
        check(self.duration, (0.0, MAX_SECONDS), "持续时间（秒）")?;
        if self.max_line_chars > 1000 {
            return Err(format!("{label}的每行字数需在 0～1000 之间"));
        }
        if self.position == TextPosition::Custom && self.point.is_none() {
            return Err(format!("{label}：请在画面上选择文字位置"));
        }
        match self.source {
            TextSource::TextFiles if self.text_files.is_empty() => {
                return Err(format!("{label}：请选择文本文件或目录"));
            }
            TextSource::Custom if normalize_text(&self.custom_text).is_empty() => {
                return Err(format!("{label}：自定义文本为空"));
            }
            _ => {}
        }
        Ok(())
    }

    /// 取得这一处要显示的文字（已规范化、折行）。
    fn content(&self, ctx: &mut BuildContext) -> Result<String, String> {
        let raw = match self.source {
            TextSource::FileName => {
                let stem = Path::new(&ctx.info.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                file_name_text(&stem, &self.strip, &self.line_breaks)
            }
            TextSource::TextFiles => {
                let path = self
                    .text_files
                    .pick(&TEXT_EXTENSIONS, ctx.task_index, &mut ctx.rng)?;
                read_text_file(&path)?
            }
            TextSource::Custom => self.custom_text.clone(),
        };
        Ok(wrap_lines(&normalize_text(&raw), self.max_line_chars))
    }

    /// 字体选项（`fontfile=…`）；找不到字体时为 None。
    fn font_option(&self, ctx: &mut BuildContext, label: &str) -> Option<String> {
        let chosen = self.font_file.trim();
        if !chosen.is_empty() {
            if Path::new(chosen).is_file() {
                return Some(format!("fontfile={}", escape_filter_path(chosen)));
            }
            ctx.warn(format!("{label}：字体文件不存在（{chosen}），改用默认字体"));
        }
        match default_font() {
            Some(path) => Some(format!(
                "fontfile={}",
                escape_filter_path(&path.to_string_lossy())
            )),
            None => {
                ctx.warn(format!(
                    "{label}：没有找到支持中文的系统字体，改用 FFmpeg 默认字体，中文可能无法显示"
                ));
                None
            }
        }
    }

    /// 文字选项：一般为 `text=转义后的文字`，太长时写进临时文件用 `textfile=`（见模块说明）。
    fn text_option(ctx: &mut BuildContext, text: &str, index: usize) -> Result<String, String> {
        let escaped = escape_value(text);
        if escaped.len() <= MAX_INLINE_TEXT {
            return Ok(format!("text={escaped}"));
        }
        let file = ctx.temp_file(&format!("text-{}.txt", index + 1))?;
        std::fs::write(&file, text).map_err(|e| format!("无法写入临时文本文件：{e}"))?;
        Ok(format!(
            "textfile={}",
            escape_filter_path(&file.to_string_lossy())
        ))
    }

    /// 生成第 `index` 处（从 0 起）的 drawtext 滤镜。
    fn filter(
        &self,
        ctx: &mut BuildContext,
        text: &str,
        index: usize,
        label: &str,
    ) -> Result<String, String> {
        let (w, h) = (ctx.stream.width, ctx.stream.height);
        let color = color_arg(&self.color, &format!("{label}的文字颜色"))?;
        let outline_color = color_arg(&self.outline_color, &format!("{label}的描边颜色"))?;
        let shadow_color = color_arg(&self.shadow_color, &format!("{label}的阴影颜色"))?;
        let box_color = color_arg(&self.background_color, &format!("{label}的背景颜色"))?;

        let mx = px(f64::from(w) * self.margin_x / 100.0);
        let my = px(f64::from(h) * self.margin_y / 100.0);
        let mut size = px(f64::from(h) * self.font_size / 100.0).max(MIN_FONT_PX);
        if self.fit_width {
            let available = if self.position == TextPosition::Custom {
                f64::from(w) * 0.92
            } else {
                f64::from(w.saturating_sub(2 * mx)).max(f64::from(w) / 2.0)
            };
            let em = widest_line_em(text);
            if em > 0.0 && f64::from(size) * em > available {
                size = ((available / em).floor() as u32).max(MIN_FONT_PX);
            }
        }
        let fsize = f64::from(size);

        let (x, y) = match self.position {
            TextPosition::Custom => {
                let point = self.point.unwrap_or_default();
                let (sw, sh) = ctx.source_size();
                let cx = if sw > 0 {
                    px(f64::from(point.x) * f64::from(w) / f64::from(sw))
                } else {
                    point.x
                };
                let cy = if sh > 0 {
                    px(f64::from(point.y) * f64::from(h) / f64::from(sh))
                } else {
                    point.y
                };
                (
                    format!("max(min({cx}-tw/2,w-tw),0)"),
                    format!("max(min({cy}-th/2,h-th),0)"),
                )
            }
            pos => {
                use TextPosition::*;
                let x = match pos {
                    TopLeft | Left | BottomLeft => mx.to_string(),
                    Top | Center | Bottom => "(w-tw)/2".to_owned(),
                    _ => format!("w-tw-{mx}"),
                };
                let y = match pos {
                    TopLeft | Top | TopRight => my.to_string(),
                    Left | Center | Right => "(h-th)/2".to_owned(),
                    _ => format!("h-th-{my}"),
                };
                (x, y)
            }
        };

        let mut opts: Vec<String> = Vec::new();
        if let Some(font) = self.font_option(ctx, label) {
            opts.push(font);
        }
        opts.push("expansion=none".into());
        opts.push(Self::text_option(ctx, text, index)?);
        opts.push(format!("fontsize={size}"));
        opts.push(format!("fontcolor={color}"));
        opts.push(format!(
            "line_spacing={}",
            px(fsize * self.line_spacing / 100.0)
        ));
        opts.push(format!(
            "text_align={}",
            match self.align {
                TextAlign::Left => "L",
                TextAlign::Center => "C",
                TextAlign::Right => "R",
            }
        ));
        opts.push(format!("x={}", escape_value(&x)));
        opts.push(format!("y={}", escape_value(&y)));
        let border = px(fsize * self.outline_width / 100.0);
        if border > 0 {
            opts.push(format!("borderw={border}:bordercolor={outline_color}"));
        }
        if self.shadow {
            let offset = px(fsize * 0.06).max(1);
            opts.push(format!(
                "shadowx={offset}:shadowy={offset}:shadowcolor={shadow_color}@0.6"
            ));
        }
        if self.background {
            opts.push(format!(
                "box=1:boxcolor={box_color}@{}:boxborderw={}",
                fmt_num(self.background_opacity),
                px(fsize * 0.3).max(1)
            ));
        }
        let (start, duration) = (self.start, self.duration);
        let enable = match (start > 0.0, duration > 0.0) {
            (_, true) => Some(format!(
                "between(t,{},{})",
                fmt_num(start),
                fmt_num(start + duration)
            )),
            (true, false) => Some(format!("gte(t,{})", fmt_num(start))),
            (false, false) => None,
        };
        if let Some(enable) = enable {
            opts.push(format!("enable={}", escape_value(&enable)));
        }
        if start > 0.0 && ctx.stream.duration > 0.0 && start >= ctx.stream.duration {
            ctx.warn(format!("{label}：出现时间超过视频时长，文字不会出现"));
        }
        Ok(format!("drawtext={}", opts.join(":")))
    }
}

impl PipelineModule for TextParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        if self.items.len() > MAX_ITEMS {
            return Err(format!("文本最多 {MAX_ITEMS} 处"));
        }
        let active: Vec<(usize, &TextItem)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.enabled)
            .collect();
        if active.is_empty() {
            ctx.warn("没有启用任何一处文本");
            return Ok(());
        }
        for (i, item) in &active {
            item.validate(&format!("文本 {}", i + 1))?;
        }
        if ctx.stream.width == 0 || ctx.stream.height == 0 {
            return Err("无法获取视频画面尺寸".into());
        }
        for (i, item) in active {
            let label = format!("文本 {}", i + 1);
            let text = item.content(ctx)?;
            if text.is_empty() {
                ctx.warn(format!("{label}：内容为空，已跳过"));
                continue;
            }
            let filter = item.filter(ctx, &text, i, &label)?;
            ctx.graph.video(filter);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PickOrder, Recipe, Settings};
    use crate::pipeline::testing::{build_with, sample_info};

    /// 单元测试用的“字体文件”：只检查存在、不会被 ffmpeg 读取，借用测试程序自身，免得留下临时文件。
    fn fake_font() -> PathBuf {
        std::env::current_exe().unwrap()
    }

    fn item(text: &str) -> TextItem {
        TextItem {
            enabled: true,
            source: TextSource::Custom,
            custom_text: text.into(),
            font_file: fake_font().to_string_lossy().into_owned(),
            ..TextItem::default()
        }
    }

    fn recipe_with(items: Vec<TextItem>) -> Recipe {
        let mut recipe = Recipe::disabled();
        recipe.modules.text = TextParams {
            enabled: true,
            items,
        };
        recipe
    }

    fn filter(items: Vec<TextItem>) -> Result<String, String> {
        build_with(&sample_info(), &recipe_with(items), 42)
            .map(|b| b.arg_after("-filter_complex").unwrap_or("").to_owned())
    }

    fn font_opt() -> String {
        format!(
            "fontfile={}",
            escape_filter_path(&fake_font().to_string_lossy())
        )
    }

    #[test]
    fn escaping_covers_option_and_graph_levels() {
        assert_eq!(escape_value("abc 中文"), "abc 中文");
        assert_eq!(escape_value("a:b"), r"a\\:b");
        assert_eq!(escape_value("it's"), r"it\\\'s");
        assert_eq!(escape_value(r"C:\x"), r"C\\:\\\\x");
        assert_eq!(escape_value("a,b;c[d]e"), r"a\,b\;c\[d\]e");
        assert_eq!(escape_value("100%"), "100%");
        assert_eq!(escape_value("1\n2"), "1\n2");
    }

    #[test]
    fn default_bottom_centred_item() {
        let fc = filter(vec![item("Hello 世界")]).unwrap();
        // 720 高：字号 36、描边 3、行距 7；底部边距 58
        assert_eq!(
            fc,
            format!(
                "[0:v:0]drawtext={}:expansion=none:text=Hello 世界:fontsize=36:fontcolor=0xffffff:\
                 line_spacing=7:text_align=C:x=(w-tw)/2:y=h-th-58:borderw=3:bordercolor=0x000000[v0]",
                font_opt()
            )
        );
    }

    #[test]
    fn tricky_text_is_escaped() {
        let fc = filter(vec![item("a:b 'q' 100% C:\\d,[x];\n第二行")]).unwrap();
        let expected = r"text=a\\:b \\\'q\\\' 100% C\\:\\\\d\,\[x\]\;";
        assert!(
            fc.contains(&format!("{expected}\n第二行:fontsize=")),
            "{fc}"
        );
    }

    #[test]
    fn long_text_goes_through_a_temp_textfile_that_is_deleted_afterwards() {
        // 每行 40 个字，转义后（逗号 → `\,`）超过上限
        let line = "长文本,".repeat(10);
        let long = vec![line.as_str(); 200].join("\n");
        assert!(escape_value(&long).len() > MAX_INLINE_TEXT);
        let recipe = recipe_with(vec![
            item("短"),
            TextItem {
                fit_width: false,
                ..item(&long)
            },
        ]);
        let built = build_with(&sample_info(), &recipe, 1).unwrap();
        let fc = built.arg_after("-filter_complex").unwrap().to_owned();
        assert!(fc.contains(":text=短:"), "短文本仍直接写进滤镜：{fc}");
        let dir = match built.temp_files.paths() {
            [dir] => dir.clone(),
            other => panic!("应登记一个临时目录：{other:?}"),
        };
        let file = dir.join("text-2.txt");
        assert!(
            fc.contains(&format!(
                ":textfile={}:fontsize=",
                escape_filter_path(&file.to_string_lossy())
            )),
            "{fc}"
        );
        assert!(!fc.contains("长文本"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), long);
        assert!(fc.len() < 8000, "命令行保持很短：{}", fc.len());
        // 构建结果（及其所有克隆）丢弃后临时文件被删除
        let copy = built.clone();
        drop(built);
        assert!(file.is_file());
        drop(copy);
        assert!(!dir.exists());

        // 短文本不创建任何临时文件
        let built = build_with(&sample_info(), &recipe_with(vec![item("x")]), 1).unwrap();
        assert!(built.temp_files.is_empty());
    }

    #[test]
    fn styles_positions_and_timing() {
        let styled = TextItem {
            color: "#FF8000".into(),
            outline_width: 0.0,
            shadow: true,
            background: true,
            background_opacity: 0.4,
            align: TextAlign::Left,
            position: TextPosition::TopRight,
            margin_x: 2.5,
            margin_y: 5.0,
            font_size: 10.0,
            start: 1.5,
            duration: 2.0,
            ..item("x")
        };
        assert_eq!(
            filter(vec![styled.clone()]).unwrap(),
            format!(
                "[0:v:0]drawtext={}:expansion=none:text=x:fontsize=72:fontcolor=0xff8000:line_spacing=14:\
                 text_align=L:x=w-tw-32:y=36:shadowx=4:shadowy=4:shadowcolor=0x000000@0.6:\
                 box=1:boxcolor=0x000000@0.4:boxborderw=22:enable=between(t\\,1.5\\,3.5)[v0]",
                font_opt()
            )
        );
        let delayed = TextItem {
            duration: 0.0,
            position: TextPosition::Left,
            ..styled.clone()
        };
        let fc = filter(vec![delayed]).unwrap();
        assert!(fc.contains(":x=32:y=(h-th)/2:"), "{fc}");
        assert!(fc.ends_with(":enable=gte(t\\,1.5)[v0]"), "{fc}");

        let custom = TextItem {
            position: TextPosition::Custom,
            point: Some(TextPoint { x: 640, y: 100 }),
            start: 0.0,
            duration: 0.0,
            ..styled
        };
        let fc = filter(vec![custom]).unwrap();
        assert!(
            fc.contains(":x=max(min(640-tw/2\\,w-tw)\\,0):y=max(min(100-th/2\\,h-th)\\,0):"),
            "{fc}"
        );
        assert!(!fc.contains("enable"), "{fc}");
    }

    #[test]
    fn three_items_chain_and_disabled_ones_are_skipped() {
        let fc = filter(vec![
            item("一"),
            TextItem {
                enabled: false,
                ..item("二")
            },
            TextItem {
                position: TextPosition::Top,
                ..item("三")
            },
        ])
        .unwrap();
        assert_eq!(fc.matches("drawtext=").count(), 2, "{fc}");
        assert!(fc.contains("text=一:") && fc.contains("text=三:") && !fc.contains("二"));
        assert!(fc.contains(",drawtext="), "同一条链：{fc}");
    }

    #[test]
    fn file_name_source_strips_and_splits() {
        assert_eq!(
            file_name_text(
                "【合集】第一集 #上篇#下篇 高清",
                &["【合集】".into(), " 高清".into(), String::new()],
                "#"
            ),
            "第一集\n上篇\n下篇"
        );
        let mut info = sample_info();
        // 用本平台的路径分隔符拼：Linux / macOS 上反斜杠不是分隔符，写死 Windows 路径会让文件名取成整条路径
        info.path = Path::new("videos")
            .join("春天，在路上_part1.mp4")
            .to_string_lossy()
            .into_owned();
        let recipe = recipe_with(vec![TextItem {
            source: TextSource::FileName,
            strip: vec!["_part1".into()],
            line_breaks: "，".into(),
            ..item("")
        }]);
        let built = build_with(&info, &recipe, 1).unwrap();
        let fc = built.arg_after("-filter_complex").unwrap();
        assert!(fc.contains("text=春天\n在路上:"), "{fc}");
    }

    #[test]
    fn normalizing_and_wrapping() {
        assert_eq!(normalize_text("  \r\n a\tb  \r\nc\u{7}d \n\n"), "a b\ncd");
        assert_eq!(wrap_lines("一二三四五\nab", 2), "一二\n三四\n五\nab");
        assert_eq!(wrap_lines("abc", 0), "abc");
        let fc = filter(vec![TextItem {
            max_line_chars: 3,
            ..item("abcdefg")
        }])
        .unwrap();
        assert!(fc.contains("text=abc\ndef\ng:"), "{fc}");
    }

    #[test]
    fn long_lines_shrink_to_fit_width() {
        // 40 个汉字 × 36 像素 = 1440 > 1280 - 2×64 = 1152 → 字号 28
        let long = "字".repeat(40);
        let fc = filter(vec![item(&long)]).unwrap();
        assert!(fc.contains(":fontsize=28:"), "{fc}");
        let fc = filter(vec![TextItem {
            fit_width: false,
            ..item(&long)
        }])
        .unwrap();
        assert!(fc.contains(":fontsize=36:"), "{fc}");
        assert_eq!(widest_line_em("ab\n中文"), 2.0);
    }

    #[test]
    fn text_files_are_picked_and_decoded() {
        let dir = std::env::temp_dir().join(format!("cr-text-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1.txt"), "\u{feff}第一段\r\n第二行").unwrap();
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("宽字符".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        std::fs::write(dir.join("2.txt"), utf16).unwrap();
        std::fs::write(dir.join("skip.md"), "x").unwrap();
        let source = |order| TextItem {
            source: TextSource::TextFiles,
            text_files: AssetSource::Directory {
                path: dir.to_string_lossy().into_owned(),
                recursive: false,
                order,
            },
            ..item("")
        };
        let fc = filter(vec![source(PickOrder::Sequential)]).unwrap();
        assert!(fc.contains("text=第一段\n第二行:"), "{fc}");
        assert_eq!(read_text_file(&dir.join("2.txt")).unwrap(), "宽字符");
        // 随机取：同一种子结果相同，且一定是两个文件之一
        let a = filter(vec![source(PickOrder::Random)]).unwrap();
        assert_eq!(a, filter(vec![source(PickOrder::Random)]).unwrap());
        assert!(
            a.contains("text=第一段") || a.contains("text=宽字符"),
            "{a}"
        );

        std::fs::write(dir.join("gbk.txt"), [0xD6, 0xD0, 0xCE, 0xC4]).unwrap();
        let err = read_text_file(&dir.join("gbk.txt")).unwrap_err();
        assert!(err.contains("UTF-8"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn invalid_params_are_rejected() {
        let cases: Vec<(TextItem, &str)> = vec![
            (
                TextItem {
                    font_size: 0.5,
                    ..item("x")
                },
                "字号",
            ),
            (
                TextItem {
                    font_size: f64::NAN,
                    ..item("x")
                },
                "字号",
            ),
            (
                TextItem {
                    outline_width: 30.0,
                    ..item("x")
                },
                "描边",
            ),
            (
                TextItem {
                    margin_y: 50.0,
                    ..item("x")
                },
                "边距",
            ),
            (
                TextItem {
                    start: -1.0,
                    ..item("x")
                },
                "出现时间",
            ),
            (
                TextItem {
                    color: "white".into(),
                    ..item("x")
                },
                "颜色",
            ),
            (
                TextItem {
                    position: TextPosition::Custom,
                    point: None,
                    ..item("x")
                },
                "位置",
            ),
            (item(" \n "), "为空"),
            (
                TextItem {
                    source: TextSource::TextFiles,
                    ..item("x")
                },
                "文本文件",
            ),
            (
                TextItem {
                    background_opacity: 1.5,
                    ..item("x")
                },
                "不透明度",
            ),
        ];
        for (params, needle) in cases {
            let err = filter(vec![params.clone()]).unwrap_err();
            assert!(err.contains(needle), "{params:?} → {err}");
            assert!(err.contains("文本 1"), "{err}");
        }
        let err = filter(vec![item("a"), item("b"), item("c"), item("d")]).unwrap_err();
        assert!(err.contains("最多"), "{err}");
    }

    #[test]
    fn warnings_for_empty_content_missing_font_and_late_start() {
        let mut info = sample_info();
        info.path = "only-strip.mp4".into();
        let recipe = recipe_with(vec![
            TextItem {
                source: TextSource::FileName,
                strip: vec!["only-strip".into()],
                ..item("")
            },
            TextItem {
                font_file: r"Z:\no\such\font.ttf".into(),
                start: 30.0,
                ..item("x")
            },
        ]);
        let built = build_with(&info, &recipe, 1).unwrap();
        let w = built.warnings.join("\n");
        assert!(w.contains("文本 1：内容为空"), "{w}");
        assert!(w.contains("文本 2：字体文件不存在"), "{w}");
        assert!(w.contains("超过视频时长"), "{w}");
        let fc = built.arg_after("-filter_complex").unwrap();
        // 退回默认字体：找到时用它，找不到时不指定字体（另有警告）
        match default_font() {
            Some(font) => assert!(
                fc.contains(&format!(
                    "fontfile={}",
                    escape_filter_path(&font.to_string_lossy())
                )),
                "{fc}"
            ),
            None => assert!(!fc.contains("fontfile") && w.contains("没有找到")),
        }

        let built = build_with(&sample_info(), &recipe_with(vec![]), 1).unwrap();
        assert_eq!(built.arg_after("-filter_complex"), None);
        assert_eq!(built.warnings.len(), 1);
    }

    #[test]
    fn windows_default_font_is_cjk_capable() {
        if cfg!(windows) {
            let candidates = default_font_candidates();
            assert!(candidates[0].ends_with("msyh.ttc"));
        }
    }

    #[test]
    fn custom_point_scales_from_source_and_stream_is_unchanged() {
        let info = sample_info();
        let settings = Settings::default();
        let mut ctx = BuildContext::new(&info, &settings, 0);
        ctx.stream.width = 640;
        ctx.stream.height = 360;
        let before = ctx.stream.clone();
        TextParams {
            enabled: true,
            items: vec![TextItem {
                position: TextPosition::Custom,
                point: Some(TextPoint { x: 1280, y: 720 }),
                ..item("x")
            }],
        }
        .apply(&mut ctx)
        .unwrap();
        assert_eq!(ctx.stream, before);
        let fc = ctx.graph.render().unwrap().filter_complex.unwrap();
        assert!(
            fc.contains("x=max(min(640-tw/2\\,w-tw)\\,0):y=max(min(360-th/2\\,h-th)\\,0)"),
            "{fc}"
        );
        assert!(fc.contains("fontsize=18:"), "{fc}");
    }

    #[test]
    fn serde_shape_and_defaults() {
        let params = TextParams::default();
        assert_eq!(params.items.len(), MAX_ITEMS);
        assert!(params.items[0].enabled && !params.items[1].enabled);
        let json = serde_json::to_value(TextItem::default()).unwrap();
        assert_eq!(json["source"], "fileName");
        assert_eq!(
            json["textFiles"],
            serde_json::json!({"mode": "file", "path": ""})
        );
        assert_eq!(json["point"], serde_json::Value::Null);
        assert_eq!(json["position"], "bottom");
        assert_eq!(json["fitWidth"], true);
        let partial: TextParams = serde_json::from_str(
            r#"{"enabled":true,"items":[{"enabled":true,"source":"custom","customText":"hi","point":{"x":3,"y":4}}]}"#,
        )
        .unwrap();
        assert_eq!(partial.items.len(), 1);
        assert_eq!(partial.items[0].font_size, 5.0);
        assert_eq!(partial.items[0].point, Some(TextPoint { x: 3, y: 4 }));
    }
}
