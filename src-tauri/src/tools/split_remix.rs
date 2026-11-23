//! `tool.split-remix` 拆分重组：在设定位置（时间点 / 百分比 / 均分）把视频拆开，每处插入一段视频（单个文件，
//! 或从目录随机 / 依次取），再拼成一个视频。
//!
//! 插入的视频统一成主视频的规格：等比缩放到主视频的显示尺寸并居中补黑边、帧率转成主视频帧率、
//! 音频重采样为主视频的采样率与声道布局（没有音频时补静音），因此可以直接用 concat 滤镜拼接。
//! 拼接用 [`crate::tools::remove_remix`] 里的共享引擎：单遍 filter_complex（`split` + `trim` + `concat`，
//! 插入的视频作为额外输入），只编码一次；理由见该文件开头。
//!
//! 位置的解析 [`resolve_positions`] 与移除重组共用。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::fsutil::VIDEO_EXTENSIONS;
use crate::model::asset::pick_from;
use crate::model::{AssetSource, PickOrder, SeededRng};
use crate::pipeline::fmt_num;
use crate::tools::common::cutlist::{render, snap_pieces, Piece, SourceFormat, Timeline};
use crate::tools::{Tool, ToolContext, ToolError};

pub const ID: &str = "tool.split-remix";

// 位置的解析已移到 `tools::common::positions`，这里保留原名的再导出。
pub use crate::tools::common::positions::{resolve_positions, PositionMode, Positions, MAX_POSITIONS};

/// 素材来源的取法（单个文件时总是那一个）。
pub fn asset_order(source: &AssetSource) -> PickOrder {
    match source {
        AssetSource::File { .. } => PickOrder::Sequential,
        AssetSource::Directory { order, .. } => *order,
    }
}

/// 为 `count` 个插入点各选一个视频：随机取时每处独立随机；依次取时接着上一个工作项往下取
/// （第 `number` 个视频的第 k 处用第 `(number-1)×count + k` 个，取完从头循环）。
pub fn pick_inserts(
    candidates: &[PathBuf],
    order: PickOrder,
    number: usize,
    count: usize,
    rng: &mut SeededRng,
) -> Vec<PathBuf> {
    if candidates.is_empty() {
        return Vec::new();
    }
    let base = number.saturating_sub(1) * count;
    (0..count)
        .map(|k| pick_from(candidates, order, base + k, rng).clone())
        .collect()
}

/// 插入的一段视频。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InsertClip {
    pub duration: f64,
    pub has_audio: bool,
}

/// 拆开主视频，在第 k 个位置插入第 k 段（额外输入 k）。位置为 0 时插在最前面。
pub fn split_timeline(points: &[f64], clips: &[InsertClip], duration: f64) -> Vec<Piece> {
    let mut pieces = Vec::with_capacity(points.len() * 2 + 1);
    let mut cursor = 0.0;
    for (k, (point, clip)) in points.iter().zip(clips).enumerate() {
        pieces.push(Piece::main(cursor, *point));
        pieces.push(Piece::Insert {
            input: k,
            duration: clip.duration,
            has_audio: clip.has_audio,
        });
        cursor = *point;
    }
    pieces.push(Piece::main(cursor, duration));
    pieces
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SplitRemixParams {
    /// 拆分位置的给法
    pub position_mode: PositionMode,
    /// 时间点（秒）
    pub times: Vec<f64>,
    /// 百分比（0～100）
    pub percents: Vec<f64>,
    /// 均分成几份
    pub parts: u32,
    /// 插入的视频：单个文件或目录
    pub insert: AssetSource,
}

impl Default for SplitRemixParams {
    fn default() -> Self {
        Self {
            position_mode: PositionMode::Parts,
            times: vec![10.0],
            percents: vec![50.0],
            parts: 2,
            insert: AssetSource::default(),
        }
    }
}

impl Tool for SplitRemixParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let info = ctx.info(0)?;
        let format = SourceFormat::from_info(&info)?;
        let positions = resolve_positions(
            self.position_mode,
            &self.times,
            &self.percents,
            self.parts,
            format.duration,
        )?;
        if positions.ignored > 0 {
            ctx.warn(format!(
                "有 {} 个位置超出视频时长，已忽略",
                positions.ignored
            ));
        }
        if self.insert.is_empty() {
            return Err("请选择要插入的视频（文件或目录）".into());
        }
        let candidates = self.insert.candidates(VIDEO_EXTENSIONS)?;
        let number = ctx.number();
        let chosen = pick_inserts(
            &candidates,
            asset_order(&self.insert),
            number,
            positions.points.len(),
            &mut ctx.rng,
        );
        let mut clips = Vec::with_capacity(chosen.len());
        for path in &chosen {
            ctx.check_cancelled()?;
            let clip = ctx.probe(path)?;
            if clip.video.is_none() || !(clip.duration.is_finite() && clip.duration > 0.0) {
                return Err(format!(
                    "插入的视频无法使用（没有画面或读不到时长）：{}",
                    path.display()
                )
                .into());
            }
            clips.push(InsertClip {
                duration: clip.duration,
                has_audio: !clip.audio.is_empty(),
            });
        }
        let names: Vec<String> = positions
            .points
            .iter()
            .zip(&chosen)
            .map(|(t, p)| {
                format!(
                    "{} 秒处插入 {}",
                    fmt_num(*t),
                    p.file_name().unwrap_or_default().to_string_lossy()
                )
            })
            .collect();
        ctx.note(names.join("；"));
        let pieces = snap_pieces(
            split_timeline(&positions.points, &clips, format.duration),
            &format,
        );
        render(ctx, &Timeline::cut(pieces), &chosen, "拆分重组")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::remove_remix::tests::sample_format;
    use crate::tools::common::cutlist::{build_graph, AudioOut};

    #[test]
    fn positions_by_mode() {
        let r = |mode, times: &[f64], percents: &[f64], parts| {
            resolve_positions(mode, times, percents, parts, 10.0)
        };
        assert_eq!(
            r(PositionMode::Times, &[7.0, 2.0, 2.0, 12.0, 0.0], &[], 0).unwrap(),
            Positions {
                points: vec![0.0, 2.0, 7.0],
                ignored: 1
            }
        );
        assert_eq!(
            r(PositionMode::Percents, &[], &[25.0, 100.0], 0).unwrap(),
            Positions {
                points: vec![2.5],
                ignored: 1
            }
        );
        assert_eq!(
            r(PositionMode::Parts, &[], &[], 4).unwrap().points,
            [2.5, 5.0, 7.5]
        );
        assert!(r(PositionMode::Times, &[], &[], 0).is_err());
        assert!(r(PositionMode::Times, &[-1.0], &[], 0).is_err());
        assert!(
            r(PositionMode::Times, &[10.0, 11.0], &[], 0).is_err(),
            "全部超出"
        );
        assert!(r(PositionMode::Percents, &[120.0], &[], 0).is_err());
        assert!(r(PositionMode::Percents, &[], &[], 0).is_err());
        assert!(r(PositionMode::Parts, &[], &[], 1).is_err());
        assert!(r(PositionMode::Parts, &[], &[], 500).is_err());
        assert!(resolve_positions(PositionMode::Parts, &[], &[], 2, 0.0).is_err());
    }

    #[test]
    fn inserts_are_picked_by_order() {
        let files: Vec<PathBuf> = ["a.mp4", "b.mp4", "c.mp4"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let mut rng = SeededRng::new(5);
        assert_eq!(
            pick_inserts(&files, PickOrder::Sequential, 1, 2, &mut rng),
            [PathBuf::from("a.mp4"), PathBuf::from("b.mp4")]
        );
        assert_eq!(
            pick_inserts(&files, PickOrder::Sequential, 2, 2, &mut rng),
            [PathBuf::from("c.mp4"), PathBuf::from("a.mp4")],
            "第二个视频接着往下取"
        );
        let a = pick_inserts(&files, PickOrder::Random, 1, 10, &mut SeededRng::new(9));
        let b = pick_inserts(&files, PickOrder::Random, 1, 10, &mut SeededRng::new(9));
        assert_eq!(a, b);
        assert!(a.iter().any(|p| p != &a[0]), "随机取会取到不同的文件");
        assert!(pick_inserts(&[], PickOrder::Random, 1, 2, &mut rng).is_empty());
        assert_eq!(
            asset_order(&AssetSource::Directory {
                path: "d".into(),
                recursive: false,
                order: PickOrder::Random
            }),
            PickOrder::Random
        );
    }

    #[test]
    fn timeline_interleaves_inserts() {
        let clips = [
            InsertClip {
                duration: 1.0,
                has_audio: true,
            },
            InsertClip {
                duration: 2.0,
                has_audio: false,
            },
        ];
        let pieces = split_timeline(&[0.0, 4.0], &clips, 10.0);
        assert_eq!(
            snap_pieces(pieces, &sample_format()),
            [
                Piece::Insert {
                    input: 0,
                    duration: 1.0,
                    has_audio: true
                },
                Piece::main(0.0, 4.0),
                Piece::Insert {
                    input: 1,
                    duration: 2.0,
                    has_audio: false
                },
                Piece::main(4.0, 10.0),
            ],
            "插在开头时前面的空片段被丢弃"
        );
        let t = Timeline::cut(split_timeline(&[5.0], &clips[..1], 10.0));
        assert_eq!(t.duration(), 11.0);
        let g = build_graph(&t, &sample_format()).unwrap();
        assert_eq!(g.audio, AudioOut::Filtered);
        assert!(g
            .filter
            .ends_with("[v0][a0][v1][a1][v2][a2]concat=n=3:v=1:a=1[vout][aout]"));
        assert!(g
            .filter
            .contains("[1:V:0]scale=trunc(iw*sar/2)*2:ih,setsar=1,scale=1280:720:force_original_aspect_ratio=decrease"));
    }

    #[test]
    fn params_defaults() {
        assert_eq!(
            serde_json::to_value(SplitRemixParams::default()).unwrap(),
            serde_json::json!({
                "positionMode": "parts",
                "times": [10.0],
                "percents": [50.0],
                "parts": 2,
                "insert": {"mode": "file", "path": ""}
            })
        );
    }
}
