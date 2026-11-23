//! `tool.subtitle-extract` 字幕提取：把 mkv / mp4 等容器里的内嵌字幕轨导出为独立文件。一进多出，一条命令。
//!
//! **参数**（括号内为默认值）
//! - `tracks` 导出哪些字幕轨（`all` 全部）：`all` 全部；`selected` 按 `selection` 选择。
//! - `selection` 选择条件（空）：逗号 / 空格分隔，数字是第几条字幕轨（从 1 起，只数字幕轨），字母是语言代码
//!   （如 `chi`、`eng`、`zh`、`en`，不区分大小写；常见的两字母与三字母代码互相匹配）。例：`1, eng`。
//! - `textFormat` 文本字幕的格式（`srt`）：`srt`、`ass`，或 `original` 保持原格式（subrip → srt、ass/ssa → ass、
//!   webvtt → vtt；mp4 的 mov_text 没有独立文件格式，转为 srt）。
//!
//! **图像字幕**（PGS / DVD / DVB 等）无法转成文字（需要 OCR），按原编码复制保存：PGS → `.sup`，
//! 其它 → `.mks`（只含字幕的 Matroska），并在处理记录里说明。
//!
//! **文件名**：`<视频名>_track<序号>_<语言>.<扩展名>`，序号是第几条字幕轨（从 1 起），没有语言标记时为 `und`，
//! 如 `movie_track1_chi.srt`。输出位置与命名规则同其它工具。
//!
//! 视频没有字幕轨，或没有符合选择条件的字幕轨时，跳过该视频（不算失败），处理记录里说明。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo, SubtitleStream};
use crate::tools::media::is_text_subtitle;
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.subtitle-extract";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackChoice {
    #[default]
    All,
    Selected,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextFormat {
    #[default]
    Srt,
    Ass,
    Original,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleExtractParams {
    pub tracks: TrackChoice,
    /// 选择条件：字幕轨序号（从 1 起）和 / 或语言代码
    pub selection: String,
    pub text_format: TextFormat,
}

/// 选择条件中的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    /// 第几条字幕轨（从 1 起）
    Track(usize),
    /// 语言代码（小写）
    Language(String),
}

/// 解析选择条件。
pub fn parse_selection(text: &str) -> Result<Vec<Selector>, String> {
    let mut selectors = Vec::new();
    for token in text
        .split(|c: char| c == ',' || c == '，' || c == ';' || c == '；' || c.is_whitespace())
        .filter(|t| !t.is_empty())
    {
        if token.chars().all(|c| c.is_ascii_digit()) {
            match token.parse::<usize>() {
                Ok(n) if n >= 1 => selectors.push(Selector::Track(n)),
                _ => return Err(format!("字幕轨序号从 1 开始：{token}")),
            }
        } else if (2..=8).contains(&token.len())
            && token.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
        {
            selectors.push(Selector::Language(token.to_ascii_lowercase()));
        } else {
            return Err(format!(
                "无法识别的选择条件“{token}”：请填写字幕轨序号（如 1）或语言代码（如 chi、eng）"
            ));
        }
    }
    if selectors.is_empty() {
        return Err("请填写要导出的字幕轨序号或语言".into());
    }
    Ok(selectors)
}

/// 常见语言的两字母 / 三字母代码（ISO 639-1 / 639-2 B / 639-2 T）。
const LANGUAGE_ALIASES: &[&[&str]] = &[
    &["zh", "chi", "zho", "chs", "cht", "cmn"],
    &["en", "eng"],
    &["ja", "jpn"],
    &["ko", "kor"],
    &["fr", "fre", "fra"],
    &["de", "ger", "deu"],
    &["es", "spa"],
    &["ru", "rus"],
    &["it", "ita"],
    &["pt", "por"],
];

fn language_matches(wanted: &str, track: Option<&str>) -> bool {
    let Some(track) = track.map(str::to_ascii_lowercase) else {
        return wanted == "und";
    };
    // `zh-Hans` 等带地区的代码只看主语言
    let primary = |s: &str| s.split('-').next().unwrap_or(s).to_owned();
    let (wanted, track) = (primary(wanted), primary(&track));
    wanted == track
        || LANGUAGE_ALIASES
            .iter()
            .any(|group| group.contains(&wanted.as_str()) && group.contains(&track.as_str()))
}

/// 一条要导出的字幕轨。
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedTrack {
    /// 第几条字幕轨（从 1 起）
    pub number: usize,
    /// 在文件中的流序号
    pub stream: u32,
    pub ext: &'static str,
    /// `-c:s` 的值
    pub codec: &'static str,
    /// `-f` 的值
    pub format: &'static str,
    pub suffix: String,
    /// 图像字幕（无法转为文字）
    pub image: bool,
}

/// 字幕轨 → 输出格式（扩展名、编码、封装）。
fn track_format(
    codec: &str,
    text_format: TextFormat,
) -> (&'static str, &'static str, &'static str, bool) {
    let codec = codec.to_ascii_lowercase();
    if !is_text_subtitle(&codec) {
        return match codec.as_str() {
            "hdmv_pgs_subtitle" => ("sup", "copy", "sup", true),
            _ => ("mks", "copy", "matroska", true),
        };
    }
    match text_format {
        TextFormat::Srt => ("srt", "srt", "srt", false),
        TextFormat::Ass => ("ass", "ass", "ass", false),
        TextFormat::Original => match codec.as_str() {
            "ass" | "ssa" => ("ass", "copy", "ass", false),
            "webvtt" => ("vtt", "copy", "webvtt", false),
            "subrip" => ("srt", "copy", "srt", false),
            _ => ("srt", "srt", "srt", false),
        },
    }
}

fn selected(selectors: &[Selector], number: usize, track: &SubtitleStream) -> bool {
    selectors.iter().any(|s| match s {
        Selector::Track(n) => *n == number,
        Selector::Language(lang) => language_matches(lang, track.language.as_deref()),
    })
}

fn language_tag(track: &SubtitleStream) -> String {
    let tag: String = track
        .language
        .as_deref()
        .unwrap_or("und")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if tag.is_empty() {
        "und".into()
    } else {
        tag.to_ascii_lowercase()
    }
}

/// 规划要导出的字幕轨。参数无效时返回错误；没有可导出的字幕轨时返回空列表。
pub fn plan_tracks(
    params: &SubtitleExtractParams,
    info: &MediaInfo,
) -> Result<Vec<PlannedTrack>, String> {
    let selectors = match params.tracks {
        TrackChoice::All => None,
        TrackChoice::Selected => Some(parse_selection(&params.selection)?),
    };
    Ok(info
        .subtitles
        .iter()
        .enumerate()
        .map(|(i, track)| (i + 1, track))
        .filter(|(number, track)| {
            selectors
                .as_deref()
                .map_or(true, |s| selected(s, *number, track))
        })
        .map(|(number, track)| {
            let (ext, codec, format, image) = track_format(&track.codec, params.text_format);
            PlannedTrack {
                number,
                stream: track.index,
                ext,
                codec,
                format,
                suffix: format!("_track{number}_{}", language_tag(track)),
                image,
            }
        })
        .collect())
}

/// 一条字幕轨输出的参数（输出路径之前）。
pub fn track_args(track: &PlannedTrack) -> Vec<String> {
    vec![
        "-map".into(),
        format!("0:{}", track.stream),
        "-c:s".into(),
        track.codec.into(),
        "-f".into(),
        track.format.into(),
    ]
}

impl Tool for SubtitleExtractParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        if self.tracks == TrackChoice::Selected {
            parse_selection(&self.selection)?;
        }
        let info = ctx.info(0)?;
        let input = ctx.input_path(0)?.to_path_buf();
        let label = ctx.item().label.clone();
        if info.subtitles.is_empty() {
            ctx.note(format!("没有内嵌字幕轨，已跳过：{label}"));
            ctx.skip_step();
            return Ok(());
        }
        let tracks = plan_tracks(self, &info)?;
        if tracks.is_empty() {
            ctx.note(format!(
                "共 {} 条字幕轨，没有符合选择条件的，已跳过：{label}",
                info.subtitles.len()
            ));
            ctx.skip_step();
            return Ok(());
        }
        let mut args = vec!["-i".to_owned(), path_arg(&input)?];
        for track in &tracks {
            if track.image {
                ctx.warn(format!(
                    "第 {} 条字幕轨是图像字幕（{}），无法转成文字，已按原格式保存为 .{}",
                    track.number,
                    info.subtitles[track.number - 1].codec,
                    track.ext
                ));
            }
            let path = ctx.output(&OutputSpec::file(track.ext).suffix(track.suffix.clone()))?;
            args.extend(track_args(track));
            args.push(path_arg(&path)?);
        }
        ctx.run_ffmpeg("字幕提取", args, info.duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::sample_info;

    fn sub(index: u32, codec: &str, language: Option<&str>) -> SubtitleStream {
        SubtitleStream {
            index,
            codec: codec.into(),
            language: language.map(str::to_owned),
            title: None,
        }
    }

    fn info() -> MediaInfo {
        let mut info = sample_info();
        info.subtitles = vec![
            sub(2, "subrip", Some("chi")),
            sub(3, "ass", Some("eng")),
            sub(4, "hdmv_pgs_subtitle", None),
            sub(5, "dvd_subtitle", Some("jpn")),
            sub(6, "mov_text", Some("zho")),
        ];
        info
    }

    fn summary(params: &SubtitleExtractParams) -> Vec<String> {
        plan_tracks(params, &info())
            .unwrap()
            .iter()
            .map(|t| format!("{}.{} {}", t.suffix, t.ext, track_args(t).join(" ")))
            .collect()
    }

    #[test]
    fn all_tracks_to_srt_and_images_kept_native() {
        assert_eq!(
            summary(&SubtitleExtractParams::default()),
            [
                "_track1_chi.srt -map 0:2 -c:s srt -f srt",
                "_track2_eng.srt -map 0:3 -c:s srt -f srt",
                "_track3_und.sup -map 0:4 -c:s copy -f sup",
                "_track4_jpn.mks -map 0:5 -c:s copy -f matroska",
                "_track5_zho.srt -map 0:6 -c:s srt -f srt",
            ]
        );
        let images: Vec<bool> = plan_tracks(&SubtitleExtractParams::default(), &info())
            .unwrap()
            .iter()
            .map(|t| t.image)
            .collect();
        assert_eq!(images, [false, false, true, true, false]);
    }

    #[test]
    fn ass_and_original_formats() {
        let ass = SubtitleExtractParams {
            text_format: TextFormat::Ass,
            ..SubtitleExtractParams::default()
        };
        assert_eq!(summary(&ass)[0], "_track1_chi.ass -map 0:2 -c:s ass -f ass");
        let original = SubtitleExtractParams {
            text_format: TextFormat::Original,
            ..SubtitleExtractParams::default()
        };
        let out = summary(&original);
        assert_eq!(out[0], "_track1_chi.srt -map 0:2 -c:s copy -f srt");
        assert_eq!(out[1], "_track2_eng.ass -map 0:3 -c:s copy -f ass");
        assert_eq!(
            out[4], "_track5_zho.srt -map 0:6 -c:s srt -f srt",
            "mov_text 转 srt"
        );
    }

    #[test]
    fn selection_by_number_and_language() {
        let pick = |selection: &str| -> Vec<usize> {
            let params = SubtitleExtractParams {
                tracks: TrackChoice::Selected,
                selection: selection.into(),
                ..SubtitleExtractParams::default()
            };
            plan_tracks(&params, &info())
                .unwrap()
                .iter()
                .map(|t| t.number)
                .collect()
        };
        assert_eq!(pick("2"), [2]);
        assert_eq!(pick("1, eng"), [1, 2]);
        assert_eq!(pick("zh"), [1, 5], "zh 匹配 chi 与 zho");
        assert_eq!(pick("ENG；4"), [2, 4]);
        assert_eq!(pick("und"), [3], "und 匹配没有语言标记的字幕轨");
        assert_eq!(pick("zh-Hans"), [1, 5]);
        assert_eq!(pick("9 kor"), Vec::<usize>::new(), "没有匹配时为空（跳过）");
        assert_eq!(
            parse_selection("1，chi").unwrap(),
            [Selector::Track(1), Selector::Language("chi".into())]
        );
    }

    #[test]
    fn invalid_selection_is_rejected() {
        for bad in ["", "  ", "0", "a", "chi!", "中文", "toolonglanguage"] {
            let params = SubtitleExtractParams {
                tracks: TrackChoice::Selected,
                selection: bad.into(),
                ..SubtitleExtractParams::default()
            };
            assert!(plan_tracks(&params, &info()).is_err(), "{bad:?}");
        }
        // 全部导出时忽略选择条件
        let all = SubtitleExtractParams {
            selection: "!!".into(),
            ..SubtitleExtractParams::default()
        };
        assert_eq!(plan_tracks(&all, &info()).unwrap().len(), 5);
    }

    #[test]
    fn no_subtitles_means_nothing_to_do() {
        assert!(
            plan_tracks(&SubtitleExtractParams::default(), &sample_info())
                .unwrap()
                .is_empty()
        );
    }
}
