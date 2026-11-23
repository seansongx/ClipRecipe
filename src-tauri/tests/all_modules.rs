//! 集成测试：同时启用全部已实现的流水线模块（参数取常见用法），检查模块之间的配合：
//! 任务成功、输出有效、音视频时长一致、进度所用的预计时长与实际输出时长接近。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{self, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, LogLevel, RunConfig, TaskSpec, TaskState};
use cliprecipe_lib::model::{AssetSource, Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::add_watermark::{Anchor, ScrollMode};
use cliprecipe_lib::pipeline::modules::adjust::{BorderParams, Tweak};
use cliprecipe_lib::pipeline::modules::bitrate::BitrateMode;
use cliprecipe_lib::pipeline::modules::crop::{CropMode, Sides};
use cliprecipe_lib::pipeline::modules::remove_watermark::{RegionMode, WatermarkPreset};
use cliprecipe_lib::pipeline::modules::resolution::ResolutionMode;
use cliprecipe_lib::pipeline::modules::rotate_flip::AngleFit;
use cliprecipe_lib::pipeline::modules::speed::SpeedMode;
use cliprecipe_lib::pipeline::modules::text::{TextItem, TextPosition, TextSource};
use cliprecipe_lib::pipeline::modules::trim::TrimMode;
use cliprecipe_lib::pipeline::ordered_modules;
use common::*;

/// 素材：片头（有声）、片尾（无声）、相框图片、背景音乐。
struct Assets {
    intro: PathBuf,
    outro: PathBuf,
    frame: PathBuf,
    music: PathBuf,
}

fn generate(paths: &FfmpegPaths, out: &Path, args: &[&str]) {
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(out)
        .output()
        .expect("无法启动 ffmpeg");
    assert!(
        output.status.success(),
        "生成素材失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn make_assets(paths: &FfmpegPaths, dir: &TempDir) -> Assets {
    let assets = Assets {
        intro: dir.join("assets/intro.mp4"),
        outro: dir.join("assets/outro.mp4"),
        frame: dir.join("assets/frame.png"),
        music: dir.join("assets/music.m4a"),
    };
    make_clip(
        paths,
        &assets.intro,
        &Clip {
            width: 320,
            height: 180,
            fps: 30,
            seconds: 1.0,
            audio: true,
        },
    );
    make_clip(
        paths,
        &assets.outro,
        &Clip {
            width: 200,
            height: 200,
            fps: 24,
            seconds: 1.0,
            audio: false,
        },
    );
    generate(
        paths,
        &assets.frame,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=320x240:d=1,format=rgba,\
             geq=r=255:g=0:b=0:a='if(between(X,16,303)*between(Y,16,223),0,255)'",
            "-frames:v",
            "1",
        ],
    );
    generate(
        paths,
        &assets.music,
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:duration=3:sample_rate=44100",
            "-c:a",
            "aac",
        ],
    );
    assets
}

fn file(path: &Path) -> AssetSource {
    AssetSource::File {
        path: path.to_string_lossy().into_owned(),
    }
}

/// 全部已实现模块都开启的配方。
fn everything(assets: &Assets) -> Recipe {
    let mut r = Recipe::default();
    let m = &mut r.modules;

    m.trim.enabled = true;
    m.trim.mode = TrimMode::HeadTail;
    m.trim.head = Range::new(0.3, 0.6);
    m.trim.tail = Range::fixed(0.5);

    m.dedupe_frames.enabled = true;

    m.drop_frames.enabled = true;
    m.drop_frames.interval = Range::new(8, 12);
    m.drop_frames.sync_audio = true;

    m.speed.enabled = true;
    m.speed.mode = SpeedMode::Fixed;
    m.speed.rate = Range::new(1.1, 1.3);

    m.crop.enabled = true;
    m.crop.mode = CropMode::Margins;
    m.crop.margins = Sides::all(7);

    m.rotate_flip.enabled = true;
    m.rotate_flip.hflip = true;
    m.rotate_flip.random_angle = true;
    m.rotate_flip.angle = Range::new(-4.0, 4.0);
    m.rotate_flip.fit = AngleFit::RemoveBlack;

    m.zoom.enabled = true;
    m.zoom.scale = Range::new(1.0, 1.15);
    m.zoom.period = Range::fixed(2.0);

    m.adjust.enabled = true;
    let on = |value: Range<f64>| Tweak {
        enabled: true,
        value,
    };
    m.adjust.brightness = on(Range::new(0.0, 0.05));
    m.adjust.contrast = on(Range::new(1.0, 1.1));
    m.adjust.saturation = on(Range::new(1.0, 1.2));
    m.adjust.sharpen = on(Range::fixed(0.6));
    m.adjust.denoise = on(Range::fixed(2.0));
    m.adjust.fade_in = on(Range::fixed(0.3));
    m.adjust.fade_out = on(Range::fixed(0.3));
    m.adjust.vignette = on(Range::fixed(30.0));
    m.adjust.mask_invert = on(Range::fixed(0.2));
    m.adjust.opening = on(Range::fixed(0.5));
    m.adjust.border = BorderParams {
        enabled: true,
        width: Range::fixed(5),
        color: "#ffffff".into(),
        random_color: false,
    };

    m.grid.enabled = true;
    m.grid.count = 2;
    m.grid.blur_ends = true;

    m.resolution.enabled = true;
    m.resolution.mode = ResolutionMode::Standard;
    m.resolution.standard_height = 360;

    m.remove_watermark.enabled = true;
    m.remove_watermark.mode = RegionMode::Preset;
    m.remove_watermark.presets = vec![WatermarkPreset::TopRight, WatermarkPreset::BottomCaption];
    m.remove_watermark.strength = 6;
    m.remove_watermark.limit_time = true;
    m.remove_watermark.start = 0.2;
    m.remove_watermark.end = 1.5;

    // 画布尺寸不变（check_output 断言高 360）；画布改变的情形见 all_modules_with_pip_canvas_change
    m.pip.enabled = true;
    m.pip.size = Range::new(75.0, 85.0);
    m.pip.offset_y = Range::new(-3.0, 3.0);
    m.pip.blur = Range::fixed(10.0);

    // 带透明通道的 PNG 主水印（反弹滚动）+ 视频子水印
    let w = &mut m.add_watermark;
    w.enabled = true;
    w.source = file(&assets.frame);
    w.anchor = Anchor::BottomRight;
    w.size = Range::new(15.0, 20.0);
    w.opacity = Range::fixed(70.0);
    w.scroll = ScrollMode::Bounce;
    w.scroll_speed = Range::fixed(30.0);
    w.sub.enabled = true;
    w.sub.source = file(&assets.outro);
    w.sub.anchor = Anchor::TopLeft;
    w.sub.size = Range::fixed(12.0);

    m.text.enabled = true;
    m.text.items[0] = TextItem {
        enabled: true,
        source: TextSource::FileName,
        line_breaks: "_".into(),
        ..Default::default()
    };
    m.text.items[1] = TextItem {
        enabled: true,
        source: TextSource::Custom,
        custom_text: "第一行: 'a,b' 100%\n[第二行]".into(),
        position: TextPosition::Top,
        shadow: true,
        background: true,
        start: 0.3,
        duration: 1.0,
        ..Default::default()
    };

    m.intro_outro.enabled = true;
    m.intro_outro.intro_enabled = true;
    m.intro_outro.intro = file(&assets.intro);
    m.intro_outro.outro_enabled = true;
    m.intro_outro.outro = file(&assets.outro);
    m.intro_outro.frame_enabled = true;
    m.intro_outro.frame = file(&assets.frame);
    m.intro_outro.frame_margin = Range::fixed(4.0);

    m.bgm.enabled = true;
    m.bgm.source = file(&assets.music);
    m.bgm.music_volume = Range::fixed(40.0);
    m.bgm.loop_music = true;
    m.bgm.fade_out = 0.5;

    m.fps.enabled = true;
    m.fps.fps = Range::new(24.0, 30.0);

    m.bitrate.enabled = true;
    m.bitrate.mode = BitrateMode::Fixed;
    m.bitrate.kbps = Range::fixed(800);

    // 所有已实现的模块都必须开启（新模块实现后在这里补上参数）
    let missing: Vec<&str> = ordered_modules(&r.modules)
        .iter()
        .filter(|m| m.implemented() && !m.enabled())
        .map(|m| m.id())
        .collect();
    assert!(missing.is_empty(), "以下已实现的模块没有开启：{missing:?}");
    r
}

/// 一个任务的全部输出，以及进度计算所用的预计时长（最后一次 100% 进度事件里的 out_time）。
struct Outcome {
    outputs: Vec<PathBuf>,
    expected: Vec<f64>,
    warnings: Vec<String>,
}

fn run(paths: &FfmpegPaths, recipe: Recipe, input: &Path, out: &Path) -> Outcome {
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(out.to_path_buf()),
            seed: 2024,
        },
        vec![TaskSpec {
            id: "t".into(),
            path: input.to_string_lossy().into_owned(),
            source_root: None,
            info: None,
            number: 0,
        }],
        Arc::clone(&c.sink),
    )
    .wait();
    let events = c.snapshot();
    assert_eq!(summary.done, 1, "{events:#?}");
    let outputs = match c.final_state("t") {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            outputs,
            ..
        }) => outputs.into_iter().map(PathBuf::from).collect(),
        other => panic!("任务应完成，实际：{other:?}"),
    };
    // 每轮结束时会发一条该轮 100% 的进度，out_time 为该轮的预计时长
    let mut expected = Vec::new();
    let mut last_round = 0;
    for event in &events {
        if let JobEvent::Progress {
            out_time, round, ..
        } = event
        {
            if *round != last_round {
                expected.push(0.0);
                last_round = *round;
            }
            if let Some(last) = expected.last_mut() {
                *last = *out_time;
            }
        }
    }
    let warnings = events
        .iter()
        .filter_map(|e| match e {
            JobEvent::Log { message, .. } if message.starts_with("[pipe.") => Some(message.clone()),
            _ => None,
        })
        .collect();
    let commands: Vec<&String> = events
        .iter()
        .filter_map(|e| match e {
            JobEvent::Log {
                level: LogLevel::Debug,
                message,
                ..
            } => Some(message),
            _ => None,
        })
        .collect();
    assert!(!commands.is_empty());
    // 每个模块的滤镜确实出现在命令里（防止某个模块被悄悄跳过）
    for command in commands {
        for needle in [
            "boxblur=lr=",                   // 去水印
            "gblur=sigma=10",                // 画中画背景虚化
            "colorchannelmixer=aa=0.7",      // 加水印
            "drawtext=",                     // 文本
            "text=input",                    // 文本：文件名
            "enable=between(t\\,0.3\\,1.3)", // 文本：延时出现
            "enable=between(t\\,0.2\\,1.5)", // 去水印：生效时段
            "zoompan",
        ] {
            assert!(command.contains(needle), "命令中缺少 {needle}：{command}");
        }
    }
    Outcome {
        outputs,
        expected,
        warnings,
    }
}

fn check_output(paths: &FfmpegPaths, out: &Path, expected: f64) {
    check_output_size(paths, out, expected, None);
}

/// `size` 为 None 时只检查高为 360（分辨率模块统一为 360 高）；否则检查宽高。
fn check_output_size(paths: &FfmpegPaths, out: &Path, expected: f64, size: Option<(u32, u32)>) {
    let info = probe(paths, out);
    let v = info.video.as_ref().expect("应有视频流");
    assert_eq!(v.codec, "h264");
    match size {
        Some(size) => assert_eq!((v.width, v.height), size),
        None => assert_eq!(v.height, 360, "分辨率模块统一为 360 高"),
    }
    assert_eq!(v.width % 2, 0);
    assert!((23.9..=30.1).contains(&v.fps), "帧率 {}", v.fps);
    assert_eq!(info.audio.len(), 1, "应有一条音轨");
    let s = streams(paths, out);
    let audio = s.audio.unwrap();
    assert!(
        (s.video - audio).abs() <= 0.12,
        "音视频时长应一致：视频 {}，音频 {audio}（{}）",
        s.video,
        out.display()
    );
    let tolerance = (expected * 0.03).max(0.15);
    assert_close(s.format, expected, tolerance, "实际时长与预计时长");
}

fn run_case(name: &str, clip: Clip) {
    let paths = ffmpeg_paths();
    let dir = TempDir::new(name);
    let assets = make_assets(&paths, &dir);
    let input = dir.join("src/input.mp4");
    make_clip(&paths, &input, &clip);
    let outcome = run(&paths, everything(&assets), &input, &dir.join("out"));
    assert_eq!(outcome.outputs.len(), 1);
    assert_eq!(outcome.expected.len(), 1);
    check_output(&paths, &outcome.outputs[0], outcome.expected[0]);
    // 重复帧移除后面有帧率 / 变速 / 抽帧 / 动态缩放：应给出提示
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.starts_with("[pipe.dedupe-frames]")),
        "{:?}",
        outcome.warnings
    );
}

#[test]
fn all_modules_on_landscape_clip_with_audio() {
    run_case(
        "all-landscape",
        Clip {
            width: 640,
            height: 360,
            fps: 30,
            seconds: 6.0,
            audio: true,
        },
    );
}

#[test]
fn all_modules_on_portrait_clip_without_audio() {
    run_case(
        "all-portrait",
        Clip {
            width: 360,
            height: 640,
            fps: 25,
            seconds: 6.0,
            audio: false,
        },
    );
}

#[test]
fn all_modules_on_odd_sized_clip() {
    run_case(
        "all-odd",
        Clip {
            width: 321,
            height: 241,
            fps: 25,
            seconds: 6.0,
            audio: true,
        },
    );
}

/// 变形（非方形像素）源：320x360、SAR 2:1（显示 640x360）。流水线先换算为方形像素，
/// 输出为方形像素、360 高、宽度按显示比例。
#[test]
fn all_modules_on_anamorphic_clip() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("all-anamorphic");
    let assets = make_assets(&paths, &dir);
    let input = dir.join("src/input.mp4");
    generate(
        &paths,
        &input,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x360:rate=30:duration=6,setsar=2/1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=6",
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
    );
    assert_eq!(
        probe(&paths, &input).video.unwrap().sar_ratio(),
        Some((2, 1))
    );
    // 同样的配方作用于显示尺寸相同（640x360）的方形像素源，输出尺寸应完全一致
    // 文件名同为 input（文本模块会写入文件名）
    let square = dir.join("square/input.mp4");
    make_clip(
        &paths,
        &square,
        &Clip {
            width: 640,
            height: 360,
            fps: 30,
            seconds: 6.0,
            audio: true,
        },
    );
    let reference = run(
        &paths,
        everything(&assets),
        &square,
        &dir.join("out-square"),
    );
    let expected_size = {
        let v = probe(&paths, &reference.outputs[0]).video.unwrap();
        (v.width, v.height)
    };
    let outcome = run(&paths, everything(&assets), &input, &dir.join("out"));
    check_output_size(
        &paths,
        &outcome.outputs[0],
        outcome.expected[0],
        Some(expected_size),
    );
    let v = probe(&paths, &outcome.outputs[0]).video.unwrap();
    assert!(
        v.sar_ratio().is_none(),
        "输出应为方形像素，实际 SAR {}",
        v.sar
    );
}

#[test]
fn all_modules_with_fission() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("all-fission");
    let assets = make_assets(&paths, &dir);
    let input = dir.join("src/input.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 480,
            height: 270,
            fps: 30,
            seconds: 5.0,
            audio: true,
        },
    );
    let mut recipe = everything(&assets);
    recipe.fission = 3;
    let outcome = run(&paths, recipe, &input, &dir.join("out"));
    assert_eq!(outcome.outputs.len(), 3);
    assert_eq!(outcome.expected.len(), 3);
    for (out, expected) in outcome.outputs.iter().zip(&outcome.expected) {
        assert!(
            out.starts_with(dir.join("out").join("input")),
            "{}",
            out.display()
        );
        check_output(&paths, out, *expected);
    }
    // 随机参数不同，三次的输出不应完全相同
    let sizes: Vec<u64> = outcome
        .outputs
        .iter()
        .map(|p| std::fs::metadata(p).unwrap().len())
        .collect();
    assert!(sizes[0] != sizes[1] || sizes[1] != sizes[2], "{sizes:?}");
}

#[test]
fn all_modules_except_bgm_keep_source_audio_in_sync() {
    // 背景音会用 apad + -shortest 把音频对齐到视频，掩盖其它模块造成的音画差异；这里去掉背景音再检查一次
    let paths = ffmpeg_paths();
    let dir = TempDir::new("all-no-bgm");
    let assets = make_assets(&paths, &dir);
    let input = dir.join("src/input.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 640,
            height: 360,
            fps: 30,
            seconds: 6.0,
            audio: true,
        },
    );
    let mut recipe = everything(&assets);
    recipe.modules.bgm.enabled = false;
    let outcome = run(&paths, recipe, &input, &dir.join("out"));
    check_output(&paths, &outcome.outputs[0], outcome.expected[0]);
}

#[test]
fn all_modules_with_pip_canvas_change() {
    // 画中画把横屏改成竖屏画布（相当于 1920x1080 → 1080x1920 的缩小版），排在分辨率之后：
    // 之后的水印、文本、片头片尾、相框都应按新的画布尺寸计算
    let paths = ffmpeg_paths();
    let dir = TempDir::new("all-pip-canvas");
    let assets = make_assets(&paths, &dir);
    let input = dir.join("src/input.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 640,
            height: 360,
            fps: 30,
            seconds: 6.0,
            audio: true,
        },
    );
    let mut recipe = everything(&assets);
    recipe.modules.pip.canvas_enabled = true;
    recipe.modules.pip.canvas_width = 360;
    recipe.modules.pip.canvas_height = 640;
    let outcome = run(&paths, recipe, &input, &dir.join("out"));
    assert_eq!(outcome.outputs.len(), 1);
    check_output_size(
        &paths,
        &outcome.outputs[0],
        outcome.expected[0],
        Some((360, 640)),
    );
}

#[test]
fn all_modules_whole_pipeline_preview() {
    // “生成预览”走同一套模块（排除变速、背景音、片头尾、帧率、码率），去水印额外画出区域框线
    use cliprecipe_lib::ffmpeg::CancelToken;
    use cliprecipe_lib::pipeline::PreviewOutput;
    use cliprecipe_lib::preview::{render_preview, PreviewDir, PreviewSpec};

    let paths = ffmpeg_paths();
    let dir = TempDir::new("all-preview");
    let assets = make_assets(&paths, &dir);
    let input = dir.join("src/input.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 640,
            height: 360,
            fps: 30,
            seconds: 6.0,
            audio: true,
        },
    );
    let result = render_preview(
        &paths,
        &PreviewSpec {
            path: input.to_string_lossy().into_owned(),
            info: None,
            recipe: everything(&assets),
            settings: Settings::default(),
            only_module: None,
            output: PreviewOutput::Clip {
                start: 0.5,
                duration: 2.0,
            },
            seed: Some(9),
            number: 1,
            max_height: 240,
            subtitle: None,
        },
        &PreviewDir::under(dir.path(), 1),
        &CancelToken::new(),
        |_| {},
    )
    .unwrap_or_else(|e| panic!("预览失败：{e}"));
    assert_eq!(result.height, 240);
    assert_eq!(result.width % 2, 0);
    assert_close(result.duration, 2.0, 0.15, "预览时长");
    assert_eq!(result.modules.len(), 13, "{:?}", result.modules);
    assert!(result.modules.iter().any(|m| m == "pipe.remove-watermark"));
}
