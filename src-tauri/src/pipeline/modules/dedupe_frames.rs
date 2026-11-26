//! `pipe.dedupe-frames` 重复帧移除：用 `mpdecimate` 去掉与前一帧（几乎）相同的连续帧。
//!
//! 参数与取值约定（docs/specs/pipeline.md 未规定的部分在此说明）：
//! - `sensitivity`：判断“重复”的灵敏度，对应 `mpdecimate` 的 `hi/lo/frac` 阈值：
//!   - `low`（只去几乎完全相同的帧）：`hi=384:lo=160:frac=0.2`；
//!   - `normal`（FFmpeg 默认）：`hi=768:lo=320:frac=0.33`，滤镜写作不带参数的 `mpdecimate`；
//!   - `high`（细微变化也算重复）：`hi=1536:lo=640:frac=0.5`。
//! - `maxDrop`：最多连续去掉多少帧（0 表示不限制，对应 `max`），避免长时间静止画面被整段去掉。
//!
//! 音画同步与时间戳：被去掉的帧不重排时间戳，剩下的帧保留原时间戳（可变帧率输出），
//! 由前一帧延长显示补位，所以音频不需要处理、音画始终对齐，时长也不变（`ctx.stream.duration` 不变）。
//! 是哪些帧重复取决于画面内容，构建命令时无法预知，因此不采用把视频压缩变短（`setpts=N/FRAME_RATE/TB`）
//! 的做法——那样音频无法同步。为保证可变帧率被保留，追加输出参数 `-fps_mode:v vfr`。
//!
//! 限制：如果视频末尾是一段重复帧，最后一帧的显示时长只按一帧计，视频流会比音频略短
//! （播放器会停留在最后一帧）。后面若启用了帧率、变速、抽帧或动态缩放，它们会重新输出恒定帧率
//! （抽帧、动态缩放通过 `ctx.ensure_cfr()`），去掉的帧会被前一帧重新补上（画面相同）——
//! 此时构建会给出警告。本模块把 `ctx.stream.vfr` 置为 true。

use serde::{Deserialize, Serialize};

use crate::pipeline::{BuildContext, PipelineModule};

pub const ID: &str = "pipe.dedupe-frames";

pub const MAX_DROP_LIMIT: u32 = 1000;

/// 会重新输出恒定帧率的后续模块（ID, 名称）：它们会把被去掉的帧补回来。
const CFR_MODULES: [(&str, &str); 4] = [
    ("pipe.drop-frames", "抽帧"),
    ("pipe.speed", "变速"),
    ("pipe.zoom", "动态缩放"),
    ("pipe.fps", "帧率"),
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DedupeSensitivity {
    /// 只去几乎完全相同的帧
    Low,
    /// FFmpeg 默认阈值
    #[default]
    Normal,
    /// 细微变化也视为重复
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DedupeFramesParams {
    pub enabled: bool,
    pub sensitivity: DedupeSensitivity,
    /// 最多连续去掉的帧数，0 表示不限制
    pub max_drop: u32,
}

impl Default for DedupeFramesParams {
    fn default() -> Self {
        Self {
            enabled: false,
            sensitivity: DedupeSensitivity::Normal,
            max_drop: 0,
        }
    }
}

impl DedupeFramesParams {
    fn filter(&self) -> String {
        let mut options: Vec<String> = match self.sensitivity {
            DedupeSensitivity::Low => vec!["hi=384".into(), "lo=160".into(), "frac=0.2".into()],
            DedupeSensitivity::Normal => Vec::new(),
            DedupeSensitivity::High => vec!["hi=1536".into(), "lo=640".into(), "frac=0.5".into()],
        };
        if self.max_drop > 0 {
            options.push(format!("max={}", self.max_drop));
        }
        if options.is_empty() {
            "mpdecimate".into()
        } else {
            format!("mpdecimate={}", options.join(":"))
        }
    }
}

impl PipelineModule for DedupeFramesParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        if self.max_drop > MAX_DROP_LIMIT {
            return Err(format!("最多连续去掉的帧数需在 0～{MAX_DROP_LIMIT} 之间"));
        }
        let later: Vec<&str> = ctx
            .later_modules()
            .iter()
            .filter_map(|id| {
                CFR_MODULES
                    .iter()
                    .find(|(m, _)| m == id)
                    .map(|(_, name)| *name)
            })
            .collect();
        if !later.is_empty() {
            ctx.warn(format!(
                "后面启用的「{}」会重新输出恒定帧率，被去掉的重复帧会由前一帧重新补上（画面相同），\
                 重复帧移除的效果会被抵消",
                later.join("」「")
            ));
        }
        ctx.graph.video(self.filter());
        ctx.stream.vfr = true;
        ctx.encode
            .output_args
            .extend(["-fps_mode:v".to_owned(), "vfr".to_owned()]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::{build_with, sample_info};
    use crate::pipeline::BuiltCommand;

    fn build(params: DedupeFramesParams) -> Result<BuiltCommand, String> {
        let mut recipe = crate::model::Recipe::disabled();
        recipe.modules.dedupe_frames = params;
        build_with(&sample_info(), &recipe, 1)
    }

    #[test]
    fn default_thresholds() {
        let built = build(DedupeFramesParams {
            enabled: true,
            ..DedupeFramesParams::default()
        })
        .unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]mpdecimate[v0]")
        );
        assert_eq!(built.arg_after("-fps_mode:v"), Some("vfr"));
        // 音频不经过滤镜，可直接复制；时长不变
        assert_eq!(built.arg_after("-c:a"), Some("copy"));
        assert_eq!(built.expected_duration, 10.0);
        // 输出参数在输出文件之前
        assert_eq!(built.args.last().map(String::as_str), Some("out.mp4"));
    }

    #[test]
    fn sensitivity_and_max_drop() {
        let built = build(DedupeFramesParams {
            enabled: true,
            sensitivity: DedupeSensitivity::High,
            max_drop: 5,
        })
        .unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]mpdecimate=hi=1536:lo=640:frac=0.5:max=5[v0]")
        );
        let built = build(DedupeFramesParams {
            enabled: true,
            sensitivity: DedupeSensitivity::Low,
            max_drop: 0,
        })
        .unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]mpdecimate=hi=384:lo=160:frac=0.2[v0]")
        );
        let built = build(DedupeFramesParams {
            enabled: true,
            sensitivity: DedupeSensitivity::Normal,
            max_drop: 3,
        })
        .unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]mpdecimate=max=3[v0]")
        );
    }

    #[test]
    fn warns_when_a_later_module_forces_constant_frame_rate() {
        let on = DedupeFramesParams {
            enabled: true,
            ..DedupeFramesParams::default()
        };
        // 单独使用：没有警告
        assert!(build(on.clone()).unwrap().warnings.is_empty());

        let mut recipe = crate::model::Recipe::default(); // 帧率默认开启
        recipe.modules.dedupe_frames = on.clone();
        let built = build_with(&sample_info(), &recipe, 1).unwrap();
        assert_eq!(built.warnings.len(), 1, "{:?}", built.warnings);
        assert!(built.warnings[0].starts_with("[pipe.dedupe-frames]"));
        assert!(
            built.warnings[0].contains("「帧率」"),
            "{:?}",
            built.warnings
        );

        recipe.modules.drop_frames.enabled = true;
        let built = build_with(&sample_info(), &recipe, 1).unwrap();
        assert!(
            built.warnings[0].contains("「抽帧」「帧率」"),
            "{:?}",
            built.warnings
        );
        // 抽帧先恢复恒定帧率再丢帧
        let fc = built.arg_after("-filter_complex").unwrap();
        assert!(fc.starts_with("[0:v:0]mpdecimate,fps=25,select="), "{fc}");
    }

    #[test]
    fn invalid_max_drop_is_rejected() {
        let err = build(DedupeFramesParams {
            enabled: true,
            max_drop: 5000,
            ..DedupeFramesParams::default()
        })
        .unwrap_err();
        assert!(err.contains("连续去掉"), "{err}");
    }

    #[test]
    fn serde_shape() {
        let parsed: DedupeFramesParams =
            serde_json::from_str(r#"{"enabled":true,"sensitivity":"high"}"#).unwrap();
        assert_eq!(parsed.sensitivity, DedupeSensitivity::High);
        assert_eq!(parsed.max_drop, 0);
        let json = serde_json::to_value(DedupeFramesParams::default()).unwrap();
        assert_eq!(json["maxDrop"], 0);
        assert_eq!(json["sensitivity"], "normal");
    }
}
