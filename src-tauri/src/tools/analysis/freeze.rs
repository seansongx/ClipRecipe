//! 静止画面检测：`freezedetect`，返回画面静止的区间。

use crate::ffmpeg::MediaInfo;
use crate::pipeline::fmt_num;

use super::{check_db, check_seconds, paired_intervals, Analysis, Interval, MetadataFrame};

/// 静止画面检测（第一条视频流）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreezeDetect {
    /// 帧间差异低于此值（dB）视为没有变化，如 -60
    pub noise_db: f64,
    /// 静止至少这么久（秒）才算
    pub min_duration: f64,
}

impl Default for FreezeDetect {
    fn default() -> Self {
        Self {
            noise_db: -60.0,
            min_duration: 1.0,
        }
    }
}

impl Analysis for FreezeDetect {
    type Output = Vec<Interval>;

    fn name(&self) -> &'static str {
        "静止画面检测"
    }

    fn args(&self, info: &MediaInfo) -> Result<Vec<String>, String> {
        check_db(self.noise_db, "噪声阈值")?;
        check_seconds(self.min_duration, "最短静止时长")?;
        if info.video.is_none() {
            return Err("文件中没有视频流，无法检测静止画面".into());
        }
        Ok(vec![
            "-map".into(),
            "0:v:0".into(),
            "-an".into(),
            "-sn".into(),
            "-dn".into(),
            "-vf".into(),
            format!(
                "freezedetect=n={}dB:d={},metadata=mode=print",
                fmt_num(self.noise_db),
                fmt_num(self.min_duration)
            ),
        ])
    }

    fn parse(&self, frames: &[MetadataFrame], info: &MediaInfo) -> Vec<Interval> {
        paired_intervals(
            frames,
            "lavfi.freezedetect.freeze_start",
            "lavfi.freezedetect.freeze_end",
            info.duration,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::lines;
    use super::*;
    use crate::pipeline::testing::sample_info;

    #[test]
    fn args_and_parse() {
        let args = FreezeDetect::default().args(&sample_info()).unwrap();
        assert_eq!(
            args.last().map(String::as_str),
            Some("freezedetect=n=-60dB:d=1,metadata=mode=print")
        );
        let frames = lines(
            "[Parsed_metadata_1 @ 0] frame:37 pts:1480000 pts_time:1.48\n\
             [Parsed_metadata_1 @ 0] lavfi.freezedetect.freeze_start=0.96\n\
             [Parsed_metadata_1 @ 0] frame:75 pts:3000000 pts_time:3\n\
             [Parsed_metadata_1 @ 0] lavfi.freezedetect.freeze_duration=2.04\n\
             [Parsed_metadata_1 @ 0] lavfi.freezedetect.freeze_end=3\n\
             [Parsed_metadata_1 @ 0] frame:200 pts:1 pts_time:9\n\
             [Parsed_metadata_1 @ 0] lavfi.freezedetect.freeze_start=8\n",
        );
        assert_eq!(
            FreezeDetect::default().parse(&frames, &sample_info()),
            [Interval::new(0.96, 3.0), Interval::new(8.0, 10.0)]
        );
    }
}
