//! `tool.merge` 视频合并：把队列中的视频（处理范围为“全部”或“选中”）按规则合成一个或多个视频。
//!
//! **合并引擎**（[`MergeOptions`]、[`merge_segments`]、[`encode_segments`] 等）在 [`crate::tools::common::merge_engine`]，
//! `tool.folder-merge`、`tool.clip-merge` 共用（这里保留同名再导出）。
//!
//! 参数与取值约定（docs/specs/tools.md 未规定的部分）：
//! - 分组 `grouping`：`all` 全部合成一个；`everyN` 每 `groupSize`（1～1000）个合成一个（最后一组可能不足）。
//!   `shuffle` 先按种子随机打乱顺序再分组。打乱在规划时完成（`req.rng`，由本次运行的种子派生、可复现），
//!   所以每组都是一个工作项（可并行），界面上显示的就是该组真正的视频与顺序。
//! - 方式 `mode`：
//!   - `lossless` 无损（默认）：concat 分离器直接复制。要求全部片段视频编码、分辨率、像素格式、帧率相同，
//!     音频有无、编码、采样率、声道数相同。不兼容时：`fallbackReencode`（默认开）改为重编码并警告，关闭则失败并说明原因。
//!     无损时不支持转场（忽略并警告）。容器按“目标格式”设置，装不下源编码时用 mp4 / mkv。
//!   - `reencode` 重编码：一条 FFmpeg 命令、concat 滤镜。每个片段统一到**第一个片段**的显示尺寸
//!     （按比例缩放、居中加黑边，宽高取偶）、帧率与音频布局（采样率、声道；第一个没有音频而其它有时用 48k 立体声），
//!     没有音频的片段补静音；每段音视频都裁 / 补到该片段的容器时长（取整到整帧：视频正好 N 帧、音频正好
//!     N / 帧率 秒），保证拼接后音画同步、不累积偏差。码率、编码器沿用框架设置。
//! - 转场（仅重编码）：`transition` 为 `none`（默认）、`random`（每个衔接处从 [`TRANSITIONS`] 中随机取）或其中之一；
//!   `transitionSeconds` 0.1～10 秒。视频用 `xfade`，音频用 `acrossfade` 同步交叉淡化，每个衔接处总时长减少一个转场时长。
//!   转场时长不小于最短片段时缩短为最短片段的一半（警告）。
//! - 中间视频 `middleEnabled` + `middle`（单个文件或目录；目录时按“随机 / 依次”为每个衔接处各取一个）：
//!   插入在每两个相邻视频之间，参与无损兼容性检查、尺寸统一与转场。
//! - 输出名：默认 `<第一个视频名>_merged`；`joinNames` 时为各源文件名（去除后的主干）以空格连接，超过
//!   [`MAX_JOINED_CHARS`] 个字符时截断。输出目录以组内第一个视频为准。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::tools::plan::ItemInput;
use crate::tools::{PlanRequest, Tool, ToolContext, ToolError, WorkItem};

pub const ID: &str = "tool.merge";

/// 每组视频个数上限。
pub const MAX_GROUP: u32 = 1000;

// 合并引擎已移到 `tools::common::merge_engine`，这里保留原名的再导出。
pub use crate::tools::common::merge_engine::*;

/// 分组方式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeGrouping {
    /// 全部合成一个
    #[default]
    All,
    /// 每 N 个合成一个
    EveryN,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MergeParams {
    pub grouping: MergeGrouping,
    /// 每组个数（`everyN`）
    pub group_size: u32,
    /// 按种子随机打乱顺序
    pub shuffle: bool,
    #[serde(flatten)]
    pub merge: MergeOptions,
}

impl Default for MergeParams {
    fn default() -> Self {
        Self {
            grouping: MergeGrouping::All,
            group_size: 2,
            shuffle: false,
            merge: MergeOptions::default(),
        }
    }
}

/// 校验每组个数。
pub fn validate_group_size(size: u32) -> Result<(), String> {
    if size == 0 || size > MAX_GROUP {
        return Err(format!("每组个数必须在 1～{MAX_GROUP} 之间"));
    }
    Ok(())
}

impl MergeParams {
    fn validate(&self) -> Result<(), String> {
        if self.grouping == MergeGrouping::EveryN {
            validate_group_size(self.group_size)?;
        }
        self.merge.validate()
    }

    /// 每组的大小（None：全部一组）。
    fn chunk(&self) -> Option<usize> {
        (self.grouping == MergeGrouping::EveryN).then_some(self.group_size as usize)
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl Tool for MergeParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn plan(&self, req: &PlanRequest) -> Result<Vec<WorkItem>, String> {
        self.validate()?;
        let videos = req.require_videos()?;
        let mut inputs: Vec<ItemInput> = videos.iter().map(ItemInput::from_task).collect();
        if self.shuffle {
            shuffle(&mut inputs, &mut req.rng(ID));
        }
        match self.chunk() {
            Some(size) => Ok(chunk_items(req.tool_id, inputs, size)),
            None => {
                if inputs.len() < 2 {
                    return Err("合并至少需要两个视频".into());
                }
                let label = format!(
                    "合并 {} 等 {} 个视频",
                    file_name(&inputs[0].path),
                    inputs.len()
                );
                Ok(vec![WorkItem::new(
                    format!("{}#all", req.tool_id),
                    label,
                    1,
                    inputs,
                )])
            }
        }
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        // 顺序与分组在规划时已经确定（含打乱）：一个工作项就是一组
        let group: Vec<usize> = (0..ctx.inputs().len()).collect();
        if group.len() < 2 {
            ctx.warn("有一组只有 1 个视频，输出即为该视频本身");
        }
        run_groups(ctx, &self.merge, vec![group], merged_stem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{AudioStream, MediaInfo, VideoCodec, VideoEncoder};
    use crate::jobs::TaskSpec;
    use crate::model::SeededRng;
    use crate::tools::analysis::Interval;
    use crate::tools::Encoding;
    use std::path::PathBuf;
    use crate::model::Settings;
    use crate::pipeline::testing::sample_info;
    use std::collections::BTreeMap;

    fn info(width: u32, height: u32, fps: f64, audio: bool) -> MediaInfo {
        let mut info = sample_info();
        let v = info.video.as_mut().unwrap();
        v.width = width;
        v.height = height;
        v.fps = fps;
        v.fps_text = format!("{}/1", fps as u32);
        if !audio {
            info.audio.clear();
        }
        info
    }

    fn seg(name: &str, info: MediaInfo) -> Segment {
        Segment::whole(name, info)
    }

    fn soft() -> Encoding {
        Encoding {
            encoder: VideoEncoder::software(VideoCodec::H264),
            video_kbps: Some(1000),
        }
    }

    #[test]
    fn concat_graph_conforms_every_segment_and_fills_silence() {
        let target = conform_target(&sample_info(), true).unwrap();
        assert_eq!(
            target,
            ConformTarget {
                width: 1280,
                height: 720,
                rate: "25".into(),
                fps: 25.0,
                audio: Some(AudioTarget {
                    sample_rate: 48_000,
                    channels: 2,
                    layout: "stereo"
                }),
            }
        );
        let graph = conform_graph(
            &[
                GraphSegment {
                    duration: 2.0,
                    has_audio: true,
                },
                GraphSegment {
                    duration: 1.5,
                    has_audio: false,
                },
            ],
            &target,
            &[],
            0.0,
        );
        let fmt = "aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo";
        let video = |i: usize, d: &str, frames: u32| {
            format!(
                "[{i}:V:0]scale=trunc(iw*sar/2)*2:ih,setsar=1,scale=1280:720:force_original_aspect_ratio=decrease,\
                 pad=1280:720:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps=25,format=yuv420p,\
                 tpad=stop_mode=clone:stop_duration={d},trim=end_frame={frames},setpts=PTS-STARTPTS[v{i}]"
            )
        };
        // 1.5 秒 × 25 帧 = 37.5 → 38 帧（1.52 秒），音频同样 1.52 秒
        let expected = [
            video(0, "2", 50),
            format!("[0:a:0]aresample=48000,{fmt},apad=whole_dur=2,atrim=duration=2,asetpts=PTS-STARTPTS[a0]"),
            video(1, "1.52", 38),
            format!("anullsrc=r=48000:cl=stereo,{fmt},atrim=duration=1.52[a1]"),
            "[v0][a0][v1][a1]concat=n=2:v=1:a=1[vout][aout]".to_owned(),
        ]
        .join(";");
        assert_eq!(graph, expected);
    }

    #[test]
    fn transitions_chain_xfade_and_acrossfade_with_offsets() {
        let target = ConformTarget {
            width: 320,
            height: 240,
            rate: "25".into(),
            fps: 25.0,
            audio: None,
        };
        let segs = [3.0, 2.0, 4.0].map(|duration| GraphSegment {
            duration,
            has_audio: false,
        });
        let graph = conform_graph(&segs, &target, &["fade".into(), "wipeleft".into()], 0.5);
        let tail: Vec<&str> = graph.split(';').skip(3).collect();
        assert_eq!(
            tail,
            [
                "[v0][v1]xfade=transition=fade:duration=0.5:offset=2.5[vx1]",
                "[vx1][v2]xfade=transition=wipeleft:duration=0.5:offset=4[vout]",
            ]
        );
        assert!(!graph.contains("anullsrc"), "没有音频时不生成音频链");
        assert_eq!(merged_duration(&[3.0, 2.0, 4.0], 2, 0.5), 8.0);

        let with_audio = ConformTarget {
            audio: Some(AudioTarget::from_stream(44_100, 1)),
            ..target
        };
        let segs = [2.0, 2.0].map(|duration| GraphSegment {
            duration,
            has_audio: true,
        });
        let graph = conform_graph(&segs, &with_audio, &["dissolve".into()], 1.0);
        assert!(graph.ends_with(
            "[v0][v1]xfade=transition=dissolve:duration=1:offset=1[vout];\
             [a0][a1]acrossfade=d=1:c1=tri:c2=tri[aout]"
        ));
        assert!(graph.contains("channel_layouts=mono"), "{graph}");
    }

    #[test]
    fn conform_target_follows_first_video_and_falls_back() {
        let mut odd = info(641, 361, 30.0, false);
        odd.video.as_mut().unwrap().rotation = 90;
        let t = conform_target(&odd, false).unwrap();
        assert_eq!(
            (t.width, t.height, t.rate.as_str(), t.fps),
            (360, 640, "30", 30.0)
        );
        assert!((rate_value("30000/1001") - 29.97).abs() < 0.001);
        assert_eq!(rate_value("x"), 25.0);
        assert_eq!((frame_count(0.001, 25.0), quantize(1.5, 30.0)), (1, 1.5));
        assert_eq!(t.audio, None);
        assert_eq!(
            conform_target(&odd, true).unwrap().audio,
            Some(AudioTarget::from_stream(48_000, 2))
        );
        let mut vfr = sample_info();
        let v = vfr.video.as_mut().unwrap();
        v.fps_text = "90000/1".into();
        v.fps = 29.97;
        assert_eq!(rate_text(v), "29.97");
        let mut none = sample_info();
        none.video = None;
        assert!(conform_target(&none, true).is_err());
        assert_eq!(AudioTarget::from_stream(0, 5).layout, "stereo");
    }

    #[test]
    fn lossless_compatibility_reports_the_first_problem() {
        let a = seg("a.mp4", sample_info());
        let b = seg("b.mp4", sample_info());
        assert_eq!(lossless_problem(&[a.clone(), b.clone()]), None);
        let small = seg("c.mp4", info(640, 360, 25.0, true));
        assert_eq!(
            lossless_problem(&[a.clone(), small]).unwrap(),
            "a.mp4 与 c.mp4 的分辨率不同（1280x720 / 640x360）"
        );
        let silent = seg("d.mp4", info(1280, 720, 25.0, false));
        assert!(lossless_problem(&[a.clone(), silent])
            .unwrap()
            .contains("没有音频"));
        let fast = seg("e.mp4", info(1280, 720, 30.0, true));
        assert!(lossless_problem(&[a.clone(), fast])
            .unwrap()
            .contains("帧率"));
        let mut mono = sample_info();
        mono.audio = vec![AudioStream {
            channels: 1,
            ..mono.audio[0].clone()
        }];
        assert!(lossless_problem(&[a.clone(), seg("f.mp4", mono)])
            .unwrap()
            .contains("音频格式不同"));
        let mut hevc = sample_info();
        hevc.video.as_mut().unwrap().codec = "hevc".into();
        assert!(lossless_problem(&[a, seg("g.mp4", hevc)])
            .unwrap()
            .contains("视频编码不同（h264 / hevc）"));
    }

    #[test]
    fn encode_and_copy_args() {
        let inputs = [
            InputArg {
                path: "a.mp4".into(),
                cut: None,
            },
            InputArg {
                path: "b.mp4".into(),
                cut: Some(Interval::new(1.5, 4.0)),
            },
        ];
        let audio = AudioTarget::from_stream(48_000, 2);
        let args = encode_args(
            &inputs,
            "g.txt",
            Some(&audio),
            &soft(),
            "mp4",
            Some(5.0),
            "out.mp4",
        );
        assert_eq!(
            args.join(" "),
            "-i a.mp4 -ss 1.5 -t 2.5 -i b.mp4 -/filter_complex g.txt -map [vout] -map [aout] \
             -c:v libx264 -preset medium -pix_fmt yuv420p -b:v 1000k -maxrate 1500k -bufsize 2000k \
             -c:a aac -b:a 192k -t 5 -movflags +faststart out.mp4"
        );
        let silent = encode_args(&inputs[..1], "g.txt", None, &soft(), "mkv", None, "o.mkv");
        assert!(!silent.join(" ").contains("[aout]"));
        assert!(!silent.join(" ").contains("movflags"));
        assert_eq!(
            concat_copy_args("l.txt", true, "mp4", "o.mp4").join(" "),
            "-f concat -safe 0 -i l.txt -map 0:v:0 -map 0:a:0 -c copy -movflags +faststart o.mp4"
        );
        assert_eq!(
            concat_copy_args("l.txt", false, "mkv", "o.mkv").join(" "),
            "-f concat -safe 0 -i l.txt -map 0:v:0 -c copy o.mkv"
        );
    }

    #[test]
    fn grouping_shuffling_and_names() {
        assert_eq!(
            chunk_order(vec![0, 1, 2, 3, 4], Some(2)),
            [vec![0, 1], vec![2, 3], vec![4]]
        );
        assert_eq!(chunk_order(vec![0, 1, 2], None), [vec![0, 1, 2]]);
        assert!(chunk_order(Vec::new(), None).is_empty());

        let shuffled = |seed| {
            let mut v: Vec<usize> = (0..10).collect();
            shuffle(&mut v, &mut SeededRng::new(seed));
            v
        };
        assert_eq!(shuffled(3), shuffled(3));
        assert_ne!(shuffled(3), (0..10).collect::<Vec<_>>());
        let mut sorted = shuffled(3);
        sorted.sort_unstable();
        assert_eq!(sorted, (0..10).collect::<Vec<_>>());

        let stems = vec!["一".to_owned(), "b c".to_owned()];
        assert_eq!(output_stem(&stems, true, "x"), "一 b c");
        assert_eq!(output_stem(&stems, false, "x"), "x");
        let long = vec!["a".repeat(100), "b".repeat(100)];
        assert_eq!(
            output_stem(&long, true, "x").chars().count(),
            MAX_JOINED_CHARS
        );
    }

    #[test]
    fn transitions_are_picked_per_junction_and_fade_is_clamped() {
        let mut rng = SeededRng::new(5);
        assert!(pick_transitions("none", 3, &mut rng).is_empty());
        assert_eq!(pick_transitions("fade", 2, &mut rng), ["fade", "fade"]);
        let a = pick_transitions("random", 6, &mut SeededRng::new(9));
        assert_eq!(a, pick_transitions("random", 6, &mut SeededRng::new(9)));
        assert!(a.iter().all(|t| TRANSITIONS.contains(&t.as_str())));
        assert_eq!(effective_fade(1.0, &[3.0, 2.0]), (1.0, false));
        assert_eq!(effective_fade(2.0, &[3.0, 1.5]), (0.75, true));
    }

    #[test]
    fn params_validate() {
        assert!(MergeParams::default().validate().is_ok());
        let bad_size = MergeParams {
            grouping: MergeGrouping::EveryN,
            group_size: 0,
            ..MergeParams::default()
        };
        assert!(bad_size.validate().is_err());
        let mut bad = MergeParams::default();
        bad.merge.transition = "spin".into();
        assert!(bad.validate().is_err());
        let mut long = MergeParams::default();
        long.merge.mode = MergeMode::Reencode;
        long.merge.transition = "fade".into();
        long.merge.transition_seconds = 20.0;
        assert!(long.validate().is_err());
        let mut middle = MergeParams::default();
        middle.merge.middle_enabled = true;
        assert!(middle.validate().is_err());
        // 平铺的 JSON 形状
        let json = serde_json::to_value(MergeParams::default()).unwrap();
        assert_eq!(json["mode"], "lossless");
        assert_eq!(json["groupSize"], 2);
        assert_eq!(json["middle"]["mode"], "file");
        let parsed: MergeParams =
            serde_json::from_value(serde_json::json!({"mode": "reencode", "groupSize": 3}))
                .unwrap();
        assert_eq!(
            (parsed.merge.mode, parsed.group_size),
            (MergeMode::Reencode, 3)
        );
    }

    #[test]
    fn plan_groups_every_n_and_shuffles_at_plan_time() {

        let videos: Vec<TaskSpec> = (1..=5)
            .map(|i| TaskSpec {
                id: format!("t{i}"),
                path: format!("/v/{i}.mp4"),
                source_root: None,
                info: None,
                number: i,
            })
            .collect();
        let folders = BTreeMap::new();
        let settings = Settings::default();
        let req = PlanRequest {
            tool_id: ID,
            videos: &videos,
            folders: &folders,
            settings: &settings,
            seed: 11,
        };
        let every = MergeParams {
            grouping: MergeGrouping::EveryN,
            group_size: 2,
            ..MergeParams::default()
        };
        let items = every.plan(&req).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[2].id, "tool.merge#3");
        assert_eq!(items[2].label, "第 3 组（1 个视频）");
        assert_eq!(items[1].inputs[0].path, PathBuf::from("/v/3.mp4"));
        let shuffled = MergeParams {
            shuffle: true,
            ..every
        };
        // 打乱在规划时完成：仍是每组一个工作项（可并行），组内是打乱后的真实输入；同一种子结果相同
        let paths = |items: &[WorkItem]| -> Vec<Vec<PathBuf>> {
            items
                .iter()
                .map(|i| i.inputs.iter().map(|x| x.path.clone()).collect())
                .collect()
        };
        let a = shuffled.plan(&req).unwrap();
        assert_eq!(a.len(), 3);
        assert_eq!(paths(&a), paths(&shuffled.plan(&req).unwrap()));
        let mut all_paths: Vec<PathBuf> = paths(&a).concat();
        assert_ne!(
            all_paths,
            (1..=5).map(|i| PathBuf::from(format!("/v/{i}.mp4"))).collect::<Vec<_>>(),
            "种子 11 打乱后顺序不同"
        );
        all_paths.sort();
        assert_eq!(all_paths.len(), 5);
        let other = PlanRequest { seed: 12, ..req };
        assert_ne!(paths(&a), paths(&shuffled.plan(&other).unwrap()));
        let shuffled_all = MergeParams {
            shuffle: true,
            ..MergeParams::default()
        };
        let one_item = shuffled_all.plan(&req).unwrap();
        assert_eq!((one_item.len(), one_item[0].inputs.len()), (1, 5));
        let all = MergeParams::default().plan(&req).unwrap();
        assert_eq!((all.len(), all[0].inputs.len()), (1, 5));
        assert_eq!(all[0].label, "合并 1.mp4 等 5 个视频");
        let one = PlanRequest {
            videos: &videos[..1],
            ..req
        };
        assert!(MergeParams::default().plan(&one).is_err());
    }
}
