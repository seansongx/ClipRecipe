//! `pipe.drop-frames` 抽帧：每 N 帧丢掉 1 帧（N 为范围值）。
//!
//! 参数与取值约定（FEATURES 未规定的部分在此说明）：
//! - `interval`：N，整数，允许 2～1000；丢掉的是每组的最后一帧（第 N、2N、3N… 帧，从 1 数起），
//!   即 `select='mod(n+1,N)'`。
//! - `syncAudio`（同步加速音频，默认开）：
//!   - 开：剩下的帧按原帧率紧密排列（`setpts=N/FRAME_RATE/TB`），视频时长变为原来的 (N−1)/N；
//!     音频用 `atempo=N/(N−1)` 同步加速，音视频仍然对齐。`ctx.stream.duration` 相应缩短。
//!   - 关：保持原时间轴——剩下的帧保留原时间戳，被丢掉的位置由前一帧延长显示补上，
//!     时长与音频都不变（仍然对齐），只是画面少了这些帧。
//! - 帧率不变（`ctx.stream.fps` 不变）。
//! - 按帧序号丢帧，前面的模块输出可变帧率时先 `ctx.ensure_cfr()`；关闭同步时输出为可变帧率（`ctx.stream.vfr`）。

use serde::{Deserialize, Serialize};

use crate::model::Range;
use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.drop-frames";

pub const MIN_INTERVAL: u32 = 2;
pub const MAX_INTERVAL: u32 = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DropFramesParams {
    pub enabled: bool,
    /// 每多少帧丢 1 帧
    pub interval: Range<u32>,
    /// 同步加速音频（缩短时长）；关闭时保持原时间轴
    pub sync_audio: bool,
}

impl Default for DropFramesParams {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: Range::new(20, 30),
            sync_audio: true,
        }
    }
}

impl PipelineModule for DropFramesParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        let (lo, hi) = self.interval.ordered();
        if lo < MIN_INTERVAL || hi > MAX_INTERVAL {
            return Err(format!(
                "抽帧间隔需在 {MIN_INTERVAL}～{MAX_INTERVAL} 帧之间"
            ));
        }
        let n = self.interval.sample(&mut ctx.rng);
        // 按帧序号丢帧：前面若有模块输出了可变帧率，先恢复恒定帧率
        ctx.ensure_cfr();
        ctx.graph.video(format!("select='mod(n+1,{n})'"));
        if self.sync_audio {
            ctx.graph.video("setpts=N/FRAME_RATE/TB");
            let tempo = f64::from(n) / f64::from(n - 1);
            ctx.graph.audio(format!("atempo={}", fmt_num(tempo)));
            ctx.stream.duration *= f64::from(n - 1) / f64::from(n);
        } else {
            // 保留原时间戳：被丢掉的位置留空
            ctx.stream.vfr = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::{build_with, sample_info};
    use crate::pipeline::BuiltCommand;

    fn recipe_with(params: DropFramesParams) -> crate::model::Recipe {
        let mut recipe = crate::model::Recipe::disabled();
        recipe.modules.drop_frames = params;
        recipe
    }

    fn build(params: DropFramesParams, seed: u64) -> Result<BuiltCommand, String> {
        build_with(&sample_info(), &recipe_with(params), seed)
    }

    fn params(interval: Range<u32>, sync_audio: bool) -> DropFramesParams {
        DropFramesParams {
            enabled: true,
            interval,
            sync_audio,
        }
    }

    #[test]
    fn drops_one_frame_per_interval_and_speeds_up_audio() {
        let built = build(params(Range::fixed(5), true), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]select='mod(n+1,5)',setpts=N/FRAME_RATE/TB[v0];[0:a:0]atempo=1.25[a1]")
        );
        assert!((built.expected_duration - 8.0).abs() < 1e-9);
    }

    #[test]
    fn without_audio_sync_keeps_timeline() {
        let built = build(params(Range::fixed(10), false), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]select='mod(n+1,10)'[v0]")
        );
        assert_eq!(built.arg_after("-c:a"), Some("copy"));
        assert_eq!(built.expected_duration, 10.0);
    }

    #[test]
    fn no_audio_source() {
        let mut info = sample_info();
        info.audio.clear();
        let built = build_with(&info, &recipe_with(params(Range::fixed(2), true)), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]select='mod(n+1,2)',setpts=N/FRAME_RATE/TB[v0]")
        );
        assert_eq!(built.expected_duration, 5.0);
    }

    #[test]
    fn ranged_interval_is_reproducible_and_bounded() {
        let p = params(Range::new(20, 30), true);
        let a = build(p.clone(), 42).unwrap();
        let b = build(p.clone(), 42).unwrap();
        assert_eq!(a.args, b.args);
        assert_eq!(
            a.arg_after("-filter_complex"),
            Some("[0:v:0]select='mod(n+1,25)',setpts=N/FRAME_RATE/TB[v0];[0:a:0]atempo=1.0417[a1]")
        );
        for seed in 0..50 {
            let fc = build(p.clone(), seed)
                .unwrap()
                .arg_after("-filter_complex")
                .unwrap()
                .to_owned();
            let n: u32 = fc
                .trim_start_matches("[0:v:0]select='mod(n+1,")
                .split('\'')
                .next()
                .unwrap()
                .trim_end_matches(')')
                .parse()
                .unwrap();
            assert!((20..=30).contains(&n), "{n}");
        }
    }

    #[test]
    fn invalid_interval_is_rejected() {
        for interval in [Range::fixed(1), Range::new(0, 5), Range::new(2, 5000)] {
            let err = build(params(interval, true), 1).unwrap_err();
            assert!(err.contains("抽帧间隔"), "{err}");
        }
    }
}
