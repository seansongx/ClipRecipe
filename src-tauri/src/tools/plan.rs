//! 工具的输入与工作项。
//!
//! 前端提交：队列中的视频（[`TaskSpec`]，选中的或全部）+ 工具声明的文件夹。
//! [`crate::tools::Tool::plan`] 把它们分成**工作项**，每个工作项独立执行（可并行），
//! 有自己的进度、状态和输出。常见的分法：
//!
//! | 形态 | 用法 |
//! |---|---|
//! | 每个视频一项（默认） | [`per_video`] |
//! | 全部视频合成一项（合并、拼接） | [`all_videos_as_one`] |
//! | 文件夹里每个文件一项 | `req.folder_files(..)` + [`per_file`] |
//! | 文件夹里全部文件一项 | `req.folder_files(..)` + [`files_as_one`] |
//! | 两个文件夹配对（音视频合成） | 分别 `folder_files`，自行配对后用 [`WorkItem::new`] |
//!
//! 规划阶段需要随机（打乱顺序、随机配对）时用 `req.rng(工具 ID)`——由本次运行的种子派生，可复现，
//! 而且规划出的工作项就是真正的输入与分组（界面上显示、可并行）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::ffmpeg::MediaInfo;
use crate::fsutil;
use crate::jobs::TaskSpec;
use crate::model::{SeededRng, Settings};

/// 工作项的一个输入文件。
#[derive(Debug, Clone, PartialEq)]
pub struct ItemInput {
    pub path: PathBuf,
    /// 已探测的媒体信息；没有时由 `ctx.info(i)` 在执行时探测
    pub info: Option<MediaInfo>,
    /// 以文件夹方式导入 / 来自文件夹输入时的那个文件夹（“输出到来源文件夹同名目录”用）
    pub source_root: Option<PathBuf>,
}

impl ItemInput {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            info: None,
            source_root: None,
        }
    }

    pub fn from_task(task: &TaskSpec) -> Self {
        Self {
            path: PathBuf::from(&task.path),
            info: task.info.clone(),
            source_root: task.source_root.as_ref().map(PathBuf::from),
        }
    }
}

/// 一个工作项：独立执行、独立汇报进度与结果。
#[derive(Debug, Clone, PartialEq)]
pub struct WorkItem {
    /// 事件里的任务 ID：每个视频一项时就是队列任务 ID，否则由工具生成（同一次运行内唯一）
    pub id: String,
    /// 界面上显示的名称
    pub label: String,
    /// 序号（从 1 起）：输出文件名的队列序号、随机数派生都用它
    pub number: usize,
    /// 输入文件；至少一个，第一个决定默认的输出名与输出目录
    pub inputs: Vec<ItemInput>,
}

impl WorkItem {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        number: usize,
        inputs: Vec<ItemInput>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            number: number.max(1),
            inputs,
        }
    }

    pub fn summary(&self) -> PlannedItem {
        PlannedItem {
            id: self.id.clone(),
            label: self.label.clone(),
            inputs: self
                .inputs
                .iter()
                .map(|i| i.path.to_string_lossy().into_owned())
                .collect(),
        }
    }
}

/// 返回给前端的工作项清单（开始运行时）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedItem {
    pub id: String,
    pub label: String,
    pub inputs: Vec<String>,
}

/// [`crate::tools::Tool::plan`] 的输入。
pub struct PlanRequest<'a> {
    pub tool_id: &'a str,
    /// 队列中要处理的视频（按队列顺序）
    pub videos: &'a [TaskSpec],
    /// 文件夹输入：键为 [`crate::tools::FolderSlot::key`]
    pub folders: &'a BTreeMap<String, PathBuf>,
    pub settings: &'a Settings,
    /// 本次运行的种子（与执行时 `ToolRunConfig::seed` 相同）：规划时需要随机（打乱、随机配对）用 [`PlanRequest::rng`]
    pub seed: u64,
}

impl PlanRequest<'_> {
    /// 规划用的随机数（种子 + `key` 派生，通常传工具 ID）：同一种子规划结果相同，可复现。
    pub fn rng(&self, key: &str) -> SeededRng {
        SeededRng::derive(self.seed, key)
    }

    /// 必选的文件夹输入。
    pub fn folder(&self, key: &str) -> Result<&Path, String> {
        let dir = self
            .folders
            .get(key)
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| format!("请选择文件夹（{key}）"))?;
        if !dir.is_dir() {
            return Err(format!("文件夹不存在：{}", dir.display()));
        }
        Ok(dir)
    }

    /// 文件夹里符合扩展名的文件（自然排序；是否包含子目录由设置 `set.recursive` 决定）。没有文件时报错。
    pub fn folder_files(&self, key: &str, extensions: &[&str]) -> Result<Vec<PathBuf>, String> {
        let dir = self.folder(key)?;
        let files = fsutil::list_files(dir, self.settings.recursive, extensions)
            .map_err(|e| format!("无法读取文件夹 {}：{e}", dir.display()))?;
        if files.is_empty() {
            return Err(format!("文件夹中没有可处理的文件：{}", dir.display()));
        }
        Ok(files)
    }

    /// 要处理的队列视频；为空时报错。
    pub fn require_videos(&self) -> Result<&[TaskSpec], String> {
        if self.videos.is_empty() {
            return Err("没有要处理的视频：请先在队列中添加或选择视频".into());
        }
        Ok(self.videos)
    }
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// 每个队列视频一项（工作项 ID = 队列任务 ID，序号 = 队列序号）。
pub fn per_video(req: &PlanRequest) -> Result<Vec<WorkItem>, String> {
    let videos = req.require_videos()?;
    Ok(videos
        .iter()
        .enumerate()
        .map(|(i, task)| {
            let number = if task.number == 0 { i + 1 } else { task.number };
            WorkItem::new(
                task.id.clone(),
                file_label(Path::new(&task.path)),
                number,
                vec![ItemInput::from_task(task)],
            )
        })
        .collect())
}

/// 全部队列视频合成一项（多进一出）。
pub fn all_videos_as_one(req: &PlanRequest, label: &str) -> Result<Vec<WorkItem>, String> {
    let videos = req.require_videos()?;
    Ok(vec![WorkItem::new(
        format!("{}#all", req.tool_id),
        label,
        1,
        videos.iter().map(ItemInput::from_task).collect(),
    )])
}

/// 文件夹中每个文件一项；ID 为 `<工具 ID>#<序号>`。
pub fn per_file(tool_id: &str, files: &[PathBuf], root: &Path) -> Vec<WorkItem> {
    files
        .iter()
        .enumerate()
        .map(|(i, path)| {
            WorkItem::new(
                format!("{tool_id}#{}", i + 1),
                file_label(path),
                i + 1,
                vec![ItemInput {
                    path: path.clone(),
                    info: None,
                    source_root: Some(root.to_path_buf()),
                }],
            )
        })
        .collect()
}

/// 文件夹中的全部文件合成一项。
pub fn files_as_one(tool_id: &str, label: &str, files: &[PathBuf], root: &Path) -> WorkItem {
    WorkItem::new(
        format!("{tool_id}#all"),
        label,
        1,
        files
            .iter()
            .map(|path| ItemInput {
                path: path.clone(),
                info: None,
                source_root: Some(root.to_path_buf()),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, path: &str, number: usize) -> TaskSpec {
        TaskSpec {
            id: id.into(),
            path: path.into(),
            source_root: None,
            info: None,
            number,
        }
    }

    #[test]
    fn per_video_keeps_task_ids_and_numbers() {
        let videos = [task("a", "/v/one.mp4", 3), task("b", "/v/two.mkv", 0)];
        let folders = BTreeMap::new();
        let settings = Settings::default();
        let req = PlanRequest {
            tool_id: "tool.x",
            videos: &videos,
            folders: &folders,
            settings: &settings,
            seed: 1,
        };
        let items = per_video(&req).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].id.as_str(), items[0].number), ("a", 3));
        assert_eq!((items[1].id.as_str(), items[1].number), ("b", 2));
        assert_eq!(items[1].label, "two.mkv");

        let merged = all_videos_as_one(&req, "合并").unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "tool.x#all");
        assert_eq!(merged[0].inputs.len(), 2);

        let empty = PlanRequest { videos: &[], ..req };
        assert!(per_video(&empty).is_err());
    }

    #[test]
    fn folder_inputs_are_validated_and_listed() {
        let dir = std::env::temp_dir().join(format!("cr-plan-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for name in ["10.mp4", "2.mp4", "note.txt", "sub/1.mp4"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let mut folders = BTreeMap::new();
        folders.insert("source".to_owned(), dir.clone());
        let settings = Settings {
            recursive: false,
            ..Settings::default()
        };
        let req = PlanRequest {
            tool_id: "tool.y",
            videos: &[],
            folders: &folders,
            settings: &settings,
            seed: 1,
        };
        let files = req.folder_files("source", &["mp4"]).unwrap();
        let names: Vec<String> = files.iter().map(|p| file_label(p)).collect();
        assert_eq!(names, ["2.mp4", "10.mp4"]);
        assert!(req.folder("missing").is_err());
        assert!(req.folder_files("source", &["mov"]).is_err());

        let items = per_file("tool.y", &files, &dir);
        assert_eq!(items[1].id, "tool.y#2");
        assert_eq!(
            items[1].inputs[0].source_root.as_deref(),
            Some(dir.as_path())
        );
        let one = files_as_one("tool.y", "拼接", &files, &dir);
        assert_eq!(one.inputs.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
