//! `tool.remove-silence` 静音移除：检测静音片段（`silencedetect`），移除它们或按倍率加速播放，音画保持同步。
//!
//! - 第 1 步 [`SilenceDetect`]（第一条音轨；没有音轨时直接报错）；
//! - 每段静音两端各留出“保留边距”（贴着开头 / 结尾的一侧不留），让语音前后不被切得太紧（[`pad_silences`]）；
//! - 第 2 步交给共享引擎（[`crate::tools::remove_remix`]）单遍编码：
//!   - 移除：保留的区间 `split` + `trim`/`atrim` + `concat`；
//!   - 加速：静音片段视频 `setpts=(PTS-STARTPTS)/倍率`、音频 `atempo=倍率`，其余原速，`concat` 后统一 `fps` 回源帧率。
//!     `concat` 以每段里较长的那条流为准对齐下一段，所以 `atempo` 的细小误差不会累积成不同步。
//! - 几百段静音也只是一个滤镜脚本文件（`-/filter_complex`），不会超出命令行长度；
//!   不用 `aselect` 是因为它只能按整个音频帧取舍，剪切多了会累积偏移。

use serde::{Deserialize, Serialize};

use crate::pipeline::fmt_num;
use crate::tools::analysis::{complement, normalize_intervals, Interval, SilenceDetect};
use crate::tools::common::cutlist::{keep_pieces, render, snap_pieces, Piece, SourceFormat, Timeline};
use crate::tools::{Tool, ToolContext, ToolError};

pub const ID: &str = "tool.remove-silence";

/// 静音片段的处理方式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SilenceMode {
    /// 移除
    #[default]
    Remove,
    /// 加速播放
    Speed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoveSilenceParams {
    /// 低于此音量（dB）视为静音
    pub noise_db: f64,
    /// 静音至少这么久（秒）才处理
    pub min_duration: f64,
    /// 每段静音两端保留的时长（秒），避免切得太紧
    pub padding_seconds: f64,
    pub mode: SilenceMode,
    /// 加速模式的倍率（1～100）
    pub speed: f64,
}

impl Default for RemoveSilenceParams {
    fn default() -> Self {
        Self {
            noise_db: -30.0,
            min_duration: 0.5,
            padding_seconds: 0.1,
            mode: SilenceMode::Remove,
            speed: 4.0,
        }
    }
}

impl RemoveSilenceParams {
    fn validate(&self) -> Result<(), String> {
        if !(self.noise_db.is_finite() && (-100.0..0.0).contains(&self.noise_db)) {
            return Err("静音阈值必须在 -100～0 dB 之间（不含 0）".into());
        }
        if !(self.min_duration.is_finite() && self.min_duration >= 0.1) {
            return Err("最短静音时长至少 0.1 秒".into());
        }
        if !(self.padding_seconds.is_finite() && self.padding_seconds >= 0.0) {
            return Err("保留边距不能为负数".into());
        }
        if self.mode == SilenceMode::Speed
            && !(self.speed.is_finite() && self.speed > 1.0 && self.speed <= 100.0)
        {
            return Err("加速倍率必须大于 1、不超过 100".into());
        }
        Ok(())
    }

    fn detect(&self) -> SilenceDetect {
        SilenceDetect {
            noise_db: self.noise_db,
            min_duration: self.min_duration,
        }
    }
}

/// 每段静音两端各缩进 `padding`（贴着视频开头 / 结尾的一侧不缩），缩没了的丢弃。
pub fn pad_silences(silences: &[Interval], padding: f64, duration: f64) -> Vec<Interval> {
    let padded = silences
        .iter()
        .map(|s| {
            let start = if s.start <= 1e-6 {
                0.0
            } else {
                s.start + padding
            };
            let end = if s.end >= duration - 1e-6 {
                duration
            } else {
                s.end - padding
            };
            Interval::new(start, end)
        })
        .collect();
    normalize_intervals(padded, duration)
}

/// 由（已加边距的）静音区间生成片段：移除模式只留有声部分，加速模式静音部分按倍率加速。
pub fn silence_pieces(
    silences: &[Interval],
    mode: SilenceMode,
    speed: f64,
    format: &SourceFormat,
) -> Result<Vec<Piece>, String> {
    let duration = format.duration;
    let pieces = match mode {
        SilenceMode::Remove => keep_pieces(&complement(silences, duration, 0.0), format),
        SilenceMode::Speed => {
            let mut raw = Vec::with_capacity(silences.len() * 2 + 1);
            let mut cursor = 0.0;
            for s in normalize_intervals(silences.to_vec(), duration) {
                if s.start > cursor {
                    raw.push(Piece::main(cursor, s.start));
                }
                raw.push(Piece::Main {
                    start: s.start,
                    end: s.end,
                    speed,
                    mirror: false,
                });
                cursor = s.end;
            }
            if duration > cursor {
                raw.push(Piece::main(cursor, duration));
            }
            snap_pieces(raw, format)
        }
    };
    if pieces.is_empty() {
        Err("整个视频都是静音，移除后没有剩余内容".into())
    } else {
        Ok(pieces)
    }
}

impl Tool for RemoveSilenceParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let info = ctx.info(0)?;
        if info.audio.is_empty() {
            return Err("视频没有音轨，无法检测静音".into());
        }
        let format = SourceFormat::from_info(&info)?;
        ctx.set_steps(&[1.0, 3.0]);
        let detected = ctx.analyze_input(0, &self.detect())?;
        let silences = pad_silences(&detected, self.padding_seconds, format.duration);
        if silences.is_empty() {
            ctx.warn("没有检测到需要处理的静音，输出内容与原视频相同（可提高静音阈值或缩短最短静音时长）");
        } else {
            let total: f64 = silences.iter().map(Interval::duration).sum();
            ctx.note(format!(
                "检测到 {} 段静音，{}共 {} 秒",
                silences.len(),
                match self.mode {
                    SilenceMode::Remove => "移除".to_owned(),
                    SilenceMode::Speed => format!("按 {} 倍加速，", fmt_num(self.speed)),
                },
                fmt_num(total)
            ));
        }
        let pieces = silence_pieces(&silences, self.mode, self.speed, &format)?;
        render(ctx, &Timeline::cut(pieces), &[], "静音移除")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::remove_remix::build_graph;
    use crate::tools::remove_remix::tests::sample_format;

    #[test]
    fn padding_keeps_speech_edges() {
        let silences = [
            Interval::new(0.0, 1.0),
            Interval::new(3.0, 5.0),
            Interval::new(6.0, 6.3),
            Interval::new(9.0, 10.0),
        ];
        assert_eq!(
            pad_silences(&silences, 0.2, 10.0),
            [
                Interval::new(0.0, 0.8),
                Interval::new(3.2, 4.8),
                Interval::new(9.2, 10.0)
            ],
            "开头 / 结尾一侧不留边距，0.3 秒的静音缩没了"
        );
        assert_eq!(pad_silences(&silences, 0.0, 10.0), silences);
    }

    #[test]
    fn remove_mode_keeps_sound() {
        let format = sample_format();
        let pieces = silence_pieces(
            &[Interval::new(0.0, 0.8), Interval::new(3.2, 4.8)],
            SilenceMode::Remove,
            4.0,
            &format,
        )
        .unwrap();
        assert_eq!(pieces, [Piece::main(0.8, 3.2), Piece::main(4.8, 10.0)]);
        assert!((Timeline::cut(pieces).duration() - 7.6).abs() < 1e-9);
        assert!(silence_pieces(
            &[Interval::new(0.0, 10.0)],
            SilenceMode::Remove,
            4.0,
            &format
        )
        .is_err());
    }

    #[test]
    fn speed_mode_speeds_up_silence() {
        let format = sample_format();
        let pieces =
            silence_pieces(&[Interval::new(2.0, 6.0)], SilenceMode::Speed, 4.0, &format).unwrap();
        assert_eq!(
            pieces,
            [
                Piece::main(0.0, 2.0),
                Piece::Main {
                    start: 2.0,
                    end: 6.0,
                    speed: 4.0,
                    mirror: false
                },
                Piece::main(6.0, 10.0)
            ]
        );
        let t = Timeline::cut(pieces);
        assert_eq!(t.duration(), 7.0);
        let g = build_graph(&t, &format).unwrap();
        assert!(g.filter.contains(
            "[s1]trim=start=2:end=6,setpts=(PTS-STARTPTS)/4[v1];\n\
             [t1]atrim=start=2:end=6,asetpts=PTS-STARTPTS,atempo=4[a1]"
        ));
        assert!(g
            .filter
            .ends_with("concat=n=3:v=1:a=1[vc][aout];\n[vc]fps=25[vout]"));
        // 全片静音：整段加速
        let all = silence_pieces(
            &[Interval::new(0.0, 10.0)],
            SilenceMode::Speed,
            2.0,
            &format,
        )
        .unwrap();
        assert_eq!(Timeline::cut(all).duration(), 5.0);
    }

    #[test]
    fn hundreds_of_silences() {
        let format = SourceFormat {
            duration: 1000.0,
            ..sample_format()
        };
        let silences: Vec<Interval> = (0..400)
            .map(|i| Interval::new(i as f64 * 2.0 + 1.0, i as f64 * 2.0 + 2.0))
            .collect();
        let padded = pad_silences(&silences, 0.2, format.duration);
        assert_eq!(padded.len(), 400);
        assert_eq!(padded[0], Interval::new(1.2, 1.8));
        let pieces = silence_pieces(&padded, SilenceMode::Remove, 4.0, &format).unwrap();
        assert_eq!(pieces.len(), 401);
        let t = Timeline::cut(pieces);
        assert!((t.duration() - (1000.0 - 400.0 * 0.6)).abs() < 1e-6);
        let g = build_graph(&t, &format).unwrap();
        assert!(g.filter.contains("concat=n=401:v=1:a=1[vout][aout]"));
        let sped = silence_pieces(&padded, SilenceMode::Speed, 4.0, &format).unwrap();
        assert_eq!(sped.len(), 801);
    }

    #[test]
    fn params() {
        let ok = RemoveSilenceParams::default();
        assert!(ok.validate().is_ok());
        assert_eq!(
            ok.detect(),
            SilenceDetect {
                noise_db: -30.0,
                min_duration: 0.5
            }
        );
        for bad in [
            RemoveSilenceParams {
                noise_db: 5.0,
                ..ok.clone()
            },
            RemoveSilenceParams {
                padding_seconds: -0.1,
                ..ok.clone()
            },
            RemoveSilenceParams {
                mode: SilenceMode::Speed,
                speed: 1.0,
                ..ok.clone()
            },
            RemoveSilenceParams {
                mode: SilenceMode::Speed,
                speed: 200.0,
                ..ok.clone()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            serde_json::json!({
                "noiseDb": -30.0,
                "minDuration": 0.5,
                "paddingSeconds": 0.1,
                "mode": "remove",
                "speed": 4.0
            })
        );
    }
}
