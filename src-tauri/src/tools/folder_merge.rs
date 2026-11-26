//! `tool.folder-merge` 文件夹合并：按与 `tool.merge` 相同的规则合并一个文件夹里的视频。
//! 合并引擎（无损 / 重编码、转场、中间视频、输出名）在 [`crate::tools::common::merge_engine`]，参数含义见 [`crate::tools::merge`]。
//!
//! 参数与取值约定（docs/specs/tools.md 未规定的部分）：
//! - 分组 `grouping`：
//!   - `subfolder`（默认）：每个一级子文件夹合成一个视频，输出名为子文件夹名；文件夹本身（顶层）的视频另成一组，
//!     输出名为文件夹名。子文件夹里更深的层级按设置“目录模式包含子目录”决定是否包含（并入所属的一级子文件夹）。
//!   - `all`：文件夹里的全部视频合成一个，输出名为文件夹名（是否包含子目录按设置）。
//!   - `everyN`：按自然顺序每 `groupSize` 个合成一个，输出名 `<第一个视频名>_merged`（是否包含子目录按设置）。
//! - `shuffle`：组内按种子随机打乱顺序；`everyN` 时先打乱全部再分组。打乱在规划时完成（`req.rng`），
//!   每组仍是一个工作项（可并行），同一种子结果相同。
//! - `joinNames` 开启时输出名改为各源文件名以空格连接。
//! - 每组至少 1 个视频；只有 1 个时输出即为它本身（警告）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::{self, natural_cmp, VIDEO_EXTENSIONS};
use crate::tools::common::merge_engine::{chunk_items, merged_stem, run_groups, shuffle, MergeOptions};
use crate::tools::merge::validate_group_size;
use crate::tools::plan::{self, ItemInput};
use crate::tools::{FolderSlot, PlanRequest, Tool, ToolContext, ToolError, ToolInputs, WorkItem};

pub const ID: &str = "tool.folder-merge";
pub const SOURCE: &str = "source";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderGrouping {
    /// 每个子文件夹合成一个
    #[default]
    Subfolder,
    /// 全部合成一个
    All,
    /// 每 N 个合成一个
    EveryN,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FolderMergeParams {
    pub grouping: FolderGrouping,
    pub group_size: u32,
    pub shuffle: bool,
    #[serde(flatten)]
    pub merge: MergeOptions,
}

impl Default for FolderMergeParams {
    fn default() -> Self {
        Self {
            grouping: FolderGrouping::Subfolder,
            group_size: 2,
            shuffle: false,
            merge: MergeOptions::default(),
        }
    }
}

impl FolderMergeParams {
    fn validate(&self) -> Result<(), String> {
        if self.grouping == FolderGrouping::EveryN {
            validate_group_size(self.group_size)?;
        }
        self.merge.validate()
    }
}

fn folder_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "合并".into())
}

/// 按一级子文件夹分组：（组名，视频）。顶层视频一组（组名为文件夹名），每个一级子文件夹一组；
/// `recursive` 时子文件夹里更深的视频并入所属的一级子文件夹。空组省略。
pub fn subfolder_groups(
    root: &Path,
    recursive: bool,
) -> Result<Vec<(String, Vec<PathBuf>)>, String> {
    let read_err = |e: std::io::Error| format!("无法读取文件夹 {}：{e}", root.display());
    let mut groups = Vec::new();
    let top = fsutil::list_files(root, false, VIDEO_EXTENSIONS).map_err(read_err)?;
    if !top.is_empty() {
        groups.push((folder_name(root), top));
    }
    let mut subdirs: Vec<PathBuf> = std::fs::read_dir(root)
        .map_err(read_err)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    subdirs.sort_by(|a, b| natural_cmp(&folder_name(a), &folder_name(b)));
    for sub in subdirs {
        let files = fsutil::list_files(&sub, recursive, VIDEO_EXTENSIONS).unwrap_or_default();
        if !files.is_empty() {
            groups.push((folder_name(&sub), files));
        }
    }
    Ok(groups)
}

fn inputs_under(files: &[PathBuf], root: &Path) -> Vec<ItemInput> {
    files
        .iter()
        .map(|path| ItemInput {
            path: path.clone(),
            info: None,
            source_root: Some(root.to_path_buf()),
        })
        .collect()
}

impl Tool for FolderMergeParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn inputs(&self) -> ToolInputs {
        const FOLDERS: &[FolderSlot] = &[FolderSlot {
            key: SOURCE,
            label: "视频文件夹",
        }];
        ToolInputs::folders(FOLDERS)
    }

    fn plan(&self, req: &PlanRequest) -> Result<Vec<WorkItem>, String> {
        self.validate()?;
        let root = req.folder(SOURCE)?.to_path_buf();
        // 打乱在规划时完成（由本次运行的种子派生）：工作项里就是真正的视频与顺序，各组可并行
        let mut rng = req.rng(ID);
        let mut order = |mut files: Vec<PathBuf>| {
            if self.shuffle {
                shuffle(&mut files, &mut rng);
            }
            files
        };
        match self.grouping {
            FolderGrouping::Subfolder => {
                let groups = subfolder_groups(&root, req.settings.recursive)?;
                if groups.is_empty() {
                    return Err(format!("文件夹中没有可处理的视频：{}", root.display()));
                }
                Ok(groups
                    .into_iter()
                    .enumerate()
                    .map(|(k, (name, files))| {
                        WorkItem::new(
                            format!("{ID}#{}", k + 1),
                            name,
                            k + 1,
                            inputs_under(&order(files), &root),
                        )
                    })
                    .collect())
            }
            FolderGrouping::All => {
                let files = order(req.folder_files(SOURCE, VIDEO_EXTENSIONS)?);
                Ok(vec![plan::files_as_one(
                    ID,
                    &folder_name(&root),
                    &files,
                    &root,
                )])
            }
            FolderGrouping::EveryN => {
                let files = order(req.folder_files(SOURCE, VIDEO_EXTENSIONS)?);
                Ok(chunk_items(
                    ID,
                    inputs_under(&files, &root),
                    self.group_size as usize,
                ))
            }
        }
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        // 分组与顺序在规划时已经确定：一个工作项就是一组
        let group: Vec<usize> = (0..ctx.inputs().len()).collect();
        if group.len() < 2 {
            ctx.warn("有一组只有 1 个视频，输出即为该视频本身");
        }
        if self.grouping == FolderGrouping::EveryN {
            run_groups(ctx, &self.merge, vec![group], merged_stem)
        } else {
            // 子文件夹 / 全部：输出名为（子）文件夹名，即工作项标签
            run_groups(ctx, &self.merge, vec![group], |ctx, _| {
                Ok(ctx.item().label.clone())
            })
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Settings;
    use std::collections::BTreeMap;

    fn names(files: &[PathBuf]) -> Vec<String> {
        files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn groups_by_first_level_subfolder() {
        let dir = std::env::temp_dir().join(format!("cr-folder-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for name in [
            "top.mp4",
            "note.txt",
            "s10/a.mp4",
            "s2/b.mp4",
            "s2/c.mkv",
            "s2/deep/d.mp4",
            "empty/readme.txt",
        ] {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"").unwrap();
        }
        let flat = subfolder_groups(&dir, false).unwrap();
        let summary: Vec<(String, Vec<String>)> =
            flat.iter().map(|(n, f)| (n.clone(), names(f))).collect();
        let root_name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            summary,
            [
                (root_name.clone(), vec!["top.mp4".to_owned()]),
                (
                    "s2".to_owned(),
                    vec!["b.mp4".to_owned(), "c.mkv".to_owned()]
                ),
                ("s10".to_owned(), vec!["a.mp4".to_owned()]),
            ]
        );
        let deep = subfolder_groups(&dir, true).unwrap();
        assert_eq!(names(&deep[1].1), ["b.mp4", "c.mkv", "d.mp4"]);

        // 规划：每组一个工作项，标签为组名；every N 按文件分组
        let mut folders = BTreeMap::new();
        folders.insert(SOURCE.to_owned(), dir.clone());
        let settings = Settings {
            recursive: true,
            ..Settings::default()
        };
        let req = PlanRequest {
            tool_id: ID,
            videos: &[],
            folders: &folders,
            settings: &settings,
            seed: 1,
        };
        let items = FolderMergeParams::default().plan(&req).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(
            (items[1].id.as_str(), items[1].label.as_str()),
            ("tool.folder-merge#2", "s2")
        );
        assert_eq!(
            items[1].inputs[0].source_root.as_deref(),
            Some(dir.as_path())
        );
        let every = FolderMergeParams {
            grouping: FolderGrouping::EveryN,
            group_size: 3,
            ..FolderMergeParams::default()
        };
        assert_eq!(every.plan(&req).unwrap().len(), 2, "5 个视频每 3 个一组");
        let all = FolderMergeParams {
            grouping: FolderGrouping::All,
            ..FolderMergeParams::default()
        };
        let one = all.plan(&req).unwrap();
        assert_eq!((one.len(), one[0].inputs.len()), (1, 5));
        assert_eq!(one[0].label, root_name);
        let bad = FolderMergeParams {
            group_size: 0,
            ..every
        };
        assert!(bad.plan(&req).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
