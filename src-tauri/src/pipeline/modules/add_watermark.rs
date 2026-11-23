//! `pipe.add-watermark` 加水印：把图片 / GIF / 视频素材叠加到画面上。
//!
//! 参数与取值约定（FEATURES 未规定的部分在此说明）：
//! - 素材：单个文件，或目录（随机 / 依次取，目录模式只认 [`WATERMARK_EXTENSIONS`]）。
//!   按扩展名区分：png / jpg / jpeg / webp / bmp 为**静态图片**（只有一帧，叠加时一直显示最后一帧）；
//!   其余（gif、mp4、mov、webm、mkv…，包括带透明通道的 mov）为**动态素材**，用 `-stream_loop -1`
//!   循环到视频结束，overlay 加 `shortest=1`，输出时长始终等于视频时长（素材更长时截断）。
//! - 位置 `anchor`：九宫格；`marginX` / `marginY` 为距画面边缘的距离，按画面宽 / 高的百分比（0～50，范围值），
//!   只作用于贴边的方向（居中的那一维不偏移）。
//! - 大小 `size`：水印宽度占画面宽度的百分比（1～100，范围值），高度按素材比例；
//!   居中（`center`）时可开 `stretch`：忽略大小与比例，拉伸铺满整个画面。
//! - 不透明度 `opacity`：0～100%（范围值），`format=rgba,colorchannelmixer=aa=`。
//! - 滚动 `scroll`：`left` / `right`（水平穿过画面，循环）、`up` / `down`（垂直穿过画面，循环）、
//!   `bounce`（在画面内斜向反弹）。`scrollSpeed` 为每秒移动画面**宽度**的百分比（1～500，范围值）。
//!   水平滚动时纵向位置按九宫格的行，垂直滚动时横向位置按九宫格的列。
//! - 倒放 `reverse`：只对动态素材生效（`reverse` + `loop` 滤镜循环；整段素材缓存在内存里，
//!   最多 32767 帧，适合短动画）。
//! - 去背景色 `keyEnabled`：`colorkey`（RGB 空间），`keyColor` 为 `#rrggbb`，
//!   `keySimilarity` 0.01～1，`keyBlend` 0～1。
//! - 目录模式的“分段依次显示”`turns`：从目录里取最多 [`MAX_TURN_ASSETS`] 个素材（随机取时打乱顺序，
//!   依次取时从第 队列序号 个开始按自然顺序），每 `segment` 秒（0.5～3600）换下一个，循环显示。
//!   视频时长已知时，素材个数不超过分段数。每个素材一个 overlay，用
//!   `enable='eq(mod(floor(t/段长),个数),序号)'` 控制显示时段。
//! - 子水印 `sub`：参数较少（素材、九宫格位置、大小、不透明度），边距沿用主水印，不滚动、不倒放、不去背景。
//! - 随机取值顺序固定：主水印素材 → 大小 → 不透明度 → 左右边距 → 上下边距 → 滚动速度 →
//!   子水印素材 → 子水印大小 → 子水印不透明度。
//! - 画面尺寸、帧率、时长都不变。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::has_extension;
use crate::model::{AssetSource, PickOrder, Range};
use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.add-watermark";

/// 静态图片（只有一帧）的扩展名。
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp"];
/// 目录模式下认作水印素材的扩展名。
pub const WATERMARK_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "bmp", "gif", "mp4", "mov", "webm", "mkv",
];
/// 分段依次显示时最多使用的素材个数。
pub const MAX_TURN_ASSETS: usize = 10;
/// 倒放后循环时 `loop` 滤镜最多缓存的帧数（滤镜上限）。
pub const LOOP_FRAMES: u32 = 32767;
pub const MAX_MARGIN: f64 = 50.0;
pub const MAX_SPEED: f64 = 500.0;
pub const MIN_SEGMENT: f64 = 0.5;
pub const MAX_SEGMENT: f64 = 3600.0;

/// 九宫格位置。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Anchor {
    TopLeft,
    Top,
    #[default]
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// 列：-1 左、0 中、1 右。
    fn column(self) -> i8 {
        match self {
            Anchor::TopLeft | Anchor::Left | Anchor::BottomLeft => -1,
            Anchor::Top | Anchor::Center | Anchor::Bottom => 0,
            Anchor::TopRight | Anchor::Right | Anchor::BottomRight => 1,
        }
    }

    /// 行：-1 上、0 中、1 下。
    fn row(self) -> i8 {
        match self {
            Anchor::TopLeft | Anchor::Top | Anchor::TopRight => -1,
            Anchor::Left | Anchor::Center | Anchor::Right => 0,
            Anchor::BottomLeft | Anchor::Bottom | Anchor::BottomRight => 1,
        }
    }
}

/// 滚动效果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScrollMode {
    #[default]
    None,
    /// 从右向左
    Left,
    /// 从左向右
    Right,
    /// 从下向上
    Up,
    /// 从上向下
    Down,
    /// 在画面内反弹
    Bounce,
}

/// 子水印：参数较少。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubWatermarkParams {
    pub enabled: bool,
    pub source: AssetSource,
    pub anchor: Anchor,
    /// 宽度占画面宽度的百分比
    pub size: Range<f64>,
    /// 不透明度（%）
    pub opacity: Range<f64>,
}

impl Default for SubWatermarkParams {
    fn default() -> Self {
        Self {
            enabled: false,
            source: AssetSource::default(),
            anchor: Anchor::BottomLeft,
            size: Range::fixed(15.0),
            opacity: Range::fixed(80.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AddWatermarkParams {
    pub enabled: bool,
    /// 水印素材：单文件或目录
    pub source: AssetSource,
    /// 目录模式：分段依次显示目录里的多个素材
    pub turns: bool,
    /// 分段依次显示时每段的秒数
    pub segment: f64,
    pub anchor: Anchor,
    /// 左右边距（占画面宽度的百分比）
    pub margin_x: Range<f64>,
    /// 上下边距（占画面高度的百分比）
    pub margin_y: Range<f64>,
    /// 宽度占画面宽度的百分比
    pub size: Range<f64>,
    /// 不透明度（%）
    pub opacity: Range<f64>,
    /// 居中时拉伸铺满画面
    pub stretch: bool,
    pub scroll: ScrollMode,
    /// 滚动速度：每秒移动画面宽度的百分比
    pub scroll_speed: Range<f64>,
    /// 动态素材倒放
    pub reverse: bool,
    /// 去除素材的背景色
    pub key_enabled: bool,
    pub key_color: String,
    pub key_similarity: f64,
    pub key_blend: f64,
    pub sub: SubWatermarkParams,
}

impl Default for AddWatermarkParams {
    fn default() -> Self {
        Self {
            enabled: false,
            source: AssetSource::default(),
            turns: false,
            segment: 5.0,
            anchor: Anchor::TopRight,
            margin_x: Range::fixed(3.0),
            margin_y: Range::fixed(3.0),
            size: Range::fixed(20.0),
            opacity: Range::fixed(80.0),
            stretch: false,
            scroll: ScrollMode::None,
            scroll_speed: Range::fixed(20.0),
            reverse: false,
            key_enabled: false,
            key_color: "#00ff00".into(),
            key_similarity: 0.1,
            key_blend: 0.05,
            sub: SubWatermarkParams::default(),
        }
    }
}

/// `#rrggbb`（或不带 #）→ 0xrrggbb。
fn parse_color(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

/// 是否为动态素材（非静态图片）。
fn is_animated(path: &Path) -> bool {
    !has_extension(path, IMAGE_EXTENSIONS)
}

fn check_range(range: &Range<f64>, lo: f64, hi: f64, what: &str, unit: &str) -> Result<(), String> {
    let (a, b) = range.ordered();
    if a.is_finite() && b.is_finite() && a >= lo && b <= hi {
        Ok(())
    } else {
        Err(format!("{what}需在 {lo}～{hi}{unit} 之间"))
    }
}

/// 一个水印的外观（同一视频内的多个素材共用）。
struct Look {
    /// 水印宽度（像素）；None 表示拉伸铺满
    width: Option<u32>,
    /// 0～1
    opacity: f64,
    /// 去背景：颜色、相似度、混合
    key: Option<(u32, f64, f64)>,
    reverse: bool,
}

/// overlay 的位置表达式。
struct Place {
    x: String,
    y: String,
}

/// 九宫格某一维的表达式：`full`/`own` 为画面 / 水印在该维的变量名（W/w 或 H/h）。
fn axis_expr(side: i8, margin: u32, full: &str, own: &str) -> String {
    match side {
        -1 => margin.to_string(),
        0 => format!("({full}-{own})/2"),
        _ => format!("{full}-{own}-{margin}"),
    }
}

impl AddWatermarkParams {
    fn validate(&self) -> Result<(), String> {
        if self.source.is_empty() {
            return Err("请选择水印素材文件或目录".into());
        }
        check_range(&self.size, 1.0, 100.0, "水印大小", "%")?;
        check_range(&self.opacity, 0.0, 100.0, "不透明度", "%")?;
        check_range(&self.margin_x, 0.0, MAX_MARGIN, "左右边距", "%")?;
        check_range(&self.margin_y, 0.0, MAX_MARGIN, "上下边距", "%")?;
        if self.scroll != ScrollMode::None {
            check_range(&self.scroll_speed, 1.0, MAX_SPEED, "滚动速度", "%")?;
        }
        if self.turns && self.is_directory() && !(MIN_SEGMENT..=MAX_SEGMENT).contains(&self.segment)
        {
            return Err(format!(
                "每段显示时长需在 {MIN_SEGMENT}～{MAX_SEGMENT} 秒之间"
            ));
        }
        if self.key_enabled {
            if parse_color(&self.key_color).is_none() {
                return Err(format!("背景色格式不正确：{}", self.key_color));
            }
            if !(0.01..=1.0).contains(&self.key_similarity) {
                return Err("去背景相似度需在 0.01～1 之间".into());
            }
            if !(0.0..=1.0).contains(&self.key_blend) {
                return Err("去背景边缘混合需在 0～1 之间".into());
            }
        }
        if self.sub.enabled {
            if self.sub.source.is_empty() {
                return Err("已开启子水印但没有选择素材".into());
            }
            check_range(&self.sub.size, 1.0, 100.0, "子水印大小", "%")?;
            check_range(&self.sub.opacity, 0.0, 100.0, "子水印不透明度", "%")?;
        }
        Ok(())
    }

    fn is_directory(&self) -> bool {
        matches!(self.source, AssetSource::Directory { .. })
    }

    /// 主水印要用的素材及各自的显示条件（`enable` 表达式）。
    fn main_assets(
        &self,
        ctx: &mut BuildContext,
    ) -> Result<Vec<(PathBuf, Option<String>)>, String> {
        let order = match &self.source {
            AssetSource::Directory { order, .. } if self.turns => *order,
            _ => {
                let path = self
                    .source
                    .pick(WATERMARK_EXTENSIONS, ctx.task_index, &mut ctx.rng)?;
                return Ok(vec![(path, None)]);
            }
        };
        let mut files = self.source.candidates(WATERMARK_EXTENSIONS)?;
        match order {
            PickOrder::Random => {
                // Fisher–Yates 洗牌
                for i in (1..files.len()).rev() {
                    let j = ctx.rng.index(i + 1);
                    files.swap(i, j);
                }
            }
            PickOrder::Sequential => {
                let start = ctx.task_index % files.len();
                files.rotate_left(start);
            }
        }
        let mut count = files.len().min(MAX_TURN_ASSETS);
        if ctx.stream.duration > 0.0 {
            let segments = (ctx.stream.duration / self.segment).ceil().max(1.0) as usize;
            count = count.min(segments);
        }
        files.truncate(count);
        if count == 1 {
            return Ok(vec![(files.remove(0), None)]);
        }
        let segment = fmt_num(self.segment);
        Ok(files
            .into_iter()
            .enumerate()
            .map(|(i, path)| {
                let enable = format!("eq(mod(floor(t/{segment}),{count}),{i})");
                (path, Some(enable))
            })
            .collect())
    }

    /// 位置表达式（含滚动）。
    fn place(&self, mx: u32, my: u32, speed_px: f64) -> Place {
        let stretched = self.anchor == Anchor::Center && self.stretch;
        let (col, row) = (self.anchor.column(), self.anchor.row());
        let x = if stretched {
            "0".to_owned()
        } else {
            axis_expr(col, mx, "W", "w")
        };
        let y = if stretched {
            "0".to_owned()
        } else {
            axis_expr(row, my, "H", "h")
        };
        let v = fmt_num(speed_px);
        match self.scroll {
            ScrollMode::None => Place { x, y },
            ScrollMode::Left => Place {
                x: format!("W-mod(t*{v},W+w)"),
                y,
            },
            ScrollMode::Right => Place {
                x: format!("mod(t*{v},W+w)-w"),
                y,
            },
            ScrollMode::Up => Place {
                x,
                y: format!("H-mod(t*{v},H+h)"),
            },
            ScrollMode::Down => Place {
                x,
                y: format!("mod(t*{v},H+h)-h"),
            },
            ScrollMode::Bounce => Place {
                x: format!("(W-w)*abs(1-mod(t*{v}/max(W-w,1)+1,2))"),
                y: format!("(H-h)*abs(1-mod(t*{v}/max(H-h,1)+1,2))"),
            },
        }
    }
}

/// 加入一个水印素材并叠加到当前画面上。
fn overlay(
    ctx: &mut BuildContext,
    path: &Path,
    look: &Look,
    place: &Place,
    enable: Option<&str>,
) -> Result<(), String> {
    let animated = is_animated(path);
    let reverse = look.reverse && animated;
    let args = if animated && !reverse {
        vec!["-stream_loop".to_owned(), "-1".to_owned()]
    } else {
        Vec::new()
    };
    let input = ctx.add_input(args, path)?;

    let mut filters: Vec<String> = Vec::new();
    if reverse {
        filters.push("reverse".into());
        filters.push(format!("loop=loop=-1:size={LOOP_FRAMES}"));
    }
    filters.push("format=rgba".into());
    if let Some((color, similarity, blend)) = look.key {
        filters.push(format!(
            "colorkey=color=0x{color:06x}:similarity={}:blend={}",
            fmt_num(similarity),
            fmt_num(blend)
        ));
    }
    filters.push(match look.width {
        Some(width) => format!("scale={width}:-1"),
        None => format!("scale={}:{}", ctx.stream.width, ctx.stream.height),
    });
    if look.opacity < 1.0 {
        filters.push(format!("colorchannelmixer=aa={}", fmt_num(look.opacity)));
    }
    let mark = ctx.graph.label("wm");
    ctx.graph
        .chain(&[&format!("{input}:v")], &filters.join(","), &[&mark]);

    let mut options = format!("overlay=x='{}':y='{}':format=auto", place.x, place.y);
    if animated {
        // 素材无限循环：以视频结束为准
        options.push_str(":shortest=1");
    }
    if let Some(enable) = enable {
        options.push_str(&format!(":enable='{enable}'"));
    }
    let main = ctx.graph.take_video();
    let out = ctx.graph.label("wo");
    ctx.graph.chain(&[&main, &mark], &options, &[&out]);
    ctx.graph.set_video(out);
    Ok(())
}

/// 百分比 → 像素（四舍五入，至少 `min`）。
fn percent_px(percent: f64, full: u32, min: u32) -> u32 {
    ((percent / 100.0 * f64::from(full)).round() as u32).max(min)
}

impl PipelineModule for AddWatermarkParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        self.validate()?;
        let (w, h) = (ctx.stream.width, ctx.stream.height);

        let assets = self.main_assets(ctx)?;
        let size = self.size.sample_rounded(&mut ctx.rng, 1);
        let opacity = self.opacity.sample_rounded(&mut ctx.rng, 0) / 100.0;
        let mx = percent_px(self.margin_x.sample_rounded(&mut ctx.rng, 1), w, 0);
        let my = percent_px(self.margin_y.sample_rounded(&mut ctx.rng, 1), h, 0);
        let speed = if self.scroll == ScrollMode::None {
            0.0
        } else {
            self.scroll_speed.sample_rounded(&mut ctx.rng, 1)
        };

        let stretched = self.anchor == Anchor::Center && self.stretch;
        let look = Look {
            width: (!stretched).then(|| percent_px(size, w, 2)),
            opacity,
            key: if self.key_enabled {
                parse_color(&self.key_color).map(|c| (c, self.key_similarity, self.key_blend))
            } else {
                None
            },
            reverse: self.reverse,
        };
        if self.reverse && !assets.iter().any(|(p, _)| is_animated(p)) {
            ctx.warn("静态图片水印不支持倒放，已忽略");
        }
        let place = self.place(mx, my, speed / 100.0 * f64::from(w));
        for (path, enable) in &assets {
            overlay(ctx, path, &look, &place, enable.as_deref())?;
        }

        if self.sub.enabled {
            let sub = &self.sub;
            let path = sub
                .source
                .pick(WATERMARK_EXTENSIONS, ctx.task_index, &mut ctx.rng)?;
            let size = sub.size.sample_rounded(&mut ctx.rng, 1);
            let opacity = sub.opacity.sample_rounded(&mut ctx.rng, 0) / 100.0;
            let look = Look {
                width: Some(percent_px(size, w, 2)),
                opacity,
                key: None,
                reverse: false,
            };
            let place = Place {
                x: axis_expr(sub.anchor.column(), mx, "W", "w"),
                y: axis_expr(sub.anchor.row(), my, "H", "h"),
            };
            overlay(ctx, &path, &look, &place, None)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Recipe;
    use crate::pipeline::testing::{build_with, sample_info};
    use crate::pipeline::BuiltCommand;

    /// 单元测试用的临时素材目录（构建命令时不读文件内容）。
    struct Assets(PathBuf);

    impl Assets {
        fn new(name: &str, files: &[&str]) -> Self {
            let dir = std::env::temp_dir().join(format!("cr-add-wm-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            for f in files {
                std::fs::write(dir.join(f), b"x").unwrap();
            }
            Self(dir)
        }

        fn file(&self, name: &str) -> AssetSource {
            AssetSource::File {
                path: self.path(name),
            }
        }

        fn dir(&self, order: PickOrder) -> AssetSource {
            AssetSource::Directory {
                path: self.0.to_string_lossy().into_owned(),
                recursive: false,
                order,
            }
        }

        fn path(&self, name: &str) -> String {
            self.0.join(name).to_string_lossy().into_owned()
        }
    }

    impl Drop for Assets {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn build(params: AddWatermarkParams, seed: u64) -> Result<BuiltCommand, String> {
        let mut recipe = Recipe::disabled();
        recipe.modules.add_watermark = AddWatermarkParams {
            enabled: true,
            ..params
        };
        build_with(&sample_info(), &recipe, seed)
    }

    fn filter(built: &BuiltCommand) -> &str {
        built.arg_after("-filter_complex").unwrap()
    }

    /// 全部额外输入：(前置参数, 路径)。
    fn inputs(built: &BuiltCommand) -> Vec<(Vec<String>, String)> {
        let mut result = Vec::new();
        let mut start = None;
        for (i, arg) in built.args.iter().enumerate() {
            if arg == "-i" {
                if let Some(s) = start {
                    result.push((built.args[s..i].to_vec(), built.args[i + 1].clone()));
                }
                start = Some(i + 2);
            }
        }
        result
    }

    #[test]
    fn still_image_at_top_right() {
        let a = Assets::new("still", &["logo.png"]);
        let built = build(
            AddWatermarkParams {
                source: a.file("logo.png"),
                ..AddWatermarkParams::default()
            },
            1,
        )
        .unwrap();
        // 1280×20% = 256；边距 1280×3% ≈ 38、720×3% ≈ 22
        assert_eq!(
            filter(&built),
            "[1:v]format=rgba,scale=256:-1,colorchannelmixer=aa=0.8[wm0];\
             [0:v:0][wm0]overlay=x='W-w-38':y='22':format=auto[wo1]"
        );
        assert_eq!(inputs(&built), [(vec![], a.path("logo.png"))]);
        assert_eq!(built.arg_after("-c:a"), Some("copy"));
        assert_eq!(built.expected_duration, 10.0);
    }

    #[test]
    fn animated_gif_reversed_keyed_at_bottom_left() {
        let a = Assets::new("gif", &["anim.gif"]);
        let params = AddWatermarkParams {
            source: a.file("anim.gif"),
            anchor: Anchor::BottomLeft,
            margin_x: Range::fixed(0.0),
            margin_y: Range::fixed(5.0),
            size: Range::fixed(10.0),
            opacity: Range::fixed(100.0),
            reverse: true,
            key_enabled: true,
            key_color: "#FFFFFF".into(),
            key_similarity: 0.2,
            key_blend: 0.1,
            ..AddWatermarkParams::default()
        };
        let built = build(params, 1).unwrap();
        assert_eq!(
            filter(&built),
            "[1:v]reverse,loop=loop=-1:size=32767,format=rgba,\
             colorkey=color=0xffffff:similarity=0.2:blend=0.1,scale=128:-1[wm0];\
             [0:v:0][wm0]overlay=x='0':y='H-h-36':format=auto:shortest=1[wo1]"
        );
        // 倒放时由 loop 滤镜循环，不加 -stream_loop
        assert_eq!(inputs(&built), [(vec![], a.path("anim.gif"))]);
    }

    #[test]
    fn video_asset_loops_and_ends_with_main() {
        let a = Assets::new("video", &["mark.MOV"]);
        let params = AddWatermarkParams {
            source: a.file("mark.MOV"),
            anchor: Anchor::Center,
            ..AddWatermarkParams::default()
        };
        let built = build(params, 1).unwrap();
        assert_eq!(
            filter(&built),
            "[1:v]format=rgba,scale=256:-1,colorchannelmixer=aa=0.8[wm0];\
             [0:v:0][wm0]overlay=x='(W-w)/2':y='(H-h)/2':format=auto:shortest=1[wo1]"
        );
        assert_eq!(
            inputs(&built),
            [(vec!["-stream_loop".into(), "-1".into()], a.path("mark.MOV"))]
        );
    }

    #[test]
    fn center_stretch_fills_frame() {
        let a = Assets::new("stretch", &["full.png"]);
        let params = AddWatermarkParams {
            source: a.file("full.png"),
            anchor: Anchor::Center,
            stretch: true,
            opacity: Range::fixed(30.0),
            ..AddWatermarkParams::default()
        };
        assert_eq!(
            filter(&build(params.clone(), 1).unwrap()),
            "[1:v]format=rgba,scale=1280:720,colorchannelmixer=aa=0.3[wm0];\
             [0:v:0][wm0]overlay=x='0':y='0':format=auto[wo1]"
        );
        // 不是居中时忽略拉伸
        let corner = AddWatermarkParams {
            anchor: Anchor::Left,
            ..params
        };
        assert!(filter(&build(corner, 1).unwrap()).contains("scale=256:-1"));
    }

    #[test]
    fn scrolling_expressions() {
        let a = Assets::new("scroll", &["logo.png"]);
        let with = |scroll: ScrollMode, anchor: Anchor| {
            let built = build(
                AddWatermarkParams {
                    source: a.file("logo.png"),
                    anchor,
                    scroll,
                    scroll_speed: Range::fixed(25.0),
                    ..AddWatermarkParams::default()
                },
                1,
            )
            .unwrap();
            let fc = filter(&built);
            fc[fc.find("overlay=").unwrap()..].to_owned()
        };
        // 25% × 1280 = 320 像素/秒
        assert_eq!(
            with(ScrollMode::Left, Anchor::Bottom),
            "overlay=x='W-mod(t*320,W+w)':y='H-h-22':format=auto[wo1]"
        );
        assert_eq!(
            with(ScrollMode::Right, Anchor::Center),
            "overlay=x='mod(t*320,W+w)-w':y='(H-h)/2':format=auto[wo1]"
        );
        assert_eq!(
            with(ScrollMode::Up, Anchor::TopLeft),
            "overlay=x='38':y='H-mod(t*320,H+h)':format=auto[wo1]"
        );
        assert_eq!(
            with(ScrollMode::Down, Anchor::Right),
            "overlay=x='W-w-38':y='mod(t*320,H+h)-h':format=auto[wo1]"
        );
        assert_eq!(
            with(ScrollMode::Bounce, Anchor::TopRight),
            "overlay=x='(W-w)*abs(1-mod(t*320/max(W-w,1)+1,2))':\
             y='(H-h)*abs(1-mod(t*320/max(H-h,1)+1,2))':format=auto[wo1]"
        );
    }

    #[test]
    fn turns_show_assets_one_after_another() {
        let a = Assets::new("turns", &["1.png", "2.gif", "3.png", "notes.txt"]);
        let params = AddWatermarkParams {
            source: a.dir(PickOrder::Sequential),
            turns: true,
            segment: 4.0,
            opacity: Range::fixed(100.0),
            ..AddWatermarkParams::default()
        };
        let built = build(params.clone(), 1).unwrap();
        // 10 秒、每段 4 秒 → 3 段，3 个素材
        assert_eq!(
            filter(&built),
            "[1:v]format=rgba,scale=256:-1[wm0];\
             [0:v:0][wm0]overlay=x='W-w-38':y='22':format=auto:enable='eq(mod(floor(t/4),3),0)'[wo1];\
             [2:v]format=rgba,scale=256:-1[wm2];\
             [wo1][wm2]overlay=x='W-w-38':y='22':format=auto:shortest=1:enable='eq(mod(floor(t/4),3),1)'[wo3];\
             [3:v]format=rgba,scale=256:-1[wm4];\
             [wo3][wm4]overlay=x='W-w-38':y='22':format=auto:enable='eq(mod(floor(t/4),3),2)'[wo5]"
        );
        let paths: Vec<String> = inputs(&built).into_iter().map(|(_, p)| p).collect();
        assert_eq!(paths, [a.path("1.png"), a.path("2.gif"), a.path("3.png")]);

        // 段数少于素材数：只用前两个
        let fewer = AddWatermarkParams {
            segment: 5.0,
            ..params.clone()
        };
        let built = build(fewer, 1).unwrap();
        assert_eq!(inputs(&built).len(), 2);
        assert!(filter(&built).contains("enable='eq(mod(floor(t/5),2),1)'"));

        // 只有一段：只用一个素材，不加 enable
        let single = AddWatermarkParams {
            segment: 20.0,
            ..params.clone()
        };
        let built = build(single, 1).unwrap();
        assert_eq!(inputs(&built).len(), 1);
        assert!(!filter(&built).contains("enable"));

        // 随机顺序：同一种子结果相同，不同种子会变化
        let random = AddWatermarkParams {
            source: a.dir(PickOrder::Random),
            ..params
        };
        let order = |seed| -> Vec<String> {
            inputs(&build(random.clone(), seed).unwrap())
                .into_iter()
                .map(|(_, p)| p)
                .collect()
        };
        assert_eq!(order(5), order(5));
        let orders: std::collections::HashSet<Vec<String>> = (0..20).map(order).collect();
        assert!(orders.len() > 1);
        assert!(orders.iter().all(|o| o.len() == 3));
    }

    #[test]
    fn turns_are_capped() {
        let names: Vec<String> = (0..15).map(|i| format!("{i}.png")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let a = Assets::new("cap", &refs);
        let built = build(
            AddWatermarkParams {
                source: a.dir(PickOrder::Sequential),
                turns: true,
                segment: 0.5,
                ..AddWatermarkParams::default()
            },
            1,
        )
        .unwrap();
        assert_eq!(inputs(&built).len(), MAX_TURN_ASSETS);
    }

    #[test]
    fn directory_random_pick_is_reproducible() {
        let a = Assets::new("pick", &["a.png", "b.jpg", "c.mp4", "readme.txt"]);
        let params = AddWatermarkParams {
            source: a.dir(PickOrder::Random),
            ..AddWatermarkParams::default()
        };
        let picked = |seed| inputs(&build(params.clone(), seed).unwrap())[0].1.clone();
        assert_eq!(picked(3), picked(3));
        let all: std::collections::HashSet<String> = (0..40).map(picked).collect();
        assert_eq!(all.len(), 3, "{all:?}");
        assert!(all.iter().all(|p| !p.ends_with(".txt")));
        // 依次取：队列序号 0 → 第一个
        let seq = AddWatermarkParams {
            source: a.dir(PickOrder::Sequential),
            ..AddWatermarkParams::default()
        };
        assert_eq!(inputs(&build(seq, 9).unwrap())[0].1, a.path("a.png"));
    }

    #[test]
    fn sub_watermark_is_added_after_main() {
        let a = Assets::new("sub", &["main.png", "sub.gif"]);
        let params = AddWatermarkParams {
            source: a.file("main.png"),
            opacity: Range::fixed(100.0),
            sub: SubWatermarkParams {
                enabled: true,
                source: a.file("sub.gif"),
                anchor: Anchor::Bottom,
                size: Range::fixed(5.0),
                opacity: Range::fixed(50.0),
            },
            ..AddWatermarkParams::default()
        };
        let built = build(params, 1).unwrap();
        assert_eq!(
            filter(&built),
            "[1:v]format=rgba,scale=256:-1[wm0];\
             [0:v:0][wm0]overlay=x='W-w-38':y='22':format=auto[wo1];\
             [2:v]format=rgba,scale=64:-1,colorchannelmixer=aa=0.5[wm2];\
             [wo1][wm2]overlay=x='(W-w)/2':y='H-h-22':format=auto:shortest=1[wo3]"
        );
        assert_eq!(
            inputs(&built),
            [
                (vec![], a.path("main.png")),
                (vec!["-stream_loop".into(), "-1".into()], a.path("sub.gif"))
            ]
        );
    }

    #[test]
    fn ranges_are_reproducible_and_bounded() {
        let a = Assets::new("ranges", &["logo.png"]);
        let params = AddWatermarkParams {
            source: a.file("logo.png"),
            size: Range::new(10.0, 30.0),
            opacity: Range::new(40.0, 90.0),
            margin_x: Range::new(1.0, 5.0),
            ..AddWatermarkParams::default()
        };
        assert_eq!(
            build(params.clone(), 8).unwrap().args,
            build(params.clone(), 8).unwrap().args
        );
        for seed in 0..40 {
            let built = build(params.clone(), seed).unwrap();
            let fc = filter(&built);
            let num = |key: &str, end: char| -> f64 {
                let s = &fc[fc.find(key).unwrap() + key.len()..];
                s[..s.find(end).unwrap()].parse().unwrap()
            };
            let width = num("scale=", ':');
            assert!((128.0..=384.0).contains(&width), "{fc}");
            let alpha = num("aa=", '[');
            assert!((0.4..=0.9).contains(&alpha), "{fc}");
            let mx = num("x='W-w-", '\'');
            assert!((12.0..=64.0).contains(&mx), "{fc}");
        }
    }

    #[test]
    fn still_image_reverse_warns() {
        let a = Assets::new("warn", &["logo.png"]);
        let built = build(
            AddWatermarkParams {
                source: a.file("logo.png"),
                reverse: true,
                ..AddWatermarkParams::default()
            },
            1,
        )
        .unwrap();
        assert!(!filter(&built).contains("reverse"));
        assert!(built.warnings.iter().any(|w| w.contains("倒放")));
    }

    #[test]
    fn invalid_params_are_rejected() {
        let a = Assets::new("invalid", &["logo.png"]);
        let empty = Assets::new("invalid-empty", &["readme.txt"]);
        let ok = AddWatermarkParams {
            source: a.file("logo.png"),
            ..AddWatermarkParams::default()
        };
        let err = |params: AddWatermarkParams| build(params, 1).unwrap_err();
        assert!(err(AddWatermarkParams::default()).contains("请选择"));
        assert!(err(AddWatermarkParams {
            source: a.file("missing.png"),
            ..ok.clone()
        })
        .contains("不存在"));
        assert!(err(AddWatermarkParams {
            source: empty.dir(PickOrder::Random),
            ..ok.clone()
        })
        .contains("没有可用文件"));
        assert!(err(AddWatermarkParams {
            source: empty.dir(PickOrder::Random),
            turns: true,
            ..ok.clone()
        })
        .contains("没有可用文件"));
        for (params, what) in [
            (
                AddWatermarkParams {
                    size: Range::fixed(0.0),
                    ..ok.clone()
                },
                "大小",
            ),
            (
                AddWatermarkParams {
                    size: Range::new(50.0, 150.0),
                    ..ok.clone()
                },
                "大小",
            ),
            (
                AddWatermarkParams {
                    opacity: Range::fixed(120.0),
                    ..ok.clone()
                },
                "不透明度",
            ),
            (
                AddWatermarkParams {
                    margin_y: Range::fixed(-1.0),
                    ..ok.clone()
                },
                "边距",
            ),
            (
                AddWatermarkParams {
                    scroll: ScrollMode::Left,
                    scroll_speed: Range::fixed(0.0),
                    ..ok.clone()
                },
                "速度",
            ),
            (
                AddWatermarkParams {
                    source: a.dir(PickOrder::Random),
                    turns: true,
                    segment: 0.0,
                    ..ok.clone()
                },
                "每段",
            ),
            (
                AddWatermarkParams {
                    key_enabled: true,
                    key_color: "green".into(),
                    ..ok.clone()
                },
                "背景色",
            ),
            (
                AddWatermarkParams {
                    key_enabled: true,
                    key_similarity: 0.0,
                    ..ok.clone()
                },
                "相似度",
            ),
            (
                AddWatermarkParams {
                    sub: SubWatermarkParams {
                        enabled: true,
                        ..SubWatermarkParams::default()
                    },
                    ..ok.clone()
                },
                "子水印",
            ),
            (
                AddWatermarkParams {
                    sub: SubWatermarkParams {
                        enabled: true,
                        source: a.file("logo.png"),
                        size: Range::fixed(200.0),
                        ..SubWatermarkParams::default()
                    },
                    ..ok.clone()
                },
                "子水印大小",
            ),
        ] {
            let e = err(params);
            assert!(e.contains(what), "{what}: {e}");
        }
        // 关闭的子水印 / 未用到的参数不校验
        assert!(build(
            AddWatermarkParams {
                scroll_speed: Range::fixed(0.0),
                segment: 0.0,
                key_color: "bad".into(),
                sub: SubWatermarkParams {
                    size: Range::fixed(0.0),
                    ..SubWatermarkParams::default()
                },
                ..ok
            },
            1
        )
        .is_ok());
    }

    #[test]
    fn serde_defaults() {
        let json = serde_json::to_value(AddWatermarkParams::default()).unwrap();
        let fixed = |v: f64| serde_json::json!({"min": v, "max": v});
        let empty = serde_json::json!({"mode": "file", "path": ""});
        assert_eq!(
            json,
            serde_json::json!({
                "enabled": false,
                "source": empty,
                "turns": false,
                "segment": 5.0,
                "anchor": "topRight",
                "marginX": fixed(3.0),
                "marginY": fixed(3.0),
                "size": fixed(20.0),
                "opacity": fixed(80.0),
                "stretch": false,
                "scroll": "none",
                "scrollSpeed": fixed(20.0),
                "reverse": false,
                "keyEnabled": false,
                "keyColor": "#00ff00",
                "keySimilarity": 0.1,
                "keyBlend": 0.05,
                "sub": {
                    "enabled": false,
                    "source": empty,
                    "anchor": "bottomLeft",
                    "size": fixed(15.0),
                    "opacity": fixed(80.0)
                }
            })
        );
        let parsed: AddWatermarkParams = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(parsed.segment, 5.0);
    }
}
