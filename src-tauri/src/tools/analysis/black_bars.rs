//! 黑边检测：抽样后用 `cropdetect` 找出非黑色区域，返回裁剪矩形（显示坐标，已考虑旋转）。
//!
//! 用 `cropdetect` 输出的**精确边界**（`x1`/`y1`/`x2`/`y2`，非黑区域的首尾行列）而不是它取整后的
//! `x`/`y`/`w`/`h`：后者把偏移对齐到偶数，奇数偏移的黑边会多裁掉最多 2 像素的画面、又留下黑线。
//! 宽高需要取整（偶数）时优先向黑边一侧扩 1 像素，不裁画面；偏移可以是奇数（`crop` 用 `exact=1`）。

use serde::Serialize;

use crate::ffmpeg::MediaInfo;
use crate::pipeline::fmt_num;

use super::{Analysis, MetadataFrame};

/// 裁剪矩形（像素）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl CropRect {
    /// `crop=w:h:x:y` 滤镜；偏移为奇数时加 `exact=1`（否则 4:2:0 视频的偏移会被向下取偶）。
    pub fn filter(&self) -> String {
        let exact = if self.x % 2 == 1 || self.y % 2 == 1 {
            ":exact=1"
        } else {
            ""
        };
        format!(
            "crop={}:{}:{}:{}{exact}",
            self.width, self.height, self.x, self.y
        )
    }
}

/// 把 `[start, start + len)` 调整为 `round` 的倍数：优先向两侧（黑边）扩展，扩不开时才缩短。
fn fit_span(start: u32, len: u32, full: u32, round: u32) -> (u32, u32) {
    let rest = len % round;
    if rest == 0 {
        return (start, len);
    }
    let need = round - rest;
    let before = start;
    let after = full.saturating_sub(start + len);
    if before + after >= need {
        // 先从富余较多的一侧取
        let (take_after, take_before) = if after >= before {
            let a = need.min(after);
            (a, need - a)
        } else {
            let b = need.min(before);
            (need - b, b)
        };
        return (start - take_before, len + take_before + take_after);
    }
    (start, len - rest)
}

/// 黑边检测（第一条视频流）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlackBarDetect {
    /// 亮度不超过此值（0～255）的像素视为黑色
    pub limit: u32,
    /// 结果宽高取整到这个数的倍数（2 = 偶数，便于 yuv420p 编码）
    pub round: u32,
    /// 每秒抽样几帧（越多越准、越慢）
    pub sample_fps: f64,
    /// 四边加起来少于这么多像素的“黑边”忽略（视为没有黑边）
    pub min_bar: u32,
}

impl Default for BlackBarDetect {
    fn default() -> Self {
        Self {
            limit: 24,
            round: 2,
            sample_fps: 2.0,
            min_bar: 4,
        }
    }
}

impl Analysis for BlackBarDetect {
    type Output = Option<CropRect>;

    fn name(&self) -> &'static str {
        "黑边检测"
    }

    fn args(&self, info: &MediaInfo) -> Result<Vec<String>, String> {
        if self.limit > 255 {
            return Err("黑色阈值必须在 0～255 之间".into());
        }
        if !(self.sample_fps.is_finite() && self.sample_fps > 0.0) {
            return Err("抽样帧率必须大于 0".into());
        }
        if info.video.is_none() {
            return Err("文件中没有视频流，无法检测黑边".into());
        }
        Ok(vec![
            "-map".into(),
            "0:v:0".into(),
            "-an".into(),
            "-sn".into(),
            "-dn".into(),
            "-vf".into(),
            format!(
                "fps={},cropdetect=limit={}:round={}:skip=0:reset=0,metadata=mode=print",
                fmt_num(self.sample_fps),
                self.limit,
                self.round.max(1)
            ),
        ])
    }

    /// `cropdetect` 不重置时会累积全部帧的非黑区域；各帧结果再求并集兜底。优先用精确边界 `x1`..`y2`
    /// （含首尾行列），没有时退回取整后的 `x`/`y`/`w`/`h`。
    fn parse(&self, frames: &[MetadataFrame], info: &MediaInfo) -> Option<CropRect> {
        let (full_w, full_h) = info.video.as_ref()?.display_size();
        let mut union: Option<(u32, u32, u32, u32)> = None;
        for frame in frames {
            let get = |k: &str| frame.number(&format!("lavfi.cropdetect.{k}"));
            // 半开区间 [x, x2) × [y, y2)
            let exact = match (get("x1"), get("y1"), get("x2"), get("y2")) {
                (Some(x1), Some(y1), Some(x2), Some(y2))
                    if x1 >= 0.0 && y1 >= 0.0 && x2 >= x1 && y2 >= y1 =>
                {
                    Some((x1 as u32, y1 as u32, x2 as u32 + 1, y2 as u32 + 1))
                }
                _ => None,
            };
            let rounded = || match (get("x"), get("y"), get("w"), get("h")) {
                (Some(x), Some(y), Some(w), Some(h))
                    if w >= 1.0 && h >= 1.0 && x >= 0.0 && y >= 0.0 =>
                {
                    let (x, y) = (x as u32, y as u32);
                    Some((x, y, x + w as u32, y + h as u32))
                }
                _ => None,
            };
            let Some((x, y, x2, y2)) = exact.or_else(rounded) else {
                continue;
            };
            union = Some(match union {
                None => (x, y, x2, y2),
                Some((ux, uy, ux2, uy2)) => (ux.min(x), uy.min(y), ux2.max(x2), uy2.max(y2)),
            });
        }
        let (x, y, x2, y2) = union?;
        let (x2, y2) = (x2.min(full_w), y2.min(full_h));
        if x2 <= x || y2 <= y {
            return None;
        }
        let round = self.round.max(1);
        let (x, width) = fit_span(x, x2 - x, full_w, round);
        let (y, height) = fit_span(y, y2 - y, full_h, round);
        if width == 0 || height == 0 {
            return None;
        }
        let removed = (full_w - width) + (full_h - height);
        if removed < self.min_bar.max(1) {
            return None;
        }
        Some(CropRect {
            x,
            y,
            width,
            height,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::lines;
    use super::*;
    use crate::pipeline::testing::sample_info;

    fn frame(x: u32, y: u32, w: u32, h: u32) -> String {
        format!(
            "[Parsed_metadata_2 @ 0] frame:1 pts:1 pts_time:0.5\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.x1={x}\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.w={w}\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.h={h}\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.x={x}\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.y={y}\n"
        )
    }

    #[test]
    fn args() {
        let args = BlackBarDetect::default().args(&sample_info()).unwrap();
        assert_eq!(
            args.last().map(String::as_str),
            Some("fps=2,cropdetect=limit=24:round=2:skip=0:reset=0,metadata=mode=print")
        );
        let bad = BlackBarDetect {
            limit: 300,
            ..BlackBarDetect::default()
        };
        assert!(bad.args(&sample_info()).is_err());
    }

    #[test]
    fn letterbox_is_detected_as_union_of_frames() {
        // sample_info 为 1280x720
        let text = format!("{}{}", frame(0, 100, 1280, 500), frame(0, 90, 1280, 520));
        let rect = BlackBarDetect::default().parse(&lines(&text), &sample_info());
        assert_eq!(
            rect,
            Some(CropRect {
                x: 0,
                y: 90,
                width: 1280,
                height: 520
            })
        );
        assert_eq!(rect.unwrap().filter(), "crop=1280:520:0:90");
    }

    fn exact(x1: u32, y1: u32, x2: u32, y2: u32) -> String {
        // cropdetect 的取整结果（偏移对齐到偶数）与精确边界同时输出
        format!(
            "[Parsed_metadata_2 @ 0] frame:1 pts:1 pts_time:0.5\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.x1={x1}\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.x2={x2}\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.y1={y1}\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.y2={y2}\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.w={}\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.h={}\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.x={}\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.y={}\n",
            (x2 - x1 + 1) & !1,
            (y2 - y1 + 1) & !1,
            (x1 + 1) & !1,
            (y1 + 1) & !1,
        )
    }

    #[test]
    fn odd_offsets_are_cropped_tightly() {
        // 1280x720，上黑边 91 行，画面 538 行（91..=628）：精确裁剪，偏移为奇数
        let rect = BlackBarDetect::default()
            .parse(&lines(&exact(0, 91, 1279, 628)), &sample_info())
            .unwrap();
        assert_eq!((rect.y, rect.height), (91, 538));
        assert_eq!(rect.filter(), "crop=1280:538:0:91:exact=1");
        // 画面 537 行（91..=627）：高度取偶时向黑边多取 1 行，不裁掉画面
        let rect = BlackBarDetect::default()
            .parse(&lines(&exact(0, 91, 1279, 627)), &sample_info())
            .unwrap();
        assert_eq!((rect.y, rect.height), (91, 538));
        // 左右奇数偏移（柱状黑边）
        let rect = BlackBarDetect::default()
            .parse(&lines(&exact(161, 0, 1118, 719)), &sample_info())
            .unwrap();
        assert_eq!(
            (rect.x, rect.width, rect.y, rect.height),
            (161, 958, 0, 720)
        );
        // 偶数偏移不加 exact
        assert_eq!(
            BlackBarDetect::default()
                .parse(&lines(&exact(0, 90, 1279, 629)), &sample_info())
                .unwrap()
                .filter(),
            "crop=1280:540:0:90"
        );
        // 扩展的方向：富余多的一侧；两侧都没有富余时缩短
        assert_eq!(fit_span(10, 5, 100, 2), (10, 6));
        assert_eq!(fit_span(10, 5, 15, 2), (9, 6));
        assert_eq!(fit_span(0, 5, 5, 2), (0, 4));
    }

    #[test]
    fn no_bars_or_garbage_gives_none() {
        let full = BlackBarDetect::default().parse(&lines(&frame(0, 0, 1280, 720)), &sample_info());
        assert_eq!(full, None);
        let tiny = BlackBarDetect::default().parse(&lines(&frame(0, 1, 1280, 718)), &sample_info());
        assert_eq!(tiny, None, "总共 2 像素的边忽略");
        let garbage = lines(
            "[Parsed_metadata_2 @ 0] frame:1 pts:1 pts_time:0.5\n\
             [Parsed_metadata_2 @ 0] lavfi.cropdetect.w=-1278\n",
        );
        assert_eq!(
            BlackBarDetect::default().parse(&garbage, &sample_info()),
            None
        );
    }
}
