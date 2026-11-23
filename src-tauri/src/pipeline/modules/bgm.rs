//! `pipe.bgm` 背景音：从素材来源（单文件，或目录里随机 / 依次取）给视频配背景音乐。
//!
//! 参数与取值范围（FEATURES.md 未规定处由本模块决定）：
//! - `musicVolume`、`originalVolume`：百分比范围值，0～300%，取值保留整数；
//!   默认音乐 50%、原声 100%。原声取到 0 时直接丢弃原声（静音）。
//! - `loopMusic`：音乐比视频短时是否循环（`-stream_loop -1`），默认开；关闭时只播一遍，之后为静音。
//! - `delay`：音乐延迟开始的秒数（范围值，0～600 秒，保留两位小数），默认 0。
//! - `fadeIn` / `fadeOut`：淡入 / 淡出秒数（0～60），默认各 1 秒。淡入从音乐开始（延迟之后）算起；
//!   淡出在音乐实际结束前完成：通常是视频结尾；音乐只播一遍且比视频短时是音乐自身的结尾
//!   （构建时用 `ctx.probe` 读取音乐时长）。
//! - 输出音频统一为 48 kHz 立体声（`ctx.stream.audio_channels = 2`）。
//!
//! 滤镜思路：
//! ```text
//! [n:a:0]aformat=sample_rates=48000:channel_layouts=stereo,volume=音乐,adelay=..,afade=in..,afade=out..[bgm]
//! [原声]aformat=..,volume=原声[orig]
//! [orig][bgm]amix=inputs=2:duration=longest:dropout_transition=0:normalize=0,apad[mix]
//! ```
//! 结尾统一 `apad` 补静音并 `atrim` 到预计的视频时长，再加输出参数 `-shortest`：输出时长始终由视频决定，
//! 音乐（无论循环与否、长短如何）不会延长视频，也不会比视频先结束。只靠 `-shortest` 不够精确
//! （机器繁忙时会多出将近 1 秒），所以已知时长时先截断。
//! 源视频没有音轨时只输出音乐；原声静音时用 `FilterGraph::discard_audio` 丢弃原声。
//! `amix` 关闭 `normalize`，各路音量就是设定值，原声结束后音乐音量不会跳变。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::{AssetSource, Range};
use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.bgm";

/// 目录模式下认作音乐的扩展名。
pub const MUSIC_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "wma"];
/// 音量上限（%）。
pub const MAX_VOLUME_PERCENT: f64 = 300.0;
/// 延迟上限（秒）。
pub const MAX_DELAY: f64 = 600.0;
/// 淡入 / 淡出上限（秒）。
pub const MAX_FADE: f64 = 60.0;

/// 混音前统一的音频格式。
const MIX_FORMAT: &str = "aformat=sample_rates=48000:channel_layouts=stereo";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BgmParams {
    pub enabled: bool,
    /// 音乐来源：单文件或目录
    pub source: AssetSource,
    /// 音乐音量（%）
    pub music_volume: Range<f64>,
    /// 原声音量（%），0 为静音
    pub original_volume: Range<f64>,
    /// 音乐比视频短时循环播放
    pub loop_music: bool,
    /// 音乐延迟开始（秒）
    pub delay: Range<f64>,
    /// 淡入（秒）
    pub fade_in: f64,
    /// 淡出（秒），在视频结尾前完成
    pub fade_out: f64,
}

impl Default for BgmParams {
    fn default() -> Self {
        Self {
            enabled: false,
            source: AssetSource::default(),
            music_volume: Range::fixed(50.0),
            original_volume: Range::fixed(100.0),
            loop_music: true,
            delay: Range::fixed(0.0),
            fade_in: 1.0,
            fade_out: 1.0,
        }
    }
}

impl BgmParams {
    /// 音乐在输出时间轴上实际结束的时刻：通常是视频结尾；只播一遍且比视频短时是音乐自身的结尾
    /// （用 `ctx.probe` 读取音乐时长）。视频时长未知时为 None。
    fn audible_end(
        &self,
        ctx: &mut BuildContext,
        music: &Path,
        delay: f64,
        duration: f64,
    ) -> Option<f64> {
        if duration <= 0.0 {
            return None;
        }
        if self.loop_music {
            return Some(duration);
        }
        match ctx.probe(music) {
            Ok(info) if info.duration > 0.0 => Some((delay + info.duration).min(duration)),
            Ok(_) => Some(duration),
            Err(e) => {
                if ctx.can_probe() {
                    ctx.warn(format!("无法读取音乐时长（{e}），淡出按视频结尾计算"));
                }
                Some(duration)
            }
        }
    }

    fn validate(&self) -> Result<(), String> {
        let in_range = |r: &Range<f64>, max: f64| {
            let (lo, hi) = r.ordered();
            lo >= 0.0 && hi <= max
        };
        if !in_range(&self.music_volume, MAX_VOLUME_PERCENT) {
            return Err(format!("音乐音量需在 0～{MAX_VOLUME_PERCENT}% 之间"));
        }
        if !in_range(&self.original_volume, MAX_VOLUME_PERCENT) {
            return Err(format!("原声音量需在 0～{MAX_VOLUME_PERCENT}% 之间"));
        }
        if !in_range(&self.delay, MAX_DELAY) {
            return Err(format!("延迟需在 0～{MAX_DELAY} 秒之间"));
        }
        for (name, v) in [("淡入", self.fade_in), ("淡出", self.fade_out)] {
            if !(0.0..=MAX_FADE).contains(&v) {
                return Err(format!("{name}时长需在 0～{MAX_FADE} 秒之间"));
            }
        }
        if self.source.is_empty() {
            return Err("请选择背景音乐文件或目录".into());
        }
        Ok(())
    }
}

impl PipelineModule for BgmParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        self.validate()?;
        // 取值顺序固定：素材、音乐音量、原声音量、延迟
        let music = self
            .source
            .pick(MUSIC_EXTENSIONS, ctx.task_index, &mut ctx.rng)?;
        let music_volume = self.music_volume.sample_rounded(&mut ctx.rng, 0) / 100.0;
        let original_volume = self.original_volume.sample_rounded(&mut ctx.rng, 0) / 100.0;
        let delay = self.delay.sample_rounded(&mut ctx.rng, 2);
        let duration = ctx.stream.duration;

        let input_args = if self.loop_music {
            vec!["-stream_loop".into(), "-1".into()]
        } else {
            Vec::new()
        };
        let input = ctx.add_input(input_args, &music)?;

        let mut filters = vec![
            MIX_FORMAT.to_owned(),
            format!("volume={}", fmt_num(music_volume)),
        ];
        if delay > 0.0 {
            filters.push(format!(
                "adelay=delays={}:all=1",
                (delay * 1000.0).round() as u64
            ));
            if duration > 0.0 && delay >= duration {
                ctx.warn("音乐延迟不短于视频时长，背景音不会被听到");
            }
        }
        if self.fade_in > 0.0 {
            filters.push(format!(
                "afade=t=in:st={}:d={}",
                fmt_num(delay),
                fmt_num(self.fade_in)
            ));
        }
        if self.fade_out > 0.0 {
            match self.audible_end(ctx, &music, delay, duration) {
                Some(end) => {
                    let d = self.fade_out.min(end - delay).max(0.0);
                    if d > 0.0 {
                        filters.push(format!(
                            "afade=t=out:st={}:d={}",
                            fmt_num(end - d),
                            fmt_num(d)
                        ));
                    }
                }
                None => ctx.warn("视频时长未知，已忽略淡出"),
            }
        }
        let bgm = ctx.graph.label("bgm");
        ctx.graph.chain(
            &[&format!("{input}:a:0")],
            &filters.join(","),
            &[bgm.as_str()],
        );

        let out = ctx.graph.label("mix");
        // 补静音后按视频时长截断：只靠 -shortest 时，机器繁忙会多出将近 1 秒
        let pad = if duration > 0.0 {
            format!("apad,atrim=duration={}", fmt_num(duration))
        } else {
            "apad".to_owned()
        };
        if ctx.graph.has_audio() && original_volume > 0.0 {
            let original = ctx.graph.take_audio().unwrap_or_default();
            let orig = ctx.graph.label("orig");
            ctx.graph.chain(
                &[&original],
                &format!("{MIX_FORMAT},volume={}", fmt_num(original_volume)),
                &[orig.as_str()],
            );
            ctx.graph.chain(
                &[&orig, &bgm],
                &format!("amix=inputs=2:duration=longest:dropout_transition=0:normalize=0,{pad}"),
                &[out.as_str()],
            );
        } else {
            // 原声静音（或没有原声）：丢弃原声
            ctx.graph.discard_audio();
            ctx.graph.chain(&[&bgm], &pad, &[out.as_str()]);
        }
        ctx.graph.set_audio(Some(out));
        ctx.stream.audio_channels = 2;
        // 音频已补成无限长，由视频决定输出时长
        ctx.encode.output_args.push("-shortest".into());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PickOrder, Recipe};
    use crate::pipeline::testing::{build_with, sample_info};
    use std::path::PathBuf;

    /// 测试用临时目录（放几个假的音乐文件，构建命令时不读内容）。
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str, files: &[&str]) -> Self {
            let dir = std::env::temp_dir().join(format!("cr-bgm-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            for f in files {
                std::fs::write(dir.join(f), b"x").unwrap();
            }
            Self(dir)
        }

        fn path(&self, name: &str) -> String {
            self.0.join(name).to_string_lossy().into_owned()
        }

        fn dir(&self) -> String {
            self.0.to_string_lossy().into_owned()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn recipe_with(params: BgmParams) -> Recipe {
        let mut recipe = Recipe::disabled();
        recipe.modules.bgm = BgmParams {
            enabled: true,
            ..params
        };
        recipe
    }

    fn file_source(path: String) -> AssetSource {
        AssetSource::File { path }
    }

    /// 某个输入前面的参数（`-i` 之前）。
    fn args_before_input(args: &[String], path: &str) -> Vec<String> {
        let pos = args.iter().position(|a| a == path).expect("找不到输入");
        let start = args[..pos - 1]
            .iter()
            .rposition(|a| a == "-i" || a.ends_with(".mp4"))
            .map_or(0, |p| p + 1);
        args[start..pos - 1].to_vec()
    }

    #[test]
    fn mixes_music_with_original_audio() {
        let s = Scratch::new("mix", &["song.mp3"]);
        let song = s.path("song.mp3");
        let params = BgmParams {
            source: file_source(song.clone()),
            music_volume: Range::fixed(80.0),
            original_volume: Range::fixed(50.0),
            delay: Range::fixed(1.5),
            fade_in: 2.0,
            fade_out: 3.0,
            ..BgmParams::default()
        };
        let built = build_with(&sample_info(), &recipe_with(params), 42).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some(
                "[1:a:0]aformat=sample_rates=48000:channel_layouts=stereo,volume=0.8,\
                 adelay=delays=1500:all=1,afade=t=in:st=1.5:d=2,afade=t=out:st=7:d=3[bgm0];\
                 [0:a:0]aformat=sample_rates=48000:channel_layouts=stereo,volume=0.5[orig2];\
                 [orig2][bgm0]amix=inputs=2:duration=longest:dropout_transition=0:normalize=0,apad,atrim=duration=10[mix1]"
            )
        );
        assert_eq!(
            args_before_input(&built.args, &song),
            ["-stream_loop", "-1"]
        );
        assert_eq!(built.arg_after("-map"), Some("0:v:0"));
        let maps: Vec<&String> = built
            .args
            .iter()
            .zip(built.args.iter().skip(1))
            .filter(|(f, _)| *f == "-map")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(maps, ["0:v:0", "[mix1]"]);
        assert!(built.args.contains(&"-shortest".to_owned()));
        assert_eq!(built.arg_after("-c:a"), Some("aac"));
        assert_eq!(built.expected_duration, 10.0);
    }

    #[test]
    fn no_loop_no_fades_and_muted_original() {
        let s = Scratch::new("mute", &["song.m4a"]);
        let song = s.path("song.m4a");
        let params = BgmParams {
            source: file_source(song.clone()),
            original_volume: Range::fixed(0.0),
            loop_music: false,
            fade_in: 0.0,
            fade_out: 0.0,
            ..BgmParams::default()
        };
        let built = build_with(&sample_info(), &recipe_with(params), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some(
                "[1:a:0]aformat=sample_rates=48000:channel_layouts=stereo,volume=0.5[bgm0];\
                 [0:a:0]anullsink;[bgm0]apad,atrim=duration=10[mix1]"
            )
        );
        assert!(args_before_input(&built.args, &song).is_empty());
    }

    #[test]
    fn play_once_short_music_fades_out_at_its_own_end() {
        use crate::ffmpeg::MediaInfo;
        use crate::pipeline::testing::build_with_probe;
        let s = Scratch::new("once", &["short.mp3"]);
        let song = s.path("short.mp3");
        let probe = |p: &std::path::Path| -> Result<MediaInfo, String> {
            assert!(p.ends_with("short.mp3"));
            Ok(MediaInfo {
                duration: 3.0,
                ..MediaInfo::default()
            })
        };
        let params = BgmParams {
            source: file_source(song),
            loop_music: false,
            delay: Range::fixed(1.0),
            fade_in: 0.0,
            fade_out: 1.5,
            ..BgmParams::default()
        };
        let built =
            build_with_probe(&sample_info(), &recipe_with(params.clone()), 1, &probe).unwrap();
        let fc = built.arg_after("-filter_complex").unwrap();
        // 音乐 1～4 秒：在 2.5 秒开始淡出，而不是视频结尾（10 秒）
        assert!(fc.contains("afade=t=out:st=2.5:d=1.5[bgm0]"), "{fc}");
        // 循环播放时仍按视频结尾
        let looped = BgmParams {
            loop_music: true,
            ..params.clone()
        };
        let built = build_with_probe(&sample_info(), &recipe_with(looped), 1, &probe).unwrap();
        assert!(built
            .arg_after("-filter_complex")
            .unwrap()
            .contains("afade=t=out:st=8.5:d=1.5"));
        // 读不到时长：按视频结尾并给出提示
        let failing =
            |_: &std::path::Path| -> Result<MediaInfo, String> { Err("坏文件".into()) };
        let built = build_with_probe(&sample_info(), &recipe_with(params), 1, &failing).unwrap();
        assert!(built
            .arg_after("-filter_complex")
            .unwrap()
            .contains("afade=t=out:st=8.5:d=1.5"));
        assert!(built
            .warnings
            .iter()
            .any(|w| w.contains("无法读取音乐时长")));
    }

    #[test]
    fn works_without_source_audio() {
        let s = Scratch::new("silent", &["song.mp3"]);
        let mut info = sample_info();
        info.audio.clear();
        let params = BgmParams {
            source: file_source(s.path("song.mp3")),
            ..BgmParams::default()
        };
        let built = build_with(&info, &recipe_with(params), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some(
                "[1:a:0]aformat=sample_rates=48000:channel_layouts=stereo,volume=0.5,\
                 afade=t=in:st=0:d=1,afade=t=out:st=9:d=1[bgm0];[bgm0]apad,atrim=duration=10[mix1]"
            )
        );
        assert!(!built.args.contains(&"-an".to_owned()));
        assert!(built.args.iter().any(|a| a == "[mix1]"));
    }

    #[test]
    fn muted_original_coexists_with_video_filters() {
        let s = Scratch::new("chain", &["a.mp3"]);
        let mut recipe = recipe_with(BgmParams {
            source: file_source(s.path("a.mp3")),
            original_volume: Range::fixed(0.0),
            fade_in: 0.0,
            fade_out: 0.0,
            ..BgmParams::default()
        });
        // 帧率模块只改视频，不影响音频链
        recipe.modules.fps.enabled = true;
        let built = build_with(&sample_info(), &recipe, 1).unwrap();
        let fc = built.arg_after("-filter_complex").unwrap();
        assert!(fc.contains("[0:a:0]anullsink"), "{fc}");
        assert!(fc.ends_with("[0:v:0]fps=30[v2]"), "{fc}");
    }

    #[test]
    fn ranges_are_reproducible_and_bounded() {
        let s = Scratch::new("range", &["a.mp3"]);
        let params = BgmParams {
            source: file_source(s.path("a.mp3")),
            music_volume: Range::new(20.0, 60.0),
            original_volume: Range::new(50.0, 100.0),
            delay: Range::new(0.5, 2.0),
            ..BgmParams::default()
        };
        let recipe = recipe_with(params);
        let a = build_with(&sample_info(), &recipe, 42).unwrap();
        let b = build_with(&sample_info(), &recipe, 42).unwrap();
        assert_eq!(a.args, b.args);
        for seed in 0..50 {
            let built = build_with(&sample_info(), &recipe, seed).unwrap();
            let fc = built.arg_after("-filter_complex").unwrap();
            let value_after = |key: &str, from: usize| -> f64 {
                let start = fc[from..].find(key).unwrap() + from + key.len();
                let end = fc[start..].find([',', '[', ':']).unwrap() + start;
                fc[start..end].parse().unwrap()
            };
            let music = value_after("volume=", 0);
            assert!((0.2..=0.6).contains(&music), "{fc}");
            let delay_ms = value_after("delays=", 0);
            assert!((500.0..=2000.0).contains(&delay_ms), "{fc}");
            let orig_at = fc.find("[0:a:0]").unwrap();
            let orig = value_after("volume=", orig_at);
            assert!((0.5..=1.0).contains(&orig), "{fc}");
        }
    }

    #[test]
    fn directory_pick_random_and_sequential() {
        let s = Scratch::new("dir", &["1.mp3", "2.m4a", "3.wav", "cover.jpg"]);
        let random = BgmParams {
            source: AssetSource::Directory {
                path: s.dir(),
                recursive: false,
                order: PickOrder::Random,
            },
            ..BgmParams::default()
        };
        let picked = |params: &BgmParams, seed: u64| -> String {
            let built = build_with(&sample_info(), &recipe_with(params.clone()), seed).unwrap();
            let pos = built.args.iter().position(|a| a == "-i").unwrap();
            let pos = pos
                + 1
                + built.args[pos + 1..]
                    .iter()
                    .position(|a| a == "-i")
                    .unwrap();
            built.args[pos + 1].clone()
        };
        let first = picked(&random, 9);
        assert_eq!(first, picked(&random, 9));
        let names: Vec<String> = (0..30).map(|seed| picked(&random, seed)).collect();
        assert!(names.iter().all(|n| !n.ends_with("cover.jpg")));
        assert!(names.iter().any(|n| n.ends_with("1.mp3")));
        assert!(names.iter().any(|n| n.ends_with("3.wav")));

        let sequential = BgmParams {
            source: AssetSource::Directory {
                path: s.dir(),
                recursive: false,
                order: PickOrder::Sequential,
            },
            ..BgmParams::default()
        };
        // build_with 的队列序号为 0：依次取第一个
        assert!(picked(&sequential, 3).ends_with("1.mp3"));
    }

    #[test]
    fn invalid_params_are_rejected() {
        let s = Scratch::new("invalid", &["a.mp3"]);
        let empty = Scratch::new("empty", &["readme.txt"]);
        let err =
            |params: BgmParams| build_with(&sample_info(), &recipe_with(params), 1).unwrap_err();

        assert!(err(BgmParams::default()).contains("请选择"));
        let missing = err(BgmParams {
            source: file_source(s.path("missing.mp3")),
            ..BgmParams::default()
        });
        assert!(missing.contains("不存在"), "{missing}");
        let empty_dir = err(BgmParams {
            source: AssetSource::Directory {
                path: empty.dir(),
                recursive: false,
                order: PickOrder::Random,
            },
            ..BgmParams::default()
        });
        assert!(empty_dir.contains("没有可用文件"), "{empty_dir}");
        let ok = file_source(s.path("a.mp3"));
        assert!(err(BgmParams {
            source: ok.clone(),
            music_volume: Range::new(-1.0, 50.0),
            ..BgmParams::default()
        })
        .contains("音乐音量"));
        assert!(err(BgmParams {
            source: ok.clone(),
            original_volume: Range::fixed(500.0),
            ..BgmParams::default()
        })
        .contains("原声音量"));
        assert!(err(BgmParams {
            source: ok.clone(),
            fade_out: -1.0,
            ..BgmParams::default()
        })
        .contains("淡出"));
        assert!(err(BgmParams {
            source: ok,
            delay: Range::fixed(1000.0),
            ..BgmParams::default()
        })
        .contains("延迟"));
    }

    #[test]
    fn serde_shape() {
        let json = serde_json::to_value(BgmParams::default()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "enabled": false,
                "source": {"mode": "file", "path": ""},
                "musicVolume": {"min": 50.0, "max": 50.0},
                "originalVolume": {"min": 100.0, "max": 100.0},
                "loopMusic": true,
                "delay": {"min": 0.0, "max": 0.0},
                "fadeIn": 1.0,
                "fadeOut": 1.0
            })
        );
    }
}
