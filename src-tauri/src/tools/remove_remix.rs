//! `tool.remove-remix` 移除重组：在设定位置（时间点 / 百分比 / 均分点）起移除一段（范围值，每处随机取）后拼接剩余部分。
//!
//! 用共享的片段列表引擎（[`crate::tools::common::cutlist`]）单遍编码；位置解析见 [`crate::tools::common::positions`]。

use serde::{Deserialize, Serialize};

use crate::model::{Range, SeededRng};
use crate::pipeline::fmt_num;
use crate::tools::analysis::{complement, normalize_intervals, Interval};
use crate::tools::common::positions::{resolve_positions, PositionMode};
use crate::tools::{Tool, ToolContext, ToolError};

pub const ID: &str = "tool.remove-remix";

// 共享引擎（片段列表 → 输出）已移到 `tools::common::cutlist`，这里保留原名的再导出。
pub use crate::tools::common::cutlist::*;

// ======================================================================
// 移除重组
// ======================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoveRemixParams {
    /// 位置的给法：时间点 / 百分比 / 均分
    pub position_mode: PositionMode,
    /// 时间点（秒）
    pub times: Vec<f64>,
    /// 百分比（0～100）
    pub percents: Vec<f64>,
    /// 均分：把视频分成几份，在每个分界处移除
    pub parts: u32,
    /// 每处移除的长度（秒，范围值，每处随机取）
    pub length: Range<f64>,
}

impl Default for RemoveRemixParams {
    fn default() -> Self {
        Self {
            position_mode: PositionMode::Percents,
            times: vec![10.0],
            percents: vec![50.0],
            parts: 2,
            length: Range::fixed(1.0),
        }
    }
}

/// 每处移除的长度（按位置顺序各取一次）。
pub fn sample_lengths(
    length: &Range<f64>,
    count: usize,
    rng: &mut SeededRng,
) -> Result<Vec<f64>, String> {
    let (lo, hi) = length.ordered();
    if !(lo.is_finite() && hi.is_finite() && lo > 0.0) {
        return Err("移除长度必须大于 0 秒".into());
    }
    Ok((0..count)
        .map(|_| (length.sample(rng) * 1000.0).round() / 1000.0)
        .collect())
}

/// 从每个位置起移除对应长度，返回要移除的区间（已限幅、合并）。
pub fn plan_removals(positions: &[f64], lengths: &[f64], duration: f64) -> Vec<Interval> {
    normalize_intervals(
        positions
            .iter()
            .zip(lengths)
            .map(|(p, len)| Interval::new(*p, p + len))
            .collect(),
        duration,
    )
}

/// 移除后保留的区间；全部被移除时报错。
pub fn kept_after_removal(removals: &[Interval], duration: f64) -> Result<Vec<Interval>, String> {
    let keep = complement(removals, duration, 0.0);
    if keep.is_empty() {
        Err("要移除的部分覆盖了整个视频，没有剩余内容".into())
    } else {
        Ok(keep)
    }
}

impl Tool for RemoveRemixParams {
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
        let positions = positions.points;
        let lengths = sample_lengths(&self.length, positions.len(), &mut ctx.rng)?;
        let removals = plan_removals(&positions, &lengths, format.duration);
        let keep = kept_after_removal(&removals, format.duration)?;
        let removed: f64 = removals.iter().map(Interval::duration).sum();
        ctx.note(format!(
            "在 {} 处共移除 {} 秒",
            positions.len(),
            fmt_num(removed)
        ));
        let pieces = keep_pieces(&keep, &format);
        render(ctx, &Timeline::cut(pieces), &[], "移除重组")
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ffmpeg::{VideoCodec, VideoEncoder};
    use crate::tools::Encoding;
    use crate::pipeline::testing::sample_info;

    /// sample_info：1280x720、25fps、10 秒、双声道 48 kHz
    pub fn sample_format() -> SourceFormat {
        SourceFormat::from_info(&sample_info()).unwrap()
    }

    fn silent_format() -> SourceFormat {
        SourceFormat {
            audio: None,
            ..sample_format()
        }
    }

    #[test]
    fn source_format_from_info() {
        let f = sample_format();
        assert_eq!(
            f,
            SourceFormat {
                width: 1280,
                height: 720,
                fps: 25.0,
                rate: "25".into(),
                duration: 10.0,
                audio: Some(AudioFormat {
                    sample_rate: 48_000,
                    channels: 2
                }),
                anamorphic: false,
            }
        );
        // 变形源：按方形像素显示的尺寸，滤镜图开头先缩放一次
        let mut wide = sample_info();
        if let Some(v) = wide.video.as_mut() {
            (v.width, v.height, v.sar) = (720, 576, "64:45".into());
        }
        let f = SourceFormat::from_info(&wide).unwrap();
        assert_eq!((f.width, f.height, f.anamorphic), (1024, 576, true));
        let graph = build_graph(
            &Timeline::cut(vec![Piece::main(0.0, 4.0), Piece::main(6.0, 10.0)]),
            &f,
        )
        .unwrap();
        assert!(
            graph
                .filter
                .starts_with("[0:V:0]scale=1024:576,setsar=1[sq];\n[sq]split=2[s0][s1]"),
            "{}",
            graph.filter
        );
        let mut ntsc = sample_info();
        if let Some(v) = ntsc.video.as_mut() {
            v.fps_text = "30000/1001".into();
            v.rotation = 90;
        }
        let f = SourceFormat::from_info(&ntsc).unwrap();
        assert_eq!(
            (f.width, f.height, f.rate.as_str()),
            (720, 1280, "30000/1001")
        );
        assert!((f.fps - 29.97003).abs() < 1e-4);
        let mut none = sample_info();
        none.video = None;
        assert!(SourceFormat::from_info(&none).is_err());
        let mut zero = sample_info();
        zero.duration = 0.0;
        assert!(SourceFormat::from_info(&zero).is_err());
    }

    #[test]
    fn snapping_to_frames() {
        let f = sample_format();
        assert_eq!(f.snap(2.01), 2.0);
        assert_eq!(f.snap(2.03), 2.04);
        assert_eq!(f.snap(0.0000001), 0.0);
        assert_eq!(f.snap(9.99), 10.0);
        let pieces = snap_pieces(
            vec![
                Piece::main(0.0, 0.01),
                Piece::main(0.01, 3.33),
                Piece::Insert {
                    input: 0,
                    duration: 1.0,
                    has_audio: false,
                },
            ],
            &f,
        );
        assert_eq!(
            pieces,
            [
                Piece::main(0.0, 3.32),
                Piece::Insert {
                    input: 0,
                    duration: 1.0,
                    has_audio: false
                }
            ],
            "对齐后为空的片段被丢弃"
        );
        assert_eq!(ts(3.32), "3.32");
        assert_eq!(ts(0.0), "0");
        assert_eq!(ts(1.0343333333), "1.034333");
        assert_eq!(ts(12.0), "12");
    }

    #[test]
    fn cut_graph_with_audio() {
        let t = Timeline::cut(vec![Piece::main(0.0, 2.0), Piece::main(4.0, 10.0)]);
        let g = build_graph(&t, &sample_format()).unwrap();
        assert_eq!(g.audio, AudioOut::Filtered);
        assert_eq!(
            g.filter,
            "[0:V:0]split=2[s0][s1];\n\
             [0:a:0]asplit=2[t0][t1];\n\
             [s0]trim=end=2,setpts=PTS-STARTPTS[v0];\n\
             [t0]atrim=end=2,asetpts=PTS-STARTPTS[a0];\n\
             [s1]trim=start=4,setpts=PTS-STARTPTS[v1];\n\
             [t1]atrim=start=4,asetpts=PTS-STARTPTS[a1];\n\
             [v0][a0][v1][a1]concat=n=2:v=1:a=1[vout][aout]"
        );
        assert_eq!(t.duration(), 8.0);
    }

    #[test]
    fn single_piece_without_audio_is_renamed() {
        let t = Timeline::cut(vec![Piece::main(1.5, 8.0)]);
        let g = build_graph(&t, &silent_format()).unwrap();
        assert_eq!(g.audio, AudioOut::None);
        assert_eq!(
            g.filter,
            "[0:V:0]trim=start=1.5:end=8,setpts=PTS-STARTPTS[vout]"
        );
        let with_audio = build_graph(&t, &sample_format()).unwrap();
        assert_eq!(
            with_audio.filter,
            "[0:V:0]trim=start=1.5:end=8,setpts=PTS-STARTPTS[vout];\n\
             [0:a:0]atrim=start=1.5:end=8,asetpts=PTS-STARTPTS[aout]"
        );
    }

    #[test]
    fn continuous_mirror_keeps_source_audio() {
        let t = Timeline::cut(vec![
            Piece::main(0.0, 3.0),
            Piece::Main {
                start: 3.0,
                end: 10.0,
                speed: 1.0,
                mirror: true,
            },
        ]);
        assert!(t.is_continuous(10.0));
        let g = build_graph(&t, &sample_format()).unwrap();
        assert_eq!(g.audio, AudioOut::Source);
        assert_eq!(
            g.filter,
            "[0:V:0]split=2[s0][s1];\n\
             [s0]trim=end=3,setpts=PTS-STARTPTS[v0];\n\
             [s1]trim=start=3,setpts=PTS-STARTPTS,hflip[v1];\n\
             [v0][v1]concat=n=2:v=1:a=0[vout]"
        );
        assert!(
            !Timeline::cut(vec![Piece::main(0.0, 3.0), Piece::main(3.5, 10.0)]).is_continuous(10.0)
        );
        assert!(!Timeline::cut(vec![Piece::main(0.0, 9.0)]).is_continuous(10.0));
    }

    #[test]
    fn speed_and_insert_graph() {
        let t = Timeline::cut(vec![
            Piece::main(0.0, 2.0),
            Piece::Insert {
                input: 0,
                duration: 1.5,
                has_audio: true,
            },
            Piece::Insert {
                input: 1,
                duration: 1.0,
                has_audio: false,
            },
            Piece::Main {
                start: 2.0,
                end: 10.0,
                speed: 4.0,
                mirror: false,
            },
        ]);
        let g = build_graph(&t, &sample_format()).unwrap();
        assert_eq!(
            g.filter,
            "[0:V:0]split=2[s0][s1];\n\
             [0:a:0]asplit=2[t0][t1];\n\
             [s0]trim=end=2,setpts=PTS-STARTPTS,setsar=1[v0];\n\
             [t0]atrim=end=2,asetpts=PTS-STARTPTS[a0];\n\
             [1:V:0]scale=trunc(iw*sar/2)*2:ih,setsar=1,scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=25,trim=duration=1.5,setpts=PTS-STARTPTS[v1];\n\
             [1:a:0]aformat=sample_rates=48000:channel_layouts=stereo,apad,atrim=duration=1.5,asetpts=PTS-STARTPTS[a1];\n\
             [2:V:0]scale=trunc(iw*sar/2)*2:ih,setsar=1,scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=25,trim=duration=1,setpts=PTS-STARTPTS[v2];\n\
             anullsrc=r=48000:cl=stereo,atrim=duration=1[a2];\n\
             [s1]trim=start=2,setpts=(PTS-STARTPTS)/4,setsar=1[v3];\n\
             [t1]atrim=start=2,asetpts=PTS-STARTPTS,atempo=4[a3];\n\
             [v0][a0][v1][a1][v2][a2][v3][a3]concat=n=4:v=1:a=1[vc][aout];\n\
             [vc]fps=25[vout]"
        );
        assert_eq!(t.duration(), 6.5);
        let bad = Timeline::cut(vec![Piece::Main {
            start: 0.0,
            end: 1.0,
            speed: 200.0,
            mirror: false,
        }]);
        assert!(build_graph(&bad, &sample_format()).is_err());
        assert!(build_graph(&Timeline::cut(Vec::new()), &sample_format()).is_err());
    }

    #[test]
    fn crossfade_graph_offsets_and_validation() {
        let t = Timeline {
            pieces: vec![
                Piece::main(0.0, 3.0),
                Piece::main(3.0, 6.0),
                Piece::main(6.0, 10.0),
            ],
            join: Join::Crossfade {
                kinds: vec!["fade".into(), "wipeleft".into()],
                duration: 0.5,
            },
        };
        assert!(!t.is_continuous(10.0), "转场时音频要交叉淡化");
        let g = build_graph(&t, &sample_format()).unwrap();
        assert_eq!(
            g.filter,
            "[0:V:0]split=3[s0][s1][s2];\n\
             [0:a:0]asplit=3[t0][t1][t2];\n\
             [s0]trim=end=3,setpts=PTS-STARTPTS[v0];\n\
             [t0]atrim=end=3,asetpts=PTS-STARTPTS[a0];\n\
             [s1]trim=start=3:end=6,setpts=PTS-STARTPTS[v1];\n\
             [t1]atrim=start=3:end=6,asetpts=PTS-STARTPTS[a1];\n\
             [s2]trim=start=6,setpts=PTS-STARTPTS[v2];\n\
             [t2]atrim=start=6,asetpts=PTS-STARTPTS[a2];\n\
             [v0][v1]xfade=transition=fade:duration=0.5:offset=2.5[x1];\n\
             [x1][v2]xfade=transition=wipeleft:duration=0.5:offset=5[vout];\n\
             [a0][a1][a2]acrossfade=n=3:d=0.5[aout]"
        );
        assert_eq!(t.duration(), 9.0);
        assert_eq!(
            t.output_spans(),
            [
                Interval::new(0.0, 3.0),
                Interval::new(2.5, 5.5),
                Interval::new(5.0, 9.0)
            ]
        );
        // 中间片段需要 2 倍转场时长
        let short = Timeline {
            pieces: vec![
                Piece::main(0.0, 3.0),
                Piece::main(3.0, 3.8),
                Piece::main(3.8, 10.0),
            ],
            join: Join::Crossfade {
                kinds: vec!["fade".into(), "fade".into()],
                duration: 0.5,
            },
        };
        assert!(build_graph(&short, &sample_format()).is_err());
        let wrong_count = Timeline {
            join: Join::Crossfade {
                kinds: vec!["fade".into()],
                duration: 0.5,
            },
            ..t.clone()
        };
        assert!(build_graph(&wrong_count, &sample_format()).is_err());
    }

    #[test]
    fn hundreds_of_pieces_build_one_graph() {
        // 一小时的视频，每 10 秒保留 4 秒：300 段
        let f = SourceFormat {
            duration: 3600.0,
            ..sample_format()
        };
        let keep: Vec<Interval> = (0..300)
            .map(|i| Interval::new(i as f64 * 10.0 + 0.01, i as f64 * 10.0 + 4.01))
            .collect();
        let pieces = keep_pieces(&keep, &f);
        assert_eq!(pieces.len(), 300);
        assert_eq!(pieces[0], Piece::main(0.0, 4.0), "对齐帧网格");
        assert_eq!(pieces[299], Piece::main(2990.0, 2994.0));
        let t = Timeline::cut(pieces);
        assert!((t.duration() - 1200.0).abs() < 1e-6);
        let g = build_graph(&t, &f).unwrap();
        assert!(g.filter.starts_with("[0:V:0]split=300[s0][s1]"));
        assert!(g
            .filter
            .contains("[s299]trim=start=2990:end=2994,setpts=PTS-STARTPTS[v299]"));
        assert!(g
            .filter
            .ends_with("[v299][a299]concat=n=300:v=1:a=1[vout][aout]"));
        assert_eq!(g.filter.matches("atrim=").count(), 300);
        assert_eq!(g.filter.lines().count(), 2 + 600 + 1);
    }

    #[test]
    fn encode_args_map_graph_outputs() {
        let encoding = Encoding {
            encoder: VideoEncoder::software(VideoCodec::H264),
            video_kbps: Some(1000),
        };
        let f = sample_format();
        let graph = Graph {
            filter: String::new(),
            audio: AudioOut::Filtered,
        };
        let args = encode_args(
            "in.mp4",
            &["ins.mov".into()],
            "g.txt",
            &graph,
            &f,
            &encoding,
            "mp4",
            "out.mp4",
        );
        assert_eq!(
            args.join(" "),
            "-i in.mp4 -i ins.mov -/filter_complex g.txt -map [vout] -map [aout] -c:v libx264 -preset medium \
             -pix_fmt yuv420p -b:v 1000k -maxrate 1500k -bufsize 2000k -c:a aac -b:a 192k -sn -dn \
             -map_chapters -1 -movflags +faststart out.mp4"
        );
        let source = Graph {
            filter: String::new(),
            audio: AudioOut::Source,
        };
        let args = encode_args(
            "in.mp4",
            &[],
            "g.txt",
            &source,
            &f,
            &encoding,
            "mkv",
            "o.mkv",
        )
        .join(" ");
        assert!(args.contains("-map [vout] -map 0:a:0 -c:v"), "{args}");
        assert!(!args.contains("faststart"));
        let none = Graph {
            filter: String::new(),
            audio: AudioOut::None,
        };
        let args = encode_args(
            "in.mp4",
            &[],
            "g.txt",
            &none,
            &silent_format(),
            &encoding,
            "mp4",
            "o.mp4",
        )
        .join(" ");
        assert!(args.contains("-bufsize 2000k -an -sn"), "{args}");
    }

    #[test]
    fn removal_planning() {
        let mut rng = SeededRng::new(1);
        let fixed = sample_lengths(&Range::fixed(1.0), 3, &mut rng).unwrap();
        assert_eq!(fixed, [1.0, 1.0, 1.0]);
        let removals = plan_removals(&[2.0, 2.5, 9.5], &[1.0, 1.0, 1.0], 10.0);
        assert_eq!(
            removals,
            [Interval::new(2.0, 3.5), Interval::new(9.5, 10.0)],
            "重叠合并、超出结尾的截断"
        );
        assert_eq!(
            kept_after_removal(&removals, 10.0).unwrap(),
            [Interval::new(0.0, 2.0), Interval::new(3.5, 9.5)]
        );
        // 从开头移除
        assert_eq!(
            kept_after_removal(&plan_removals(&[0.0], &[2.0], 10.0), 10.0).unwrap(),
            [Interval::new(2.0, 10.0)]
        );
        // 覆盖全片
        assert!(kept_after_removal(&plan_removals(&[0.0], &[20.0], 10.0), 10.0).is_err());
        assert!(sample_lengths(&Range::fixed(0.0), 1, &mut rng).is_err());
    }

    #[test]
    fn random_lengths_are_seeded_and_in_range() {
        let range = Range::new(0.5, 2.0);
        let a = sample_lengths(&range, 20, &mut SeededRng::new(42)).unwrap();
        let b = sample_lengths(&range, 20, &mut SeededRng::new(42)).unwrap();
        assert_eq!(a, b);
        assert!(a.iter().all(|l| (0.5..=2.0).contains(l)));
        assert!(a.windows(2).any(|w| w[0] != w[1]));
    }

    #[test]
    fn params_defaults_and_serde() {
        let json = serde_json::to_value(RemoveRemixParams::default()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "positionMode": "percents",
                "times": [10.0],
                "percents": [50.0],
                "parts": 2,
                "length": {"min": 1.0, "max": 1.0}
            })
        );
        let p: RemoveRemixParams =
            serde_json::from_value(serde_json::json!({"positionMode": "times", "times": [1, 4]}))
                .unwrap();
        assert_eq!(p.times, [1.0, 4.0]);
        assert_eq!(p.length, Range::fixed(1.0));
    }
}
