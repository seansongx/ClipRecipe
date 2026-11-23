//! `tool.subtitle-asr` 语音生成字幕：用本机的 whisper.cpp 语音识别为每个视频生成 SRT 字幕，可选再烧录进新视频。
//! **不使用任何云服务**，程序也不会自己下载任何东西：模型文件（ggml 格式，如 `ggml-base.bin`）由用户从
//! whisper.cpp 官方的 Hugging Face 仓库（ggerganov/whisper.cpp）下载后选择。
//!
//! **识别引擎** `engine`（`auto`）：
//! - `builtin` FFmpeg 内置的 `whisper` 滤镜（FFmpeg 8.0 起，需编译时启用 whisper.cpp；本机 9.1 full 构建可用）：
//!   `-af whisper=model=…:language=…:queue=…:use_gpu=…:destination=临时.srt:format=srt`；
//! - `cli` 外部 whisper.cpp 命令行程序（`whisper-cli`，路径 `cliPath`）：先用 FFmpeg 把第一条音轨转成
//!   16 kHz 单声道 WAV，再运行 `whisper-cli -m 模型 -f 音频.wav -l 语言 -osrt -of 临时名`；
//! - `auto` 当前 FFmpeg 有 whisper 滤镜时用内置，否则用 whisper-cli（此时必须填写 `cliPath`）。
//!
//! **参数**（括号内为默认值）
//! - `modelPath`（空，必填）ggml 模型文件，检查存在且文件头为 ggml 标记（`lmgg`）。
//! - `language`（`auto`）识别语言：`auto` 自动识别，或语言代码（`zh`、`en`、`ja`…，2～3 个小写字母）。
//! - `queueSeconds`（10，1～60）内置滤镜每次送去识别的音频长度：越长上下文越完整、时间轴越粗。
//! - `useGpu`（false）内置滤镜使用 GPU（FFmpeg 构建不支持时 whisper.cpp 自动用 CPU）。
//! - `vadModelPath`（空）可选的 VAD（语音活动检测）模型，只用于内置滤镜；`vadThreshold`（0.5，0～1）。
//! - `maxLength`（0，0～200）每条字幕最多字符数（0 不限制），内置滤镜为 `max_len`，whisper-cli 为 `-ml`。
//! - `srtLocation`（`output`）SRT 放在：`output` 输出位置（与其它工具相同的命名规则，`<视频名>.srt`）；
//!   `source` 源视频旁（`<源文件名>.srt`，已存在时按“目标已存在时”覆盖或加 ` (n)`；不出现在完成输出列表里，
//!   只在处理记录里显示路径，且只在全部步骤成功后才写入）。
//! - `burn`（false）另外把字幕烧录进新视频 `<视频名>_subtitled.<扩展名>`（视频重编码，音频能复制就复制）；样式：
//!   `fontSize` 字号（画面高度的 5%，1～20）、`fontColor`（#ffffff）、`outlineColor`（#000000）、
//!   `outlineWidth` 描边（2，0～10）、`marginBottom` 距底部（画面高度的 5%，0～40）。样式通过 libass 的
//!   `force_style` 设置（以 288 行高的脚本坐标换算）；Windows 上字体用“Microsoft YaHei”以显示中文。
//!
//! whisper 输出的 SRT 序号从 0 开始、文字前带空格，写出前统一整理：序号从 1 起、去掉首尾空白与空条目。
//! 视频没有音轨时失败；没有识别到任何语音时仍输出空字幕并提示（不烧录）。

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{self, path_arg, MediaInfo};
use crate::model::OnExists;
use crate::pipeline::{escape_filter_path, fmt_num};
use crate::tools::subtitle_erase::audio_args;
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.subtitle-asr";

/// 模型下载地址（只在界面上作为文字显示，程序不会访问）。
pub const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AsrEngine {
    #[default]
    Auto,
    Builtin,
    Cli,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SrtLocation {
    #[default]
    Output,
    Source,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleAsrParams {
    pub engine: AsrEngine,
    pub model_path: String,
    pub cli_path: String,
    pub language: String,
    pub queue_seconds: f64,
    pub use_gpu: bool,
    pub vad_model_path: String,
    pub vad_threshold: f64,
    pub max_length: u32,
    pub srt_location: SrtLocation,
    pub burn: bool,
    /// 字号（画面高度的百分比）
    pub font_size: f64,
    pub font_color: String,
    pub outline_color: String,
    pub outline_width: f64,
    /// 距底部（画面高度的百分比）
    pub margin_bottom: f64,
}

impl Default for SubtitleAsrParams {
    fn default() -> Self {
        Self {
            engine: AsrEngine::Auto,
            model_path: String::new(),
            cli_path: String::new(),
            language: "auto".into(),
            queue_seconds: 10.0,
            use_gpu: false,
            vad_model_path: String::new(),
            vad_threshold: 0.5,
            max_length: 0,
            srt_location: SrtLocation::Output,
            burn: false,
            font_size: 5.0,
            font_color: "#ffffff".into(),
            outline_color: "#000000".into(),
            outline_width: 2.0,
            margin_bottom: 5.0,
        }
    }
}

/// 实际使用的引擎。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Builtin,
    Cli,
}

fn check_range(value: f64, (min, max): (f64, f64), what: &str) -> Result<(), String> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!("{what}必须在 {min}～{max} 之间"))
    }
}

/// `#rrggbb` → ASS 颜色 `&H00BBGGRR`。
pub fn ass_color(text: &str, what: &str) -> Result<String, String> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("{what}无效：{text}"));
    }
    let hex = hex.to_ascii_uppercase();
    Ok(format!("&H00{}{}{}", &hex[4..6], &hex[2..4], &hex[0..2]))
}

/// 校验语言代码：`auto` 或 2～3 个小写字母。
pub fn check_language(language: &str) -> Result<String, String> {
    let lang = language.trim().to_ascii_lowercase();
    if lang == "auto"
        || ((2..=3).contains(&lang.len()) && lang.chars().all(|c| c.is_ascii_lowercase()))
    {
        Ok(lang)
    } else {
        Err(format!(
            "识别语言无效：“{language}”，请填写 auto 或语言代码（如 zh、en、ja）"
        ))
    }
}

/// 检查 ggml 模型文件的文件头。
pub fn check_model_header(header: &[u8]) -> Result<(), String> {
    if header.starts_with(b"lmgg") {
        Ok(())
    } else {
        Err("所选文件不是 whisper.cpp 的 ggml 模型（应为 ggml-*.bin），请从 whisper.cpp 的 Hugging Face 仓库下载".into())
    }
}

/// 检查模型文件：已填写、存在、文件头正确。
pub fn check_model(path: &str) -> Result<PathBuf, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("请选择 whisper 模型文件（ggml-*.bin）".into());
    }
    let file = PathBuf::from(path);
    if !file.is_file() {
        return Err(format!("模型文件不存在：{path}"));
    }
    let mut header = [0u8; 4];
    std::fs::File::open(&file)
        .and_then(|mut f| f.read_exact(&mut header))
        .map_err(|e| format!("无法读取模型文件 {path}：{e}"))?;
    check_model_header(&header)?;
    Ok(file)
}

/// 检查 whisper-cli 可执行文件。
pub fn check_cli(path: &str) -> Result<PathBuf, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("当前 FFmpeg 没有 whisper 滤镜，请选择 whisper.cpp 的 whisper-cli 程序".into());
    }
    let file = PathBuf::from(path);
    if !file.is_file() {
        return Err(format!("whisper-cli 程序不存在：{path}"));
    }
    Ok(file)
}

/// `ffmpeg -h filter=whisper` 的输出里是否有该滤镜。
pub fn help_has_whisper(help: &str) -> bool {
    help.contains("whisper AVOptions") || help.contains("Filter whisper")
}

impl SubtitleAsrParams {
    pub fn validate(&self) -> Result<(), String> {
        check_language(&self.language)?;
        check_range(self.queue_seconds, (1.0, 60.0), "识别片段长度（秒）")?;
        check_range(self.vad_threshold, (0.0, 1.0), "VAD 阈值")?;
        if self.max_length > 200 {
            return Err("每条字幕最多字符数必须在 0～200 之间".into());
        }
        if self.model_path.trim().is_empty() {
            return Err("请选择 whisper 模型文件（ggml-*.bin）".into());
        }
        if self.burn {
            check_range(self.font_size, (1.0, 20.0), "字号（%）")?;
            check_range(self.outline_width, (0.0, 10.0), "描边粗细")?;
            check_range(self.margin_bottom, (0.0, 40.0), "距底部（%）")?;
            ass_color(&self.font_color, "文字颜色")?;
            ass_color(&self.outline_color, "描边颜色")?;
        }
        Ok(())
    }

    /// 按设置与滤镜是否可用选择引擎。
    pub fn pick_engine(&self, has_filter: bool) -> Result<Engine, String> {
        match self.engine {
            AsrEngine::Builtin if has_filter => Ok(Engine::Builtin),
            AsrEngine::Builtin => Err(
                "当前 FFmpeg 没有 whisper 滤镜（需要 FFmpeg 8.0 以上且编译时启用 whisper.cpp），请改用 whisper-cli".into(),
            ),
            AsrEngine::Cli => Ok(Engine::Cli),
            AsrEngine::Auto if has_filter => Ok(Engine::Builtin),
            AsrEngine::Auto => Ok(Engine::Cli),
        }
    }
}

/// 内置 whisper 滤镜的完整参数（不含开头的通用参数）。
pub fn builtin_args(
    params: &SubtitleAsrParams,
    input: &str,
    model: &str,
    destination: &str,
) -> Result<Vec<String>, String> {
    let language = check_language(&params.language)?;
    let mut filter = format!(
        "whisper=model={}:language={language}:queue={}:use_gpu={}:destination={}:format=srt",
        escape_filter_path(model),
        fmt_num(params.queue_seconds),
        params.use_gpu,
        escape_filter_path(destination)
    );
    if params.max_length > 0 {
        filter.push_str(&format!(":max_len={}", params.max_length));
    }
    let vad = params.vad_model_path.trim();
    if !vad.is_empty() {
        filter.push_str(&format!(
            ":vad_model={}:vad_threshold={}",
            escape_filter_path(vad),
            fmt_num(params.vad_threshold)
        ));
    }
    Ok(vec![
        "-i".into(),
        input.into(),
        "-map".into(),
        "0:a:0".into(),
        "-vn".into(),
        "-sn".into(),
        "-dn".into(),
        "-af".into(),
        filter,
        "-f".into(),
        "null".into(),
        "-".into(),
    ])
}

/// whisper-cli 需要的 16 kHz 单声道 WAV。
pub fn wav_args(input: &str, wav: &str) -> Vec<String> {
    [
        "-i",
        input,
        "-map",
        "0:a:0",
        "-vn",
        "-sn",
        "-dn",
        "-ac",
        "1",
        "-ar",
        "16000",
        "-c:a",
        "pcm_s16le",
        wav,
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// whisper-cli 的参数：输出 `<out_base>.srt`。
pub fn cli_args(
    params: &SubtitleAsrParams,
    model: &str,
    wav: &str,
    out_base: &str,
) -> Result<Vec<String>, String> {
    let language = check_language(&params.language)?;
    let mut args: Vec<String> = [
        "-m", model, "-f", wav, "-l", &language, "-osrt", "-of", out_base,
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    if params.max_length > 0 {
        args.extend(["-ml".into(), params.max_length.to_string()]);
    }
    if !params.use_gpu {
        args.push("-ng".into());
    }
    Ok(args)
}

/// 整理 whisper 输出的 SRT：序号从 1 起、去掉文字首尾空白与空条目，统一换行。返回文本与条数。
pub fn normalize_srt(text: &str) -> (String, usize) {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut out = String::new();
    let mut count = 0;
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let Some(timing) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let body: Vec<&str> = lines[timing + 1..].to_vec();
        if body.is_empty() {
            continue;
        }
        count += 1;
        out.push_str(&format!(
            "{count}\n{}\n{}\n\n",
            lines[timing],
            body.join("\n")
        ));
    }
    (out, count)
}

/// 烧录字幕的 libass `force_style`（脚本坐标：行高 288）。
pub fn force_style(params: &SubtitleAsrParams, windows: bool) -> Result<String, String> {
    let units = |percent: f64| fmt_num((288.0 * percent / 100.0 * 10.0).round() / 10.0);
    let mut parts = Vec::new();
    if windows {
        parts.push("FontName=Microsoft YaHei".to_owned());
    }
    parts.extend([
        format!("FontSize={}", units(params.font_size)),
        format!(
            "PrimaryColour={}",
            ass_color(&params.font_color, "文字颜色")?
        ),
        format!(
            "OutlineColour={}",
            ass_color(&params.outline_color, "描边颜色")?
        ),
        "BorderStyle=1".into(),
        format!("Outline={}", fmt_num(params.outline_width)),
        "Shadow=0".into(),
        "Alignment=2".into(),
        format!("MarginV={}", units(params.margin_bottom)),
    ]);
    Ok(parts.join(","))
}

/// 烧录用的视频滤镜。
pub fn burn_filter(params: &SubtitleAsrParams, srt: &str, windows: bool) -> Result<String, String> {
    Ok(format!(
        "subtitles={}:force_style='{}'",
        escape_filter_path(srt),
        force_style(params, windows)?
    ))
}

/// 源视频旁的字幕路径：`<源文件名>.srt`；已存在时覆盖或加 ` (n)`。
pub fn beside_source(
    source: &Path,
    on_exists: OnExists,
    exists: impl Fn(&Path) -> bool,
) -> Result<PathBuf, String> {
    let stem = source
        .file_stem()
        .ok_or_else(|| format!("无效的文件名：{}", source.display()))?
        .to_string_lossy()
        .into_owned();
    let dir = source.parent().unwrap_or(Path::new(""));
    let first = dir.join(format!("{stem}.srt"));
    if on_exists == OnExists::Overwrite || !exists(&first) {
        return Ok(first);
    }
    (1..10_000)
        .map(|n| dir.join(format!("{stem} ({n}).srt")))
        .find(|p| !exists(p))
        .ok_or_else(|| "无法为字幕文件找到可用的名字".to_owned())
}

fn ffmpeg_has_whisper(ffmpeg_path: &Path) -> bool {
    ffmpeg::command(ffmpeg_path)
        .args(["-hide_banner", "-h", "filter=whisper"])
        .output()
        .map(|o| help_has_whisper(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or(false)
}

/// 运行 whisper-cli，响应取消；失败时返回输出末尾。
fn run_cli(ctx: &mut ToolContext, exe: &Path, args: &[String]) -> Result<(), ToolError> {
    ctx.check_cancelled()?;
    let log = ctx.temp_file("whisper-cli.log")?;
    let log_file = std::fs::File::create(&log).map_err(|e| format!("无法创建日志文件：{e}"))?;
    let log_err = log_file
        .try_clone()
        .map_err(|e| format!("无法创建日志文件：{e}"))?;
    ctx.note(format!("whisper-cli 识别中：{}", ctx.item().label));
    let mut child = ffmpeg::command(exe)
        .args(args)
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err))
        .spawn()
        .map_err(|e| format!("无法启动 whisper-cli（{}）：{e}", exe.display()))?;
    let status = loop {
        if ctx.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ToolError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(format!("whisper-cli 运行出错：{e}").into()),
        }
    };
    if !status.success() {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let tail: Vec<&str> = text.lines().rev().take(8).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!("whisper-cli 识别失败（{status}）：\n{}", tail.join("\n")).into());
    }
    ctx.skip_step();
    Ok(())
}

/// 识别一个视频，返回 whisper 原始 SRT 文本。
fn transcribe(
    params: &SubtitleAsrParams,
    ctx: &mut ToolContext,
    engine: Engine,
    info: &MediaInfo,
    input: &str,
    model: &Path,
) -> Result<String, ToolError> {
    let raw = ctx.temp_file("whisper.srt")?;
    match engine {
        Engine::Builtin => {
            let args = builtin_args(params, input, &path_arg(model)?, &path_arg(&raw)?)?;
            ctx.run_ffmpeg("语音识别（whisper）", args, info.duration)?;
            Ok(std::fs::read_to_string(&raw).unwrap_or_default())
        }
        Engine::Cli => {
            let exe = check_cli(&params.cli_path)?;
            let wav = ctx.temp_file("audio.wav")?;
            ctx.run_ffmpeg("提取音频", wav_args(input, &path_arg(&wav)?), info.duration)?;
            let base = ctx.temp_file("whisper-cli")?;
            let args = cli_args(
                params,
                &path_arg(model)?,
                &path_arg(&wav)?,
                &path_arg(&base)?,
            )?;
            run_cli(ctx, &exe, &args)?;
            let mut produced = base.into_os_string();
            produced.push(".srt");
            std::fs::read_to_string(PathBuf::from(produced))
                .map_err(|e| format!("whisper-cli 没有生成字幕文件：{e}").into())
        }
    }
}

impl Tool for SubtitleAsrParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let model = check_model(&self.model_path)?;
        let vad = self.vad_model_path.trim();
        if !vad.is_empty() && !Path::new(vad).is_file() {
            return Err(format!("VAD 模型文件不存在：{vad}").into());
        }
        let info = ctx.info(0)?;
        if info.audio.is_empty() {
            return Err("视频没有音轨，无法识别语音".into());
        }
        let has_filter = match self.engine {
            AsrEngine::Cli => false,
            _ => ffmpeg_has_whisper(&ctx.paths().ffmpeg),
        };
        let engine = self.pick_engine(has_filter)?;
        if engine == Engine::Cli {
            check_cli(&self.cli_path)?;
            if !vad.is_empty() {
                ctx.warn("VAD 模型只用于 FFmpeg 内置 whisper 滤镜，whisper-cli 下忽略");
            }
        }
        let mut steps = match engine {
            Engine::Builtin => vec![3.0],
            Engine::Cli => vec![1.0, 3.0],
        };
        if self.burn {
            steps.push(2.0);
        }
        ctx.set_steps(&steps);

        let input_path = ctx.input_path(0)?.to_path_buf();
        let input = path_arg(&input_path)?;
        let raw = transcribe(self, ctx, engine, &info, &input, &model)?;
        let (srt, count) = normalize_srt(&raw);
        if count == 0 {
            ctx.warn("没有识别到语音，输出的字幕为空");
        } else {
            ctx.note(format!("识别到 {count} 条字幕：{}", ctx.item().label));
        }

        if self.srt_location == SrtLocation::Output {
            let out = ctx.output(&OutputSpec::file("srt"))?;
            std::fs::write(&out, &srt).map_err(|e| format!("无法写入字幕文件：{e}"))?;
        }

        if self.burn {
            if count == 0 {
                ctx.skip_step();
            } else {
                let subs = ctx.temp_file("burn.srt")?;
                std::fs::write(&subs, &srt).map_err(|e| format!("无法写入字幕文件：{e}"))?;
                let filter = burn_filter(self, &path_arg(&subs)?, cfg!(windows))?;
                let ext = ctx.video_extension(0)?;
                let output = path_arg(&ctx.output(&OutputSpec::file(&ext).suffix("_subtitled"))?)?;
                let audio = audio_args(&info, &ext);
                ctx.run_encode("烧录字幕", info.duration, |enc| {
                    let mut args = vec![
                        "-i".to_owned(),
                        input.clone(),
                        "-map".into(),
                        "0:V:0".into(),
                        "-vf".into(),
                        filter.clone(),
                    ];
                    args.extend(enc.video_args(&ext));
                    args.extend(audio.iter().cloned());
                    args.extend(["-sn".into(), "-dn".into()]);
                    if matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
                        args.extend(["-movflags".into(), "+faststart".into()]);
                    }
                    args.push(output.clone());
                    Ok(args)
                })?;
            }
        }

        if self.srt_location == SrtLocation::Source {
            ctx.check_cancelled()?;
            let on_exists = ctx.settings().on_exists;
            let mut path = beside_source(&input_path, on_exists, Path::exists)?;
            let mut file = loop {
                let opened = if on_exists == OnExists::Overwrite {
                    std::fs::File::create(&path)
                } else {
                    OpenOptions::new().write(true).create_new(true).open(&path)
                };
                match opened {
                    Ok(file) => break file,
                    // 并行时另一个工作项刚好占用了同一个名字：换下一个
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        path = beside_source(&input_path, on_exists, |p| {
                            p == path.as_path() || p.exists()
                        })?;
                    }
                    Err(e) => {
                        return Err(format!("无法写入字幕文件 {}：{e}", path.display()).into())
                    }
                }
            };
            file.write_all(srt.as_bytes())
                .map_err(|e| format!("无法写入字幕文件 {}：{e}", path.display()))?;
            ctx.note(format!("字幕已保存到源视频旁：{}", path.display()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_model() -> SubtitleAsrParams {
        SubtitleAsrParams {
            model_path: "m.bin".into(),
            ..SubtitleAsrParams::default()
        }
    }

    #[test]
    fn builtin_filter_arguments() {
        let args = builtin_args(
            &with_model(),
            "in.mp4",
            r"C:\models\ggml-tiny.bin",
            r"C:\t\o.srt",
        )
        .unwrap();
        assert_eq!(
            args.join(" "),
            r"-i in.mp4 -map 0:a:0 -vn -sn -dn -af whisper=model=C\\:/models/ggml-tiny.bin:language=auto:queue=10:use_gpu=false:destination=C\\:/t/o.srt:format=srt -f null -"
        );
        let full = SubtitleAsrParams {
            language: "ZH".into(),
            queue_seconds: 4.5,
            use_gpu: true,
            max_length: 20,
            vad_model_path: "vad.bin".into(),
            vad_threshold: 0.6,
            ..with_model()
        };
        let args = builtin_args(&full, "a.mkv", "m.bin", "o.srt").unwrap();
        assert_eq!(
            args[8],
            "whisper=model=m.bin:language=zh:queue=4.5:use_gpu=true:destination=o.srt:format=srt:max_len=20:vad_model=vad.bin:vad_threshold=0.6"
        );
    }

    #[test]
    fn cli_arguments() {
        assert_eq!(
            wav_args("in.mp4", "a.wav").join(" "),
            "-i in.mp4 -map 0:a:0 -vn -sn -dn -ac 1 -ar 16000 -c:a pcm_s16le a.wav"
        );
        assert_eq!(
            cli_args(&with_model(), "m.bin", "a.wav", "out")
                .unwrap()
                .join(" "),
            "-m m.bin -f a.wav -l auto -osrt -of out -ng"
        );
        let gpu = SubtitleAsrParams {
            language: "en".into(),
            max_length: 30,
            use_gpu: true,
            ..with_model()
        };
        assert_eq!(
            cli_args(&gpu, "m.bin", "a.wav", "out").unwrap().join(" "),
            "-m m.bin -f a.wav -l en -osrt -of out -ml 30"
        );
    }

    #[test]
    fn engine_selection() {
        let p = with_model();
        assert_eq!(p.pick_engine(true), Ok(Engine::Builtin));
        assert_eq!(p.pick_engine(false), Ok(Engine::Cli));
        let builtin = SubtitleAsrParams {
            engine: AsrEngine::Builtin,
            ..with_model()
        };
        assert!(builtin
            .pick_engine(false)
            .unwrap_err()
            .contains("whisper 滤镜"));
        let cli = SubtitleAsrParams {
            engine: AsrEngine::Cli,
            ..with_model()
        };
        assert_eq!(cli.pick_engine(true), Ok(Engine::Cli));
        assert!(help_has_whisper(
            "Filter whisper\n  Transcribe audio\nwhisper AVOptions:"
        ));
        assert!(!help_has_whisper("Unknown filter 'whisper'."));
    }

    #[test]
    fn validation_and_paths() {
        assert!(SubtitleAsrParams::default()
            .validate()
            .unwrap_err()
            .contains("模型"));
        assert!(with_model().validate().is_ok());
        for bad in ["", "chinese", "z", "zh-CN", "12"] {
            assert!(check_language(bad).is_err(), "{bad:?}");
        }
        assert_eq!(check_language(" EN ").unwrap(), "en");
        let bad = [
            SubtitleAsrParams {
                queue_seconds: 0.5,
                ..with_model()
            },
            SubtitleAsrParams {
                vad_threshold: 1.5,
                ..with_model()
            },
            SubtitleAsrParams {
                max_length: 201,
                ..with_model()
            },
            SubtitleAsrParams {
                burn: true,
                font_color: "white".into(),
                ..with_model()
            },
            SubtitleAsrParams {
                burn: true,
                font_size: 30.0,
                ..with_model()
            },
        ];
        for p in bad {
            assert!(p.validate().is_err(), "{p:?}");
        }
        assert!(check_model_header(b"lmgg\x01\x00").is_ok());
        assert!(check_model_header(b"GGUF").unwrap_err().contains("ggml"));
        assert!(check_model_header(b"").is_err());
        assert!(check_model("").unwrap_err().contains("请选择"));
        assert!(check_model("Z:/no/such/model.bin")
            .unwrap_err()
            .contains("不存在"));
        assert!(check_cli("").is_err());
        assert!(check_cli("Z:/no/whisper-cli.exe")
            .unwrap_err()
            .contains("不存在"));
    }

    #[test]
    fn srt_is_renumbered_and_trimmed() {
        let raw = "0\n00:00:00,000 --> 00:00:03,380\n Hello world, this is a test.\n\n1\n00:00:03,380 --> 00:00:05,000\n\n\n2\r\n00:00:05,000 --> 00:00:06,000\r\n 你好 \r\n 世界\r\n";
        let (srt, count) = normalize_srt(raw);
        assert_eq!(count, 2);
        assert_eq!(
            srt,
            "1\n00:00:00,000 --> 00:00:03,380\nHello world, this is a test.\n\n2\n00:00:05,000 --> 00:00:06,000\n你好\n世界\n\n"
        );
        assert_eq!(normalize_srt(""), (String::new(), 0));
    }

    #[test]
    fn burn_style() {
        let p = SubtitleAsrParams {
            font_color: "#ffcc00".into(),
            ..with_model()
        };
        assert_eq!(
            force_style(&p, false).unwrap(),
            "FontSize=14.4,PrimaryColour=&H0000CCFF,OutlineColour=&H00000000,BorderStyle=1,Outline=2,Shadow=0,Alignment=2,MarginV=14.4"
        );
        assert!(force_style(&p, true)
            .unwrap()
            .starts_with("FontName=Microsoft YaHei,"));
        assert_eq!(
            burn_filter(&p, r"C:\t\s.srt", false).unwrap(),
            r"subtitles=C\\:/t/s.srt:force_style='FontSize=14.4,PrimaryColour=&H0000CCFF,OutlineColour=&H00000000,BorderStyle=1,Outline=2,Shadow=0,Alignment=2,MarginV=14.4'"
        );
    }

    #[test]
    fn srt_beside_source_never_clobbers_by_default() {
        let src = Path::new("d/clip.mp4");
        assert_eq!(
            beside_source(src, OnExists::Rename, |_| false).unwrap(),
            Path::new("d/clip.srt")
        );
        let taken = |p: &Path| p == Path::new("d/clip.srt") || p == Path::new("d/clip (1).srt");
        assert_eq!(
            beside_source(src, OnExists::Rename, taken).unwrap(),
            Path::new("d/clip (2).srt")
        );
        assert_eq!(
            beside_source(src, OnExists::Overwrite, taken).unwrap(),
            Path::new("d/clip.srt")
        );
    }
}
