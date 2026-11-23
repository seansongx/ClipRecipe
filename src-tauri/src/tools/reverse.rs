//! `tool.reverse` 视频倒放：画面倒序播放；音频同时倒放（`areverse`），或按参数去掉音频。一进一出，重编码。
//!
//! FFmpeg 的 `reverse` / `areverse` 要把整段画面缓存在内存里才能倒序输出（1080p 每帧约 3 MB），
//! 长视频一次倒放会耗尽内存，所以**分段处理**：
//!
//! 1. 按 `chunkSeconds`（默认 3 秒，0.5～120 秒）把视频分段，段长取整到帧（第 `a`～`b-1` 帧为一段，最后一段到结尾）；
//!    每段先在输入端跳转到第 `a` 帧之前半帧（快速且精确），再用 `trim=end_frame` 取正好这些帧、
//!    `atrim` 取正好 `[a/帧率, b/帧率)` 的音频——每一帧、每个采样恰好属于一段，各段画面与音频等长；
//! 2. 每段时间戳归零后单独倒放、编码成临时文件（视频按设置编码，音频存为 PCM，拼接时不会产生缝隙）；
//! 3. 用 concat 分离器按**逆序**（最后一段在前）拼接，视频直接复制，音频转 AAC。
//!
//! 视频不长于一段时直接一步倒放到输出。分段结果与一次倒放逐帧一致（集成测试比对）。
//! 帧率读不到时按 25 分段（只影响段长，不影响正确性以外的东西：可变帧率视频的段界可能落在帧中间）。
//! 进度：每段一步（权重为段长），拼接一步。
//!
//! 参数：`removeAudio`（默认 false：音频倒放；true：输出不含音频）、`chunkSeconds`（默认 3）。
//! 输出与源同名（按设置的目标格式）。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::tools::context::Encoding;
use crate::tools::media::write_concat_list;
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.reverse";

/// 分段时长的范围（秒）。
pub const CHUNK_RANGE: (f64, f64) = (0.5, 120.0);
/// 段数上限（避免误填导致生成成千上万个临时文件）。
pub const MAX_CHUNKS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReverseParams {
    /// 去掉音频（否则音频一起倒放）
    pub remove_audio: bool,
    /// 每段时长（秒）：越短内存占用越少
    pub chunk_seconds: f64,
}

impl Default for ReverseParams {
    fn default() -> Self {
        Self {
            remove_audio: false,
            chunk_seconds: 3.0,
        }
    }
}

impl ReverseParams {
    fn validate(&self) -> Result<(), String> {
        let (min, max) = CHUNK_RANGE;
        if !(self.chunk_seconds.is_finite() && (min..=max).contains(&self.chunk_seconds)) {
            return Err(format!("分段时长必须在 {min}～{max} 秒之间"));
        }
        Ok(())
    }
}

/// 一段：从第 `first` 帧起的 `frames` 帧（None 表示到结尾）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chunk {
    pub first: u64,
    pub frames: Option<u64>,
    /// 帧率（段界换算成时间用）
    pub fps: f64,
    /// 预计时长（进度用）
    pub duration: f64,
}

/// 完整精度的小数（段界要精确到采样，不能只保留 4 位小数）。
fn exact(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// 把 `duration` 秒、`fps` 帧率的视频分段，段长取整到帧（至少 1 帧）。
pub fn plan_chunks(duration: f64, fps: f64, chunk_seconds: f64) -> Result<Vec<Chunk>, String> {
    if !(duration.is_finite() && duration > 0.0) {
        return Err("无法读取视频时长".into());
    }
    let fps = if fps.is_finite() && fps > 0.0 && fps <= 1000.0 {
        fps
    } else {
        25.0
    };
    let total = (duration * fps).round().max(1.0) as u64;
    let per = ((chunk_seconds * fps).round() as u64).max(1);
    let count = total.div_ceil(per);
    if count as usize > MAX_CHUNKS {
        return Err(format!("分段太多（超过 {MAX_CHUNKS} 段），请加大分段时长"));
    }
    Ok((0..count)
        .map(|k| {
            let first = k * per;
            let last = k + 1 == count;
            Chunk {
                first,
                frames: (!last).then_some(per),
                fps,
                duration: if last {
                    (duration - first as f64 / fps).max(0.0)
                } else {
                    per as f64 / fps
                },
            }
        })
        .collect())
}

/// 一段的输入参数与滤镜：输入端跳到第一帧之前半帧，`trim` / `atrim` 精确取这一段，时间戳归零。
/// 返回（输入前的参数，视频滤镜，音频滤镜）。
pub fn chunk_filters(chunk: &Chunk) -> (Vec<String>, String, String) {
    let at = |frames: f64| frames / chunk.fps;
    let mut before = Vec::new();
    let lead = if chunk.first > 0 {
        before.extend(["-ss".to_owned(), exact(at(chunk.first as f64 - 0.5))]);
        at(0.5)
    } else {
        0.0
    };
    let mut video = Vec::new();
    let mut audio = Vec::new();
    if lead > 0.0 {
        audio.push(format!("start={}", exact(lead)));
    }
    if let Some(n) = chunk.frames {
        // 多读一帧的余量，trim 精确截断
        before.extend(["-t".to_owned(), exact(at((n + 1) as f64))]);
        video.push(format!("trim=end_frame={n},"));
        audio.push(format!("duration={}", exact(at(n as f64))));
    }
    let video = format!("{}setpts=PTS-STARTPTS,reverse", video.concat());
    let audio = if audio.is_empty() {
        "asetpts=PTS-STARTPTS,areverse".to_owned()
    } else {
        format!("atrim={},asetpts=PTS-STARTPTS,areverse", audio.join(":"))
    };
    (before, video, audio)
}

/// 倒放一段（`chunk` 为 None 时整段）的参数。`audio`：要倒放的音频声道数（None 表示不要音频）；
/// `pcm` 为 true 时音频存为 PCM（分段的临时文件），否则转 AAC。
pub fn reverse_args(
    input: &str,
    chunk: Option<&Chunk>,
    audio: Option<u32>,
    pcm: bool,
    encoding: &Encoding,
    container: &str,
    output: &str,
) -> Vec<String> {
    let (before, vf, af) = match chunk {
        Some(chunk) => chunk_filters(chunk),
        None => (Vec::new(), "reverse".to_owned(), "areverse".to_owned()),
    };
    let mut args = before;
    args.extend([
        "-i".into(),
        input.into(),
        "-map".into(),
        "0:V:0".into(),
        "-vf".into(),
        vf,
    ]);
    match audio {
        Some(channels) => {
            args.extend(["-map".into(), "0:a:0".into(), "-af".into(), af]);
            args.extend(encoding.video_args(container));
            if pcm {
                args.extend(["-c:a".into(), "pcm_s16le".into()]);
            } else {
                args.extend(Encoding::audio_args(channels));
            }
        }
        None => {
            args.push("-an".into());
            args.extend(encoding.video_args(container));
        }
    }
    args.extend(["-sn".into(), "-dn".into()]);
    if matches!(container, "mp4" | "mov" | "m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

/// 逆序拼接分段结果的参数：视频直接复制，音频转 AAC。
pub fn concat_args(
    list: &str,
    audio: Option<u32>,
    hevc: bool,
    container: &str,
    output: &str,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "-f", "concat", "-safe", "0", "-i", list, "-map", "0:v:0", "-c:v", "copy",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let mp4 = matches!(container, "mp4" | "mov" | "m4v");
    if hevc && mp4 {
        args.extend(["-tag:v".into(), "hvc1".into()]);
    }
    match audio {
        Some(channels) => {
            args.extend(["-map".into(), "0:a:0".into()]);
            args.extend(Encoding::audio_args(channels));
        }
        None => args.push("-an".into()),
    }
    if mp4 {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

fn audio_channels(params: &ReverseParams, info: &MediaInfo) -> Option<u32> {
    if params.remove_audio {
        return None;
    }
    info.audio.first().map(|a| a.channels.max(1))
}

impl Tool for ReverseParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let info = ctx.info(0)?;
        let fps = info.video.as_ref().ok_or("文件中没有视频流")?.fps;
        let input = path_arg(ctx.input_path(0)?)?;
        let chunks = plan_chunks(info.duration, fps, self.chunk_seconds)?;
        let audio = audio_channels(self, &info);
        let container = ctx.video_extension(0)?;

        if chunks.len() == 1 {
            let output = path_arg(&ctx.output(&OutputSpec::file(&container))?)?;
            return ctx.run_encode("倒放", info.duration, |enc| {
                Ok(reverse_args(
                    &input, None, audio, false, enc, &container, &output,
                ))
            });
        }

        ctx.note(format!("分 {} 段倒放后拼接", chunks.len()));
        let mut weights: Vec<f64> = chunks.iter().map(|c| c.duration).collect();
        weights.push(info.duration * 0.05);
        ctx.set_steps(&weights);
        let mut pieces = Vec::with_capacity(chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            ctx.check_cancelled()?;
            let piece = ctx.temp_file(&format!("chunk_{:05}.mkv", i + 1))?;
            let piece_arg = path_arg(&piece)?;
            ctx.run_encode("分段倒放", chunk.duration, |enc| {
                Ok(reverse_args(
                    &input,
                    Some(chunk),
                    audio,
                    true,
                    enc,
                    "mkv",
                    &piece_arg,
                ))
            })?;
            pieces.push(piece);
        }
        pieces.reverse();
        let list = ctx.temp_file("reverse.txt")?;
        write_concat_list(&list, &pieces)?;
        let output = path_arg(&ctx.output(&OutputSpec::file(&container))?)?;
        let hevc = ctx.settings().hevc;
        ctx.run_ffmpeg(
            "拼接倒放片段",
            concat_args(&path_arg(&list)?, audio, hevc, &container, &output),
            info.duration,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{VideoCodec, VideoEncoder};
    use crate::pipeline::testing::sample_info;

    fn enc() -> Encoding {
        Encoding {
            encoder: VideoEncoder::software(VideoCodec::H264),
            video_kbps: Some(1000),
        }
    }

    #[test]
    fn chunks_are_whole_frames() {
        // 10 秒、25fps、每段 3 秒 → 75 帧一段：0..75、75..150、150..225、225..250
        let chunks = plan_chunks(10.0, 25.0, 3.0).unwrap();
        let firsts: Vec<(u64, Option<u64>)> = chunks.iter().map(|c| (c.first, c.frames)).collect();
        assert_eq!(
            firsts,
            [(0, Some(75)), (75, Some(75)), (150, Some(75)), (225, None)]
        );
        assert_eq!(chunks[0].duration, 3.0);
        assert!((chunks[3].duration - 1.0).abs() < 1e-9);
    }

    #[test]
    fn short_video_is_one_chunk_and_bad_input_fails() {
        assert_eq!(plan_chunks(2.0, 30.0, 3.0).unwrap().len(), 1);
        assert_eq!(plan_chunks(3.0, 25.0, 3.0).unwrap().len(), 1);
        let unknown = plan_chunks(1.0, 0.0, 0.5).unwrap();
        assert_eq!((unknown.len(), unknown[0].fps), (2, 25.0), "帧率未知按 25");
        assert!(plan_chunks(0.0, 25.0, 3.0).is_err());
        assert!(plan_chunks(1e9, 25.0, 0.5).is_err());
        for bad in [0.1, 500.0, f64::NAN] {
            let p = ReverseParams {
                chunk_seconds: bad,
                ..ReverseParams::default()
            };
            assert!(p.validate().is_err(), "{bad}");
        }
        assert!(ReverseParams::default().validate().is_ok());
    }

    #[test]
    fn chunk_filters_cut_on_frame_boundaries() {
        let chunks = plan_chunks(4.0, 25.0, 1.0).unwrap();
        let (before, vf, af) = chunk_filters(&chunks[0]);
        assert_eq!(before, ["-t", "1.04"]);
        assert_eq!(vf, "trim=end_frame=25,setpts=PTS-STARTPTS,reverse");
        assert_eq!(af, "atrim=duration=1,asetpts=PTS-STARTPTS,areverse");
        let (before, vf, af) = chunk_filters(&chunks[1]);
        assert_eq!(before, ["-ss", "0.98", "-t", "1.04"]);
        assert_eq!(vf, "trim=end_frame=25,setpts=PTS-STARTPTS,reverse");
        assert_eq!(
            af,
            "atrim=start=0.02:duration=1,asetpts=PTS-STARTPTS,areverse"
        );
        let (before, vf, af) = chunk_filters(&chunks[3]);
        assert_eq!(before, ["-ss", "2.98"]);
        assert_eq!(vf, "setpts=PTS-STARTPTS,reverse");
        assert_eq!(af, "atrim=start=0.02,asetpts=PTS-STARTPTS,areverse");
    }

    #[test]
    fn whole_reverse_args() {
        let args = reverse_args("in.mp4", None, Some(2), false, &enc(), "mp4", "out.mp4");
        assert_eq!(
            args.join(" "),
            "-i in.mp4 -map 0:V:0 -vf reverse -map 0:a:0 -af areverse -c:v libx264 -preset medium \
             -pix_fmt yuv420p -b:v 1000k -maxrate 1500k -bufsize 2000k -c:a aac -b:a 192k -sn -dn \
             -movflags +faststart out.mp4"
        );
        let mute = reverse_args("in.mp4", None, None, false, &enc(), "mkv", "out.mkv").join(" ");
        assert_eq!(
            mute,
            "-i in.mp4 -map 0:V:0 -vf reverse -an -c:v libx264 -preset medium -pix_fmt yuv420p \
             -b:v 1000k -maxrate 1500k -bufsize 2000k -sn -dn out.mkv"
        );
    }

    #[test]
    fn chunk_args_keep_pcm() {
        let chunks = plan_chunks(4.0, 25.0, 1.0).unwrap();
        let args = reverse_args(
            "in.mp4",
            Some(&chunks[1]),
            Some(1),
            true,
            &enc(),
            "mkv",
            "c.mkv",
        );
        assert_eq!(
            args.join(" "),
            "-ss 0.98 -t 1.04 -i in.mp4 -map 0:V:0 -vf trim=end_frame=25,setpts=PTS-STARTPTS,reverse \
             -map 0:a:0 -af atrim=start=0.02:duration=1,asetpts=PTS-STARTPTS,areverse -c:v libx264 \
             -preset medium -pix_fmt yuv420p -b:v 1000k -maxrate 1500k -bufsize 2000k \
             -c:a pcm_s16le -sn -dn c.mkv"
        );
    }

    #[test]
    fn concat_copies_video_and_encodes_audio() {
        assert_eq!(
            concat_args("l.txt", Some(2), true, "mp4", "o.mp4").join(" "),
            "-f concat -safe 0 -i l.txt -map 0:v:0 -c:v copy -tag:v hvc1 -map 0:a:0 -c:a aac \
             -b:a 192k -movflags +faststart o.mp4"
        );
        assert_eq!(
            concat_args("l.txt", None, false, "mkv", "o.mkv").join(" "),
            "-f concat -safe 0 -i l.txt -map 0:v:0 -c:v copy -an o.mkv"
        );
    }

    #[test]
    fn audio_follows_params_and_source() {
        let info = sample_info();
        assert_eq!(audio_channels(&ReverseParams::default(), &info), Some(2));
        let removed = ReverseParams {
            remove_audio: true,
            ..ReverseParams::default()
        };
        assert_eq!(audio_channels(&removed, &info), None);
        let mut silent = sample_info();
        silent.audio.clear();
        assert_eq!(audio_channels(&ReverseParams::default(), &silent), None);
    }
}
