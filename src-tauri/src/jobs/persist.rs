//! 未完成任务的保存与恢复（`queue.persist` / `set.keep-queue`）。
//!
//! 前端在队列变化时（防抖）把未完成的任务交给 [`save_queue`]，写到配置目录的 `queue.json`；
//! 下次启动时 [`restore_queue`] 读回：已不存在的视频被丢弃（列在 `missing` 里，前端写进处理记录），
//! 关联的字幕 / 封面已不存在的被取消关联（`notes`），缺少媒体信息的重新探测。

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::import::ImportedItem;
use super::sidecar::Sidecars;
use crate::ffmpeg::MediaInfo;

pub const QUEUE_FILE: &str = "queue.json";
const QUEUE_VERSION: u32 = 1;

/// 保存的一个任务。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedTask {
    pub path: String,
    pub source_root: Option<String>,
    pub info: Option<MediaInfo>,
    pub subtitle: Option<String>,
    pub cover: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SavedQueue {
    version: u32,
    tasks: Vec<SavedTask>,
}

/// 恢复结果。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredQueue {
    pub items: Vec<ImportedItem>,
    /// 已不存在、被丢弃的视频
    pub missing: Vec<String>,
    /// 其它说明（如关联的字幕已不存在）
    pub notes: Vec<String>,
}

/// 保存队列；列表为空时删除文件。
pub fn save_queue(path: &Path, tasks: &[SavedTask]) -> Result<(), String> {
    if tasks.is_empty() {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("无法删除队列文件：{e}")),
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("无法创建配置目录：{e}"))?;
    }
    let text = serde_json::to_string(&SavedQueue {
        version: QUEUE_VERSION,
        tasks: tasks.to_vec(),
    })
    .map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("无法保存队列：{e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("无法保存队列：{e}"))
}

/// 读取保存的队列。没有文件时返回空结果；文件损坏时返回错误。
pub fn restore_queue(
    path: &Path,
    probe: impl Fn(&Path) -> Result<MediaInfo, String>,
) -> Result<RestoredQueue, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(RestoredQueue::default()),
        Err(e) => return Err(format!("无法读取保存的队列：{e}")),
    };
    let saved: SavedQueue =
        serde_json::from_str(&text).map_err(|e| format!("保存的队列已损坏：{e}"))?;
    let mut out = RestoredQueue::default();
    for task in saved.tasks {
        let video = Path::new(&task.path);
        if !video.is_file() {
            out.missing.push(task.path);
            continue;
        }
        let mut keep_file = |file: Option<String>, what: &str| {
            let file = file?;
            if Path::new(&file).is_file() {
                Some(file)
            } else {
                out.notes
                    .push(format!("{} 关联的{what}已不存在：{file}", task.path));
                None
            }
        };
        let sidecars = Sidecars {
            subtitle: keep_file(task.subtitle, "字幕"),
            cover: keep_file(task.cover, "封面"),
        };
        let (info, error) = match task.info {
            Some(info) => (Some(info), None),
            None => match probe(video) {
                Ok(info) if info.video.is_some() => (Some(info), None),
                Ok(info) => (Some(info), Some("文件中没有视频流".to_owned())),
                Err(e) => (None, Some(e)),
            },
        };
        out.items.push(ImportedItem {
            path: task.path,
            source_root: task.source_root,
            info,
            error,
            sidecars,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::VideoStream;

    #[test]
    fn save_restore_drops_missing_and_reprobes() {
        let dir = std::env::temp_dir().join(format!("cr-persist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(QUEUE_FILE);
        let a = dir.join("a.mp4");
        let b = dir.join("b.mp4");
        let srt = dir.join("a.srt");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();
        std::fs::write(&srt, b"x").unwrap();
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let info = MediaInfo {
            duration: 3.0,
            video: Some(VideoStream::default()),
            ..MediaInfo::default()
        };
        let tasks = vec![
            SavedTask {
                path: s(&a),
                info: Some(info.clone()),
                subtitle: Some(s(&srt)),
                cover: Some(s(&dir.join("gone.jpg"))),
                ..SavedTask::default()
            },
            SavedTask {
                path: s(&b),
                source_root: Some(s(&dir)),
                ..SavedTask::default()
            },
            SavedTask {
                path: s(&dir.join("deleted.mp4")),
                ..SavedTask::default()
            },
        ];
        save_queue(&file, &tasks).unwrap();

        let probed = std::cell::Cell::new(0);
        let probe = |_: &Path| {
            probed.set(probed.get() + 1);
            Ok(MediaInfo {
                duration: 7.0,
                video: Some(VideoStream::default()),
                ..MediaInfo::default()
            })
        };
        let restored = restore_queue(&file, probe).unwrap();
        assert_eq!(probed.get(), 1, "只探测缺少信息的 b");
        assert_eq!(restored.items.len(), 2);
        assert_eq!(restored.items[0].info, Some(info));
        assert_eq!(restored.items[0].sidecars.subtitle, Some(s(&srt)));
        assert_eq!(restored.items[0].sidecars.cover, None);
        assert_eq!(restored.items[1].info.as_ref().unwrap().duration, 7.0);
        assert_eq!(restored.items[1].source_root, Some(s(&dir)));
        assert_eq!(restored.missing, [s(&dir.join("deleted.mp4"))]);
        assert_eq!(restored.notes.len(), 1);

        save_queue(&file, &[]).unwrap();
        assert!(!file.exists());
        assert_eq!(
            restore_queue(&file, probe).unwrap(),
            RestoredQueue::default()
        );
        std::fs::write(&file, "broken").unwrap();
        assert!(restore_queue(&file, probe).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
