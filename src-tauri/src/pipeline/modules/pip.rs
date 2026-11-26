//! `pipe.pip` 画中画：画面缩小后放在一个背景上。
//!
//! 参数与取值约定（docs/specs/pipeline.md 未规定的部分在此说明）：
//! - 画布：默认沿用当前画面尺寸；开启 `canvasEnabled` 时改为 `canvasWidth` × `canvasHeight`
//!   （16～7680，奇数向下取偶并提示），可借此做横竖屏转换（如横屏 → 1080×1920）。`ctx.stream` 的宽高随之更新。
//! - 背景 `background`：
//!   - `video`：画面自身铺满画布作背景；
//!   - `asset`：外部图片或视频（`asset` 为单文件或目录随机 / 依次取，目录模式只认 [`BACKGROUND_EXTENSIONS`]）。
//!     图片用 `-loop 1`、动态素材（gif、视频）用 `-stream_loop -1` 无限循环，再按视频帧率 `fps` 转换，
//!     overlay 加 `shortest=1`：素材比视频短时循环、更长时截断，输出时长始终等于视频时长。
//!   - 背景铺满画布的方式由 `blurCenter` 决定（与分辨率模块一致）：true 时等比放大后取中间，
//!     false 时把整幅拉伸到画布大小，null 时跟随设置 `set.blur-center`。`blur` 为 `gblur` 的 sigma（0～100，范围值，0 为不虚化）。
//! - 前景（视频本身）等比缩放，位置 `layout`：
//!   - `centered`：放进占画布 `size`%（5～100，范围值；四边边距即 (100 − size)/2 %）的框内，
//!     再相对画布中心偏移 `offsetX` / `offsetY`（占画布宽 / 高的百分比，-50～50，范围值，正数向右 / 向下）；
//!   - `rect`：放进 `rect` 矩形（**输出画布**的像素坐标）内并居中；超出画布的部分会被裁掉并提示。
//!     前端用 RegionPicker 在当前视频的源画面上框选，再按“源尺寸 → 画布尺寸”的比例换算成画布坐标
//!     （未改画布尺寸时两者相同；前面的模块改了尺寸时以画布为准，前端只能按源尺寸近似）。
//! - 前景抠像 `keyEnabled`：`chromakey`（YUV 空间，适合绿幕），`keyColor` 为 `#rrggbb`，
//!   `keySimilarity` 0.01～1，`keyBlend` 0～1。
//! - 声音始终是视频自身的，不做处理。
//! - 随机取值顺序固定：背景素材 → 虚化强度 → 大小 → 左右偏移 → 上下偏移。
//!
//! 滤镜思路：
//! ```text
//! 自身背景：[v]split=2[a][b];[a]scale..,crop..,setsar=1,gblur..[bg];[b]scale=前景,setsar=1[fg];[bg][fg]overlay=x:y
//! 外部背景：[n:v]scale..,crop..,setsar=1,setpts=PTS-STARTPTS,fps=帧率,gblur..[bg];[v]scale=前景..[fg];
//!          [bg][fg]overlay=x:y:shortest=1
//! ```

use serde::{Deserialize, Serialize};

use crate::fsutil::has_extension;
use crate::model::{AssetSource, Range};
use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.pip";

/// 静态图片扩展名（用 `-loop 1` 输入）。
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp"];
/// 目录模式下认作背景素材的扩展名。
pub const BACKGROUND_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "bmp", "gif", "mp4", "mov", "mkv", "webm", "avi", "flv", "m4v",
];
pub const MIN_CANVAS: u32 = 16;
pub const MAX_CANVAS: u32 = 7680;
pub const MAX_BLUR: f64 = 100.0;
pub const MIN_SIZE: f64 = 5.0;
pub const MAX_OFFSET: f64 = 50.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PipBackground {
    /// 画面自身
    #[default]
    Video,
    /// 外部图片 / 视频
    Asset,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PipLayout {
    /// 按大小居中，可偏移
    #[default]
    Centered,
    /// 自定义矩形
    Rect,
}

/// 画布上的矩形（像素）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PipRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PipParams {
    pub enabled: bool,
    pub background: PipBackground,
    /// 外部背景素材
    pub asset: AssetSource,
    /// 背景虚化强度（gblur sigma，0 为不虚化）
    pub blur: Range<f64>,
    /// 背景取中间（true）还是拉伸整幅（false）
    /// 为空（None）时跟随设置 `set.blur-center`（`Settings::blur_center`）；旧配方里的 true / false 仍然有效且优先
    pub blur_center: Option<bool>,
    /// 自定义输出画布尺寸
    pub canvas_enabled: bool,
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub layout: PipLayout,
    /// 前景所占画布的百分比
    pub size: Range<f64>,
    /// 相对画布中心的偏移（占画布宽 / 高的百分比）
    pub offset_x: Range<f64>,
    pub offset_y: Range<f64>,
    /// 自定义前景区域（画布像素）
    pub rect: PipRect,
    /// 前景抠除纯色（绿幕）
    pub key_enabled: bool,
    pub key_color: String,
    pub key_similarity: f64,
    pub key_blend: f64,
}

impl Default for PipParams {
    fn default() -> Self {
        Self {
            enabled: false,
            background: PipBackground::Video,
            asset: AssetSource::default(),
            blur: Range::fixed(20.0),
            blur_center: None,
            canvas_enabled: false,
            canvas_width: 1080,
            canvas_height: 1920,
            layout: PipLayout::Centered,
            size: Range::fixed(80.0),
            offset_x: Range::fixed(0.0),
            offset_y: Range::fixed(0.0),
            rect: PipRect::default(),
            key_enabled: false,
            key_color: "#00ff00".into(),
            key_similarity: 0.1,
            key_blend: 0.05,
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

fn check_range(range: &Range<f64>, lo: f64, hi: f64, what: &str) -> Result<(), String> {
    let (a, b) = range.ordered();
    if a.is_finite() && b.is_finite() && a >= lo && b <= hi {
        Ok(())
    } else {
        Err(format!("{what}需在 {lo}～{hi} 之间"))
    }
}

/// 向下取偶数，至少 2。
fn even_floor(value: f64) -> u32 {
    let v = value.max(2.0).floor() as u32;
    v - v % 2
}

/// 源画面 (sw, sh) 等比放进 (bw, bh) 的框内，返回偶数宽高。
fn fit(sw: u32, sh: u32, bw: f64, bh: f64) -> (u32, u32) {
    let (sw, sh) = (f64::from(sw.max(1)), f64::from(sh.max(1)));
    if sw * bh >= bw * sh {
        (even_floor(bw), even_floor(bw * sh / sw))
    } else {
        (even_floor(bh * sw / sh), even_floor(bh))
    }
}

/// 前景在画布上的位置与大小。
#[derive(Debug, PartialEq)]
struct Inset {
    x: i64,
    y: i64,
    width: u32,
    height: u32,
}

impl PipParams {
    fn validate(&self) -> Result<(), String> {
        if self.background == PipBackground::Asset && self.asset.is_empty() {
            return Err("请选择背景图片或视频".into());
        }
        check_range(&self.blur, 0.0, MAX_BLUR, "虚化强度")?;
        if self.canvas_enabled {
            for (value, what) in [
                (self.canvas_width, "画布宽度"),
                (self.canvas_height, "画布高度"),
            ] {
                if !(MIN_CANVAS..=MAX_CANVAS).contains(&value) {
                    return Err(format!("{what}需在 {MIN_CANVAS}～{MAX_CANVAS} 之间"));
                }
            }
        }
        match self.layout {
            PipLayout::Centered => {
                check_range(&self.size, MIN_SIZE, 100.0, "前景大小（%）")?;
                check_range(&self.offset_x, -MAX_OFFSET, MAX_OFFSET, "左右偏移（%）")?;
                check_range(&self.offset_y, -MAX_OFFSET, MAX_OFFSET, "上下偏移（%）")?;
            }
            PipLayout::Rect => {
                if self.rect.width < 2 || self.rect.height < 2 {
                    return Err("请先框选前景区域".into());
                }
            }
        }
        if self.key_enabled {
            if parse_color(&self.key_color).is_none() {
                return Err(format!("抠像颜色格式不正确：{}", self.key_color));
            }
            if !(0.01..=1.0).contains(&self.key_similarity) {
                return Err("抠像相似度需在 0.01～1 之间".into());
            }
            if !(0.0..=1.0).contains(&self.key_blend) {
                return Err("抠像边缘混合需在 0～1 之间".into());
            }
        }
        Ok(())
    }

    /// 输出画布尺寸。
    fn canvas(&self, ctx: &mut BuildContext) -> (u32, u32) {
        if !self.canvas_enabled {
            return (ctx.stream.width, ctx.stream.height);
        }
        let w = self.canvas_width - self.canvas_width % 2;
        let h = self.canvas_height - self.canvas_height % 2;
        if (w, h) != (self.canvas_width, self.canvas_height) {
            ctx.warn(format!("画布宽高需为偶数，已调整为 {w}x{h}"));
        }
        (w, h)
    }

    /// 计算前景位置（随机取值：大小 → 左右偏移 → 上下偏移）。
    fn inset(&self, ctx: &mut BuildContext, cw: u32, ch: u32) -> Inset {
        let (sw, sh) = (ctx.stream.width, ctx.stream.height);
        match self.layout {
            PipLayout::Centered => {
                let size = self.size.sample_rounded(&mut ctx.rng, 1) / 100.0;
                let ox = self.offset_x.sample_rounded(&mut ctx.rng, 1) / 100.0;
                let oy = self.offset_y.sample_rounded(&mut ctx.rng, 1) / 100.0;
                let (cwf, chf) = (f64::from(cw), f64::from(ch));
                let (width, height) = fit(sw, sh, cwf * size, chf * size);
                Inset {
                    x: ((cwf - f64::from(width)) / 2.0 + ox * cwf).round() as i64,
                    y: ((chf - f64::from(height)) / 2.0 + oy * chf).round() as i64,
                    width,
                    height,
                }
            }
            PipLayout::Rect => {
                let r = self.rect;
                let x0 = r.x.min(cw.saturating_sub(2));
                let y0 = r.y.min(ch.saturating_sub(2));
                let bw = r.width.min(cw - x0);
                let bh = r.height.min(ch - y0);
                if (x0, y0, bw, bh) != (r.x, r.y, r.width, r.height) {
                    ctx.warn(format!(
                        "前景区域超出画布 {cw}x{ch}，已裁剪为 ({x0}, {y0}) {bw}x{bh}"
                    ));
                }
                let (width, height) = fit(sw, sh, f64::from(bw), f64::from(bh));
                Inset {
                    x: i64::from(x0) + (i64::from(bw) - i64::from(width)) / 2,
                    y: i64::from(y0) + (i64::from(bh) - i64::from(height)) / 2,
                    width,
                    height,
                }
            }
        }
    }
}

impl PipelineModule for PipParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        self.validate()?;
        let asset = match self.background {
            PipBackground::Asset => Some(self.asset.pick(
                BACKGROUND_EXTENSIONS,
                ctx.task_index,
                &mut ctx.rng,
            )?),
            PipBackground::Video => None,
        };
        let sigma = self.blur.sample_rounded(&mut ctx.rng, 1);
        let (cw, ch) = self.canvas(ctx);
        let inset = self.inset(ctx, cw, ch);

        // 背景：铺满画布
        let mut bg = if self.blur_center.unwrap_or(ctx.settings.blur_center) {
            format!("scale={cw}:{ch}:force_original_aspect_ratio=increase,crop={cw}:{ch}")
        } else {
            format!("scale={cw}:{ch}")
        };
        bg.push_str(",setsar=1");
        if asset.is_some() {
            bg.push_str(",setpts=PTS-STARTPTS");
            if let Some(rate) = ctx.rate_text() {
                bg.push_str(&format!(",fps={rate}"));
                ctx.stream.vfr = false;
            }
        }
        if sigma > 0.0 {
            bg.push_str(&format!(",gblur=sigma={}", fmt_num(sigma)));
        }

        // 前景
        let mut fg = format!("scale={}:{},setsar=1", inset.width, inset.height);
        if self.key_enabled {
            let color = parse_color(&self.key_color).unwrap_or(0x00ff00);
            fg.push_str(&format!(
                ",format=yuva444p,chromakey=color=0x{color:06x}:similarity={}:blend={}",
                fmt_num(self.key_similarity),
                fmt_num(self.key_blend)
            ));
        }

        let main = ctx.graph.take_video();
        let [bg_label, fg_label, out] = ["pb", "pf", "po"].map(|p| ctx.graph.label(p));
        let mut overlay = format!("overlay={}:{}", inset.x, inset.y);
        match asset {
            Some(path) => {
                let args = if has_extension(&path, IMAGE_EXTENSIONS) {
                    vec!["-loop".to_owned(), "1".to_owned()]
                } else {
                    vec!["-stream_loop".to_owned(), "-1".to_owned()]
                };
                let input = ctx.add_input(args, &path)?;
                ctx.graph.chain(&[&format!("{input}:v")], &bg, &[&bg_label]);
                ctx.graph.chain(&[&main], &fg, &[&fg_label]);
                // 背景无限循环：以视频结束为准
                overlay.push_str(":shortest=1");
            }
            None => {
                let [a, b] = ["ps", "ps"].map(|p| ctx.graph.label(p));
                ctx.graph.chain(&[&main], "split=2", &[&a, &b]);
                ctx.graph.chain(&[&a], &bg, &[&bg_label]);
                ctx.graph.chain(&[&b], &fg, &[&fg_label]);
            }
        }
        overlay.push_str(":format=auto");
        ctx.graph.chain(&[&bg_label, &fg_label], &overlay, &[&out]);
        ctx.graph.set_video(out);
        ctx.stream.width = cw;
        ctx.stream.height = ch;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PickOrder, Recipe, SeededRng, Settings};
    use crate::pipeline::testing::{build_with, sample_info};
    use crate::pipeline::BuiltCommand;
    use std::path::PathBuf;

    struct Assets(PathBuf);

    impl Assets {
        fn new(name: &str, files: &[&str]) -> Self {
            let dir = std::env::temp_dir().join(format!("cr-pip-{name}-{}", std::process::id()));
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

    fn build(params: PipParams, seed: u64) -> Result<BuiltCommand, String> {
        let mut recipe = Recipe::disabled();
        recipe.modules.pip = PipParams {
            enabled: true,
            ..params
        };
        build_with(&sample_info(), &recipe, seed)
    }

    fn filter(built: &BuiltCommand) -> &str {
        built.arg_after("-filter_complex").unwrap()
    }

    /// 应用后的画布尺寸。
    fn stream_size(params: &PipParams) -> (u32, u32) {
        let info = sample_info();
        let settings = Settings::default();
        let mut ctx = BuildContext::new(&info, &settings, 0);
        ctx.rng = SeededRng::derive(1, ID);
        params.apply(&mut ctx).unwrap();
        (ctx.stream.width, ctx.stream.height)
    }

    /// 主输入之后第一个额外输入的 (前置参数, 路径)。
    fn extra_input(built: &BuiltCommand) -> (Vec<String>, String) {
        let first = built.args.iter().position(|a| a == "-i").unwrap();
        let second = first
            + 2
            + built.args[first + 2..]
                .iter()
                .position(|a| a == "-i")
                .unwrap();
        (
            built.args[first + 2..second].to_vec(),
            built.args[second + 1].clone(),
        )
    }

    #[test]
    fn own_video_as_blurred_background() {
        let built = build(PipParams::default(), 1).unwrap();
        // 80%：1024×576，居中 (128, 72)
        assert_eq!(
            filter(&built),
            "[0:v:0]split=2[ps3][ps4];\
             [ps3]scale=1280:720:force_original_aspect_ratio=increase,crop=1280:720,setsar=1,gblur=sigma=20[pb0];\
             [ps4]scale=1024:576,setsar=1[pf1];\
             [pb0][pf1]overlay=128:72:format=auto[po2]"
        );
        assert_eq!(built.arg_after("-c:a"), Some("copy"));
        assert_eq!(built.expected_duration, 10.0);
        assert_eq!(stream_size(&PipParams::default()), (1280, 720));
    }

    #[test]
    fn portrait_canvas_with_stretched_background_and_chroma_key() {
        let params = PipParams {
            canvas_enabled: true,
            canvas_width: 1080,
            canvas_height: 1920,
            size: Range::fixed(100.0),
            offset_y: Range::fixed(-10.0),
            blur_center: Some(false),
            blur: Range::fixed(0.0),
            key_enabled: true,
            key_color: "#00FF00".into(),
            key_similarity: 0.15,
            key_blend: 0.0,
            ..PipParams::default()
        };
        // 1280×720 放进 1080×1920：1080×606（取偶），y = (1920-606)/2 - 192 = 465
        assert_eq!(
            filter(&build(params.clone(), 1).unwrap()),
            "[0:v:0]split=2[ps3][ps4];[ps3]scale=1080:1920,setsar=1[pb0];\
             [ps4]scale=1080:606,setsar=1,format=yuva444p,\
             chromakey=color=0x00ff00:similarity=0.15:blend=0[pf1];\
             [pb0][pf1]overlay=0:465:format=auto[po2]"
        );
        assert_eq!(stream_size(&params), (1080, 1920));
    }

    #[test]
    fn external_image_background() {
        let a = Assets::new("image", &["bg.JPG"]);
        let params = PipParams {
            background: PipBackground::Asset,
            asset: a.file("bg.JPG"),
            size: Range::fixed(50.0),
            offset_x: Range::fixed(25.0),
            ..PipParams::default()
        };
        let built = build(params, 1).unwrap();
        // 50%：640×360；x = 320 + 320 = 640，y = 180
        assert_eq!(
            filter(&built),
            "[1:v]scale=1280:720:force_original_aspect_ratio=increase,crop=1280:720,setsar=1,\
             setpts=PTS-STARTPTS,fps=25,gblur=sigma=20[pb0];\
             [0:v:0]scale=640:360,setsar=1[pf1];\
             [pb0][pf1]overlay=640:180:shortest=1:format=auto[po2]"
        );
        assert_eq!(
            extra_input(&built),
            (vec!["-loop".into(), "1".into()], a.path("bg.JPG"))
        );
        assert_eq!(built.arg_after("-c:a"), Some("copy"));
    }

    #[test]
    fn external_video_background_loops() {
        let a = Assets::new("video", &["bg.mp4"]);
        let params = PipParams {
            background: PipBackground::Asset,
            asset: a.file("bg.mp4"),
            ..PipParams::default()
        };
        let built = build(params, 1).unwrap();
        assert_eq!(
            extra_input(&built),
            (vec!["-stream_loop".into(), "-1".into()], a.path("bg.mp4"))
        );
        assert!(filter(&built).contains("overlay=128:72:shortest=1:format=auto"));
    }

    #[test]
    fn custom_rect_layout() {
        let params = PipParams {
            layout: PipLayout::Rect,
            rect: PipRect {
                x: 100,
                y: 50,
                width: 400,
                height: 400,
            },
            blur: Range::fixed(5.0),
            ..PipParams::default()
        };
        // 1280×720 放进 400×400：400×224，在矩形内垂直居中：y = 50 + 88
        let built = build(params.clone(), 1).unwrap();
        assert_eq!(
            filter(&built),
            "[0:v:0]split=2[ps3][ps4];\
             [ps3]scale=1280:720:force_original_aspect_ratio=increase,crop=1280:720,setsar=1,gblur=sigma=5[pb0];\
             [ps4]scale=400:224,setsar=1[pf1];[pb0][pf1]overlay=100:138:format=auto[po2]"
        );
        assert!(built.warnings.is_empty());

        // 超出画布：裁剪并提示
        let outside = PipParams {
            rect: PipRect {
                x: 1000,
                y: 600,
                width: 600,
                height: 300,
            },
            ..params
        };
        let built = build(outside, 1).unwrap();
        assert!(filter(&built).contains("scale=212:120,setsar=1[pf1]"));
        assert!(filter(&built).contains("overlay=1034:600"));
        assert!(built.warnings.iter().any(|w| w.contains("超出画布")));
    }

    #[test]
    fn directory_background_pick() {
        let a = Assets::new("dir", &["1.png", "2.mp4", "3.gif", "x.txt"]);
        let random = PipParams {
            background: PipBackground::Asset,
            asset: a.dir(PickOrder::Random),
            ..PipParams::default()
        };
        let picked = |seed| extra_input(&build(random.clone(), seed).unwrap()).1;
        assert_eq!(picked(4), picked(4));
        let all: std::collections::HashSet<String> = (0..40).map(picked).collect();
        assert_eq!(all.len(), 3);
        let sequential = PipParams {
            asset: a.dir(PickOrder::Sequential),
            ..random
        };
        assert_eq!(
            extra_input(&build(sequential, 7).unwrap()).1,
            a.path("1.png")
        );
    }

    #[test]
    fn ranges_are_reproducible_and_bounded() {
        let params = PipParams {
            blur: Range::new(5.0, 30.0),
            size: Range::new(60.0, 90.0),
            offset_x: Range::new(-10.0, 10.0),
            ..PipParams::default()
        };
        assert_eq!(
            build(params.clone(), 3).unwrap().args,
            build(params.clone(), 3).unwrap().args
        );
        for seed in 0..40 {
            let built = build(params.clone(), seed).unwrap();
            let fc = filter(&built);
            let after = |key: &str| -> &str {
                let s = &fc[fc.find(key).unwrap() + key.len()..];
                &s[..s.find(['[', ':', ',']).unwrap()]
            };
            let sigma: f64 = after("gblur=sigma=").parse().unwrap();
            assert!((5.0..=30.0).contains(&sigma), "{fc}");
            let width: u32 = after("[ps4]scale=").parse().unwrap();
            assert!((768..=1152).contains(&width), "{fc}");
            let x: i64 = after("overlay=").parse().unwrap();
            let centered = (1280 - i64::from(width)) / 2;
            assert!((x - centered).abs() <= 129, "{fc}");
        }
    }

    #[test]
    fn odd_canvas_is_made_even() {
        let params = PipParams {
            canvas_enabled: true,
            canvas_width: 721,
            canvas_height: 1281,
            ..PipParams::default()
        };
        let built = build(params.clone(), 1).unwrap();
        assert!(built.warnings.iter().any(|w| w.contains("偶数")));
        assert_eq!(stream_size(&params), (720, 1280));
    }

    #[test]
    fn invalid_params_are_rejected() {
        let a = Assets::new("invalid", &["bg.png"]);
        let empty = Assets::new("invalid-empty", &["readme.txt"]);
        let err = |params: PipParams| build(params, 1).unwrap_err();
        let asset = |source: AssetSource| PipParams {
            background: PipBackground::Asset,
            asset: source,
            ..PipParams::default()
        };
        assert!(err(asset(AssetSource::default())).contains("请选择"));
        assert!(err(asset(a.file("missing.png"))).contains("不存在"));
        assert!(err(asset(empty.dir(PickOrder::Random))).contains("没有可用文件"));
        for (params, what) in [
            (
                PipParams {
                    canvas_enabled: true,
                    canvas_width: 8,
                    ..PipParams::default()
                },
                "画布宽度",
            ),
            (
                PipParams {
                    canvas_enabled: true,
                    canvas_height: 10000,
                    ..PipParams::default()
                },
                "画布高度",
            ),
            (
                PipParams {
                    size: Range::fixed(0.0),
                    ..PipParams::default()
                },
                "大小",
            ),
            (
                PipParams {
                    size: Range::new(50.0, 120.0),
                    ..PipParams::default()
                },
                "大小",
            ),
            (
                PipParams {
                    offset_x: Range::fixed(60.0),
                    ..PipParams::default()
                },
                "偏移",
            ),
            (
                PipParams {
                    blur: Range::fixed(-1.0),
                    ..PipParams::default()
                },
                "虚化",
            ),
            (
                PipParams {
                    layout: PipLayout::Rect,
                    ..PipParams::default()
                },
                "框选",
            ),
            (
                PipParams {
                    key_enabled: true,
                    key_color: "#12345".into(),
                    ..PipParams::default()
                },
                "颜色",
            ),
            (
                PipParams {
                    key_enabled: true,
                    key_blend: 2.0,
                    ..PipParams::default()
                },
                "混合",
            ),
        ] {
            let e = err(params);
            assert!(e.contains(what), "{what}: {e}");
        }
        // 没有用到的参数不校验
        assert!(build(
            PipParams {
                canvas_width: 1,
                key_color: "bad".into(),
                asset: a.file("missing.png"),
                ..PipParams::default()
            },
            1
        )
        .is_ok());
    }

    #[test]
    fn serde_defaults() {
        let fixed = |v: f64| serde_json::json!({"min": v, "max": v});
        assert_eq!(
            serde_json::to_value(PipParams::default()).unwrap(),
            serde_json::json!({
                "enabled": false,
                "background": "video",
                "asset": {"mode": "file", "path": ""},
                "blur": fixed(20.0),
                "blurCenter": null,
                "canvasEnabled": false,
                "canvasWidth": 1080,
                "canvasHeight": 1920,
                "layout": "centered",
                "size": fixed(80.0),
                "offsetX": fixed(0.0),
                "offsetY": fixed(0.0),
                "rect": {"x": 0, "y": 0, "width": 0, "height": 0},
                "keyEnabled": false,
                "keyColor": "#00ff00",
                "keySimilarity": 0.1,
                "keyBlend": 0.05
            })
        );
    }

    #[test]
    fn fit_helper() {
        assert_eq!(fit(1280, 720, 1024.0, 576.0), (1024, 576));
        assert_eq!(fit(720, 1280, 1080.0, 1920.0), (1080, 1920));
        assert_eq!(fit(321, 241, 321.0, 241.0), (320, 240));
        assert_eq!(fit(100, 100, 1.0, 1.0), (2, 2));
    }
}
