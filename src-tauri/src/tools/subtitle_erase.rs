//! `tool.subtitle-erase` 字幕擦除：在本机用 FFmpeg 滤镜去掉画面里“烧录”的硬字幕。原软件调用云端服务，
//! 本项目不使用任何云服务——这是**本地近似处理**：只能用周围像素填补 / 模糊 / 遮盖所选区域，
//! 背景复杂（人物、运动画面）时会留下可见的涂抹痕迹，效果不如基于深度学习的云端修复。
//!
//! **参数**（括号内为默认值）
//! - `area`（`bottom`）擦除区域：`bottom` 底部字幕条——整个宽度，从画面高度的 `bandTop`%（78）到
//!   `bandBottom`%（94）；`regions` 自定义 1～4 个矩形（`regions`，**源视频像素**，已按旋转转正，用 RegionPicker 框选）。
//! - `mode`（`delogo`）擦除方式：
//!   - `delogo` 修补：`delogo` 滤镜用区域边缘的像素向内插值填补，适合颜色平缓的背景；
//!   - `blur` 模糊：区域内高斯模糊（`gblur`，强度 `blurStrength` 为 sigma，默认 20，1～100），文字变成色块；
//!   - `fill` 纯色遮盖：`drawbox` 用 `fillColor`（`#000000`）填满区域（适合本来就有底色条的字幕）。
//! - `ranges`（空）只在这些时段擦除：每段 `开始-结束`（秒或 `分:秒`），多段用逗号、分号或换行分隔；空为整段视频。
//!
//! **滤镜的取舍**：`removelogo` 需要一张与画面同尺寸的黑白遮罩图片，效果与 `delogo` 相近（都是用边缘像素插值 /
//! 模糊），对矩形区域没有优势，所以没有采用；单纯 `boxblur` 有半径上限（区域短边的一半），改用 `gblur`；
//! `drawbox` 用于纯色遮盖。`delogo` 要求区域与画面边缘至少相距 1 像素，所以贴边的区域会向内收 1 像素。
//! **自动识别字幕位置**没有实现：FFmpeg 没有可靠的文字检测滤镜，按亮度 / 边缘猜测的结果在真实视频上很不稳定，
//! 因此只提供预设字幕条与手动框选。
//!
//! 区域会裁到画面之内，左上角与宽高取偶数（模糊模式要 crop / overlay 到 yuv420 画面上）。
//! 输出：视频重编码（编码器与码率按设置与 `pipe.bitrate`），音频容器装得下时直接复制，否则转 AAC；尺寸、帧率、时长不变
//! （奇数尺寸裁掉最右 / 最下一像素）。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::pipeline::fmt_num;
use crate::tools::media::{container_accepts, StreamKind};
use crate::tools::{Encoding, OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.subtitle-erase";

/// 自定义区域的最大个数。
pub const MAX_REGIONS: usize = 4;
/// 区域裁剪到画面内后宽、高的最小像素数。
pub const MIN_REGION_PX: u32 = 4;
/// 最多几个时段。
pub const MAX_RANGES: usize = 100;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EraseArea {
    #[default]
    Bottom,
    Regions,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EraseMode {
    #[default]
    Delogo,
    Blur,
    Fill,
}

/// 矩形区域（源视频像素）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EraseRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleEraseParams {
    pub area: EraseArea,
    /// 底部字幕条的上沿（画面高度的百分比）
    pub band_top: f64,
    /// 底部字幕条的下沿（画面高度的百分比）
    pub band_bottom: f64,
    pub regions: Vec<EraseRegion>,
    pub mode: EraseMode,
    /// 模糊强度（gblur sigma）
    pub blur_strength: f64,
    pub fill_color: String,
    /// 只在这些时段擦除
    pub ranges: String,
}

impl Default for SubtitleEraseParams {
    fn default() -> Self {
        Self {
            area: EraseArea::Bottom,
            band_top: 78.0,
            band_bottom: 94.0,
            regions: Vec::new(),
            mode: EraseMode::Delogo,
            blur_strength: 20.0,
            fill_color: "#000000".into(),
            ranges: String::new(),
        }
    }
}

/// `#rrggbb` / `#rgb` → `0xrrggbb`。
pub fn color_hex(text: &str) -> Result<String, String> {
    let hex = text.trim().trim_start_matches('#');
    let full: String = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect(),
        6 => hex.to_owned(),
        _ => return Err(format!("遮盖颜色无效：{text}")),
    };
    if !full.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("遮盖颜色无效：{text}"));
    }
    Ok(format!("0x{}", full.to_ascii_lowercase()))
}

fn parse_clock(text: &str) -> Option<f64> {
    let parts: Vec<&str> = text.trim().split([':', '：']).collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut seconds = 0.0;
    for (i, part) in parts.iter().enumerate() {
        let v: f64 = part.trim().parse().ok()?;
        if !v.is_finite() || v < 0.0 || (i + 1 < parts.len() && v.fract() != 0.0) {
            return None;
        }
        seconds = seconds * 60.0 + v;
    }
    Some(seconds)
}

/// 解析时段（见模块说明），按开始时间排序。
pub fn parse_ranges(text: &str) -> Result<Vec<(f64, f64)>, String> {
    let mut ranges = Vec::new();
    for entry in text
        .split([',', '，', ';', '；', '\n'])
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        let invalid = || format!("无法识别的时段“{entry}”：请写成 开始-结束，如 0-5 或 1:10-1:20");
        let (a, b) = entry
            .split_once(['-', '~', '～', '—'])
            .ok_or_else(invalid)?;
        let (start, end) = (
            parse_clock(a).ok_or_else(invalid)?,
            parse_clock(b).ok_or_else(invalid)?,
        );
        if end <= start {
            return Err(format!("时段“{entry}”的结束时间必须大于开始时间"));
        }
        ranges.push((start, end));
    }
    if ranges.len() > MAX_RANGES {
        return Err(format!("时段太多（最多 {MAX_RANGES} 段）"));
    }
    ranges.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(ranges)
}

/// 时段 → 滤镜的 `enable` 选项（含开头的冒号）；整段视频时为空。
pub fn enable_option(ranges: &[(f64, f64)]) -> String {
    if ranges.is_empty() {
        return String::new();
    }
    let terms: Vec<String> = ranges
        .iter()
        .map(|(a, b)| format!("between(t,{},{})", fmt_num(*a), fmt_num(*b)))
        .collect();
    format!(":enable='{}'", terms.join("+"))
}

impl SubtitleEraseParams {
    fn validate(&self) -> Result<(), String> {
        match self.area {
            EraseArea::Bottom => {
                let ok = |v: f64| v.is_finite() && (0.0..=100.0).contains(&v);
                if !ok(self.band_top) || !ok(self.band_bottom) {
                    return Err("字幕条位置必须在 0～100% 之间".into());
                }
                if self.band_bottom - self.band_top < 1.0 {
                    return Err("字幕条的下沿必须比上沿至少低 1%".into());
                }
            }
            EraseArea::Regions => {
                if self.regions.is_empty() {
                    return Err("请框选要擦除的字幕区域".into());
                }
                if self.regions.len() > MAX_REGIONS {
                    return Err(format!("最多 {MAX_REGIONS} 个区域"));
                }
            }
        }
        match self.mode {
            EraseMode::Blur
                if !(self.blur_strength.is_finite()
                    && (1.0..=100.0).contains(&self.blur_strength)) =>
            {
                return Err("模糊强度必须在 1～100 之间".into());
            }
            EraseMode::Fill => {
                color_hex(&self.fill_color)?;
            }
            _ => {}
        }
        parse_ranges(&self.ranges)?;
        Ok(())
    }

    /// 在 `w x h` 画面上实际处理的区域（裁到画面内、取偶）。
    pub fn compute_regions(&self, w: u32, h: u32) -> Result<Vec<EraseRegion>, String> {
        self.validate()?;
        let raw: Vec<EraseRegion> = match self.area {
            EraseArea::Bottom => {
                let top = (f64::from(h) * self.band_top / 100.0).round() as u32;
                let bottom = (f64::from(h) * self.band_bottom / 100.0).round() as u32;
                vec![EraseRegion {
                    x: 0,
                    y: top,
                    width: w,
                    height: bottom.saturating_sub(top),
                }]
            }
            EraseArea::Regions => self.regions.clone(),
        };
        raw.iter()
            .enumerate()
            .map(|(i, r)| {
                let x = r.x.min(w) & !1;
                let y = r.y.min(h) & !1;
                let right = r.x.saturating_add(r.width).min(w);
                let bottom = r.y.saturating_add(r.height).min(h);
                let width = right.saturating_sub(x) & !1;
                let height = bottom.saturating_sub(y) & !1;
                if width < MIN_REGION_PX || height < MIN_REGION_PX {
                    return Err(format!("第 {} 个区域在画面（{w}x{h}）之外或太小", i + 1));
                }
                Ok(EraseRegion {
                    x,
                    y,
                    width,
                    height,
                })
            })
            .collect()
    }
}

/// 生成 `-filter_complex`（输入 `[0:v:0]`，输出 `[v]`）。`(w, h)` 是解码后（已旋转）的画面尺寸。
pub fn build_filter(params: &SubtitleEraseParams, (w, h): (u32, u32)) -> Result<String, String> {
    let regions = params.compute_regions(w, h)?;
    let ranges = parse_ranges(&params.ranges)?;
    let enable = enable_option(&ranges);
    let even = if w % 2 == 1 || h % 2 == 1 {
        format!(",crop={}:{}:0:0", w & !1, h & !1)
    } else {
        String::new()
    };
    let graph = match params.mode {
        EraseMode::Delogo => {
            let parts: Vec<String> = regions
                .iter()
                .map(|r| {
                    // delogo 要求区域离画面边缘至少 1 像素
                    let x = r.x.max(1);
                    let y = r.y.max(1);
                    let width = (r.x + r.width).min(w - 1).saturating_sub(x);
                    let height = (r.y + r.height).min(h - 1).saturating_sub(y);
                    format!("delogo=x={x}:y={y}:w={width}:h={height}{enable}")
                })
                .collect();
            format!("[0:v:0]{}{even}[v]", parts.join(","))
        }
        EraseMode::Fill => {
            let color = color_hex(&params.fill_color)?;
            let parts: Vec<String> = regions
                .iter()
                .map(|r| {
                    format!(
                        "drawbox=x={}:y={}:w={}:h={}:color={color}@1:t=fill{enable}",
                        r.x, r.y, r.width, r.height
                    )
                })
                .collect();
            format!("[0:v:0]{}{even}[v]", parts.join(","))
        }
        EraseMode::Blur => {
            let n = regions.len();
            let crops: Vec<String> = (1..=n).map(|i| format!("[c{i}]")).collect();
            let mut chains = vec![format!("[0:v:0]split={}[base]{}", n + 1, crops.concat())];
            let sigma = fmt_num(params.blur_strength);
            let mut current = "[base]".to_owned();
            for (i, r) in regions.iter().enumerate() {
                let i = i + 1;
                chains.push(format!(
                    "[c{i}]crop={}:{}:{}:{},gblur=sigma={sigma}:steps=3[b{i}]",
                    r.width, r.height, r.x, r.y
                ));
                let out = format!("[o{i}]");
                chains.push(format!(
                    "{current}[b{i}]overlay=x={}:y={}{enable}{out}",
                    r.x, r.y
                ));
                current = out;
            }
            chains.push(format!("{current}null{even}[v]"));
            chains.join(";")
        }
    };
    Ok(graph)
}

/// 解码后（已按旋转转正）的画面尺寸。
fn display_size(info: &MediaInfo) -> Result<(u32, u32), String> {
    let video = info.video.as_ref().ok_or("文件中没有视频流")?;
    let (w, h) = video.display_size();
    if w < 2 || h < 2 {
        return Err("读不到视频尺寸".into());
    }
    Ok((w, h))
}

/// 音频参数：容器装得下时全部音轨直接复制，否则第一条音轨转 AAC。
pub fn audio_args(info: &MediaInfo, container: &str) -> Vec<String> {
    if info.audio.is_empty() {
        return vec!["-an".into()];
    }
    let copy = info
        .audio
        .iter()
        .all(|a| container_accepts(container, StreamKind::Audio, &a.codec) != Some(false));
    if copy {
        vec!["-map".into(), "0:a?".into(), "-c:a".into(), "copy".into()]
    } else {
        let mut args = vec!["-map".into(), "0:a:0".into()];
        args.extend(Encoding::audio_args(info.audio[0].channels));
        args
    }
}

impl Tool for SubtitleEraseParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let info = ctx.info(0)?;
        let input = ctx.input_path(0)?.to_path_buf();
        let graph = build_filter(self, display_size(&info)?)?;
        let ext = ctx.video_extension(0)?;
        let output = ctx.output(&OutputSpec::file(&ext).suffix("_erased"))?;
        let input = path_arg(&input)?;
        let output = path_arg(&output)?;
        let audio = audio_args(&info, &ext);
        ctx.run_encode("字幕擦除", info.duration, |enc| {
            let mut args = vec![
                "-i".to_owned(),
                input.clone(),
                "-filter_complex".into(),
                graph.clone(),
                "-map".into(),
                "[v]".into(),
            ];
            args.extend(enc.video_args(&ext));
            args.extend(audio.iter().cloned());
            args.extend(["-sn".into(), "-dn".into()]);
            if matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
                args.extend(["-movflags".into(), "+faststart".into()]);
            }
            args.push(output.clone());
            Ok(args)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::sample_info;

    fn regions(list: &[(u32, u32, u32, u32)]) -> Vec<EraseRegion> {
        list.iter()
            .map(|&(x, y, width, height)| EraseRegion {
                x,
                y,
                width,
                height,
            })
            .collect()
    }

    #[test]
    fn bottom_band_with_delogo_stays_inside_the_frame() {
        let params = SubtitleEraseParams::default();
        assert_eq!(
            params.compute_regions(1280, 720).unwrap(),
            regions(&[(0, 562, 1280, 114)])
        );
        assert_eq!(
            build_filter(&params, (1280, 720)).unwrap(),
            "[0:v:0]delogo=x=1:y=562:w=1278:h=114[v]"
        );
        let to_edge = SubtitleEraseParams {
            band_bottom: 100.0,
            ..SubtitleEraseParams::default()
        };
        assert_eq!(
            build_filter(&to_edge, (320, 240)).unwrap(),
            "[0:v:0]delogo=x=1:y=186:w=318:h=53[v]"
        );
    }

    #[test]
    fn fill_and_time_ranges() {
        let params = SubtitleEraseParams {
            area: EraseArea::Regions,
            regions: regions(&[(11, 21, 101, 41), (200, 300, 50, 20)]),
            mode: EraseMode::Fill,
            fill_color: "#F00".into(),
            ranges: "0-1.5, 1:00-1:02.5".into(),
            ..SubtitleEraseParams::default()
        };
        assert_eq!(
            build_filter(&params, (1280, 720)).unwrap(),
            "[0:v:0]drawbox=x=10:y=20:w=102:h=42:color=0xff0000@1:t=fill:enable='between(t,0,1.5)+between(t,60,62.5)',\
             drawbox=x=200:y=300:w=50:h=20:color=0xff0000@1:t=fill:enable='between(t,0,1.5)+between(t,60,62.5)'[v]"
        );
    }

    #[test]
    fn blur_crops_blurs_and_overlays_each_region() {
        let params = SubtitleEraseParams {
            area: EraseArea::Regions,
            regions: regions(&[(0, 0, 100, 40), (1200, 700, 200, 100)]),
            mode: EraseMode::Blur,
            blur_strength: 15.0,
            ranges: "2-4".into(),
            ..SubtitleEraseParams::default()
        };
        assert_eq!(
            build_filter(&params, (1281, 721)).unwrap(),
            "[0:v:0]split=3[base][c1][c2];\
             [c1]crop=100:40:0:0,gblur=sigma=15:steps=3[b1];\
             [base][b1]overlay=x=0:y=0:enable='between(t,2,4)'[o1];\
             [c2]crop=80:20:1200:700,gblur=sigma=15:steps=3[b2];\
             [o1][b2]overlay=x=1200:y=700:enable='between(t,2,4)'[o2];\
             [o2]null,crop=1280:720:0:0[v]"
        );
    }

    #[test]
    fn ranges_parse_and_sort() {
        assert_eq!(parse_ranges("").unwrap(), []);
        assert_eq!(
            parse_ranges("5-6\n0:01~0:02；1.5-2").unwrap(),
            [(1.0, 2.0), (1.5, 2.0), (5.0, 6.0)]
        );
        for bad in ["5", "3-2", "a-b", "1-1"] {
            assert!(parse_ranges(bad).is_err(), "{bad:?}");
        }
        assert_eq!(enable_option(&[]), "");
    }

    #[test]
    fn invalid_params() {
        let cases = [
            SubtitleEraseParams {
                area: EraseArea::Regions,
                ..SubtitleEraseParams::default()
            },
            SubtitleEraseParams {
                area: EraseArea::Regions,
                regions: regions(&[(0, 0, 10, 10); 5]),
                ..SubtitleEraseParams::default()
            },
            SubtitleEraseParams {
                band_top: 90.0,
                band_bottom: 80.0,
                ..SubtitleEraseParams::default()
            },
            SubtitleEraseParams {
                mode: EraseMode::Blur,
                blur_strength: 0.0,
                ..SubtitleEraseParams::default()
            },
            SubtitleEraseParams {
                mode: EraseMode::Fill,
                fill_color: "red".into(),
                ..SubtitleEraseParams::default()
            },
            SubtitleEraseParams {
                ranges: "x".into(),
                ..SubtitleEraseParams::default()
            },
        ];
        for params in cases {
            assert!(build_filter(&params, (640, 360)).is_err(), "{params:?}");
        }
        let outside = SubtitleEraseParams {
            area: EraseArea::Regions,
            regions: regions(&[(700, 0, 100, 100)]),
            ..SubtitleEraseParams::default()
        };
        assert!(outside
            .compute_regions(640, 360)
            .unwrap_err()
            .contains("之外"));
    }

    #[test]
    fn audio_is_copied_when_the_container_accepts_it() {
        let info = sample_info();
        assert_eq!(audio_args(&info, "mp4").join(" "), "-map 0:a? -c:a copy");
        let mut pcm = sample_info();
        pcm.audio[0].codec = "pcm_s16le".into();
        pcm.audio[0].channels = 1;
        assert_eq!(
            audio_args(&pcm, "mp4").join(" "),
            "-map 0:a:0 -c:a aac -b:a 96k"
        );
        let mut silent = sample_info();
        silent.audio.clear();
        assert_eq!(audio_args(&silent, "mp4"), ["-an"]);
    }
}
