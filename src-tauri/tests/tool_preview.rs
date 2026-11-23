//! 集成测试：工具预览（`tools::preview::render_tool_preview`）——低清片段经工具自己的运行逻辑处理后放进
//! 预览目录；时长 / 高度受限；临时文件清理；不支持预览的工具报错；取消时不留文件。

mod common;
mod tool_support;

use std::path::Path;
use std::sync::{Arc, Mutex};

use cliprecipe_lib::ffmpeg::CancelToken;
use cliprecipe_lib::model::Settings;
use cliprecipe_lib::preview::{PreviewDir, PreviewError};
use cliprecipe_lib::tools::preview::{render_tool_preview, ToolPreviewSpec};
use cliprecipe_lib::tools::{default_tool, TOOL_IDS};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn spec(dir: &TempDir, tool: &str, input: &Path, params: Value) -> ToolPreviewSpec {
    ToolPreviewSpec {
        tool: tool.into(),
        params,
        path: input.to_string_lossy().into_owned(),
        info: None,
        settings: Settings::default(),
        bitrate: None,
        start: 1.0,
        duration: 3.0,
        max_height: 240,
        seed: Some(5),
        number: 1,
        temp_root: Some(dir.join("tmp")),
    }
}

fn source(paths: &cliprecipe_lib::ffmpeg::FfmpegPaths, out: &Path) {
    make_lavfi(
        paths,
        out,
        "testsrc2=s=640x480:r=25:d=8",
        Some("sine=frequency=440:duration=8"),
        &[],
    );
}

#[test]
fn previewable_tools_render_a_short_low_res_clip() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("tool-preview");
    let input = dir.join("src/clip.mp4");
    source(&paths, &input);
    let previews = PreviewDir::under(&dir.join("preview"), 1);
    for (tool, params) in [
        ("tool.camera-move", json!({})),
        ("tool.progress-bar", json!({})),
        (
            "tool.freeze-play",
            json!({"positions": "50%", "pauseSeconds": 0.5}),
        ),
        ("tool.insert-frame", Value::Null),
        ("tool.mirror-remix", json!({"fallbackSeconds": 1.0})),
        ("tool.freeze-anim", json!({})),
    ] {
        let percents = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&percents);
        let result = render_tool_preview(
            &paths,
            &spec(&dir, tool, &input, params),
            &previews,
            &CancelToken::new(),
            move |p| seen.lock().unwrap().push(p),
        );
        // 插帧没有选择图片时按工具自己的规则失败——预览把工具的错误原样返回
        if tool == "tool.insert-frame" {
            assert!(
                matches!(&result, Err(PreviewError::Failed(_))),
                "{tool}: {result:?}"
            );
            continue;
        }
        let result = result.unwrap_or_else(|e| panic!("{tool}: {e}"));
        let file = Path::new(&result.path);
        assert!(previews.contains(file), "{tool}：结果在预览目录里");
        assert!(file.is_file());
        assert_eq!(result.modules, [tool]);
        assert!(
            result.height <= 240 && result.height > 0,
            "{tool}: {result:?}"
        );
        let expected = if tool == "tool.freeze-play" { 3.5 } else { 3.0 };
        assert_close(result.duration, expected, 0.15, tool);
        let percents = percents.lock().unwrap();
        assert!(
            percents.windows(2).all(|w| w[0] <= w[1] + 1e-9),
            "{tool}：进度不倒退"
        );
        assert_eq!(percents.last().copied(), Some(100.0));
        previews.release(file).unwrap();
    }
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
}

#[test]
fn unsupported_tools_and_cancel_leave_nothing() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("tool-preview-errors");
    let input = dir.join("src/clip.mp4");
    source(&paths, &input);
    let previews = PreviewDir::under(&dir.join("preview"), 2);
    let err = render_tool_preview(
        &paths,
        &spec(&dir, "tool.merge", &input, Value::Null),
        &previews,
        &CancelToken::new(),
        |_| {},
    )
    .unwrap_err();
    assert!(err.to_string().contains("不支持预览"), "{err}");

    let token = CancelToken::new();
    token.cancel();
    let err = render_tool_preview(
        &paths,
        &spec(&dir, "tool.camera-move", &input, Value::Null),
        &previews,
        &token,
        |_| {},
    )
    .unwrap_err();
    assert_eq!(err, PreviewError::Cancelled);
    assert!(
        all_files(&dir.join("tmp")).is_empty(),
        "取消后临时文件已清理"
    );
    assert!(
        all_files(&dir.join("preview")).is_empty(),
        "取消后没有预览文件"
    );

    // 至少这些一进一出的视频效果工具声明了预览
    let previewable: Vec<&str> = TOOL_IDS
        .iter()
        .copied()
        .filter(|id| default_tool(id).is_some_and(|t| t.supports_preview()))
        .collect();
    for id in [
        "tool.camera-move",
        "tool.progress-bar",
        "tool.freeze-play",
        "tool.insert-frame",
        "tool.remove-black-bars",
        "tool.mirror-remix",
    ] {
        assert!(previewable.contains(&id), "{id} 应支持预览");
    }
}
