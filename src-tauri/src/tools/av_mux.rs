//! `tool.av-mux` 音视频合成：音频文件夹 + 视频文件夹，配对后每对合成一个视频。
//!
//! 参数与取值约定（FEATURES 未规定的部分）：
//! - 配对 `pairing`：
//!   - `name`（默认）：文件名（不含扩展名，忽略大小写）相同的配成一对，找不到同名音频的视频跳过；
//!   - `order`：两边各按自然顺序一一配对，多出的文件忽略；
//!   - `random`：每个视频随机取一个音频（可重复取）。在规划时用 `req.rng`（本次运行的种子派生）选好，
//!     所以每个工作项就是“视频 + 选中的音频”，界面上直接显示配对结果，同一种子结果相同。
//! - 文件夹是否包含子目录按设置“目录模式包含子目录”。音频扩展名见 [`AUDIO_EXTENSIONS`]。
//! - 原声 `keepOriginal`：关闭（默认）时用新音频替换原声；开启时与原声混合（`amix`，不做归一化）。
//!   `originalVolume`、`audioVolume` 为音量百分比（0～400，默认 100）。视频没有原声时只用新音频（提示）。
//! - 时长跟随视频（`-t` 视频时长）：音频较短时 `loopAudio`（默认开）循环播放，关闭时用静音补齐；较长时截断。
//! - 视频直接复制（不重编码，所以不涉及码率设置）；音频统一为 48kHz 立体声 AAC。容器按“目标格式”设置
//!   （保持源格式但装不下 AAC 时用 mkv）。输出名与视频相同（规则同批量处理）。
//! - 变速适应（设置 `set.speed-fit`）：开启后，视频与新音频时长不同（相差超过 0.5%）时把**视频**整体变速到
//!   新音频的时长（`setpts`，重编码视频，码率按框架设置），保留的原声用 `atempo` 同步变速；新音频不循环、不截断。
//!   变速倍数超出 0.25～4 时不变速，按上面的“时长跟随视频”处理并警告。见 [`crate::pipeline::speed_fit`]。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ffmpeg::path_arg;
use crate::fsutil::VIDEO_EXTENSIONS;
use crate::model::SeededRng;
use crate::pipeline::fmt_num;
use crate::pipeline::speed_fit::{speed_fit, SpeedFit};
use crate::tools::context::Encoding;
use crate::tools::media::{container_accepts, copy_container, secs, StreamKind};
use crate::tools::plan::ItemInput;
use crate::tools::{
    FolderSlot, OutputSpec, PlanRequest, Tool, ToolContext, ToolError, ToolInputs, WorkItem,
};

pub const ID: &str = "tool.av-mux";
pub const AUDIO: &str = "audio";
pub const VIDEO: &str = "video";
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "wma"];
pub const MAX_VOLUME: f64 = 400.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Pairing {
    #[default]
    Name,
    Order,
    Random,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AvMuxParams {
    pub pairing: Pairing,
    /// 保留原声并与新音频混合
    pub keep_original: bool,
    /// 原声音量（%）
    pub original_volume: f64,
    /// 新音频音量（%）
    pub audio_volume: f64,
    /// 音频比视频短时循环
    pub loop_audio: bool,
}

impl Default for AvMuxParams {
    fn default() -> Self {
        Self {
            pairing: Pairing::Name,
            keep_original: false,
            original_volume: 100.0,
            audio_volume: 100.0,
            loop_audio: true,
        }
    }
}

impl AvMuxParams {
    fn validate(&self) -> Result<(), String> {
        for v in [self.original_volume, self.audio_volume] {
            if !(0.0..=MAX_VOLUME).contains(&v) {
                return Err(format!("音量必须在 0～{}% 之间", fmt_num(MAX_VOLUME)));
            }
        }
        Ok(())
    }
}

fn stem_key(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 配对：每个视频与它的音频（`random` 时用 `rng` 为每个视频随机选一个，可重复）。
pub fn pair_files(
    videos: &[PathBuf],
    audios: &[PathBuf],
    pairing: Pairing,
    rng: &mut SeededRng,
) -> Vec<(PathBuf, PathBuf)> {
    match pairing {
        Pairing::Name => {
            let mut by_name: HashMap<String, &PathBuf> = HashMap::new();
            for audio in audios {
                by_name.entry(stem_key(audio)).or_insert(audio);
            }
            videos
                .iter()
                .filter_map(|v| by_name.get(&stem_key(v)).map(|a| (v.clone(), (*a).clone())))
                .collect()
        }
        Pairing::Order => videos
            .iter()
            .zip(audios)
            .map(|(v, a)| (v.clone(), a.clone()))
            .collect(),
        Pairing::Random if audios.is_empty() => Vec::new(),
        Pairing::Random => videos
            .iter()
            .map(|v| (v.clone(), audios[rng.index(audios.len())].clone()))
            .collect(),
    }
}

/// 音频滤镜图，输出 `[aout]`（新音频是输入 1，原声是输入 0）。
pub fn audio_graph(keep_original: bool, original_volume: f64, audio_volume: f64) -> String {
    let conform = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";
    let new_volume = fmt_num(audio_volume / 100.0);
    if keep_original {
        format!(
            "[0:a:0]{conform},volume={}[a0];[1:a:0]{conform},volume={new_volume}[a1];\
             [a0][a1]amix=inputs=2:duration=longest:dropout_transition=0:normalize=0,apad[aout]",
            fmt_num(original_volume / 100.0)
        )
    } else {
        format!("[1:a:0]{conform},volume={new_volume},apad[aout]")
    }
}

/// 变速适应时的滤镜图：视频变速输出 `[vout]`；新音频不变、保留的原声同步变速，输出 `[aout]`。
pub fn fit_graph(
    fit: &SpeedFit,
    keep_original: bool,
    original_volume: f64,
    audio_volume: f64,
) -> String {
    let conform = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";
    let new_volume = fmt_num(audio_volume / 100.0);
    let video = format!("[0:v:0]{}[vout]", fit.video);
    if keep_original {
        format!(
            "{video};[0:a:0]{conform},{},volume={}[a0];[1:a:0]{conform},volume={new_volume}[a1];[a0][a1]amix=inputs=2:duration=longest:dropout_transition=0:normalize=0[aout]",
            fit.audio,
            fmt_num(original_volume / 100.0)
        )
    } else {
        format!("{video};[1:a:0]{conform},volume={new_volume}[aout]")
    }
}

/// 变速适应时的参数（重编码视频，时长为新音频时长；不含通用开头参数）。
pub fn fit_args(
    video: &str,
    audio: &str,
    graph: &str,
    video_args: Vec<String>,
    duration: f64,
    container: &str,
    output: &str,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-i".into(),
        video.into(),
        "-i".into(),
        audio.into(),
        "-filter_complex".into(),
        graph.into(),
        "-map".into(),
        "[vout]".into(),
        "-map".into(),
        "[aout]".into(),
    ];
    args.extend(video_args);
    args.extend(Encoding::audio_args(2));
    args.extend(["-t".into(), secs(duration), "-sn".into(), "-dn".into()]);
    if matches!(container, "mp4" | "mov" | "m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

/// 合成参数（不含通用开头参数）。
pub fn mux_args(
    video: &str,
    audio: &str,
    graph: &str,
    loop_audio: bool,
    duration: f64,
    container: &str,
    output: &str,
) -> Vec<String> {
    let mut args: Vec<String> = vec!["-i".into(), video.into()];
    if loop_audio {
        args.extend(["-stream_loop".into(), "-1".into()]);
    }
    args.extend([
        "-i".into(),
        audio.into(),
        "-filter_complex".into(),
        graph.into(),
        "-map".into(),
        "0:V:0".into(),
        "-map".into(),
        "[aout]".into(),
        "-c:v".into(),
        "copy".into(),
    ]);
    args.extend(Encoding::audio_args(2));
    args.extend(["-t".into(), secs(duration), "-sn".into(), "-dn".into()]);
    if matches!(container, "mp4" | "mov" | "m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

impl Tool for AvMuxParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn inputs(&self) -> ToolInputs {
        const FOLDERS: &[FolderSlot] = &[
            FolderSlot {
                key: AUDIO,
                label: "音频文件夹",
            },
            FolderSlot {
                key: VIDEO,
                label: "视频文件夹",
            },
        ];
        ToolInputs::folders(FOLDERS)
    }

    fn plan(&self, req: &PlanRequest) -> Result<Vec<WorkItem>, String> {
        self.validate()?;
        let audio_root = req.folder(AUDIO)?.to_path_buf();
        let video_root = req.folder(VIDEO)?.to_path_buf();
        let audios = req.folder_files(AUDIO, AUDIO_EXTENSIONS)?;
        let videos = req.folder_files(VIDEO, VIDEO_EXTENSIONS)?;
        let pairs = pair_files(&videos, &audios, self.pairing, &mut req.rng(ID));
        if pairs.is_empty() {
            return Err("没有配对成功的音频与视频（按文件名配对时需要文件名相同）".into());
        }
        Ok(pairs
            .into_iter()
            .enumerate()
            .map(|(k, (video, audio))| {
                let label = format!("{} + {}", name_of(&video), name_of(&audio));
                let inputs = vec![
                    ItemInput {
                        path: video,
                        info: None,
                        source_root: Some(video_root.clone()),
                    },
                    ItemInput {
                        path: audio,
                        info: None,
                        source_root: Some(audio_root.clone()),
                    },
                ];
                WorkItem::new(format!("{ID}#{}", k + 1), label, k + 1, inputs)
            })
            .collect())
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let info = ctx.info(0)?;
        let video = info
            .video
            .as_ref()
            .ok_or_else(|| format!("{} 没有视频流", info.path))?;
        if ctx.inputs().len() < 2 {
            return Err("没有配对的音频".into());
        }
        // 配对（含随机配对）在规划时已经确定：第 1 个输入就是要用的音频
        let audio_path = ctx.input_path(1)?.to_path_buf();
        let audio_info = ctx.info(1)?;
        if audio_info.audio.is_empty() {
            return Err(format!("{} 没有音频流", audio_path.display()).into());
        }
        let keep = self.keep_original && !info.audio.is_empty();
        if self.keep_original && !keep {
            ctx.note("视频没有原声，只使用新音频");
        }
        let video_path = ctx.input_path(0)?.to_path_buf();
        let mut container = copy_container(ctx.settings(), &video_path, &video.codec);
        if container_accepts(&container, StreamKind::Audio, "aac") == Some(false) {
            container = "mkv".into();
        }
        if ctx.settings().speed_fit {
            match speed_fit(info.duration, audio_info.duration) {
                Ok(Some(fit)) => {
                    let container = ctx.video_extension(0)?;
                    let out = ctx.output(&OutputSpec::file(&container))?;
                    ctx.note(format!(
                        "变速适应：视频 {} 秒 → 音频 {} 秒（{} 倍时长）",
                        fmt_num(info.duration),
                        fmt_num(audio_info.duration),
                        fmt_num(fit.factor)
                    ));
                    let graph = fit_graph(&fit, keep, self.original_volume, self.audio_volume);
                    let (video_arg, audio_arg, out_arg) = (
                        path_arg(&video_path)?,
                        path_arg(&audio_path)?,
                        path_arg(&out)?,
                    );
                    let duration = audio_info.duration;
                    return ctx.run_encode("音视频合成（变速适应）", duration, |enc| {
                        Ok(fit_args(
                            &video_arg,
                            &audio_arg,
                            &graph,
                            enc.video_args(&container),
                            duration,
                            &container,
                            &out_arg,
                        ))
                    });
                }
                Ok(None) => {}
                Err(e) => ctx.warn(format!("{e}，未变速")),
            }
        }
        let out = ctx.output(&OutputSpec::file(&container))?;
        let graph = audio_graph(keep, self.original_volume, self.audio_volume);
        ctx.run_ffmpeg(
            "音视频合成",
            mux_args(
                &path_arg(&video_path)?,
                &path_arg(&audio_path)?,
                &graph,
                self.loop_audio,
                info.duration,
                &container,
                &path_arg(&out)?,
            ),
            info.duration,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn pairing_by_name_order_and_random() {
        let videos = paths(&["v/A.mp4", "v/b.mp4", "v/c.mp4"]);
        let audios = paths(&["a/b.mp3", "a/a.m4a", "a/z.wav"]);
        let rng = || SeededRng::new(5);
        let by_name = pair_files(&videos, &audios, Pairing::Name, &mut rng());
        assert_eq!(
            by_name,
            [
                (PathBuf::from("v/A.mp4"), PathBuf::from("a/a.m4a")),
                (PathBuf::from("v/b.mp4"), PathBuf::from("a/b.mp3")),
            ]
        );
        let by_order = pair_files(&videos, &audios[..2], Pairing::Order, &mut rng());
        assert_eq!(by_order.len(), 2);
        assert_eq!(by_order[1].1, PathBuf::from("a/a.m4a"));
        // 随机配对：每个视频一个音频，同一种子结果相同，不同种子（多数情况下）不同
        let random = pair_files(&videos, &audios, Pairing::Random, &mut rng());
        assert_eq!(random.len(), 3);
        assert!(random.iter().all(|(_, a)| audios.contains(a)));
        assert_eq!(
            random,
            pair_files(&videos, &audios, Pairing::Random, &mut rng())
        );
        let many: Vec<PathBuf> = (0..30)
            .map(|i| PathBuf::from(format!("v/{i}.mp4")))
            .collect();
        let picks = |seed| {
            pair_files(&many, &audios, Pairing::Random, &mut SeededRng::new(seed))
                .into_iter()
                .map(|(_, a)| a)
                .collect::<Vec<_>>()
        };
        assert_ne!(picks(1), picks(2));
        assert!(pair_files(&videos, &[], Pairing::Random, &mut rng()).is_empty());
    }

    #[test]
    fn graphs_replace_or_mix() {
        assert_eq!(
            audio_graph(false, 100.0, 80.0),
            "[1:a:0]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,volume=0.8,apad[aout]"
        );
        let mix = audio_graph(true, 50.0, 150.0);
        assert!(mix.starts_with("[0:a:0]aresample=48000"), "{mix}");
        assert!(mix.contains("volume=0.5[a0]"), "{mix}");
        assert!(mix.contains("volume=1.5[a1]"), "{mix}");
        assert!(mix.ends_with(
            "[a0][a1]amix=inputs=2:duration=longest:dropout_transition=0:normalize=0,apad[aout]"
        ));
    }

    #[test]
    fn mux_args_copy_video_and_follow_its_duration() {
        let args = mux_args("v.mp4", "a.mp3", "G", true, 12.5, "mp4", "o.mp4").join(" ");
        assert_eq!(
            args,
            "-i v.mp4 -stream_loop -1 -i a.mp3 -filter_complex G -map 0:V:0 -map [aout] -c:v copy \
             -c:a aac -b:a 192k -t 12.5 -sn -dn -movflags +faststart o.mp4"
        );
        let once = mux_args("v.mkv", "a.mp3", "G", false, 3.0, "mkv", "o.mkv").join(" ");
        assert!(once.starts_with("-i v.mkv -i a.mp3"), "{once}");
        assert!(!once.contains("movflags"));
    }

    #[test]
    fn speed_fit_graph_stretches_video_and_original_audio() {
        let fit = speed_fit(10.0, 20.0).unwrap().unwrap();
        assert_eq!(
            fit_graph(&fit, false, 100.0, 50.0),
            "[0:v:0]setpts=2*PTS[vout];[1:a:0]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,volume=0.5[aout]"
        );
        let mixed = fit_graph(&fit, true, 100.0, 100.0);
        assert!(mixed.contains("[0:a:0]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,atempo=0.5,volume=1[a0]"), "{mixed}");
        let args = fit_args(
            "v.mp4",
            "a.mp3",
            "G",
            vec!["-c:v".into(), "libx264".into()],
            20.0,
            "mp4",
            "o.mp4",
        )
        .join(" ");
        assert_eq!(
            args,
            "-i v.mp4 -i a.mp3 -filter_complex G -map [vout] -map [aout] -c:v libx264 -c:a aac -b:a 192k -t 20 -sn -dn -movflags +faststart o.mp4"
        );
    }

    #[test]
    fn volumes_are_validated() {
        assert!(AvMuxParams::default().validate().is_ok());
        let loud = AvMuxParams {
            audio_volume: 500.0,
            ..AvMuxParams::default()
        };
        assert!(loud.validate().is_err());
    }
}
