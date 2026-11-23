//! 同名资源（`queue.sidecar`）：视频同目录下文件名（不含扩展名）相同的字幕与图片。
//! 导入时按设置 `set.auto-subtitle` / `set.auto-cover` 自动关联，队列菜单“全部重载”时重新查找。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::pipeline::steps::{COVER_EXTENSIONS, SUBTITLE_EXTENSIONS};

/// 查找哪些同名资源。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SidecarOptions {
    pub subtitle: bool,
    pub cover: bool,
}

impl SidecarOptions {
    pub fn any(&self) -> bool {
        self.subtitle || self.cover
    }
}

/// 找到的同名资源。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Sidecars {
    pub subtitle: Option<String>,
    pub cover: Option<String>,
}

/// 同目录下第一个存在的 `<名称>.<扩展名>`（按扩展名顺序；扩展名不区分大小写）。
fn find_with(video: &Path, extensions: &[&str]) -> Option<PathBuf> {
    let dir = video.parent()?;
    let stem = video.file_stem()?.to_string_lossy().into_owned();
    let entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    extensions.iter().find_map(|ext| {
        entries
            .iter()
            .find(|p| {
                p.file_stem().is_some_and(|s| s.to_string_lossy() == stem)
                    && p.extension()
                        .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(ext))
            })
            .cloned()
    })
}

/// 查找一个视频的同名字幕 / 封面。
pub fn find_sidecars(video: &Path, options: SidecarOptions) -> Sidecars {
    let text = |p: PathBuf| p.to_string_lossy().into_owned();
    Sidecars {
        subtitle: options
            .subtitle
            .then(|| find_with(video, &SUBTITLE_EXTENSIONS))
            .flatten()
            .map(text),
        cover: options
            .cover
            .then(|| find_with(video, &COVER_EXTENSIONS))
            .flatten()
            .map(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_same_name_files_in_priority_order() {
        let dir = std::env::temp_dir().join(format!("cr-sidecar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "clip.mp4",
            "clip.ASS",
            "clip.srt",
            "clip.PNG",
            "other.jpg",
            "clip.txt",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let video = dir.join("clip.mp4");
        let all = SidecarOptions {
            subtitle: true,
            cover: true,
        };
        let found = find_sidecars(&video, all);
        assert_eq!(
            found.subtitle,
            Some(dir.join("clip.srt").to_string_lossy().into_owned())
        );
        assert_eq!(
            found.cover,
            Some(dir.join("clip.PNG").to_string_lossy().into_owned())
        );
        assert_eq!(
            find_sidecars(&video, SidecarOptions::default()),
            Sidecars::default()
        );
        assert_eq!(
            find_sidecars(&dir.join("none.mp4"), all),
            Sidecars::default()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
