//! 素材来源：单个文件，或一个目录（随机 / 依次取其中的文件）。
//! 水印、背景、片头尾、背景音等模块共用。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::rng::SeededRng;
use crate::fsutil;

/// 目录模式下的取法。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PickOrder {
    /// 每个视频随机取一个（由种子决定）
    #[default]
    Random,
    /// 按队列序号依次取，取完从头循环
    Sequential,
}

/// JSON 形如 `{"mode":"file","path":"..."}` 或
/// `{"mode":"directory","path":"...","recursive":false,"order":"random"}`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum AssetSource {
    #[serde(rename_all = "camelCase")]
    File { path: String },
    #[serde(rename_all = "camelCase")]
    Directory {
        path: String,
        #[serde(default)]
        recursive: bool,
        #[serde(default)]
        order: PickOrder,
    },
}

impl Default for AssetSource {
    fn default() -> Self {
        AssetSource::File {
            path: String::new(),
        }
    }
}

impl AssetSource {
    /// 是否还没选择任何路径。
    pub fn is_empty(&self) -> bool {
        match self {
            AssetSource::File { path } | AssetSource::Directory { path, .. } => {
                path.trim().is_empty()
            }
        }
    }

    /// 列出所有候选文件。单文件模式不检查扩展名。
    pub fn candidates(&self, extensions: &[&str]) -> Result<Vec<PathBuf>, String> {
        match self {
            AssetSource::File { path } => {
                let p = PathBuf::from(path);
                if p.is_file() {
                    Ok(vec![p])
                } else {
                    Err(format!("素材文件不存在：{path}"))
                }
            }
            AssetSource::Directory {
                path, recursive, ..
            } => {
                let files = fsutil::list_files(Path::new(path), *recursive, extensions)
                    .map_err(|e| format!("无法读取素材目录 {path}：{e}"))?;
                if files.is_empty() {
                    Err(format!("素材目录中没有可用文件：{path}"))
                } else {
                    Ok(files)
                }
            }
        }
    }

    /// 为第 `task_index` 个视频（从 0 起）选一个素材。
    pub fn pick(
        &self,
        extensions: &[&str],
        task_index: usize,
        rng: &mut SeededRng,
    ) -> Result<PathBuf, String> {
        let files = self.candidates(extensions)?;
        Ok(pick_from(&files, self.order(), task_index, rng).clone())
    }

    fn order(&self) -> PickOrder {
        match self {
            AssetSource::File { .. } => PickOrder::Sequential,
            AssetSource::Directory { order, .. } => *order,
        }
    }
}

/// 从非空列表中按取法选一个。
pub fn pick_from<'a, T>(
    items: &'a [T],
    order: PickOrder,
    task_index: usize,
    rng: &mut SeededRng,
) -> &'a T {
    assert!(!items.is_empty(), "pick_from 需要非空列表");
    match order {
        PickOrder::Random => &items[rng.index(items.len())],
        PickOrder::Sequential => &items[task_index % items.len()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_shape() {
        let src: AssetSource =
            serde_json::from_str(r#"{"mode":"directory","path":"D:/a","order":"sequential"}"#)
                .unwrap();
        assert_eq!(
            src,
            AssetSource::Directory {
                path: "D:/a".into(),
                recursive: false,
                order: PickOrder::Sequential
            }
        );
        let json = serde_json::to_string(&AssetSource::default()).unwrap();
        assert_eq!(json, r#"{"mode":"file","path":""}"#);
    }

    #[test]
    fn pick_orders() {
        let items = ["a", "b", "c"];
        let mut rng = SeededRng::new(1);
        assert_eq!(*pick_from(&items, PickOrder::Sequential, 4, &mut rng), "b");
        let first: Vec<&str> = {
            let mut rng = SeededRng::new(11);
            (0..10)
                .map(|i| *pick_from(&items, PickOrder::Random, i, &mut rng))
                .collect()
        };
        let second: Vec<&str> = {
            let mut rng = SeededRng::new(11);
            (0..10)
                .map(|i| *pick_from(&items, PickOrder::Random, i, &mut rng))
                .collect()
        };
        assert_eq!(first, second);
    }

    #[test]
    fn directory_listing_with_extension_filter() {
        let dir = std::env::temp_dir().join(format!("cr-asset-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for name in ["2.png", "10.png", "note.txt", "sub/1.png"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let flat = AssetSource::Directory {
            path: dir.to_string_lossy().into_owned(),
            recursive: false,
            order: PickOrder::Sequential,
        };
        let names: Vec<String> = flat
            .candidates(&["png"])
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["2.png", "10.png"]);
        let deep = AssetSource::Directory {
            path: dir.to_string_lossy().into_owned(),
            recursive: true,
            order: PickOrder::Sequential,
        };
        assert_eq!(deep.candidates(&["png"]).unwrap().len(), 3);
        let picked = deep.pick(&["png"], 2, &mut SeededRng::new(0)).unwrap();
        assert!(picked.ends_with("1.png"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
