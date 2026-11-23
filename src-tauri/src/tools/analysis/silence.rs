//! 静音检测：`silencedetect`（全部声道都低于阈值才算静音），返回静音区间。

use crate::ffmpeg::MediaInfo;
use crate::pipeline::fmt_num;

use super::{check_db, check_seconds, paired_intervals, Analysis, Interval, MetadataFrame};

/// 静音检测（第一条音频流）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SilenceDetect {
    /// 低于此音量（dB）视为静音，如 -30
    pub noise_db: f64,
    /// 持续至少这么久（秒）才算一段静音
    pub min_duration: f64,
}

impl Default for SilenceDetect {
    fn default() -> Self {
        Self {
            noise_db: -30.0,
            min_duration: 0.5,
        }
    }
}

impl Analysis for SilenceDetect {
    type Output = Vec<Interval>;

    fn name(&self) -> &'static str {
        "静音检测"
    }

    fn args(&self, info: &MediaInfo) -> Result<Vec<String>, String> {
        check_db(self.noise_db, "静音阈值")?;
        check_seconds(self.min_duration, "最短静音时长")?;
        if info.audio.is_empty() {
            return Err("文件中没有音频流，无法检测静音".into());
        }
        Ok(vec![
            "-map".into(),
            "0:a:0".into(),
            "-vn".into(),
            "-sn".into(),
            "-dn".into(),
            "-af".into(),
            format!(
                "silencedetect=n={}dB:d={},ametadata=mode=print",
                fmt_num(self.noise_db),
                fmt_num(self.min_duration)
            ),
        ])
    }

    fn parse(&self, frames: &[MetadataFrame], info: &MediaInfo) -> Vec<Interval> {
        paired_intervals(
            frames,
            "lavfi.silence_start",
            "lavfi.silence_end",
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
    fn args_and_validation() {
        let args = SilenceDetect::default().args(&sample_info()).unwrap();
        assert_eq!(
            args.last().map(String::as_str),
            Some("silencedetect=n=-30dB:d=0.5,ametadata=mode=print")
        );
        assert_eq!(args[1], "0:a:0");
        let mut silent_film = sample_info();
        silent_film.audio.clear();
        assert!(SilenceDetect::default().args(&silent_film).is_err());
        let bad = SilenceDetect {
            noise_db: 3.0,
            ..SilenceDetect::default()
        };
        assert!(bad.args(&sample_info()).is_err());
    }

    #[test]
    fn intervals_from_start_end_pairs_and_trailing_silence() {
        let frames = lines(
            "[Parsed_ametadata_1 @ 0] frame:64 pts:65536 pts_time:1.486077\n\
             [Parsed_ametadata_1 @ 0] lavfi.silence_start=1\n\
             [Parsed_ametadata_1 @ 0] frame:86 pts:88064 pts_time:1.996916\n\
             [Parsed_ametadata_1 @ 0] lavfi.silence_end=2.000023\n\
             [Parsed_ametadata_1 @ 0] lavfi.silence_duration=1.000023\n\
             [Parsed_ametadata_1 @ 0] frame:300 pts:1 pts_time:8.5\n\
             [Parsed_ametadata_1 @ 0] lavfi.silence_start=8\n",
        );
        let silences = SilenceDetect::default().parse(&frames, &sample_info());
        assert_eq!(
            silences,
            [Interval::new(1.0, 2.000023), Interval::new(8.0, 10.0)]
        );
    }
}
