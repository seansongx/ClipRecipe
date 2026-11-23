//! 工具输出的命名：与批量处理相同的规则（输出目录 / 源目录、按来源文件夹分目录、输出到视频同名文件夹、
//! 文件名去除、队列序号、目标已存在时重命名或覆盖、永不覆盖输入文件），再加上工具自己的后缀与扩展名。
//!
//! 例：`clip.mkv` + `OutputSpec::file("mp3").suffix("_audio")` → `<输出目录>/clip_audio.mp3`；
//! `OutputSpec::file("mp4").suffix("_01").in_stem_folder()` → `<输出目录>/clip/clip_01.mp4`。

use std::path::{Path, PathBuf};

use crate::jobs::output::{folder_relative, path_key, same_path, strip_name, OutputPlan};
use crate::model::{OnExists, Settings};

/// 一个输出（文件或文件夹）的命名方式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputSpec {
    /// 扩展名（不带点）；文件夹输出时忽略
    pub ext: String,
    /// 加在文件名后的后缀，如 `_audio`、`_03`
    pub suffix: String,
    /// 代替输入文件名的名字（仍会应用文件名去除与队列序号）；None 时用输入文件名
    pub stem: Option<String>,
    /// 放进以（去除后的）输入文件名命名的子文件夹（一进多出时常用）
    pub stem_folder: bool,
    /// 以第几个输入为准（决定默认名字、输出目录）
    pub input: usize,
    /// 输出是一个文件夹（如 HLS 切片）；文件夹已存在时总是改名，不会覆盖
    pub folder: bool,
}

impl OutputSpec {
    pub fn file(ext: &str) -> Self {
        Self {
            ext: ext.trim_start_matches('.').to_ascii_lowercase(),
            suffix: String::new(),
            stem: None,
            stem_folder: false,
            input: 0,
            folder: false,
        }
    }

    pub fn folder() -> Self {
        Self {
            folder: true,
            ..Self::file("")
        }
    }

    pub fn suffix(mut self, suffix: impl Into<String>) -> Self {
        self.suffix = sanitize_name(&suffix.into());
        self
    }

    pub fn stem(mut self, stem: impl Into<String>) -> Self {
        self.stem = Some(sanitize_name(&stem.into()));
        self
    }

    pub fn in_stem_folder(mut self) -> Self {
        self.stem_folder = true;
        self
    }

    pub fn from_input(mut self, index: usize) -> Self {
        self.input = index;
        self
    }
}

/// 把文件名中 Windows 不允许的字符换成 `_`。
pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// 规划一个工具输出所需的信息。
pub struct ToolOutputRequest<'a> {
    pub input: &'a Path,
    pub source_root: Option<&'a Path>,
    /// None 表示输出到输入文件所在目录
    pub output_dir: Option<&'a Path>,
    pub settings: &'a Settings,
    /// 序号（从 1 起）与总数（决定补零位数）
    pub number: usize,
    pub total: usize,
    pub spec: &'a OutputSpec,
    /// 不能被覆盖的文件（工作项的全部输入）
    pub protected: &'a [PathBuf],
}

/// 输出名去除后的主干（不含扩展名）。
pub fn output_stem(input: &Path, settings: &Settings) -> Result<String, String> {
    let original = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| format!("无效的文件名：{}", input.display()))?;
    strip_name(&original, &settings.name_strip)
}

/// 规划输出路径。`on_disk`：是否已存在；`reserved`：是否已被本次运行的其它输出占用。
pub fn plan_tool_output(
    req: &ToolOutputRequest,
    on_disk: impl Fn(&Path) -> bool,
    reserved: impl Fn(&Path) -> bool,
) -> Result<OutputPlan, String> {
    let spec = req.spec;
    let settings = req.settings;
    let source_dir = req.input.parent().unwrap_or(Path::new(""));
    let base = req
        .output_dir
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(source_dir);
    let mut dir = base.to_path_buf();
    if settings.out_by_folder {
        if let Some(root) = req.source_root {
            dir.push(folder_relative(root, source_dir));
        }
    }
    let input_stem = output_stem(req.input, settings)?;
    if settings.out_by_name || spec.stem_folder {
        dir.push(&input_stem);
    }
    let mut stem = match &spec.stem {
        Some(stem) if !stem.trim().is_empty() => strip_name(stem.trim(), &settings.name_strip)?,
        _ => input_stem,
    };
    if settings.number_prefix {
        let width = req.total.max(1).to_string().len();
        stem = format!("{:0width$}_{stem}", req.number);
    }
    let name = format!("{stem}{}", spec.suffix);
    let file_name = |n: Option<usize>| {
        let base = match n {
            Some(n) => format!("{name} ({n})"),
            None => name.clone(),
        };
        if spec.folder || spec.ext.is_empty() {
            base
        } else {
            format!("{base}.{}", spec.ext)
        }
    };

    let same_dir_as_source = same_path(&dir, source_dir);
    let blocked = |p: &Path| {
        same_path(p, req.input) || req.protected.iter().any(|x| same_path(p, x)) || reserved(p)
    };
    let first = dir.join(file_name(None));
    if !blocked(&first) {
        if !on_disk(&first) {
            return Ok(OutputPlan {
                path: first,
                overwrite: false,
                same_dir_as_source,
            });
        }
        if settings.on_exists == OnExists::Overwrite && !spec.folder {
            return Ok(OutputPlan {
                path: first,
                overwrite: true,
                same_dir_as_source,
            });
        }
    }
    for n in 1..10_000 {
        let candidate = dir.join(file_name(Some(n)));
        if !blocked(&candidate) && !on_disk(&candidate) {
            return Ok(OutputPlan {
                path: candidate,
                overwrite: false,
                same_dir_as_source,
            });
        }
    }
    Err(format!("无法为 {name} 找到可用的输出文件名"))
}

/// 写入时使用的临时名（同目录、以点开头），成功后改名为最终名。
/// 文件保留扩展名（某些 muxer 按扩展名选择格式），文件夹直接加后缀。
pub fn part_path(final_path: &Path, unique: u64, folder: bool) -> PathBuf {
    let dir = final_path.parent().unwrap_or(Path::new(""));
    let name = final_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    match final_path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
    {
        Some(ext) if !folder => {
            let stem = name.strip_suffix(&format!(".{ext}")).unwrap_or(&name);
            dir.join(format!(".{stem}.cliprecipe-tool-{unique}.part.{ext}"))
        }
        _ => dir.join(format!(".{name}.cliprecipe-tool-{unique}.part")),
    }
}

/// 与 [`path_key`] 相同，便于工具代码比较路径。
pub fn key(path: &Path) -> String {
    path_key(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NameStrip;
    use std::collections::HashSet;

    fn plan(
        spec: &OutputSpec,
        settings: &Settings,
        existing: &[&str],
        reserved: &[&str],
    ) -> PathBuf {
        let existing: HashSet<String> = existing.iter().map(|p| key(Path::new(p))).collect();
        let reserved: HashSet<String> = reserved.iter().map(|p| key(Path::new(p))).collect();
        plan_tool_output(
            &ToolOutputRequest {
                input: Path::new("/src/clip.mkv"),
                source_root: None,
                output_dir: Some(Path::new("/out")),
                settings,
                number: 3,
                total: 12,
                spec,
                protected: &[],
            },
            |p| existing.contains(&key(p)),
            |p| reserved.contains(&key(p)),
        )
        .unwrap()
        .path
    }

    #[test]
    fn suffix_extension_and_stem_folder() {
        let s = Settings::default();
        let out = Path::new("/out");
        assert_eq!(
            plan(&OutputSpec::file("mp3").suffix("_audio"), &s, &[], &[]),
            out.join("clip_audio.mp3")
        );
        assert_eq!(
            plan(
                &OutputSpec::file(".MP4").suffix("_01").in_stem_folder(),
                &s,
                &[],
                &[]
            ),
            out.join("clip").join("clip_01.mp4")
        );
        assert_eq!(
            plan(&OutputSpec::folder().suffix("_hls"), &s, &[], &[]),
            out.join("clip_hls")
        );
        assert_eq!(
            plan(&OutputSpec::file("mp4").stem("a b|c"), &s, &[], &[]),
            out.join("a b_c.mp4")
        );
    }

    #[test]
    fn collisions_rename_or_overwrite_but_folders_never_overwrite() {
        let s = Settings::default();
        let spec = OutputSpec::file("mp4");
        assert_eq!(
            plan(&spec, &s, &["/out/clip.mp4"], &["/out/clip (1).mp4"]),
            Path::new("/out").join("clip (2).mp4")
        );
        let over = Settings {
            on_exists: OnExists::Overwrite,
            ..Settings::default()
        };
        assert_eq!(
            plan(&spec, &over, &["/out/clip.mp4"], &[]),
            Path::new("/out").join("clip.mp4")
        );
        assert_eq!(
            plan(&OutputSpec::folder(), &over, &["/out/clip"], &[]),
            Path::new("/out").join("clip (1)")
        );
    }

    #[test]
    fn settings_rules_apply_and_inputs_are_protected() {
        let s = Settings {
            number_prefix: true,
            out_by_name: true,
            name_strip: NameStrip {
                text: "cl".into(),
                regex: false,
            },
            ..Settings::default()
        };
        // out_by_name 与 stem_folder 只套一层
        assert_eq!(
            plan(
                &OutputSpec::file("mp4").suffix("_1").in_stem_folder(),
                &s,
                &[],
                &[]
            ),
            Path::new("/out").join("ip").join("03_ip_1.mp4")
        );
        let protected = [PathBuf::from("/src/other.mp4")];
        let p = plan_tool_output(
            &ToolOutputRequest {
                input: Path::new("/src/clip.mp4"),
                source_root: None,
                output_dir: None,
                settings: &Settings {
                    on_exists: OnExists::Overwrite,
                    ..Settings::default()
                },
                number: 1,
                total: 1,
                spec: &OutputSpec::file("mp4").stem("other"),
                protected: &protected,
            },
            |p| key(p) == key(Path::new("/src/other.mp4")),
            |_| false,
        )
        .unwrap();
        assert_eq!(p.path, Path::new("/src").join("other (1).mp4"));
        assert!(p.same_dir_as_source);
    }

    #[test]
    fn part_paths_are_hidden_siblings() {
        assert_eq!(
            part_path(Path::new("/out/a.b.mp4"), 7, false),
            Path::new("/out").join(".a.b.cliprecipe-tool-7.part.mp4")
        );
        assert_eq!(
            part_path(Path::new("/out/clip.v2"), 2, true),
            Path::new("/out").join(".clip.v2.cliprecipe-tool-2.part")
        );
    }
}
