//! 输出路径解析：输出目录、按来源文件夹 / 视频名分目录、文件名去除、队列序号、
//! 裂变（`pipe.fission`）、目标已存在时重命名或覆盖。

use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Serialize;

use crate::model::{NameStrip, OnExists, Settings};
use crate::pipeline::output_extension;

/// 裂变中的第几次（从 1 起）与总次数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FissionSlot {
    pub round: usize,
    pub total: usize,
}

impl FissionSlot {
    /// 不裂变。
    pub const SINGLE: Self = Self { round: 1, total: 1 };

    pub fn is_fission(&self) -> bool {
        self.total > 1
    }
}

/// “第 x 次裂变”目录名（`set.fission-folder`）。
pub fn fission_folder_name(round: usize) -> String {
    format!("第 {round} 次裂变")
}

/// 解析输出路径所需的信息。
pub struct OutputRequest<'a> {
    pub input: &'a Path,
    /// 以文件夹方式导入时的那个文件夹
    pub source_root: Option<&'a Path>,
    /// 输出目录；None 表示输出到源文件所在目录
    pub output_dir: Option<&'a Path>,
    pub settings: &'a Settings,
    /// 队列序号（从 1 起）
    pub number: usize,
    /// 队列总数（决定序号补零的位数）
    pub total: usize,
    /// 裂变轮次；不裂变时为 [`FissionSlot::SINGLE`]
    pub fission: FissionSlot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputPlan {
    pub path: PathBuf,
    /// 将覆盖已存在的文件
    pub overwrite: bool,
    /// 输出目录与源文件目录相同
    pub same_dir_as_source: bool,
}

/// 用于比较路径的键：Windows 下不区分大小写、统一分隔符。
pub fn path_key(path: &Path) -> String {
    let s = path.to_string_lossy().replace('/', "\\");
    let s = s.trim_end_matches('\\');
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s.replace('\\', "/")
    }
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

/// 应用“文件名去除”；结果为空时退回原名。
pub fn strip_name(stem: &str, strip: &NameStrip) -> Result<String, String> {
    let mut name = stem.to_owned();
    for line in strip.text.lines().filter(|l| !l.is_empty()) {
        if strip.regex {
            let re = Regex::new(line)
                .map_err(|e| format!("文件名去除的正则表达式无效（{line}）：{e}"))?;
            name = re.replace_all(&name, "").into_owned();
        } else {
            name = name.replace(line, "");
        }
    }
    let name = name.trim().trim_end_matches('.').to_owned();
    Ok(if name.is_empty() {
        stem.to_owned()
    } else {
        name
    })
}

/// 规划输出路径。
///
/// - `on_disk(path)`：文件是否已存在；
/// - `reserved(path)`：是否已被本批次其它任务占用（并行时避免同名互相覆盖）。
pub fn plan_output(
    req: &OutputRequest,
    on_disk: impl Fn(&Path) -> bool,
    reserved: impl Fn(&Path) -> bool,
) -> Result<OutputPlan, String> {
    let source_dir = req.input.parent().unwrap_or(Path::new(""));
    let base = req
        .output_dir
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(source_dir);

    let mut dir = base.to_path_buf();
    if req.settings.out_by_folder {
        if let Some(root) = req.source_root {
            dir.push(folder_relative(root, source_dir));
        }
    }

    let original_stem = req
        .input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| format!("无效的文件名：{}", req.input.display()))?;
    let mut stem = strip_name(&original_stem, &req.settings.name_strip)?;
    let fission = req.fission;
    // 裂变次数 >1 时总是输出到视频同名文件夹
    if req.settings.out_by_name || fission.is_fission() {
        dir.push(&stem);
    }
    let round_folders = fission.is_fission() && req.settings.fission_folder;
    if round_folders {
        dir.push(fission_folder_name(fission.round));
    }
    if req.settings.number_prefix {
        let width = req.total.max(1).to_string().len();
        stem = format!("{:0width$}_{stem}", req.number);
    }
    if fission.is_fission() && !round_folders {
        let width = fission.total.to_string().len();
        stem = format!("{stem}_{:0width$}", fission.round);
    }
    let ext = output_extension(req.settings, req.input);

    let same_dir_as_source = same_path(&dir, source_dir);
    let first = dir.join(format!("{stem}.{ext}"));
    // 永远不覆盖源文件本身，也不和本批次其它任务抢同一个名字
    let blocked = |p: &Path| same_path(p, req.input) || reserved(p);

    if !blocked(&first) {
        if !on_disk(&first) {
            return Ok(OutputPlan {
                path: first,
                overwrite: false,
                same_dir_as_source,
            });
        }
        if req.settings.on_exists == OnExists::Overwrite {
            return Ok(OutputPlan {
                path: first,
                overwrite: true,
                same_dir_as_source,
            });
        }
    }
    for n in 1..10_000 {
        let candidate = dir.join(format!("{stem} ({n}).{ext}"));
        if !blocked(&candidate) && !on_disk(&candidate) {
            return Ok(OutputPlan {
                path: candidate,
                overwrite: false,
                same_dir_as_source,
            });
        }
    }
    Err(format!("无法为 {stem} 找到可用的输出文件名"))
}

/// 文件夹导入时的相对目录：`<导入文件夹名>/<子目录…>`。
pub fn folder_relative(root: &Path, source_dir: &Path) -> PathBuf {
    let root_name = root
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "root".into());
    let mut rel = PathBuf::from(root_name);
    if let Ok(sub) = source_dir.strip_prefix(root) {
        rel.push(sub);
    }
    rel
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn plan(
        input: &str,
        out: Option<&str>,
        settings: &Settings,
        existing: &[&str],
        reserved: &[&str],
    ) -> Result<OutputPlan, String> {
        let existing: HashSet<String> = existing.iter().map(|p| path_key(Path::new(p))).collect();
        let reserved: HashSet<String> = reserved.iter().map(|p| path_key(Path::new(p))).collect();
        let out = out.map(Path::new);
        plan_output(
            &OutputRequest {
                input: Path::new(input),
                source_root: None,
                output_dir: out,
                settings,
                number: 3,
                total: 12,
                fission: FissionSlot::SINGLE,
            },
            |p| existing.contains(&path_key(p)),
            |p| reserved.contains(&path_key(p)),
        )
    }

    fn name(plan: &OutputPlan) -> String {
        plan.path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn basic_output_in_dir() {
        let p = plan("/src/a.mkv", Some("/out"), &Settings::default(), &[], &[]).unwrap();
        assert_eq!(p.path, Path::new("/out").join("a.mp4"));
        assert!(!p.overwrite);
        assert!(!p.same_dir_as_source);
    }

    #[test]
    fn rename_on_collision_and_reserved() {
        let s = Settings::default();
        let p = plan(
            "/src/a.mp4",
            Some("/out"),
            &s,
            &["/out/a.mp4", "/out/a (1).mp4"],
            &[],
        )
        .unwrap();
        assert_eq!(name(&p), "a (2).mp4");
        let p = plan("/src/a.mp4", Some("/out"), &s, &[], &["/out/a.mp4"]).unwrap();
        assert_eq!(name(&p), "a (1).mp4");
    }

    #[test]
    fn overwrite_mode_but_never_the_source_or_a_reserved_name() {
        let s = Settings {
            on_exists: OnExists::Overwrite,
            ..Settings::default()
        };
        let p = plan("/src/a.mp4", Some("/out"), &s, &["/out/a.mp4"], &[]).unwrap();
        assert_eq!(name(&p), "a.mp4");
        assert!(p.overwrite);
        // 输出到源目录且同名同扩展名：不能覆盖源文件
        let p = plan("/src/a.mp4", None, &s, &["/src/a.mp4"], &[]).unwrap();
        assert_eq!(name(&p), "a (1).mp4");
        assert!(p.same_dir_as_source);
        let p = plan("/src/a.mp4", Some("/out"), &s, &[], &["/out/a.mp4"]).unwrap();
        assert_eq!(name(&p), "a (1).mp4");
    }

    #[test]
    fn name_strip_prefix_and_folders() {
        let s = Settings {
            name_strip: NameStrip {
                text: "_raw\n【.*?】".into(),
                regex: true,
            },
            number_prefix: true,
            out_by_name: true,
            ..Settings::default()
        };
        let p = plan("/src/【广告】clip_raw.mp4", Some("/out"), &s, &[], &[]).unwrap();
        assert_eq!(p.path, Path::new("/out").join("clip").join("03_clip.mp4"));
    }

    #[test]
    fn out_by_folder_keeps_structure() {
        let s = Settings {
            out_by_folder: true,
            ..Settings::default()
        };
        let p = plan_output(
            &OutputRequest {
                input: Path::new("/videos/catA/sub/x.mp4"),
                source_root: Some(Path::new("/videos/catA")),
                output_dir: Some(Path::new("/out")),
                settings: &s,
                number: 1,
                total: 1,
                fission: FissionSlot::SINGLE,
            },
            |_| false,
            |_| false,
        )
        .unwrap();
        assert_eq!(
            p.path,
            Path::new("/out").join("catA").join("sub").join("x.mp4")
        );
    }

    /// 规划某个视频全部裂变轮次的输出（依次占用，模拟调度器）。
    fn plan_rounds(settings: &Settings, total_rounds: usize) -> Vec<PathBuf> {
        let mut taken: Vec<String> = Vec::new();
        (1..=total_rounds)
            .map(|round| {
                let plan = plan_output(
                    &OutputRequest {
                        input: Path::new("/src/clip.mp4"),
                        source_root: None,
                        output_dir: Some(Path::new("/out")),
                        settings,
                        number: 2,
                        total: 10,
                        fission: FissionSlot {
                            round,
                            total: total_rounds,
                        },
                    },
                    |_| false,
                    |p| taken.contains(&path_key(p)),
                )
                .unwrap();
                taken.push(path_key(&plan.path));
                plan.path
            })
            .collect()
    }

    #[test]
    fn fission_outputs_go_into_video_folder() {
        let paths = plan_rounds(&Settings::default(), 3);
        let dir = Path::new("/out").join("clip");
        assert_eq!(
            paths,
            [
                dir.join("clip_1.mp4"),
                dir.join("clip_2.mp4"),
                dir.join("clip_3.mp4")
            ]
        );
        // 已开启“输出到视频同名文件夹”时不会多套一层
        let by_name = Settings {
            out_by_name: true,
            number_prefix: true,
            ..Settings::default()
        };
        let paths = plan_rounds(&by_name, 12);
        assert_eq!(paths[0], dir.join("02_clip_01.mp4"));
        assert_eq!(paths[11], dir.join("02_clip_12.mp4"));
    }

    #[test]
    fn fission_folder_puts_each_round_in_its_own_folder() {
        let settings = Settings {
            fission_folder: true,
            ..Settings::default()
        };
        let paths = plan_rounds(&settings, 2);
        let dir = Path::new("/out").join("clip");
        assert_eq!(
            paths,
            [
                dir.join("第 1 次裂变").join("clip.mp4"),
                dir.join("第 2 次裂变").join("clip.mp4")
            ]
        );
        // 裂变次数为 1 时这些规则都不生效
        assert_eq!(
            plan_rounds(&settings, 1),
            [Path::new("/out").join("clip.mp4")]
        );
    }

    #[test]
    fn strip_rules() {
        let plain = NameStrip {
            text: "abc".into(),
            regex: false,
        };
        assert_eq!(strip_name("xabcy", &plain).unwrap(), "xy");
        assert_eq!(strip_name("abc", &plain).unwrap(), "abc", "去空后退回原名");
        let bad = NameStrip {
            text: "(".into(),
            regex: true,
        };
        assert!(strip_name("x", &bad).is_err());
    }
}
