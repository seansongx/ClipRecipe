//! `tool.remove-still` 静止帧移除：检测画面静止的片段（`freezedetect`），把它们连同对应的音频一起移除，
//! 拼接剩余部分。
//!
//! - 第 1 步 [`FreezeDetect`]：帧间差异低于“灵敏度”（噪声阈值，dB）且持续至少“最短静止时长”的区间；
//!   区间从静止开始后的第一帧算起，静止前的最后一帧保留，所以画面不会突兀地跳过；
//! - 第 2 步保留区间（`analysis::complement`）对齐帧网格后交给共享引擎
//!   （[`crate::tools::remove_remix`]）：单遍 `split` + `trim`/`atrim` + `concat`，音画逐段等长、不累积偏移；
//!   静止片段再多也只是一个滤镜脚本文件，不会超出命令行长度。

use serde::{Deserialize, Serialize};

use crate::pipeline::fmt_num;
use crate::tools::analysis::{complement, FreezeDetect, Interval};
use crate::tools::common::cutlist::{keep_pieces, render, Piece, SourceFormat, Timeline};
use crate::tools::{Tool, ToolContext, ToolError};

pub const ID: &str = "tool.remove-still";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoveStillParams {
    /// 灵敏度：帧间差异低于此值（dB）视为没有变化；越接近 0 越容易判为静止
    pub noise_db: f64,
    /// 静止至少这么久（秒）才移除
    pub min_duration: f64,
}

impl Default for RemoveStillParams {
    fn default() -> Self {
        Self {
            noise_db: -60.0,
            min_duration: 1.0,
        }
    }
}

impl RemoveStillParams {
    fn validate(&self) -> Result<(), String> {
        if !(self.noise_db.is_finite() && (-100.0..0.0).contains(&self.noise_db)) {
            return Err("灵敏度必须在 -100～0 dB 之间（不含 0）".into());
        }
        if !(self.min_duration.is_finite() && self.min_duration >= 0.1) {
            return Err("最短静止时长至少 0.1 秒".into());
        }
        Ok(())
    }

    fn detect(&self) -> FreezeDetect {
        FreezeDetect {
            noise_db: self.noise_db,
            min_duration: self.min_duration,
        }
    }
}

/// 去掉静止区间后保留的片段；整个视频都静止时报错。
pub fn still_pieces(stills: &[Interval], format: &SourceFormat) -> Result<Vec<Piece>, String> {
    let pieces = keep_pieces(&complement(stills, format.duration, 0.0), format);
    if pieces.is_empty() {
        Err("整个视频都是静止画面，移除后没有剩余内容".into())
    } else {
        Ok(pieces)
    }
}

impl Tool for RemoveStillParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let info = ctx.info(0)?;
        let format = SourceFormat::from_info(&info)?;
        ctx.set_steps(&[1.0, 3.0]);
        let stills = ctx.analyze_input(0, &self.detect())?;
        if stills.is_empty() {
            ctx.warn("没有检测到静止画面，输出内容与原视频相同（可提高灵敏度或缩短最短静止时长）");
        } else {
            let total: f64 = stills.iter().map(Interval::duration).sum();
            ctx.note(format!(
                "检测到 {} 段静止画面，共 {} 秒",
                stills.len(),
                fmt_num(total)
            ));
        }
        let pieces = still_pieces(&stills, &format)?;
        render(ctx, &Timeline::cut(pieces), &[], "静止帧移除")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::remove_remix::tests::sample_format;
    use crate::tools::common::cutlist::{build_graph, AudioOut};

    #[test]
    fn stills_are_cut_out_with_their_audio() {
        let format = sample_format();
        let pieces = still_pieces(
            &[Interval::new(2.01, 5.0), Interval::new(8.5, 10.0)],
            &format,
        )
        .unwrap();
        assert_eq!(pieces, [Piece::main(0.0, 2.0), Piece::main(5.0, 8.52)]);
        let t = Timeline::cut(pieces);
        assert!((t.duration() - 5.52).abs() < 1e-9);
        let g = build_graph(&t, &format).unwrap();
        assert_eq!(g.audio, AudioOut::Filtered);
        assert!(g
            .filter
            .contains("[t1]atrim=start=5:end=8.52,asetpts=PTS-STARTPTS[a1]"));
    }

    #[test]
    fn edge_cases() {
        let format = sample_format();
        // 没有静止：整段
        assert_eq!(
            still_pieces(&[], &format).unwrap(),
            [Piece::main(0.0, 10.0)]
        );
        // 开头就是静止画面
        assert_eq!(
            still_pieces(&[Interval::new(0.0, 3.0)], &format).unwrap(),
            [Piece::main(3.0, 10.0)]
        );
        // 全片静止
        assert!(still_pieces(&[Interval::new(0.0, 10.0)], &format).is_err());
        // 10 分钟里 300 段静止（每 2 秒中静止 0.6 秒）
        let long = SourceFormat {
            duration: 600.0,
            ..format
        };
        let many: Vec<Interval> = (0..300)
            .map(|i| Interval::new(i as f64 * 2.0 + 1.0, i as f64 * 2.0 + 1.6))
            .collect();
        let pieces = still_pieces(&many, &long).unwrap();
        assert_eq!(pieces.len(), 301);
        assert_eq!(pieces[1], Piece::main(1.6, 3.0));
        let t = Timeline::cut(pieces);
        assert!((t.duration() - 420.0).abs() < 1e-6);
        let g = build_graph(&t, &long).unwrap();
        assert!(g.filter.contains("concat=n=301:v=1:a=1[vout][aout]"));
    }

    #[test]
    fn params() {
        let ok = RemoveStillParams::default();
        assert!(ok.validate().is_ok());
        assert_eq!(
            ok.detect(),
            FreezeDetect {
                noise_db: -60.0,
                min_duration: 1.0
            }
        );
        assert!(RemoveStillParams {
            noise_db: 0.0,
            ..ok.clone()
        }
        .validate()
        .is_err());
        assert!(RemoveStillParams {
            min_duration: 0.0,
            ..ok.clone()
        }
        .validate()
        .is_err());
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            serde_json::json!({"noiseDb": -60.0, "minDuration": 1.0})
        );
    }
}
