//! 集成测试：`pipe.text` 文本。在纯色背景的视频上真实运行，按像素断言文字确实画上去了、
//! 位置与延时出现正确；含中文与各种特殊字符（与 `textfile=` 的渲染结果逐像素比对，验证转义）。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{self, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{Recipe, Settings};
use cliprecipe_lib::pipeline::modules::text::{TextItem, TextPoint, TextPosition, TextSource};
use cliprecipe_lib::pipeline::modules::TextParams;
use cliprecipe_lib::pipeline::{self, escape_filter_path, BuildRequest};
use common::*;

/// 深色纯背景（#203040）视频。
fn make_plain(paths: &FfmpegPaths, out: &Path, (w, h): (u32, u32), seconds: f64, audio: bool) {
    let odd = w % 2 == 1 || h % 2 == 1;
    let mut cmd = ffmpeg::command(&paths.ffmpeg);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
    ])
    .arg(format!("color=c=0x203040:s={w}x{h}:r=25:d={seconds}"));
    if audio {
        cmd.args(["-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=440:duration={seconds}"));
    }
    // 纯色画面的实际码率极低，而任务默认按源码率编码会把文字糊掉：用 CBR 填充把源码率撑到 2 Mbps
    cmd.args([
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-x264-params",
        "nal-hrd=cbr",
    ]);
    cmd.args([
        "-b:v", "2M", "-minrate", "2M", "-maxrate", "2M", "-bufsize", "1M",
    ]);
    cmd.args(["-pix_fmt", if odd { "yuv444p" } else { "yuv420p" }]);
    if audio {
        cmd.args(["-c:a", "aac", "-shortest"]);
    }
    let output = cmd.arg(out).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn recipe_with(items: Vec<TextItem>) -> Recipe {
    let mut recipe = Recipe::disabled();
    recipe.modules.text = TextParams {
        enabled: true,
        items,
    };
    recipe
}

fn run_job(paths: &FfmpegPaths, items: Vec<TextItem>, input: &Path, out_dir: &Path) -> PathBuf {
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe: recipe_with(items),
            settings: Settings::default(),
            output_dir: Some(out_dir.to_path_buf()),
            seed: 7,
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
    assert_eq!(
        summary,
        RunSummary {
            done: 1,
            failed: 0,
            cancelled: 0
        },
        "{:#?}",
        c.snapshot()
    );
    match c.final_state("t") {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            output: Some(output),
            ..
        }) => PathBuf::from(output),
        other => panic!("任务应完成，实际：{other:?}"),
    }
}

fn custom(text: &str) -> TextItem {
    TextItem {
        enabled: true,
        source: TextSource::Custom,
        custom_text: text.into(),
        ..TextItem::default()
    }
}

/// 某帧某区域内的亮度：(最大值, 平均值)。
fn luma(
    paths: &FfmpegPaths,
    file: &Path,
    time: f64,
    (x, y, w, h): (u32, u32, u32, u32),
) -> (u8, f64) {
    let px = gray_frame(paths, file, time, &format!("crop={w}:{h}:{x}:{y},"), None);
    let max = px.iter().copied().max().unwrap();
    let mean = px.iter().map(|&v| f64::from(v)).sum::<f64>() / px.len() as f64;
    (max, mean)
}

/// 取一帧灰度像素：`vf` 为前置滤镜（以逗号结尾或为空），`complex` 为 (filter_complex, 输出标签) 时改用它。
fn gray_frame(
    paths: &FfmpegPaths,
    file: &Path,
    time: f64,
    vf: &str,
    complex: Option<(&str, &str)>,
) -> Vec<u8> {
    let mut cmd = ffmpeg::command(&paths.ffmpeg);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-ss",
        &time.to_string(),
        "-i",
    ])
    .arg(file);
    match complex {
        Some((fc, map)) => {
            cmd.arg("-filter_complex")
                .arg(format!("{fc};{map}format=gray[gray]"))
                .args(["-map", "[gray]"]);
        }
        None => {
            cmd.arg("-vf").arg(format!("{vf}format=gray"));
        }
    }
    let output = cmd
        .args(["-frames:v", "1", "-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

/// 用正式的构建流程生成滤镜，直接对一帧渲染（不编码），返回灰度像素。
fn render_with_pipeline(
    paths: &FfmpegPaths,
    input: &Path,
    items: Vec<TextItem>,
) -> (String, Vec<u8>) {
    let info = probe(paths, input);
    let recipe = recipe_with(items);
    let built = pipeline::build(&BuildRequest {
        input,
        output: Path::new("unused.mp4"),
        info: &info,
        recipe: &recipe,
        settings: &Settings::default(),
        encoders: &EncoderCaps::software_only(),
        seed: 1,
        task_index: 0,
        force_software: true,
        probe: None,
        sidecar: cliprecipe_lib::pipeline::Sidecar::NONE,
    })
    .unwrap();
    assert!(built.warnings.is_empty(), "{:?}", built.warnings);
    let fc = built.arg_after("-filter_complex").unwrap().to_owned();
    let map = built.arg_after("-map").unwrap().to_owned();
    let pixels = gray_frame(paths, input, 0.5, "", Some((&fc, &map)));
    (fc, pixels)
}

#[test]
fn chinese_text_is_drawn_in_the_expected_place() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("text-cn");
    let input = dir.join("in.mp4");
    make_plain(&paths, &input, (640, 360), 2.0, true);
    let item = TextItem {
        font_size: 10.0,
        ..custom("中文测试")
    };
    let output = run_job(&paths, vec![item], &input, &dir.join("out"));
    let info = probe(&paths, &output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (640, 360));
    assert_close(info.duration, 2.0, 0.15, "时长");
    assert_eq!(info.audio.len(), 1);
    // 字号 36，底部居中、距底 29：文字约在 x 248～392、y 295～331
    let (bg_max, bg_mean) = luma(&paths, &output, 1.0, (20, 20, 200, 200));
    let (max, mean) = luma(&paths, &output, 1.0, (250, 298, 140, 30));
    assert!(
        max > 200 && mean > bg_mean + 15.0,
        "文字区域应有白色笔画：{max} {mean}（背景 {bg_mean}）"
    );
    // 文字区域以外仍是背景
    assert!(bg_max < 70, "背景区域不应有文字：{bg_max}");
}

#[test]
fn chinese_glyphs_are_real_not_placeholder_boxes() {
    // 字体缺字时每个字都画成同样的方框；不同的汉字应得到不同的画面
    let paths = ffmpeg_paths();
    let dir = TempDir::new("text-glyphs");
    let input = dir.join("in.mp4");
    make_plain(&paths, &input, (320, 240), 1.0, false);
    let big = |t: &str| TextItem {
        font_size: 25.0,
        outline_width: 0.0,
        position: TextPosition::Center,
        ..custom(t)
    };
    let (_, a) = render_with_pipeline(&paths, &input, vec![big("中")]);
    let (_, b) = render_with_pipeline(&paths, &input, vec![big("永")]);
    let blank = gray_frame(&paths, &input, 0.5, "", None);
    let diff = |p: &[u8], q: &[u8]| p.iter().zip(q).filter(|(x, y)| x != y).count();
    assert!(diff(&a, &blank) > 500, "应画出文字");
    assert!(
        diff(&a, &b) > 300,
        "两个不同汉字的画面应明显不同：{}",
        diff(&a, &b)
    );
}

#[test]
fn special_characters_render_exactly_like_textfile() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("text-escape");
    let input = dir.join("in.mp4");
    make_plain(&paths, &input, (480, 270), 1.0, false);
    let tricky = "a:b 'q' 100% %{pts} C:\\x\\y,[z];\n第二行 \"引号\" =#@";
    let (fc, ours) = render_with_pipeline(&paths, &input, vec![custom(tricky)]);
    assert!(fc.contains("expansion=none:text="), "{fc}");

    // 同样的文字写进文件，用 textfile= 渲染作为参照
    let text_file = dir.join("text.txt");
    std::fs::write(&text_file, tricky).unwrap();
    let (placeholder_fc, _) = render_with_pipeline(&paths, &input, vec![custom("PLACEHOLDER")]);
    let reference_fc = placeholder_fc.replacen(
        "text=PLACEHOLDER",
        &format!(
            "textfile={}",
            escape_filter_path(&text_file.to_string_lossy())
        ),
        1,
    );
    assert_ne!(reference_fc, placeholder_fc);
    let map = "[v0]";
    let reference = gray_frame(&paths, &input, 0.5, "", Some((&reference_fc, map)));
    assert_eq!(ours.len(), reference.len());
    assert!(
        ours != gray_frame(&paths, &input, 0.5, "", None),
        "应画出文字"
    );
    assert!(
        ours == reference,
        "转义后的 text= 应与 textfile= 渲染结果完全一致"
    );
    // 反证：少一个字符（或转义出错）时画面不同
    let (_, shorter) = render_with_pipeline(&paths, &input, vec![custom(&tricky[1..])]);
    assert!(shorter != reference);
}

#[test]
fn delayed_appearance_and_duration() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("text-delay");
    let input = dir.join("in.mp4");
    make_plain(&paths, &input, (640, 360), 4.0, true);
    let item = TextItem {
        font_size: 12.0,
        position: TextPosition::Center,
        start: 1.0,
        duration: 1.5,
        ..custom("延时 Delay")
    };
    let output = run_job(&paths, vec![item], &input, &dir.join("out"));
    let area = (200, 150, 240, 60);
    let visible = |t: f64| luma(&paths, &output, t, area).0 > 200;
    assert!(!visible(0.5), "1 秒前不应出现");
    assert!(visible(1.6), "1～2.5 秒应出现");
    assert!(!visible(3.2), "2.5 秒后应消失");
}

#[test]
fn portrait_odd_and_silent_clips_with_file_name_and_point() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("text-shapes");

    // 竖屏、文件名来源（删子串 + 分行）、三处文本
    let portrait = dir.join("片名_第1集#副标题.mp4");
    make_plain(&paths, &portrait, (360, 640), 1.5, true);
    let items = vec![
        TextItem {
            enabled: true,
            source: TextSource::FileName,
            strip: vec!["_第1集".into()],
            line_breaks: "#".into(),
            position: TextPosition::Top,
            ..TextItem::default()
        },
        TextItem {
            background: true,
            shadow: true,
            position: TextPosition::BottomRight,
            ..custom("右下")
        },
        TextItem {
            position: TextPosition::Custom,
            point: Some(TextPoint { x: 180, y: 320 }),
            ..custom("中")
        },
    ];
    let output = run_job(&paths, items, &portrait, &dir.join("out1"));
    let info = probe(&paths, &output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (360, 640));
    assert_eq!(info.audio.len(), 1);
    // 顶部两行（距顶 6%≈38）、右下角、画面中心都有文字
    assert!(luma(&paths, &output, 0.5, (20, 40, 320, 60)).0 > 200);
    assert!(luma(&paths, &output, 0.5, (250, 560, 90, 40)).0 > 200);
    assert!(luma(&paths, &output, 0.5, (160, 300, 40, 40)).0 > 200);
    // 左下角没有文字
    assert!(luma(&paths, &output, 0.5, (10, 450, 100, 100)).0 < 70);

    // 奇数尺寸、无音频
    let odd = dir.join("odd.mp4");
    make_plain(&paths, &odd, (321, 241), 1.0, false);
    let output = run_job(&paths, vec![custom("奇数 Odd")], &odd, &dir.join("out2"));
    let info = probe(&paths, &output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (320, 240));
    assert!(info.audio.is_empty());
    // 字号 12、距底 19：文字在 y≈210～222
    assert!(luma(&paths, &output, 0.5, (110, 205, 100, 22)).0 > 200);
}
