//! 集成测试（M4）：流水线级步骤——烧录字幕、嵌入封面、HDR 转换 / 保留、体积压缩、版权信息。
//! 全部用 lavfi 现场生成素材，真实运行 ffmpeg 后用 ffprobe / 像素断言。

mod common;

use std::collections::HashMap;
use std::path::Path;

use cliprecipe_lib::ffmpeg::{self, CancelToken, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{
    self, JobEvent, RunConfig, RunExtras, TaskAttachments, TaskSpec, TaskState,
};
use cliprecipe_lib::model::{Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::TrimParams;
use cliprecipe_lib::pipeline::PreviewOutput;
use cliprecipe_lib::preview::{render_preview, PreviewDir, PreviewSpec};
use common::*;

fn run_ffmpeg(paths: &FfmpegPaths, args: &[&str], out: &Path) {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 纯黑的测试视频（便于检测字幕 / 封面像素），带音频。
fn make_black(paths: &FfmpegPaths, out: &Path, seconds: f64) {
    run_ffmpeg(
        paths,
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=black:size=640x360:rate=25:duration={seconds}"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration={seconds}"),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ],
        out,
    );
}

/// 纯色图片。
fn make_image(paths: &FfmpegPaths, out: &Path, color: &str, size: &str) {
    run_ffmpeg(
        paths,
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:size={size}"),
            "-frames:v",
            "1",
        ],
        out,
    );
}

fn config(paths: &FfmpegPaths, recipe: Recipe, settings: Settings, out: &Path) -> RunConfig {
    RunConfig {
        paths: paths.clone(),
        encoders: EncoderCaps::software_only(),
        recipe,
        settings,
        output_dir: Some(out.to_path_buf()),
        seed: 5,
    }
}

fn task(path: &Path) -> TaskSpec {
    TaskSpec {
        id: "t1".into(),
        path: path.to_string_lossy().into_owned(),
        source_root: None,
        info: None,
        number: 0,
    }
}

/// 跑一个任务，返回输出路径与全部事件。
fn run_one(
    config: RunConfig,
    input: &Path,
    attachments: TaskAttachments,
) -> (std::path::PathBuf, Vec<JobEvent>) {
    let c = collector();
    let extras = RunExtras {
        attachments: HashMap::from([("t1".to_owned(), attachments)]),
        ..RunExtras::default()
    };
    let summary = jobs::start_with(config, vec![task(input)], extras, c.sink.clone()).wait();
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let output = match c.final_state("t1") {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            output: Some(output),
            ..
        }) => output,
        other => panic!("任务应完成：{other:?}"),
    };
    (output.into(), c.snapshot())
}

fn brightness(rgb: [i32; 3]) -> i32 {
    rgb.iter().sum::<i32>() / 3
}

/// 画面下方字幕区域（640x360 画面的下部 100 像素）的亮度（黑底白字）。
const SUBTITLE_AREA: (u32, u32, u32, u32) = (0, 260, 640, 100);

fn subtitle_area(paths: &FfmpegPaths, file: &Path, time: f64) -> i32 {
    brightness(avg_rgb(paths, file, time, SUBTITLE_AREA))
}

/// 大号字幕（以 1080 为基准 150，在 360 高的画面上约 50 像素），便于像素检测。
fn big_subtitles() -> Settings {
    let mut settings = Settings::default();
    settings.subtitle_style.size = 150;
    settings
}

/// 图片某区域的平均颜色。
fn image_rgb(paths: &FfmpegPaths, file: &Path, (x, y, w, h): (u32, u32, u32, u32)) -> [i32; 3] {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!(
            "crop={w}:{h}:{x}:{y},scale=1:1:flags=area,format=rgb24"
        ))
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert_eq!(
        output.stdout.len(),
        3,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    [0, 1, 2].map(|i| i32::from(output.stdout[i]))
}

fn write_srt(path: &Path, cues: &[(f64, f64, &str)]) {
    let clock = |t: f64| {
        let ms = (t * 1000.0).round() as u64;
        format!("00:00:{:02},{:03}", ms / 1000, ms % 1000)
    };
    let text: String = cues
        .iter()
        .enumerate()
        .map(|(i, (a, b, line))| {
            format!("{}\n{} --> {}\n{}\n\n", i + 1, clock(*a), clock(*b), line)
        })
        .collect();
    std::fs::write(path, text).unwrap();
}

#[test]
fn subtitle_is_burned_with_chinese_text_and_follows_trim() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("m4-subtitle");
    let input = dir.join("src/clip.mp4");
    make_black(&paths, &input, 3.0);
    let srt = dir.join("src/clip.srt");
    // 只在源视频的 1～2 秒显示
    write_srt(&srt, &[(1.0, 2.0, "中文字幕 测试 ABC")]);
    let attach = TaskAttachments {
        subtitle: Some(srt.to_string_lossy().into_owned()),
        cover: None,
    };

    let (out, _) = run_one(
        config(
            &paths,
            Recipe::disabled(),
            big_subtitles(),
            &dir.join("out"),
        ),
        &input,
        attach.clone(),
    );
    assert!(subtitle_area(&paths, &out, 0.5) < 5, "字幕出现之前画面为黑");
    assert!(
        subtitle_area(&paths, &out, 1.5) > 12,
        "1.5 秒处应有烧录的字幕"
    );
    assert_close(
        probe(&paths, &out).duration,
        3.0,
        0.15,
        "烧录字幕不改变时长",
    );

    // 去头 1 秒：输出 0.5 秒对应源视频 1.5 秒，字幕时间跟随源视频
    let mut recipe = Recipe::disabled();
    recipe.modules.trim = TrimParams {
        enabled: true,
        head: Range::fixed(1.0),
        tail: Range::fixed(0.0),
        ..TrimParams::default()
    };
    let (trimmed, _) = run_one(
        config(&paths, recipe, big_subtitles(), &dir.join("out-trim")),
        &input,
        attach,
    );
    assert!(
        subtitle_area(&paths, &trimmed, 0.5) > 12,
        "去头后字幕仍对齐源视频时间"
    );
    assert!(
        subtitle_area(&paths, &trimmed, 1.5) < 5,
        "源视频 2.5 秒处已无字幕"
    );
}

#[test]
fn subtitle_shows_in_preview() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("m4-subtitle-preview");
    let input = dir.join("src/clip.mp4");
    make_black(&paths, &input, 2.0);
    let srt = dir.join("src/clip.srt");
    write_srt(&srt, &[(0.0, 2.0, "预览字幕")]);
    let preview_dir = PreviewDir::under(dir.path(), 1);
    let spec = |subtitle: Option<String>| PreviewSpec {
        path: input.to_string_lossy().into_owned(),
        info: None,
        recipe: Recipe::disabled(),
        settings: big_subtitles(),
        only_module: None,
        output: PreviewOutput::Frame { time: 1.0 },
        seed: Some(1),
        number: 1,
        max_height: 360,
        subtitle,
    };
    let with = render_preview(
        &paths,
        &spec(Some(srt.to_string_lossy().into_owned())),
        &preview_dir,
        &CancelToken::new(),
        |_| {},
    )
    .unwrap();
    let without = render_preview(
        &paths,
        &spec(None),
        &preview_dir,
        &CancelToken::new(),
        |_| {},
    )
    .unwrap();
    // 单帧 jpg（预览缩到 360 高，尺寸不变）：图片不能用 -ss，所以不用 avg_rgb
    let lum = |p: &str| brightness(image_rgb(&paths, Path::new(p), SUBTITLE_AREA));
    assert!(lum(&with.path) > 5, "预览帧上应有字幕");
    assert!(lum(&without.path) < 2);
}

#[test]
fn cover_overlays_first_frames_and_is_attached() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("m4-cover");
    let input = dir.join("src/clip.mp4");
    make_black(&paths, &input, 2.0);
    let cover = dir.join("src/clip.png");
    make_image(&paths, &cover, "red", "320x180");
    let settings = Settings {
        cover_frames: 5,
        ..Settings::default()
    };
    let (out, _) = run_one(
        config(&paths, Recipe::disabled(), settings, &dir.join("out")),
        &input,
        TaskAttachments {
            subtitle: None,
            cover: Some(cover.to_string_lossy().into_owned()),
        },
    );
    let first = avg_rgb(&paths, &out, 0.0, (0, 0, 640, 360));
    assert!(
        first[0] > 200 && first[1] < 60,
        "第一帧是封面（红色）：{first:?}"
    );
    let later = avg_rgb(&paths, &out, 1.0, (0, 0, 640, 360));
    assert!(brightness(later) < 10, "5 帧之后恢复原画面：{later:?}");
    let s = streams(&paths, &out);
    assert_close(s.format, 2.0, 0.15, "帧数不超过帧率时时长不变");
    let info = probe(&paths, &out);
    assert!(info.has_cover, "mp4 应写入封面图（attached_pic）");
    assert_eq!(info.video.as_ref().unwrap().width, 640);
}

#[test]
fn cover_longer_than_a_second_is_prepended_keeping_aspect() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("m4-cover-prepend");
    let input = dir.join("src/clip.mp4");
    make_black(&paths, &input, 2.0);
    let cover = dir.join("src/clip.jpg");
    // 4:1 的图片，等比缩放后上下留黑边
    make_image(&paths, &cover, "red", "640x160");
    let settings = Settings {
        cover_frames: 50,
        cover_stretch: false,
        ..Settings::default()
    };
    let (out, _) = run_one(
        config(&paths, Recipe::disabled(), settings, &dir.join("out")),
        &input,
        TaskAttachments {
            subtitle: None,
            cover: Some(cover.to_string_lossy().into_owned()),
        },
    );
    let s = streams(&paths, &out);
    assert_close(s.video, 4.0, 0.15, "50 帧（2 秒）封面接在开头");
    assert_close(s.audio.unwrap(), 4.0, 0.2, "音频相应延后");
    let center = avg_rgb(&paths, &out, 1.0, (0, 150, 640, 60));
    assert!(center[0] > 200, "开头 2 秒中间为封面：{center:?}");
    let top = avg_rgb(&paths, &out, 1.0, (0, 0, 640, 60));
    assert!(brightness(top) < 10, "等比缩放，上方为黑边：{top:?}");
    assert!(
        brightness(avg_rgb(&paths, &out, 3.0, (0, 0, 640, 360))) < 10,
        "之后为原画面"
    );
}

/// 10 bit H.265、带 PQ / BT.2020 标记的“HDR”测试视频。
fn make_hdr(paths: &FfmpegPaths, out: &Path) {
    run_ffmpeg(
        paths,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=25:duration=1",
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p10le",
            "-vf",
            "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc",
            "-x265-params",
            "log-level=error",
        ],
        out,
    );
}

fn color_tags(paths: &FfmpegPaths, file: &Path) -> (String, String, String) {
    let info = probe(paths, file);
    let v = info.video.unwrap();
    (v.codec, v.pix_fmt, v.color_transfer)
}

#[test]
fn hdr_is_tone_mapped_or_kept_as_10bit_hevc() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("m4-hdr");
    let input = dir.join("src/hdr.mp4");
    make_hdr(&paths, &input);
    assert!(
        probe(&paths, &input).video.unwrap().is_hdr(),
        "测试素材应被识别为 HDR"
    );

    let (sdr, events) = run_one(
        config(
            &paths,
            Recipe::disabled(),
            Settings::default(),
            &dir.join("out-sdr"),
        ),
        &input,
        TaskAttachments::default(),
    );
    let (codec, pix_fmt, trc) = color_tags(&paths, &sdr);
    assert_eq!(
        (codec.as_str(), pix_fmt.as_str(), trc.as_str()),
        ("h264", "yuv420p", "bt709")
    );
    assert!(events
        .iter()
        .any(|e| matches!(e, JobEvent::Log { message, .. } if message.contains("转换为 SDR"))));

    let keep = Settings {
        hdr_to_sdr: false,
        ..Settings::default()
    };
    let (hdr, events) = run_one(
        config(&paths, Recipe::disabled(), keep, &dir.join("out-hdr")),
        &input,
        TaskAttachments::default(),
    );
    let (codec, pix_fmt, trc) = color_tags(&paths, &hdr);
    assert_eq!(
        (codec.as_str(), pix_fmt.as_str(), trc.as_str()),
        ("hevc", "yuv420p10le", "smpte2084")
    );
    assert!(probe(&paths, &hdr).video.unwrap().is_hdr());
    assert!(
        events.iter().any(
            |e| matches!(e, JobEvent::Log { message, .. } if message.contains("H.264 无法保留 HDR"))
        ),
        "选了 H.264 时应提示已改用 H.265"
    );
}

fn format_tag(paths: &FfmpegPaths, file: &Path, tag: &str) -> String {
    let output = ffmpeg::command(&paths.ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            &format!("format_tags={tag}"),
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn reduce_size_and_copyright() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("m4-slim");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 640,
            height: 360,
            seconds: 2.0,
            ..Clip::default()
        },
    );
    let normal = Settings {
        copyright: "© 2026 ClipRecipe 测试".into(),
        ..Settings::default()
    };
    let (plain, _) = run_one(
        config(
            &paths,
            Recipe::disabled(),
            normal.clone(),
            &dir.join("out-plain"),
        ),
        &input,
        TaskAttachments::default(),
    );
    assert_eq!(
        format_tag(&paths, &plain, "copyright"),
        "© 2026 ClipRecipe 测试"
    );

    let slim = Settings {
        reduce_size: true,
        ..normal
    };
    let (small, events) = run_one(
        config(&paths, Recipe::disabled(), slim, &dir.join("out-slim")),
        &input,
        TaskAttachments::default(),
    );
    let command = events
        .iter()
        .find_map(|e| match e {
            JobEvent::Log { message, .. } if message.contains("-crf") => Some(message.clone()),
            _ => None,
        })
        .expect("日志中应有命令行");
    assert!(
        command.contains("-preset slow") && command.contains("-crf 28"),
        "{command}"
    );
    let size = |p: &Path| std::fs::metadata(p).unwrap().len();
    assert!(
        size(&small) < size(&plain),
        "瘦身后体积更小：{} vs {}",
        size(&small),
        size(&plain)
    );
    assert_close(probe(&paths, &small).duration, 2.0, 0.15, "时长不变");
}
