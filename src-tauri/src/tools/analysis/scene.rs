//! 场景切换检测：`select` 计算每帧的场景分数，`metadata` 只打印超过阈值的帧，返回切换点时间。

use crate::ffmpeg::MediaInfo;
use crate::pipeline::fmt_num;

use super::{check_ratio, check_seconds, Analysis, MetadataFrame};

/// 场景切换检测。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneDetect {
    /// 场景变化分数阈值（0～1），越小越敏感；常用 0.3～0.4
    pub threshold: f64,
    /// 相邻切换点（以及与开头、结尾）的最小间隔（秒），更近的切换点被忽略
    pub min_gap: f64,
}

impl Default for SceneDetect {
    fn default() -> Self {
        Self {
            threshold: 0.3,
            min_gap: 1.0,
        }
    }
}

/// 分析前把画面缩小到这个宽度，速度快很多，对场景分数影响很小。
const ANALYSIS_WIDTH: u32 = 320;

impl Analysis for SceneDetect {
    type Output = Vec<f64>;

    fn name(&self) -> &'static str {
        "场景检测"
    }

    fn args(&self, info: &MediaInfo) -> Result<Vec<String>, String> {
        check_ratio(self.threshold, "场景切换阈值")?;
        check_seconds(self.min_gap, "最小间隔")?;
        if info.video.is_none() {
            return Err("文件中没有视频流，无法检测场景".into());
        }
        Ok(vec![
            "-map".into(),
            "0:v:0".into(),
            "-an".into(),
            "-sn".into(),
            "-dn".into(),
            "-vf".into(),
            // 所有帧都通过（进度与卡死保护才能正常工作），只打印分数超过阈值的帧
            format!(
                "scale={ANALYSIS_WIDTH}:-2,select='gte(scene,0)',\
                 metadata=mode=print:key=lavfi.scene_score:value={}:function=greater",
                fmt_num(self.threshold)
            ),
        ])
    }

    fn parse(&self, frames: &[MetadataFrame], info: &MediaInfo) -> Vec<f64> {
        let mut times: Vec<f64> = frames
            .iter()
            .filter(|f| f.pts_time.is_finite())
            .filter(|f| {
                f.number("lavfi.scene_score")
                    .map_or(true, |score| score >= self.threshold)
            })
            .map(|f| f.pts_time)
            .collect();
        times.sort_by(f64::total_cmp);
        let duration = info.duration;
        let mut cuts: Vec<f64> = Vec::with_capacity(times.len());
        for t in times {
            let after_start = t >= self.min_gap.max(1e-3);
            let before_end = duration <= 0.0 || duration - t >= self.min_gap.max(1e-3);
            let far_from_previous = cuts.last().map_or(true, |prev| t - prev >= self.min_gap);
            if after_start && before_end && far_from_previous {
                cuts.push(t);
            }
        }
        cuts
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::lines;
    use super::*;
    use crate::pipeline::testing::sample_info;

    #[test]
    fn args_scale_select_and_print() {
        let args = SceneDetect::default().args(&sample_info()).unwrap();
        assert_eq!(
            args,
            [
                "-map",
                "0:v:0",
                "-an",
                "-sn",
                "-dn",
                "-vf",
                "scale=320:-2,select='gte(scene,0)',metadata=mode=print:key=lavfi.scene_score:value=0.3:function=greater"
            ]
        );
        let bad = SceneDetect {
            threshold: 1.5,
            ..SceneDetect::default()
        };
        assert!(bad.args(&sample_info()).is_err());
        let mut no_video = sample_info();
        no_video.video = None;
        assert!(SceneDetect::default().args(&no_video).is_err());
    }

    #[test]
    fn cuts_respect_gap_start_and_end() {
        // sample_info 时长 10 秒
        let frames = lines(
            "[Parsed_metadata_2 @ 0] frame:0 pts:1 pts_time:0.4\n\
             [Parsed_metadata_2 @ 0] lavfi.scene_score=0.9\n\
             [Parsed_metadata_2 @ 0] frame:1 pts:2 pts_time:2\n\
             [Parsed_metadata_2 @ 0] lavfi.scene_score=1.000000\n\
             [Parsed_metadata_2 @ 0] frame:2 pts:3 pts_time:2.5\n\
             [Parsed_metadata_2 @ 0] lavfi.scene_score=0.8\n\
             [Parsed_metadata_2 @ 0] frame:3 pts:4 pts_time:6\n\
             [Parsed_metadata_2 @ 0] lavfi.scene_score=0.2\n\
             [Parsed_metadata_2 @ 0] frame:4 pts:5 pts_time:4\n\
             [Parsed_metadata_2 @ 0] lavfi.scene_score=0.5\n\
             [Parsed_metadata_2 @ 0] frame:5 pts:6 pts_time:9.6\n\
             [Parsed_metadata_2 @ 0] lavfi.scene_score=0.7\n",
        );
        let cuts = SceneDetect::default().parse(&frames, &sample_info());
        assert_eq!(cuts, [2.0, 4.0]);
        let loose = SceneDetect {
            threshold: 0.1,
            min_gap: 0.0,
        };
        assert_eq!(
            loose.parse(&frames, &sample_info()),
            [0.4, 2.0, 2.5, 4.0, 6.0, 9.6]
        );
    }
}
