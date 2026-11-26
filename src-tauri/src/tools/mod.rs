//! 独立工具（`tool.*`）：单一用途的批处理，只沿用流水线的“码率”（`pipe.bitrate`）和“输出位置”。
//!
//! 框架（本目录下的非工具文件）：
//! - [`Tool`]：每个工具的参数结构体实现它——声明输入（[`ToolInputs`]）、把输入分成工作项（[`Tool::plan`]）、
//!   执行一个工作项（[`Tool::run`]）；
//! - [`plan`]：输入（队列视频、文件夹）与工作项（[`WorkItem`]），常用的分组方式；
//! - [`context`]：执行一个工作项时可用的服务（[`ToolContext`]）：探测、编码器与软件回退、码率、
//!   输出命名与冲突处理、临时目录、分步进度、取消、日志、随机数；
//! - [`analysis`]：场景切换、静音、黑边、静止画面检测（先分析、再按结果处理的多步工具用）；
//! - [`media`]：容器/编码兼容表、concat 列表等小工具；
//! - [`output`]：工具输出文件的命名规则；
//! - [`runner`]：并行调度工作项，推送事件（与批量处理相同的 [`crate::jobs::JobEvent`]）；
//! - [`preview`]：工具预览（[`Tool::supports_preview`] 的工具：低清片段 → 工具自己的 `run` → 预览窗口）；
//! - [`common`]：多个工具共用的引擎（片段列表、合并、滤镜命令、位置解析、场景分段）。
//!
//! 全部 34 个工具已在下方 [`registry!`] 中注册（M3）；实现某个工具只需修改它自己的文件，
//! 见 docs/architecture/tools.md。

pub mod analysis;
pub mod common;
pub mod context;
pub mod media;
pub mod output;
pub mod plan;
pub mod preview;
pub mod runner;

use std::fmt;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::ffmpeg::RunError;

pub use context::{EncodeBasis, Encoding, ToolContext};
pub use output::OutputSpec;
pub use plan::{ItemInput, PlanRequest, PlannedItem, WorkItem};
pub use runner::{ToolRunConfig, ToolRunHandle};

/// 占位工具运行时返回的错误信息。
pub const NOT_IMPLEMENTED: &str = "该工具尚未实现";

/// 一个独立工具。实现者是该工具的参数结构体（serde camelCase，带 `Default`）。
pub trait Tool: Send + Sync {
    /// 功能 ID，与 registry.ts、docs/specs/ 一致，如 `tool.remux`。
    fn id(&self) -> &'static str;

    /// 是否已实现。占位工具返回 false，界面上不可启动，后端也会拒绝。
    fn implemented(&self) -> bool {
        true
    }

    /// 工具需要的输入：队列中的视频，和 / 或若干个文件夹。前端 `defineTool` 的 `inputs` 必须一致
    /// （`cargo test` 的 tests/recipe_defaults.rs 会检查）。
    fn inputs(&self) -> ToolInputs {
        ToolInputs::VIDEOS
    }

    /// 把输入分成可并行执行的工作项。默认每个队列视频一项（工作项 ID 就是队列任务 ID）。
    /// 多个输入合成一个输出（合并、拼接）时返回一项；文件夹类工具在这里列出文件、配对。
    fn plan(&self, req: &PlanRequest) -> Result<Vec<WorkItem>, String> {
        plan::per_video(req)
    }

    /// 执行一个工作项。参数无效等错误返回中文原因（`Err("...".into())`）。
    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError>;

    /// 是否支持预览（工具对话框的“预览”按钮）。开启后框架把第一个选中视频的一小段低清片段作为唯一输入，
    /// 用本工具的 [`Tool::run`] 处理，取第一个视频输出在预览窗口播放（见 [`preview`]）。
    /// 只适合一进一出、处理队列视频、几秒片段就能看出效果的工具；前端 `defineTool` 的 `preview` 必须一致。
    fn supports_preview(&self) -> bool {
        false
    }
}

/// 工具需要选择的一个文件夹（如“音频文件夹”）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSlot {
    /// 前后端共用的键，如 `audio`
    pub key: &'static str,
    /// 界面上显示的名称
    pub label: &'static str,
}

/// 工具的输入声明。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInputs {
    /// 处理队列中的视频（选中的或全部）
    pub videos: bool,
    /// 需要选择的文件夹
    pub folders: &'static [FolderSlot],
}

impl ToolInputs {
    /// 只处理队列中的视频（大多数工具）。
    pub const VIDEOS: Self = Self {
        videos: true,
        folders: &[],
    };

    /// 只用文件夹作输入。
    pub const fn folders(folders: &'static [FolderSlot]) -> Self {
        Self {
            videos: false,
            folders,
        }
    }

    /// 队列视频 + 文件夹。
    pub const fn videos_and_folders(folders: &'static [FolderSlot]) -> Self {
        Self {
            videos: true,
            folders,
        }
    }
}

/// 执行一个工作项的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    Cancelled,
    Failed(String),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::Cancelled => write!(f, "已取消"),
            ToolError::Failed(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ToolError {}

impl From<String> for ToolError {
    fn from(message: String) -> Self {
        ToolError::Failed(message)
    }
}

impl From<&str> for ToolError {
    fn from(message: &str) -> Self {
        ToolError::Failed(message.to_owned())
    }
}

impl From<RunError> for ToolError {
    fn from(err: RunError) -> Self {
        match err {
            RunError::Cancelled => ToolError::Cancelled,
            other => ToolError::Failed(other.to_string()),
        }
    }
}

/// 把前端传来的参数 JSON 解析成参数结构体；`null` 表示默认参数。
fn parse_params<P: DeserializeOwned + Default>(id: &str, params: Value) -> Result<P, String> {
    if params.is_null() {
        return Ok(P::default());
    }
    serde_json::from_value(params).map_err(|e| format!("工具参数无效（{id}）：{e}"))
}

/// 注册全部工具：生成 `pub mod`、[`TOOL_IDS`]、[`create`]、[`default_params`]。
macro_rules! registry {
    ($($id:literal => $module:ident :: $params:ident),* $(,)?) => {
        $(pub mod $module;)*

        /// 全部工具 ID（顺序同 registry.ts）。
        pub const TOOL_IDS: &[&str] = &[$($id),*];

        /// 按 ID 和参数 JSON 创建工具。
        pub fn create(id: &str, params: Value) -> Result<Arc<dyn Tool>, String> {
            match id {
                $($id => Ok(Arc::new(parse_params::<$module::$params>(id, params)?)),)*
                _ => Err(format!("未知的工具：{id}")),
            }
        }

        /// 全部工具的默认参数（ID → JSON），用于前后端默认值一致性检查。
        pub fn default_params() -> Map<String, Value> {
            let mut map = Map::new();
            $(map.insert(
                $id.to_owned(),
                serde_json::to_value($module::$params::default()).expect("工具参数总能序列化"),
            );)*
            map
        }
    };
}

registry! {
    "tool.merge" => merge::MergeParams,
    "tool.split" => split::SplitParams,
    "tool.folder-merge" => folder_merge::FolderMergeParams,
    "tool.clip-merge" => clip_merge::ClipMergeParams,
    "tool.hls" => hls::HlsParams,

    "tool.mirror-remix" => mirror_remix::MirrorRemixParams,
    "tool.transition-remix" => transition_remix::TransitionRemixParams,
    "tool.split-remix" => split_remix::SplitRemixParams,
    "tool.remove-remix" => remove_remix::RemoveRemixParams,
    "tool.simple-mix" => simple_mix::SimpleMixParams,

    "tool.freeze-anim" => freeze_anim::FreezeAnimParams,
    "tool.freeze-play" => freeze_play::FreezePlayParams,
    "tool.camera-move" => camera_move::CameraMoveParams,
    "tool.remove-black-bars" => remove_black_bars::RemoveBlackBarsParams,
    "tool.insert-frame" => insert_frame::InsertFrameParams,
    "tool.progress-bar" => progress_bar::ProgressBarParams,
    "tool.orientation" => orientation::OrientationParams,
    "tool.stitch" => stitch::StitchParams,
    "tool.reverse" => reverse::ReverseParams,
    "tool.image-to-video" => image_to_video::ImageToVideoParams,
    "tool.gif" => gif::GifParams,
    "tool.screenshots" => screenshots::ScreenshotsParams,

    "tool.remove-still" => remove_still::RemoveStillParams,
    "tool.remove-silence" => remove_silence::RemoveSilenceParams,
    "tool.smart-trim" => smart_trim::SmartTrimParams,
    "tool.change-duration" => change_duration::ChangeDurationParams,

    "tool.extract-av" => extract_av::ExtractAvParams,
    "tool.av-mux" => av_mux::AvMuxParams,
    "tool.volume-pitch" => volume_pitch::VolumePitchParams,
    "tool.remux" => remux::RemuxParams,

    "tool.subtitle-asr" => subtitle_asr::SubtitleAsrParams,
    "tool.subtitle-extract" => subtitle_extract::SubtitleExtractParams,
    "tool.subtitle-erase" => subtitle_erase::SubtitleEraseParams,
    "tool.cover" => cover::CoverParams,
}

/// 默认参数的工具实例（读取 `implemented()` / `inputs()` 用）。
pub fn default_tool(id: &str) -> Option<Arc<dyn Tool>> {
    create(id, Value::Null).ok()
}

/// 每个工具的输入声明（ID → JSON），用于与前端 `inputs` 对照。
pub fn input_declarations() -> Map<String, Value> {
    TOOL_IDS
        .iter()
        .filter_map(|id| {
            let tool = default_tool(id)?;
            let inputs = serde_json::to_value(tool.inputs()).ok()?;
            Some(((*id).to_owned(), inputs))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_id_creates_a_tool_with_that_id() {
        assert_eq!(TOOL_IDS.len(), 34);
        for id in TOOL_IDS {
            let tool = default_tool(id).unwrap_or_else(|| panic!("{id} 无法创建"));
            assert_eq!(tool.id(), *id);
            assert!(id.starts_with("tool."));
        }
        let mut sorted = TOOL_IDS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), TOOL_IDS.len(), "ID 不能重复");
        assert_eq!(default_params().len(), TOOL_IDS.len());
        assert_eq!(input_declarations().len(), TOOL_IDS.len());
    }

    #[test]
    fn reference_tools_are_implemented() {
        for id in ["tool.remux", "tool.extract-av", "tool.split"] {
            assert!(default_tool(id).unwrap().implemented(), "{id}");
        }
    }

    #[test]
    fn params_are_parsed_strictly_and_null_means_default() {
        assert!(create("tool.remux", Value::Null).is_ok());
        assert!(create("tool.remux", serde_json::json!({})).is_ok());
        assert!(create("tool.remux", serde_json::json!({"container": 3})).is_err());
        assert!(create("tool.nope", Value::Null).is_err());
    }

    #[test]
    fn errors_convert() {
        assert_eq!(ToolError::from(RunError::Cancelled), ToolError::Cancelled);
        assert_eq!(ToolError::from("x"), ToolError::Failed("x".into()));
        assert_eq!(ToolError::Failed("原因".into()).to_string(), "原因");
    }
}
