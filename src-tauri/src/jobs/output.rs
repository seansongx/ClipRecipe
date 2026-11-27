//! 输出路径解析：输出目录、按来源文件夹 / 视频名分目录、文件名去除、输出文件名格式（`set.name-template`）、
//! 队列序号、裂变（`pipe.fission`）、目标已存在时重命名或覆盖。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::model::{NameStrip, OnExists, Settings};
use crate::pipeline::output_extension;

/// 本批开始处理的本地时间，用于输出文件名格式的 `{date}` / `{time}`；同一批所有任务相同。
/// 由前端在点“开始”时给出（浏览器的本地时间，各平台一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl BatchTime {
    /// `YYYYMMDD`
    pub fn date(&self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }

    /// `HHMMSS`
    pub fn time(&self) -> String {
        format!("{:02}{:02}{:02}", self.hour, self.minute, self.second)
    }

    /// Unix 时间戳（秒）对应的 UTC 时间。
    pub fn from_unix_utc(secs: u64) -> Self {
        let days = (secs / 86_400) as i64;
        let rem = secs % 86_400;
        // Howard Hinnant 的 civil_from_days
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self {
            year: year.clamp(0, 9999) as u16,
            month: month as u8,
            day: day as u8,
            hour: (rem / 3_600) as u8,
            minute: (rem % 3_600 / 60) as u8,
            second: (rem % 60) as u8,
        }
    }

    /// 调用方没有给出时间时的后备：当前 UTC 时间（前端开始处理与生成命令时总会给出本地时间）。
    pub fn now_utc() -> Self {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self::from_unix_utc(secs)
    }
}

/// 输出文件名格式里的变量取值。
pub struct NameVars<'a> {
    /// 去除后的源文件名（不含扩展名）
    pub name: &'a str,
    /// 已补零的队列序号
    pub index: &'a str,
    pub date: &'a str,
    pub time: &'a str,
    /// 源文件所在文件夹的名字
    pub folder: &'a str,
}

/// 按输出文件名格式（`set.name-template`）生成文件名主体：替换 `{name}` `{index}` `{date}` `{time}` `{folder}`
/// （区分大小写），其它文字与不认识的 `{…}` 原样保留；非法文件名字符换成 `_`，去掉首尾空格与结尾的点；
/// 格式为空或结果为空时退回 `{name}`。
pub fn render_name(template: &str, vars: &NameVars) -> String {
    let mut out = String::with_capacity(template.len() + vars.name.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after.find('}').and_then(|close| {
            let value = match &after[..close] {
                "name" => vars.name,
                "index" => vars.index,
                "date" => vars.date,
                "time" => vars.time,
                "folder" => vars.folder,
                _ => return None,
            };
            Some((value, close))
        });
        match value {
            Some((value, close)) => {
                out.push_str(value);
                rest = &after[close + 1..];
            }
            // 不认识的变量（或没有闭合的花括号）：只保留这个 `{`，后面的文字继续匹配
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    let cleaned: String = out
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned
        .trim()
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    if cleaned.is_empty() {
        vars.name.to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// 队列序号补零的位数：按本批总数。
fn index_width(total: usize) -> usize {
    total.max(1).to_string().len()
}

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
    /// 本批开始处理的时间（输出文件名格式的 `{date}` / `{time}`）
    pub started: BatchTime,
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
    let stripped = strip_name(&original_stem, &req.settings.name_strip)?;
    let fission = req.fission;
    // 裂变次数 >1 时总是输出到视频同名文件夹；文件夹名用去除后的源文件名，不套输出文件名格式
    // （否则带日期时间的格式每批都会新建文件夹）
    if req.settings.out_by_name || fission.is_fission() {
        dir.push(&stripped);
    }
    let round_folders = fission.is_fission() && req.settings.fission_folder;
    if round_folders {
        dir.push(fission_folder_name(fission.round));
    }
    let width = index_width(req.total);
    let index = format!("{:0width$}", req.number);
    let folder = source_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut stem = render_name(
        &req.settings.name_template,
        &NameVars {
            name: &stripped,
            index: &index,
            date: &req.started.date(),
            time: &req.started.time(),
            folder: &folder,
        },
    );
    if req.settings.number_prefix {
        stem = format!("{index}_{stem}");
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

    /// 固定的“本批开始时间”：2026-11-26 09:05:07
    const STARTED: BatchTime = BatchTime {
        year: 2026,
        month: 11,
        day: 26,
        hour: 9,
        minute: 5,
        second: 7,
    };

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
                started: STARTED,
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
                started: STARTED,
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
                        started: STARTED,
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

    fn with_template(template: &str) -> Settings {
        Settings {
            name_template: template.into(),
            ..Settings::default()
        }
    }

    #[test]
    fn default_name_template_keeps_the_source_name() {
        assert_eq!(Settings::default().name_template, "{name}");
        for s in [Settings::default(), with_template("")] {
            let p = plan("/src/a.mkv", Some("/out"), &s, &[], &[]).unwrap();
            assert_eq!(p.path, Path::new("/out").join("a.mp4"));
        }
    }

    #[test]
    fn name_template_variables() {
        let s = with_template("{index}-{date}-{time}-{folder}-{name}");
        let p = plan("/videos/旅行/clip.mov", Some("/out"), &s, &[], &[]).unwrap();
        assert_eq!(name(&p), "03-20261126-090507-旅行-clip.mp4");
        // 每个变量单独使用
        for (template, expected) in [
            ("{name}", "clip.mp4"),
            ("{index}", "03.mp4"),
            ("{date}", "20261126.mp4"),
            ("{time}", "090507.mp4"),
            ("{folder}", "旅行.mp4"),
            ("前缀 {name} 后缀", "前缀 clip 后缀.mp4"),
        ] {
            let p = plan(
                "/videos/旅行/clip.mov",
                Some("/out"),
                &with_template(template),
                &[],
                &[],
            )
            .unwrap();
            assert_eq!(name(&p), expected, "{template}");
        }
    }

    #[test]
    fn unknown_variables_and_case_are_kept_as_text() {
        for (template, expected) in [
            ("{Name}_{foo}_{name}", "{Name}_{foo}_clip.mp4"),
            ("{{name}}", "{clip}.mp4"),
            ("{name}{", "clip{.mp4"),
            ("{}{name}", "{}clip.mp4"),
        ] {
            let p = plan(
                "/src/clip.mp4",
                Some("/out"),
                &with_template(template),
                &[],
                &[],
            )
            .unwrap();
            assert_eq!(name(&p), expected, "{template}");
        }
    }

    #[test]
    fn name_template_illegal_chars_and_empty_results() {
        let p = plan(
            "/src/clip.mp4",
            Some("/out"),
            &with_template("a:b*c?\"<>|/\\\t{name}"),
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(name(&p), "a_b_c________clip.mp4");
        let p = plan(
            "/src/clip.mp4",
            Some("/out"),
            &with_template("  {name}. . "),
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(name(&p), "clip.mp4");
        // 结果为空时退回去除后的源文件名
        for template in ["   ", "...", " . "] {
            let p = plan(
                "/src/clip.mp4",
                Some("/out"),
                &with_template(template),
                &[],
                &[],
            )
            .unwrap();
            assert_eq!(name(&p), "clip.mp4", "{template:?}");
        }
    }

    #[test]
    fn name_template_with_strip_prefix_and_video_folder() {
        let s = Settings {
            name_strip: NameStrip {
                text: "_raw\n【.*?】".into(),
                regex: true,
            },
            number_prefix: true,
            out_by_name: true,
            name_template: "{date}_{name}".into(),
            ..Settings::default()
        };
        let p = plan("/src/【广告】clip_raw.mp4", Some("/out"), &s, &[], &[]).unwrap();
        // 视频同名文件夹仍是去除后的源文件名；序号前缀加在格式结果之前
        assert_eq!(
            p.path,
            Path::new("/out").join("clip").join("03_20261126_clip.mp4")
        );
    }

    #[test]
    fn name_template_with_fission() {
        let s = with_template("x_{name}");
        let paths = plan_rounds(&s, 3);
        let dir = Path::new("/out").join("clip");
        assert_eq!(
            paths,
            [
                dir.join("x_clip_1.mp4"),
                dir.join("x_clip_2.mp4"),
                dir.join("x_clip_3.mp4")
            ]
        );
        let folders = Settings {
            fission_folder: true,
            ..with_template("{date}")
        };
        assert_eq!(
            plan_rounds(&folders, 2),
            [
                dir.join("第 1 次裂变").join("20261126.mp4"),
                dir.join("第 2 次裂变").join("20261126.mp4")
            ]
        );
    }

    /// 同一批的多个视频套同一个不含 `{name}` 的格式：靠现有的重名规则各得其名，即使是覆盖模式也不互相覆盖。
    #[test]
    fn same_template_for_a_whole_batch_does_not_collide() {
        for on_exists in [OnExists::Rename, OnExists::Overwrite] {
            let s = Settings {
                on_exists,
                ..with_template("成片_{date}")
            };
            let mut taken: Vec<String> = Vec::new();
            let names: Vec<String> = ["/src/a.mp4", "/src/b.mov", "/other/c.mkv"]
                .iter()
                .enumerate()
                .map(|(i, input)| {
                    let plan = plan_output(
                        &OutputRequest {
                            input: Path::new(input),
                            source_root: None,
                            output_dir: Some(Path::new("/out")),
                            settings: &s,
                            number: i + 1,
                            total: 3,
                            fission: FissionSlot::SINGLE,
                            started: STARTED,
                        },
                        |p| path_key(p) == path_key(&Path::new("/out").join("成片_20261126.mp4")),
                        |p| taken.contains(&path_key(p)),
                    )
                    .unwrap();
                    taken.push(path_key(&plan.path));
                    name(&plan)
                })
                .collect();
            let expected = match on_exists {
                // 磁盘上已有同名文件：重命名模式从 (1) 开始
                OnExists::Rename => [
                    "成片_20261126 (1).mp4",
                    "成片_20261126 (2).mp4",
                    "成片_20261126 (3).mp4",
                ],
                // 覆盖模式只有第一个覆盖已有文件，其余的不抢本批已占用的名字
                OnExists::Overwrite => [
                    "成片_20261126.mp4",
                    "成片_20261126 (1).mp4",
                    "成片_20261126 (2).mp4",
                ],
            };
            assert_eq!(names, expected, "{on_exists:?}");
        }
    }

    #[test]
    fn batch_time_formatting() {
        assert_eq!(STARTED.date(), "20261126");
        assert_eq!(STARTED.time(), "090507");
        let epoch = BatchTime::from_unix_utc(0);
        assert_eq!(
            (epoch.date(), epoch.time()),
            ("19700101".into(), "000000".into())
        );
        let leap = BatchTime::from_unix_utc(1_709_210_096);
        assert_eq!(
            (leap.date(), leap.time()),
            ("20240229".into(), "123456".into())
        );
        let march = BatchTime::from_unix_utc(951_868_800);
        assert_eq!(march.date(), "20000301");
    }
}
