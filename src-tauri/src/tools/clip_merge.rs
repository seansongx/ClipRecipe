//! `tool.clip-merge` 截取合并：从文件夹（总是包含子文件夹）里的每个视频截取一段，拼成一个或多个视频。
//! 重编码（统一到第一个片段的尺寸 / 帧率 / 音频布局），复用 [`crate::tools::merge`] 的合并引擎：
//! 一个输出一条 FFmpeg 命令（输入端 `-ss` / `-t` 截取，帧精确）。
//!
//! 参数与取值约定（docs/specs/tools.md 未规定的部分）：
//! - 片段长度 `segmentSeconds`：范围值（秒，0.5～3600），每个视频各取一次；视频比最小值还短时跳过（警告），
//!   比取到的长度短时整段使用。
//! - 截取位置 `position`：`head` 开头、`middle` 正中、`tail` 结尾、`random` 随机（默认，按种子）。
//! - `shuffle`：按种子打乱视频顺序（默认按自然顺序）。
//! - 输出 `outputMode`：`single` 全部片段拼成一个（输出名为文件夹名）；`target` 按目标总时长 `targetSeconds`
//!   （1～86400 秒）依次累加片段，达到目标就开始下一个输出（`<文件夹名>_01`、`_02`…）；最后一个不足目标时仍输出（警告）。
//! - 截去超出长度 `trimExcess`：每个输出裁到正好 `targetSeconds`（单个输出时也用它作为目标长度；不足时警告）。
//!   开启后中间视频、片头、片尾不可用（设置了也忽略并警告）。
//! - 变速适应（设置 `set.speed-fit`）：有目标时长（`target` 模式或开启了“截去超出长度”）时，每个输出不再截断 /
//!   不足，而是整体变速（`setpts` + `atempo`）到正好 `targetSeconds`；变速倍数超出 0.25～4 时退回截断并警告。
//!   单个输出且未开启“截去超出长度”时没有目标时长，不变速。见 [`crate::pipeline::speed_fit`]。
//! - 中间视频 `middle`（插在相邻片段之间）、片头 `intro`、片尾 `outro`：素材来源为单个文件或目录
//!   （目录时随机 / 依次取，片头片尾按输出序号取），参与尺寸统一，不计入目标时长。

use serde::{Deserialize, Serialize};

use crate::fsutil::{self, VIDEO_EXTENSIONS};
use crate::model::{AssetSource, Range, SeededRng};
use crate::pipeline::fmt_num;
use crate::tools::analysis::Interval;
use crate::tools::common::merge_engine::{
    encode_segments_to, insert_middles, shuffle, OutputEnd, OutputName, Segment,
};
use crate::tools::plan;
use crate::tools::split::piece_suffix;
use crate::tools::{FolderSlot, PlanRequest, Tool, ToolContext, ToolError, ToolInputs, WorkItem};

pub const ID: &str = "tool.clip-merge";
pub const SOURCE: &str = "source";
pub const MIN_CLIP: f64 = 0.5;
pub const MAX_CLIP: f64 = 3600.0;
pub const MAX_TARGET: f64 = 86400.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipPosition {
    Head,
    Middle,
    Tail,
    #[default]
    Random,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipOutput {
    /// 全部片段拼成一个
    #[default]
    Single,
    /// 按目标总时长分成多个
    Target,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClipMergeParams {
    pub segment_seconds: Range<f64>,
    pub position: ClipPosition,
    pub shuffle: bool,
    pub output_mode: ClipOutput,
    pub target_seconds: f64,
    pub trim_excess: bool,
    pub middle_enabled: bool,
    pub middle: AssetSource,
    pub intro_enabled: bool,
    pub intro: AssetSource,
    pub outro_enabled: bool,
    pub outro: AssetSource,
}

impl Default for ClipMergeParams {
    fn default() -> Self {
        Self {
            segment_seconds: Range::new(3.0, 5.0),
            position: ClipPosition::Random,
            shuffle: false,
            output_mode: ClipOutput::Single,
            target_seconds: 60.0,
            trim_excess: false,
            middle_enabled: false,
            middle: AssetSource::default(),
            intro_enabled: false,
            intro: AssetSource::default(),
            outro_enabled: false,
            outro: AssetSource::default(),
        }
    }
}

impl ClipMergeParams {
    fn validate(&self) -> Result<(), String> {
        let (lo, hi) = self.segment_seconds.ordered();
        if !(lo.is_finite() && hi.is_finite() && lo >= MIN_CLIP && hi <= MAX_CLIP) {
            return Err(format!(
                "片段长度必须在 {}～{} 秒之间",
                fmt_num(MIN_CLIP),
                fmt_num(MAX_CLIP)
            ));
        }
        let needs_target = self.output_mode == ClipOutput::Target || self.trim_excess;
        if needs_target && !(self.target_seconds >= 1.0 && self.target_seconds <= MAX_TARGET) {
            return Err(format!("目标时长必须在 1～{} 秒之间", fmt_num(MAX_TARGET)));
        }
        if !self.trim_excess {
            for (enabled, source, what) in [
                (self.middle_enabled, &self.middle, "中间视频"),
                (self.intro_enabled, &self.intro, "片头"),
                (self.outro_enabled, &self.outro, "片尾"),
            ] {
                if enabled && source.is_empty() {
                    return Err(format!("已开启{what}，但没有选择文件或目录"));
                }
            }
        }
        Ok(())
    }
}

/// 从时长为 `duration` 的视频中截取一段；比最短长度还短时返回 None。
pub fn pick_clip(
    duration: f64,
    length: &Range<f64>,
    position: ClipPosition,
    rng: &mut SeededRng,
) -> Option<Interval> {
    let (lo, _) = length.ordered();
    if !(duration.is_finite() && duration > 0.0) || duration + 1e-6 < lo {
        return None;
    }
    let len = length.sample_rounded(rng, 2).min(duration);
    let room = (duration - len).max(0.0);
    let start = match position {
        ClipPosition::Head => 0.0,
        ClipPosition::Middle => room / 2.0,
        ClipPosition::Tail => room,
        ClipPosition::Random => (rng.f64_in(0.0, room) * 1000.0).floor() / 1000.0,
    };
    Some(Interval::new(start, start + len))
}

/// 按目标总时长分组：依次累加，达到 `target` 就结束一组。
pub fn group_by_target(lengths: &[f64], target: f64) -> Vec<Vec<usize>> {
    let mut groups = Vec::new();
    let mut current = Vec::new();
    let mut sum = 0.0;
    for (i, len) in lengths.iter().enumerate() {
        current.push(i);
        sum += len;
        if sum + 1e-6 >= target {
            groups.push(std::mem::take(&mut current));
            sum = 0.0;
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

impl Tool for ClipMergeParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn inputs(&self) -> ToolInputs {
        const FOLDERS: &[FolderSlot] = &[FolderSlot {
            key: SOURCE,
            label: "视频文件夹",
        }];
        ToolInputs::folders(FOLDERS)
    }

    fn plan(&self, req: &PlanRequest) -> Result<Vec<WorkItem>, String> {
        self.validate()?;
        let root = req.folder(SOURCE)?.to_path_buf();
        // 总是包含子文件夹
        let mut files = fsutil::list_files(&root, true, VIDEO_EXTENSIONS)
            .map_err(|e| format!("无法读取文件夹 {}：{e}", root.display()))?;
        if self.shuffle {
            // 规划时打乱：工作项里就是真正的顺序（由本次运行的种子派生，可复现）
            shuffle(&mut files, &mut req.rng(ID));
        }
        if files.is_empty() {
            return Err(format!("文件夹中没有可处理的视频：{}", root.display()));
        }
        let label = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "截取合并".into());
        Ok(vec![plan::files_as_one(ID, &label, &files, &root)])
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let order: Vec<usize> = (0..ctx.inputs().len()).collect();
        let mut clips = Vec::new();
        for i in order {
            ctx.check_cancelled()?;
            let info = ctx.info(i)?;
            let path = ctx.input_path(i)?.to_path_buf();
            match pick_clip(
                info.duration,
                &self.segment_seconds,
                self.position,
                &mut ctx.rng,
            ) {
                Some(cut) => clips.push(Segment {
                    path,
                    info,
                    cut: Some(cut),
                }),
                None => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                    ctx.warn(format!(
                        "{} 比片段最短长度还短，已跳过",
                        name.unwrap_or_default()
                    ));
                }
            }
        }
        if clips.is_empty() {
            return Err("没有符合条件的视频（都比片段最短长度短）".into());
        }

        let lengths: Vec<f64> = clips.iter().map(Segment::duration).collect();
        let groups = match self.output_mode {
            ClipOutput::Single => vec![(0..clips.len()).collect::<Vec<_>>()],
            ClipOutput::Target => group_by_target(&lengths, self.target_seconds),
        };
        let has_target = self.output_mode == ClipOutput::Target || self.trim_excess;
        let fit = has_target && ctx.settings().speed_fit;
        let end = if fit {
            OutputEnd::SpeedFit(self.target_seconds)
        } else if self.trim_excess {
            OutputEnd::Trim(self.target_seconds)
        } else {
            OutputEnd::Natural
        };
        let extras = !self.trim_excess;
        if self.trim_excess && (self.middle_enabled || self.intro_enabled || self.outro_enabled) {
            ctx.warn("已开启“截去超出长度”，中间视频、片头、片尾不可用，已忽略");
        }
        for group in &groups {
            let sum: f64 = group.iter().map(|&i| lengths[i]).sum();
            let target = self.target_seconds;
            if has_target && !fit && sum + 1e-6 < target {
                ctx.warn(format!(
                    "片段总长 {} 秒，不足目标时长 {} 秒",
                    fmt_num(sum),
                    fmt_num(target)
                ));
            }
        }
        let weights: Vec<f64> = groups
            .iter()
            .map(|g| g.iter().map(|&i| lengths[i]).sum::<f64>().max(0.1))
            .collect();
        if groups.len() > 1 {
            ctx.set_steps(&weights);
        }

        let folder = ctx.item().label.clone();
        let total = groups.len();
        for (k, group) in groups.into_iter().enumerate() {
            ctx.check_cancelled()?;
            let group_clips: Vec<Segment> = group.iter().map(|&i| clips[i].clone()).collect();
            let mut segments = Vec::new();
            if extras && self.intro_enabled {
                segments.push(pick_extra(ctx, &self.intro, k, "片头")?);
            }
            // 规格以第一个片段（而不是片头）为准
            let reference = segments.len();
            if extras && self.middle_enabled {
                segments.extend(insert_middles(ctx, &self.middle, group_clips)?);
            } else {
                segments.extend(group_clips);
            }
            if extras && self.outro_enabled {
                segments.push(pick_extra(ctx, &self.outro, k, "片尾")?);
            }
            let stem = if total > 1 || self.output_mode == ClipOutput::Target {
                format!("{folder}{}", piece_suffix(k, total))
            } else {
                folder.clone()
            };
            let name = OutputName { stem, input: 0 };
            encode_segments_to(ctx, &segments, reference, &[], 0.0, end, &name)?;
        }
        Ok(())
    }
}

/// 取一个片头 / 片尾素材并读取媒体信息。
fn pick_extra(
    ctx: &mut ToolContext,
    source: &AssetSource,
    index: usize,
    what: &str,
) -> Result<Segment, String> {
    let path = source.pick(VIDEO_EXTENSIONS, index, &mut ctx.rng)?;
    let info = ctx
        .probe(&path)
        .map_err(|e| format!("无法读取{what} {}：{e}", path.display()))?;
    Ok(Segment::whole(path, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_follow_position_and_length() {
        let fixed = Range::fixed(2.0);
        let mut rng = SeededRng::new(1);
        assert_eq!(
            pick_clip(10.0, &fixed, ClipPosition::Head, &mut rng),
            Some(Interval::new(0.0, 2.0))
        );
        assert_eq!(
            pick_clip(10.0, &fixed, ClipPosition::Middle, &mut rng),
            Some(Interval::new(4.0, 6.0))
        );
        assert_eq!(
            pick_clip(10.0, &fixed, ClipPosition::Tail, &mut rng),
            Some(Interval::new(8.0, 10.0))
        );
        assert_eq!(pick_clip(1.5, &fixed, ClipPosition::Head, &mut rng), None);
        let range = Range::new(1.0, 3.0);
        assert_eq!(
            pick_clip(2.0, &range, ClipPosition::Tail, &mut SeededRng::new(4)).map(|c| c.end),
            Some(2.0)
        );
        let pick = |seed| {
            pick_clip(
                10.0,
                &range,
                ClipPosition::Random,
                &mut SeededRng::new(seed),
            )
        };
        assert_eq!(pick(7), pick(7));
        for seed in 0..50 {
            let c = pick(seed).unwrap();
            assert!(c.start >= 0.0 && c.end <= 10.0 + 1e-9, "{c:?}");
            assert!((1.0..=3.0).contains(&c.duration()), "{c:?}");
        }
    }

    #[test]
    fn target_groups_accumulate_until_reached() {
        assert_eq!(
            group_by_target(&[2.0, 2.0, 2.0, 2.0, 1.0], 3.0),
            [vec![0, 1], vec![2, 3], vec![4]]
        );
        assert_eq!(group_by_target(&[5.0, 1.0], 3.0), [vec![0], vec![1]]);
        assert_eq!(group_by_target(&[1.0, 2.0], 3.0), [vec![0, 1]]);
    }

    #[test]
    fn params_validate() {
        assert!(ClipMergeParams::default().validate().is_ok());
        let short = ClipMergeParams {
            segment_seconds: Range::new(0.1, 1.0),
            ..ClipMergeParams::default()
        };
        assert!(short.validate().is_err());
        let target = ClipMergeParams {
            output_mode: ClipOutput::Target,
            target_seconds: 0.0,
            ..ClipMergeParams::default()
        };
        assert!(target.validate().is_err());
        let intro = ClipMergeParams {
            intro_enabled: true,
            ..ClipMergeParams::default()
        };
        assert!(intro.validate().is_err());
        let trimmed = ClipMergeParams {
            trim_excess: true,
            ..intro
        };
        assert!(
            trimmed.validate().is_ok(),
            "截去超出长度时片头不可用，不再检查"
        );
    }
}
