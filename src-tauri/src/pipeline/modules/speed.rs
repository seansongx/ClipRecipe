//! `pipe.speed` 变速：固定倍率，或把视频等分成若干段、每段各用一个倍率。
//!
//! 参数与取值约定（FEATURES 未规定的部分在此说明）：
//! - 倍率 > 1 加速、< 1 减速，允许 0.25～4，取值保留两位小数。
//! - 分段变速：段数是范围值（2～20），视频按时长等分；
//!   - `shuffle`（随机组合，默认开）：每段在“分段倍率”范围内各自随机取值；
//!   - 关闭时：各段倍率在范围内由慢到快均匀递增（最小值 → 最大值），形成渐快的效果。
//!     分段始终按原顺序拼接，不会打乱画面内容。
//! - `minDuration`（秒，0 表示不限制）：变速后的时长不短于它。不足时把所有倍率按同一比例降低，
//!   使结果恰好等于下限；但不会为此把整体放慢到比原视频更长——源视频本身短于下限时，
//!   降到总时长与原视频相同为止，并给出警告。
//! - `changePitch`（变调，默认关）：
//!   - 关：`atempo` 保持音高；超出 0.5～2 的倍率拆成多个 `atempo` 串联；
//!   - 开：`asetrate=采样率×倍率,aresample=采样率`，音高随速度变化（源采样率未知时退回不变调并警告）。
//!
//! 视频用 `setpts` 改时间戳，再用 `fps` 滤镜恢复为当前帧率的恒定帧率，保证时长与音频一致、
//! 帧率不变（`ctx.stream.fps` 不变）。注意：这会让前面“重复帧移除”留下的空缺重新被前一帧补上。
//! 分段方式用 `split`/`asplit` + `trim`/`atrim` 切段，每段单独变速，再用 `concat` 拼接，音视频同段对齐。

use serde::{Deserialize, Serialize};

use crate::model::Range;
use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.speed";

pub const MIN_RATE: f64 = 0.25;
pub const MAX_RATE: f64 = 4.0;
pub const MIN_SEGMENTS: u32 = 2;
pub const MAX_SEGMENTS: u32 = 20;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpeedMode {
    /// 整段固定倍率
    #[default]
    Fixed,
    /// 分段变速
    Segmented,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpeedParams {
    pub enabled: bool,
    pub mode: SpeedMode,
    /// 固定方式的倍率
    pub rate: Range<f64>,
    /// 分段数
    pub segment_count: Range<u32>,
    /// 每段倍率的范围
    pub segment_rate: Range<f64>,
    /// 各段随机取倍率；关闭时由慢到快均匀递增
    pub shuffle: bool,
    /// 变速后的最短时长（秒），0 表示不限制
    pub min_duration: f64,
    /// 音高是否随速度变化
    pub change_pitch: bool,
}

impl Default for SpeedParams {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: SpeedMode::Fixed,
            rate: Range::fixed(1.2),
            segment_count: Range::fixed(3),
            segment_rate: Range::new(0.8, 1.5),
            shuffle: true,
            min_duration: 0.0,
            change_pitch: false,
        }
    }
}

/// 保留四位小数并向下取整（降低倍率时用，保证结果不短于下限）。
fn floor4(value: f64) -> f64 {
    (value * 10_000.0).floor() / 10_000.0
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn check_rate(range: &Range<f64>, name: &str) -> Result<(), String> {
    let (lo, hi) = range.ordered();
    if !lo.is_finite() || !hi.is_finite() || lo < MIN_RATE || hi > MAX_RATE {
        return Err(format!("{name}需在 {MIN_RATE}～{MAX_RATE} 之间"));
    }
    Ok(())
}

/// 保持音高的变速：`atempo` 每级限制在 0.5～2，超出时串联多级。倍率为 1 时返回空。
pub fn atempo_chain(rate: f64) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = rate;
    while rest > 2.0 + 1e-9 {
        parts.push("atempo=2".to_owned());
        rest /= 2.0;
    }
    while rest < 0.5 - 1e-9 {
        parts.push("atempo=0.5".to_owned());
        rest /= 0.5;
    }
    if (rest - 1.0).abs() > 1e-9 {
        parts.push(format!("atempo={}", fmt_num(rest)));
    }
    parts
}

impl SpeedParams {
    fn validate(&self) -> Result<(), String> {
        match self.mode {
            SpeedMode::Fixed => check_rate(&self.rate, "倍率")?,
            SpeedMode::Segmented => {
                check_rate(&self.segment_rate, "分段倍率")?;
                let (lo, hi) = self.segment_count.ordered();
                if lo < MIN_SEGMENTS || hi > MAX_SEGMENTS {
                    return Err(format!("分段数需在 {MIN_SEGMENTS}～{MAX_SEGMENTS} 之间"));
                }
            }
        }
        if !self.min_duration.is_finite() || self.min_duration < 0.0 {
            return Err("最短时长不能小于 0".into());
        }
        Ok(())
    }

    /// 分段倍率：随机组合时各自取样，否则由慢到快均匀分布。
    fn segment_rates(&self, ctx: &mut BuildContext, count: usize) -> Vec<f64> {
        if self.shuffle {
            return (0..count)
                .map(|_| self.segment_rate.sample_rounded(&mut ctx.rng, 2))
                .collect();
        }
        let (lo, hi) = self.segment_rate.ordered();
        (0..count)
            .map(|i| {
                let t = i as f64 / (count - 1) as f64;
                ((lo + (hi - lo) * t) * 100.0).round() / 100.0
            })
            .collect()
    }

    /// 按最短时长下限降低倍率。`lengths[i]` 为第 i 段原时长。
    fn apply_floor(&self, ctx: &mut BuildContext, lengths: &[f64], rates: &mut [f64]) {
        let duration: f64 = lengths.iter().sum();
        if self.min_duration <= 0.0 || duration <= 0.0 {
            return;
        }
        let target = self.min_duration.min(duration);
        let output: f64 = lengths.iter().zip(rates.iter()).map(|(l, r)| l / r).sum();
        if output >= target - 1e-9 {
            return;
        }
        if duration < self.min_duration {
            ctx.warn(format!(
                "视频时长 {} 秒短于最短时长 {} 秒，变速后保持原时长",
                fmt_num(duration),
                fmt_num(self.min_duration)
            ));
        }
        let k = output / target;
        for rate in rates.iter_mut() {
            *rate = floor4(*rate * k);
        }
    }

    /// 一段音频的变速滤镜（不含前面的切段）。
    fn audio_filters(&self, rate: f64, sample_rate: Option<u32>) -> Vec<String> {
        match sample_rate {
            Some(sr) if self.change_pitch => {
                if (rate - 1.0).abs() < 1e-9 {
                    return Vec::new();
                }
                vec![
                    format!("asetrate={}", (f64::from(sr) * rate).round() as u64),
                    format!("aresample={sr}"),
                ]
            }
            _ => atempo_chain(rate),
        }
    }

    fn sample_rate(&self, ctx: &mut BuildContext) -> Option<u32> {
        if !self.change_pitch || !ctx.graph.has_audio() {
            return None;
        }
        let rate = ctx
            .info
            .audio
            .first()
            .map(|a| a.sample_rate)
            .filter(|r| *r > 0);
        if rate.is_none() {
            ctx.warn("源音频采样率未知，无法变调，改为保持音高变速");
        }
        rate
    }

    fn apply_fixed(&self, ctx: &mut BuildContext) {
        let duration = ctx.stream.duration;
        let mut rates = [self.rate.sample_rounded(&mut ctx.rng, 2)];
        self.apply_floor(ctx, &[duration], &mut rates);
        let rate = rates[0];
        if (rate - 1.0).abs() < 1e-9 {
            return;
        }
        let sample_rate = self.sample_rate(ctx);
        ctx.graph.video(format!("setpts=PTS/{}", fmt_num(rate)));
        if let Some(fps) = ctx.rate_text() {
            ctx.graph.video(format!("fps={fps}"));
        }
        for filter in self.audio_filters(rate, sample_rate) {
            ctx.graph.audio(filter);
        }
        ctx.stream.duration = duration / rate;
    }

    fn apply_segmented(&self, ctx: &mut BuildContext) -> Result<(), String> {
        let duration = ctx.stream.duration;
        if duration <= 0.0 {
            return Err("源视频时长未知，无法分段变速".into());
        }
        let count = self.segment_count.sample(&mut ctx.rng) as usize;
        let mut rates = self.segment_rates(ctx, count);
        let bounds: Vec<f64> = (0..=count)
            .map(|i| round4(duration * i as f64 / count as f64))
            .collect();
        let lengths: Vec<f64> = bounds.windows(2).map(|w| w[1] - w[0]).collect();
        self.apply_floor(ctx, &lengths, &mut rates);
        let sample_rate = self.sample_rate(ctx);

        let video = ctx.graph.take_video();
        let audio = ctx.graph.take_audio();
        let video_parts: Vec<String> = (0..count).map(|_| ctx.graph.label("sv")).collect();
        let video_refs: Vec<&str> = video_parts.iter().map(String::as_str).collect();
        ctx.graph
            .chain(&[&video], &format!("split={count}"), &video_refs);
        let audio_parts: Option<Vec<String>> = audio.as_ref().map(|audio| {
            let parts: Vec<String> = (0..count).map(|_| ctx.graph.label("sa")).collect();
            let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
            ctx.graph.chain(&[audio], &format!("asplit={count}"), &refs);
            parts
        });

        let mut concat_inputs = Vec::new();
        for (i, rate) in rates.iter().enumerate() {
            let window = match i {
                0 => format!("end={}", fmt_num(bounds[1])),
                _ if i == count - 1 => format!("start={}", fmt_num(bounds[i])),
                _ => format!(
                    "start={}:end={}",
                    fmt_num(bounds[i]),
                    fmt_num(bounds[i + 1])
                ),
            };
            let out = ctx.graph.label("tv");
            ctx.graph.chain(
                &[&video_parts[i]],
                &format!("trim={window},setpts=(PTS-STARTPTS)/{}", fmt_num(*rate)),
                &[&out],
            );
            concat_inputs.push(out);
            if let Some(parts) = &audio_parts {
                let out = ctx.graph.label("ta");
                let mut filters = vec![format!("atrim={window}"), "asetpts=PTS-STARTPTS".into()];
                filters.extend(self.audio_filters(*rate, sample_rate));
                ctx.graph.chain(&[&parts[i]], &filters.join(","), &[&out]);
                concat_inputs.push(out);
            }
        }
        let video_out = ctx.graph.label("sp");
        let inputs: Vec<&str> = concat_inputs.iter().map(String::as_str).collect();
        match audio_parts {
            Some(_) => {
                let audio_out = ctx.graph.label("sp");
                ctx.graph.chain(
                    &inputs,
                    &format!("concat=n={count}:v=1:a=1"),
                    &[&video_out, &audio_out],
                );
                ctx.graph.set_audio(Some(audio_out));
            }
            None => ctx
                .graph
                .chain(&inputs, &format!("concat=n={count}:v=1:a=0"), &[&video_out]),
        }
        ctx.graph.set_video(video_out);
        if let Some(fps) = ctx.rate_text() {
            ctx.graph.video(format!("fps={fps}"));
        }
        ctx.stream.duration = lengths.iter().zip(&rates).map(|(l, r)| l / r).sum();
        Ok(())
    }
}

impl PipelineModule for SpeedParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        self.validate()?;
        match self.mode {
            SpeedMode::Fixed => {
                self.apply_fixed(ctx);
                Ok(())
            }
            SpeedMode::Segmented => self.apply_segmented(ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::{build_with, sample_info};
    use crate::pipeline::BuiltCommand;

    fn recipe_with(params: SpeedParams) -> crate::model::Recipe {
        let mut recipe = crate::model::Recipe::disabled();
        recipe.modules.speed = params;
        recipe
    }

    fn build(params: SpeedParams, seed: u64) -> Result<BuiltCommand, String> {
        build_with(&sample_info(), &recipe_with(params), seed)
    }

    fn fixed(rate: Range<f64>) -> SpeedParams {
        SpeedParams {
            enabled: true,
            rate,
            ..SpeedParams::default()
        }
    }

    fn segmented(count: Range<u32>, rate: Range<f64>, shuffle: bool) -> SpeedParams {
        SpeedParams {
            enabled: true,
            mode: SpeedMode::Segmented,
            segment_count: count,
            segment_rate: rate,
            shuffle,
            ..SpeedParams::default()
        }
    }

    #[test]
    fn atempo_chain_splits_out_of_range_rates() {
        assert_eq!(atempo_chain(1.5), ["atempo=1.5"]);
        assert_eq!(atempo_chain(4.0), ["atempo=2", "atempo=2"]);
        assert_eq!(atempo_chain(3.0), ["atempo=2", "atempo=1.5"]);
        assert_eq!(atempo_chain(0.25), ["atempo=0.5", "atempo=0.5"]);
        assert_eq!(atempo_chain(0.3), ["atempo=0.5", "atempo=0.6"]);
        assert!(atempo_chain(1.0).is_empty());
    }

    #[test]
    fn fixed_rate_keeps_pitch() {
        let built = build(fixed(Range::fixed(1.25)), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]setpts=PTS/1.25,fps=25[v0];[0:a:0]atempo=1.25[a1]")
        );
        assert_eq!(built.expected_duration, 8.0);
        assert_eq!(built.arg_after("-c:a"), Some("aac"));
    }

    #[test]
    fn fixed_rate_changes_pitch() {
        let params = SpeedParams {
            change_pitch: true,
            ..fixed(Range::fixed(0.5))
        };
        let built = build(params, 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]setpts=PTS/0.5,fps=25[v0];[0:a:0]asetrate=24000,aresample=48000[a1]")
        );
        assert_eq!(built.expected_duration, 20.0);
        // 采样率未知时退回 atempo
        let mut info = sample_info();
        info.audio[0].sample_rate = 0;
        let params = SpeedParams {
            change_pitch: true,
            ..fixed(Range::fixed(3.0))
        };
        let built = build_with(&info, &recipe_with(params), 1).unwrap();
        assert!(built
            .arg_after("-filter_complex")
            .unwrap()
            .ends_with("[0:a:0]atempo=2,atempo=1.5[a1]"));
        assert!(built.warnings[0].contains("采样率未知"));
    }

    #[test]
    fn rate_one_is_a_no_op_and_no_audio_is_fine() {
        let built = build(fixed(Range::fixed(1.0)), 1).unwrap();
        assert_eq!(built.arg_after("-filter_complex"), None);
        let mut info = sample_info();
        info.audio.clear();
        let built = build_with(&info, &recipe_with(fixed(Range::fixed(2.0))), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]setpts=PTS/2,fps=25[v0]")
        );
        assert_eq!(built.expected_duration, 5.0);
    }

    #[test]
    fn ranged_rate_is_reproducible_and_bounded() {
        let params = fixed(Range::new(1.1, 1.9));
        let a = build(params.clone(), 42).unwrap();
        let b = build(params.clone(), 42).unwrap();
        assert_eq!(a.args, b.args);
        assert_eq!(
            a.arg_after("-filter_complex"),
            Some("[0:v:0]setpts=PTS/1.63,fps=25[v0];[0:a:0]atempo=1.63[a1]")
        );
        for seed in 0..50 {
            let built = build(params.clone(), seed).unwrap();
            let rate = 10.0 / built.expected_duration;
            assert!((1.1 - 1e-9..=1.9 + 1e-9).contains(&rate), "{rate}");
        }
    }

    #[test]
    fn min_duration_lowers_rate() {
        // 10 秒 ×2 → 5 秒，下限 8 秒 → 倍率降为 1.25
        let params = SpeedParams {
            min_duration: 8.0,
            ..fixed(Range::fixed(2.0))
        };
        let built = build(params, 1).unwrap();
        assert!(built
            .arg_after("-filter_complex")
            .unwrap()
            .starts_with("[0:v:0]setpts=PTS/1.25,"));
        assert_eq!(built.expected_duration, 8.0);
        assert!(built.warnings.is_empty());
        // 源视频短于下限：降到 1 倍（不处理），并警告
        let params = SpeedParams {
            min_duration: 30.0,
            ..fixed(Range::fixed(2.0))
        };
        let built = build(params, 1).unwrap();
        assert_eq!(built.arg_after("-filter_complex"), None);
        assert_eq!(built.expected_duration, 10.0);
        assert!(built.warnings[0].contains("最短时长"));
        // 减速不受下限影响
        let params = SpeedParams {
            min_duration: 30.0,
            ..fixed(Range::fixed(0.5))
        };
        assert_eq!(build(params, 1).unwrap().expected_duration, 20.0);
    }

    #[test]
    fn segmented_ladder_filter_graph() {
        let built = build(segmented(Range::fixed(2), Range::new(0.5, 2.0), false), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some(
                "[0:v:0]split=2[sv0][sv1];[0:a:0]asplit=2[sa2][sa3];\
                 [sv0]trim=end=5,setpts=(PTS-STARTPTS)/0.5[tv4];\
                 [sa2]atrim=end=5,asetpts=PTS-STARTPTS,atempo=0.5[ta5];\
                 [sv1]trim=start=5,setpts=(PTS-STARTPTS)/2[tv6];\
                 [sa3]atrim=start=5,asetpts=PTS-STARTPTS,atempo=2[ta7];\
                 [tv4][ta5][tv6][ta7]concat=n=2:v=1:a=1[sp8][sp9];[sp8]fps=25[v10]"
            )
        );
        assert_eq!(built.arg_after("-map"), Some("[v10]"));
        assert!(built.args.contains(&"[sp9]".to_owned()));
        // 5/0.5 + 5/2
        assert!((built.expected_duration - 12.5).abs() < 1e-9);
    }

    #[test]
    fn segmented_three_segments_without_audio() {
        let mut info = sample_info();
        info.audio.clear();
        let params = segmented(Range::fixed(3), Range::new(1.0, 2.0), false);
        let built = build_with(&info, &recipe_with(params), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some(
                "[0:v:0]split=3[sv0][sv1][sv2];\
                 [sv0]trim=end=3.3333,setpts=(PTS-STARTPTS)/1[tv3];\
                 [sv1]trim=start=3.3333:end=6.6667,setpts=(PTS-STARTPTS)/1.5[tv4];\
                 [sv2]trim=start=6.6667,setpts=(PTS-STARTPTS)/2[tv5];\
                 [tv3][tv4][tv5]concat=n=3:v=1:a=0[sp6];[sp6]fps=25[v7]"
            )
        );
        assert!(built.args.contains(&"-an".to_owned()));
    }

    #[test]
    fn segmented_random_is_reproducible_and_bounded() {
        let params = segmented(Range::new(2, 6), Range::new(0.8, 1.6), true);
        let a = build(params.clone(), 42).unwrap();
        let b = build(params.clone(), 42).unwrap();
        assert_eq!(a.args, b.args);
        for seed in 0..30 {
            let built = build(params.clone(), seed).unwrap();
            let fc = built.arg_after("-filter_complex").unwrap();
            let n: usize = fc
                .split("concat=n=")
                .nth(1)
                .unwrap()
                .split(':')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            assert!((2..=6).contains(&n), "{n}");
            for part in fc.split("setpts=(PTS-STARTPTS)/").skip(1) {
                let rate: f64 = part.split('[').next().unwrap().parse().unwrap();
                assert!((0.8..=1.6).contains(&rate), "{rate}");
            }
            assert!(built.expected_duration > 10.0 / 1.6 - 1e-6);
            assert!(built.expected_duration < 10.0 / 0.8 + 1e-6);
        }
    }

    #[test]
    fn segmented_min_duration_scales_all_rates() {
        // 两段 ×2、×4：5/2 + 5/4 = 3.75 秒；下限 7.5 → 倍率减半
        let params = SpeedParams {
            min_duration: 7.5,
            ..segmented(Range::fixed(2), Range::new(2.0, 4.0), false)
        };
        let built = build(params, 1).unwrap();
        let fc = built.arg_after("-filter_complex").unwrap();
        assert!(fc.contains("setpts=(PTS-STARTPTS)/1[") && fc.contains("setpts=(PTS-STARTPTS)/2["));
        assert!((built.expected_duration - 7.5).abs() < 1e-9);
    }

    #[test]
    fn invalid_params_are_rejected() {
        let cases = [
            fixed(Range::fixed(0.1)),
            fixed(Range::new(1.0, 5.0)),
            segmented(Range::fixed(1), Range::fixed(1.5), true),
            segmented(Range::new(2, 30), Range::fixed(1.5), true),
            segmented(Range::fixed(2), Range::fixed(f64::NAN), true),
            SpeedParams {
                min_duration: -1.0,
                ..fixed(Range::fixed(1.5))
            },
        ];
        for params in cases {
            let err = build(params.clone(), 1).unwrap_err();
            assert!(err.starts_with("[pipe.speed]"), "{params:?}: {err}");
        }
        let mut info = sample_info();
        info.duration = 0.0;
        let params = segmented(Range::fixed(2), Range::fixed(1.5), true);
        assert!(build_with(&info, &recipe_with(params), 1).is_err());
    }
}
