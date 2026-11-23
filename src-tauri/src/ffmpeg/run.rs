//! 运行 ffmpeg：解析 `-progress pipe:1` 输出、支持取消、保留 stderr 末尾用于报错。

use std::collections::VecDeque;
use std::fmt;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::command;

/// 保留的 stderr 行数。
const STDERR_TAIL: usize = 30;

/// 可在线程间共享的取消标记。
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// 一次进度汇报（对应 `-progress` 的一个块）。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// 已输出的时长（秒）
    pub out_time: f64,
    pub frame: u64,
    pub fps: f64,
    /// 处理速度倍率，如 2.5 表示 2.5x
    pub speed: f64,
    /// 已写入字节数
    pub total_size: u64,
    /// `progress=end`
    pub finished: bool,
}

/// 逐行解析 `-progress` 输出。
#[derive(Debug, Default)]
pub struct ProgressParser {
    current: Progress,
}

impl ProgressParser {
    /// 喂入一行；遇到 `progress=continue|end` 时返回完整的一块。
    pub fn feed(&mut self, line: &str) -> Option<Progress> {
        let (key, value) = line.trim().split_once('=')?;
        let value = value.trim();
        match key {
            "frame" => self.current.frame = value.parse().unwrap_or(self.current.frame),
            "fps" => self.current.fps = value.parse().unwrap_or(self.current.fps),
            "total_size" => {
                self.current.total_size = value.parse().unwrap_or(self.current.total_size)
            }
            // out_time_ms 实际也是微秒（FFmpeg 的历史遗留）
            "out_time_us" | "out_time_ms" => {
                if let Ok(us) = value.parse::<i64>() {
                    self.current.out_time = us.max(0) as f64 / 1_000_000.0;
                }
            }
            "out_time" => {
                if let Some(secs) = parse_clock(value) {
                    self.current.out_time = secs;
                }
            }
            "speed" => {
                if let Ok(speed) = value.trim_end_matches('x').trim().parse() {
                    self.current.speed = speed;
                }
            }
            "progress" => {
                self.current.finished = value == "end";
                return Some(self.current.clone());
            }
            _ => {}
        }
        None
    }
}

/// 解析 `HH:MM:SS.ffffff`。
pub fn parse_clock(s: &str) -> Option<f64> {
    let mut parts = s.trim().rsplitn(3, ':');
    let secs: f64 = parts.next()?.parse().ok()?;
    let mins: f64 = parts.next().map_or(Some(0.0), |m| m.parse().ok())?;
    let hours: f64 = parts
        .next()
        .map_or(Some(0.0), |h| h.trim_start_matches('-').parse().ok())?;
    Some((hours * 3600.0 + mins * 60.0 + secs).max(0.0))
}

/// 默认的卡死判定：这么久输出没有任何进展（已输出时长、帧数、字节数都不变）就结束 ffmpeg。
pub const DEFAULT_STALL_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, PartialEq)]
pub enum RunError {
    Cancelled,
    Spawn(String),
    Failed {
        code: Option<i32>,
        /// stderr 的最后若干行
        stderr_tail: Vec<String>,
    },
    /// 长时间没有任何进展，已被强制结束（可能卡死）
    Stalled {
        seconds: u64,
        stderr_tail: Vec<String>,
    },
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Cancelled => write!(f, "已取消"),
            RunError::Spawn(e) => write!(f, "无法启动 ffmpeg：{e}"),
            RunError::Failed { code, stderr_tail } => {
                let code = code.map_or("无".to_owned(), |c| c.to_string());
                let reason = stderr_tail
                    .iter()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .map(String::as_str)
                    .unwrap_or("没有错误输出");
                write!(f, "ffmpeg 处理失败（退出码 {code}）：{reason}")
            }
            RunError::Stalled { seconds, .. } => write!(
                f,
                "ffmpeg 已有 {seconds} 秒没有任何进展，判定为卡死并已终止（可尝试调整参数后重试）"
            ),
        }
    }
}

impl std::error::Error for RunError {}

/// 运行 ffmpeg 直到结束。
///
/// `args` 不含程序名；调用方应自行加上 `-progress pipe:1 -nostats`（[`crate::pipeline`] 生成的命令已包含）。
/// 取消时会结束子进程并返回 [`RunError::Cancelled`]。成功时返回 stderr 末尾（通常是警告）。
///
/// 超过 [`DEFAULT_STALL_TIMEOUT`] 没有任何进展时结束子进程并返回 [`RunError::Stalled`]。
pub fn run(
    ffmpeg: &Path,
    args: &[String],
    cancel: &CancelToken,
    on_progress: impl FnMut(&Progress),
) -> Result<Vec<String>, RunError> {
    run_with_stall_timeout(ffmpeg, args, cancel, DEFAULT_STALL_TIMEOUT, on_progress)
}

/// 同 [`run`]，可指定卡死判定时间：从启动起、或从上一次进展起，超过 `stall` 没有进展
/// （`-progress` 报告的已输出时长、帧数、字节数都不变）就结束子进程。
pub fn run_with_stall_timeout(
    ffmpeg: &Path,
    args: &[String],
    cancel: &CancelToken,
    stall: Duration,
    on_progress: impl FnMut(&Progress),
) -> Result<Vec<String>, RunError> {
    run_with_stderr(ffmpeg, args, cancel, stall, on_progress, |_| {})
}

/// 同 [`run_with_stall_timeout`]，并把 stderr 的**每一行**按顺序交给 `on_stderr`（在调用线程上调用），
/// 用于解析分析类滤镜（`metadata=mode=print`、`silencedetect` 等）写进日志的结果。
pub fn run_with_stderr(
    ffmpeg: &Path,
    args: &[String],
    cancel: &CancelToken,
    stall: Duration,
    mut on_progress: impl FnMut(&Progress),
    mut on_stderr: impl FnMut(&str),
) -> Result<Vec<String>, RunError> {
    if cancel.is_cancelled() {
        return Err(RunError::Cancelled);
    }
    let mut child = command(ffmpeg)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| RunError::Spawn(format!("{}：{e}", ffmpeg.display())))?;

    let tail: Arc<Mutex<VecDeque<String>>> = Arc::default();
    let stderr = child.stderr.take().expect("stderr 已设为 piped");
    let stderr_tail = Arc::clone(&tail);
    let (line_tx, line_rx) = mpsc::channel::<String>();
    let stderr_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            {
                let mut tail = stderr_tail.lock().unwrap_or_else(|e| e.into_inner());
                if tail.len() == STDERR_TAIL {
                    tail.pop_front();
                }
                tail.push_back(line.clone());
            }
            // 接收端在结束后才会被丢弃，发送失败可以忽略
            let _ = line_tx.send(line);
        }
    });

    let stdout = child.stdout.take().expect("stdout 已设为 piped");
    let (tx, rx) = mpsc::channel();
    let stdout_thread = std::thread::spawn(move || {
        let mut parser = ProgressParser::default();
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(progress) = parser.feed(&line) {
                if tx.send(progress).is_err() {
                    break;
                }
            }
        }
    });

    let mut cancelled = false;
    let mut stalled = false;
    let mut last_change = Instant::now();
    let mut last_key: Option<(i64, u64, u64)> = None;
    loop {
        for line in line_rx.try_iter() {
            on_stderr(&line);
        }
        if cancel.is_cancelled() {
            cancelled = true;
            let _ = child.kill();
            break;
        }
        if last_change.elapsed() >= stall {
            stalled = true;
            let _ = child.kill();
            break;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(progress) => {
                let key = (
                    (progress.out_time * 1000.0) as i64,
                    progress.frame,
                    progress.total_size,
                );
                if last_key != Some(key) {
                    last_key = Some(key);
                    last_change = Instant::now();
                }
                on_progress(&progress);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait();
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    for line in line_rx.try_iter() {
        on_stderr(&line);
    }
    let tail: Vec<String> = tail
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect();

    if cancelled {
        return Err(RunError::Cancelled);
    }
    if stalled {
        return Err(RunError::Stalled {
            seconds: stall.as_secs().max(1),
            stderr_tail: tail,
        });
    }
    match status {
        Ok(status) if status.success() => Ok(tail),
        Ok(status) => Err(RunError::Failed {
            code: status.code(),
            stderr_tail: tail,
        }),
        Err(e) => Err(RunError::Spawn(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_blocks() {
        let text = "frame=10\nfps=25.00\nbitrate=N/A\ntotal_size=1024\nout_time_us=400000\n\
                    out_time_ms=400000\nout_time=00:00:00.400000\nspeed=1.5x\nprogress=continue\n\
                    frame=25\nout_time_us=N/A\nout_time=00:00:01.000000\nspeed=N/A\nprogress=end\n";
        let mut parser = ProgressParser::default();
        let blocks: Vec<Progress> = text.lines().filter_map(|l| parser.feed(l)).collect();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].frame, 10);
        assert_eq!(blocks[0].out_time, 0.4);
        assert_eq!(blocks[0].speed, 1.5);
        assert_eq!(blocks[0].total_size, 1024);
        assert!(!blocks[0].finished);
        assert_eq!(blocks[1].frame, 25);
        assert_eq!(blocks[1].out_time, 1.0);
        assert!(blocks[1].finished);
    }

    #[test]
    fn clock_parsing() {
        assert_eq!(parse_clock("01:02:03.5"), Some(3723.5));
        assert_eq!(parse_clock("00:00:00.000000"), Some(0.0));
        assert_eq!(parse_clock("garbage"), None);
    }

    #[test]
    fn failed_error_message_uses_last_stderr_line() {
        let err = RunError::Failed {
            code: Some(1),
            stderr_tail: vec!["first".into(), "No such file".into(), "".into()],
        };
        assert_eq!(err.to_string(), "ffmpeg 处理失败（退出码 1）：No such file");
    }
}
