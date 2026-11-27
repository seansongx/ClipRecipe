//! 集成测试：预览片段 / 单帧（pipe.preview）与源视频取帧（区域框选用）。

mod common;

use std::path::Path;

use cliprecipe_lib::ffmpeg::CancelToken;
use cliprecipe_lib::model::{Range, Recipe, Settings};
use cliprecipe_lib::pipeline::PreviewOutput;
use cliprecipe_lib::preview::{
    extract_frame, render_preview, PreviewDir, PreviewError, PreviewSpec,
};
use common::*;

fn spec(path: &Path, recipe: Recipe, output: PreviewOutput) -> PreviewSpec {
    PreviewSpec {
        path: path.to_string_lossy().into_owned(),
        info: None,
        recipe,
        settings: Settings::default(),
        only_module: None,
        output,
        seed: Some(3),
        number: 1,
        max_height: 360,
        subtitle: None,
    }
}

fn files_in(dir: &PreviewDir) -> usize {
    std::fs::read_dir(dir.path()).map_or(0, |rd| rd.count())
}

#[test]
fn preview_clip_frame_and_source_frame() {
    let paths = ffmpeg_paths();
    let temp = TempDir::new("preview");
    let input = temp.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 1280,
            height: 720,
            fps: 25,
            seconds: 4.0,
            audio: true,
        },
    );
    let dir = PreviewDir::under(temp.path(), 1);

    // 整条流水线：帧率、码率等不参与预览；画面缩小到 360p
    let mut recipe = Recipe::default();
    recipe.modules.fps.fps = Range::fixed(10.0);
    recipe.modules.bitrate.enabled = true;
    recipe.fission = 4;
    let mut progress = Vec::new();
    let clip = render_preview(
        &paths,
        &spec(
            &input,
            recipe,
            PreviewOutput::Clip {
                start: 1.0,
                duration: 2.0,
            },
        ),
        &dir,
        &CancelToken::new(),
        |p| progress.push(p),
    )
    .unwrap();
    assert_eq!((clip.width, clip.height), (640, 360));
    assert!(
        (clip.duration - 2.0).abs() < 0.3,
        "片段时长 {}",
        clip.duration
    );
    assert!(clip.modules.is_empty(), "{:?}", clip.modules);
    let info = probe(&paths, Path::new(&clip.path));
    let video = info.video.unwrap();
    assert_eq!(video.codec, "h264");
    assert!((video.fps - 25.0).abs() < 0.01, "帧率模块不参与预览");
    assert_eq!(info.audio.len(), 1);
    assert!(Path::new(&clip.path).starts_with(dir.path()));
    assert!(!progress.is_empty());

    // 单帧
    let frame = render_preview(
        &paths,
        &spec(
            &input,
            Recipe::disabled(),
            PreviewOutput::Frame { time: 1.0 },
        ),
        &dir,
        &CancelToken::new(),
        |_| {},
    )
    .unwrap();
    assert!(frame.path.ends_with(".jpg"));
    assert_eq!((frame.width, frame.height), (640, 360));

    // 取帧时间不在开头时直接定位过去：去头尾、动态缩放（按帧序号计算）一起开也能出图，结尾附近同样
    let mut seeked = Recipe::disabled();
    seeked.modules.trim.enabled = true;
    seeked.modules.zoom.enabled = true;
    for time in [1.5, 99.0] {
        let late = render_preview(
            &paths,
            &spec(&input, seeked.clone(), PreviewOutput::Frame { time }),
            &dir,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
        assert_eq!((late.width, late.height), (640, 360));
        assert_eq!(late.modules, ["pipe.trim", "pipe.zoom"]);
        dir.release(Path::new(&late.path)).unwrap();
    }
    // 抽帧并同步加速音频：不能定位（从头解码），加了音频滤镜也照样能出单帧
    seeked.modules.drop_frames.enabled = true;
    let dropped = render_preview(
        &paths,
        &spec(&input, seeked, PreviewOutput::Frame { time: 1.0 }),
        &dir,
        &CancelToken::new(),
        |_| {},
    )
    .unwrap();
    assert!(dropped.modules.contains(&"pipe.drop-frames".to_owned()));
    dir.release(Path::new(&dropped.path)).unwrap();

    // 源视频取帧：返回源尺寸
    let source = extract_frame(&paths, &input, None, 2.5, &dir, &CancelToken::new()).unwrap();
    assert_eq!((source.width, source.height), (1280, 720));
    assert_eq!(source.time, 2.5);
    let image = probe(&paths, Path::new(&source.path));
    assert_eq!(image.video.unwrap().width, 1280);
    // 超出时长的时间被限制在结尾附近
    let end = extract_frame(&paths, &input, None, 99.0, &dir, &CancelToken::new()).unwrap();
    assert!(end.time < 4.0 && Path::new(&end.path).is_file());

    // 用完删除；目录清理
    assert_eq!(files_in(&dir), 4);
    dir.release(Path::new(&clip.path)).unwrap();
    assert_eq!(files_in(&dir), 3);
    dir.remove_all();
    assert!(!dir.path().exists());
}

#[test]
fn single_module_preview_and_cancellation() {
    let paths = ffmpeg_paths();
    let temp = TempDir::new("preview-cancel");
    let input = temp.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 1.0,
            ..Clip::default()
        },
    );
    let dir = PreviewDir::under(temp.path(), 1);

    let mut only = spec(
        &input,
        Recipe::default(),
        PreviewOutput::Clip {
            start: 0.0,
            duration: 1.0,
        },
    );
    only.only_module = Some("pipe.fps".into());
    let err = render_preview(&paths, &only, &dir, &CancelToken::new(), |_| {}).unwrap_err();
    assert_eq!(err, PreviewError::Failed("该模块不支持预览".into()));

    let token = CancelToken::new();
    token.cancel();
    let err = render_preview(
        &paths,
        &spec(
            &input,
            Recipe::disabled(),
            PreviewOutput::Frame { time: 0.0 },
        ),
        &dir,
        &token,
        |_| {},
    )
    .unwrap_err();
    assert_eq!(err, PreviewError::Cancelled);
    assert_eq!(files_in(&dir), 0, "取消后不留文件");

    let missing = render_preview(
        &paths,
        &spec(
            &temp.join("none.mp4"),
            Recipe::disabled(),
            PreviewOutput::Frame { time: 0.0 },
        ),
        &dir,
        &CancelToken::new(),
        |_| {},
    );
    assert!(matches!(missing, Err(PreviewError::Failed(_))));
}
