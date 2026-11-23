//! `tool.change-duration` 改变视频时长：只改写容器里**报告的总时长**，音视频数据与实际播放内容完全不变
//! （逐字节复制源文件，不重编码、也不重新封装）。
//!
//! **参数**（括号内为默认值）
//! - `mode`（`multiplier`）：`multiplier` 实际时长的倍数 `multiplier`（范围值，默认固定 2，0.01～100）；
//!   `absolute` 指定时长 `seconds`（范围值，秒，默认固定 600 = 10:00，0.1～360000）。
//!   范围值按工作项的种子取值（同一种子、同一序号结果相同），保留 3 位小数。
//!
//! **支持的容器**：MP4 / MOV 系（ffprobe 格式名含 `mov` 或 `mp4`：mp4、m4v、mov、3gp）。
//! 其它容器（mkv、webm、avi、flv、ts…）失败并说明原因：Matroska 的时长是 Info 里的浮点元素，
//! 改写需要完整的 EBML 解析且很多播放器按 Cues / 实际数据计算时长；avi / flv / ts 没有可靠的总时长字段。
//! 分片 MP4（moov 里有 `mvex`）也拒绝：FFmpeg 等读取这类文件时忽略 mvhd 的时长。
//!
//! **改了什么**：只改 `moov/mvhd` 的 duration（影片总时长，以 mvhd 的 timescale 为单位），支持 32 位
//! （version 0）与 64 位（version 1）两种 mvhd，`moov` 在文件开头或结尾都可以。刻意**不改**：
//! - `tkhd` duration（每条轨道的时长）与编辑列表 `elst`：QuickTime / Safari 等按轨道与编辑列表决定实际呈现，
//!   改短会把内容截掉、改长会在结尾补空白或黑屏，等于改变了播放内容；
//! - `mdhd` duration（媒体时长）：解码器按它与采样表计算时间戳，改动会造成音画错位或解码出错。
//!
//! **各类读取方的表现（实测 / 已知）**：
//! - **FFmpeg 9.1 / ffprobe 不看 mvhd**：容器时长与流时长都按采样表（stts）与编辑列表计算，仍显示真实时长。
//!   实测把 mvhd、tkhd、mdhd、elst 改长都不改变 ffprobe 的结果；只有改采样时间表（让最后一帧持续更久）
//!   才会改变，那等于改变了播放内容（定格最后一帧），本工具不这样做。所以用 FFmpeg 解析的网站不受影响。
//! - 读取影片头的程序（Windows 资源管理器的“时长”列、MediaInfo 的 General 时长、QuickTime 的影片属性等）
//!   显示目标时长；播放时多数播放器按轨道数据播完真实内容（改长时进度条可能提前结束，改短时仍播完全部内容）。
//!
//! **流程**：把源文件复制为输出（临时名）→ 在**输出副本**上定位并改写 mvhd（源文件只读不写）→ 重新解析输出确认
//! mvhd 已是目标值、ffprobe 能读取且流数量不变 → 完整解码一遍（`-xerror`）确认文件仍然可用；任一步失败都删除输出。

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::model::Range;
use crate::pipeline::fmt_num;
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.change-duration";

/// 指定时长的上限（秒）。
pub const MAX_SECONDS: f64 = 360_000.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DurationMode {
    #[default]
    Multiplier,
    Absolute,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChangeDurationParams {
    pub mode: DurationMode,
    /// 实际时长的倍数
    pub multiplier: Range<f64>,
    /// 指定时长（秒）
    pub seconds: Range<f64>,
}

impl Default for ChangeDurationParams {
    fn default() -> Self {
        Self {
            mode: DurationMode::Multiplier,
            multiplier: Range::fixed(2.0),
            seconds: Range::fixed(600.0),
        }
    }
}

fn check_range(range: &Range<f64>, (lo, hi): (f64, f64), what: &str) -> Result<(), String> {
    let (min, max) = range.ordered();
    if min.is_finite() && max.is_finite() && min >= lo && max <= hi {
        Ok(())
    } else {
        Err(format!("{what}必须在 {lo}～{hi} 之间"))
    }
}

impl ChangeDurationParams {
    pub fn validate(&self) -> Result<(), String> {
        match self.mode {
            DurationMode::Multiplier => check_range(&self.multiplier, (0.01, 100.0), "倍数"),
            DurationMode::Absolute => {
                check_range(&self.seconds, (0.1, MAX_SECONDS), "目标时长（秒）")
            }
        }
    }

    /// 目标时长（秒）：按种子取值。
    pub fn target(&self, real: f64, rng: &mut crate::model::SeededRng) -> Result<f64, String> {
        self.validate()?;
        let target = match self.mode {
            DurationMode::Multiplier => {
                if !(real.is_finite() && real > 0.0) {
                    return Err("读不到视频的实际时长，无法按倍数计算".into());
                }
                real * self.multiplier.sample_rounded(rng, 3)
            }
            DurationMode::Absolute => self.seconds.sample_rounded(rng, 3),
        };
        Ok((target * 1000.0).round() / 1000.0)
    }
}

/// 是否为 MP4 / MOV 系容器（ffprobe 的格式名）。
pub fn is_mp4_family(format_name: &str) -> bool {
    format_name
        .split(',')
        .any(|f| matches!(f.trim(), "mov" | "mp4" | "m4a" | "3gp" | "3g2" | "mj2"))
}

/// 一个 ISO BMFF 盒子的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxHeader {
    pub kind: [u8; 4],
    pub start: u64,
    pub header_len: u64,
    pub end: u64,
}

/// mvhd 中与时长有关的字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mvhd {
    pub version: u8,
    pub timescale: u32,
    pub duration: u64,
    /// duration 字段在文件中的偏移
    pub duration_offset: u64,
}

fn io_err(e: std::io::Error) -> String {
    format!("读取文件出错：{e}")
}

fn read_exact_at<R: Read + Seek>(r: &mut R, pos: u64, buf: &mut [u8]) -> Result<(), String> {
    r.seek(SeekFrom::Start(pos)).map_err(io_err)?;
    r.read_exact(buf).map_err(io_err)
}

/// 读 `pos` 处的盒子头；盒子必须完整位于 `limit` 之内。
pub fn read_box_header<R: Read + Seek>(
    r: &mut R,
    pos: u64,
    limit: u64,
) -> Result<BoxHeader, String> {
    let bad = |why: &str| format!("文件结构无效（偏移 {pos}）：{why}");
    if limit.saturating_sub(pos) < 8 {
        return Err(bad("盒子头不完整"));
    }
    let mut head = [0u8; 8];
    read_exact_at(r, pos, &mut head)?;
    let size32 = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
    let kind = [head[4], head[5], head[6], head[7]];
    let (size, header_len) = match size32 {
        0 => (limit - pos, 8),
        1 => {
            if limit - pos < 16 {
                return Err(bad("64 位盒子头不完整"));
            }
            let mut large = [0u8; 8];
            read_exact_at(r, pos + 8, &mut large)?;
            (u64::from_be_bytes(large), 16)
        }
        n => (u64::from(n), 8),
    };
    if size < header_len {
        return Err(bad("盒子大小小于盒子头"));
    }
    let end = pos.checked_add(size).ok_or_else(|| bad("盒子大小溢出"))?;
    if end > limit {
        return Err(bad("盒子超出所在范围"));
    }
    Ok(BoxHeader {
        kind,
        start: pos,
        header_len,
        end,
    })
}

/// 列出 `[start, end)` 范围内的盒子。
fn boxes_in<R: Read + Seek>(r: &mut R, start: u64, end: u64) -> Result<Vec<BoxHeader>, String> {
    let mut boxes = Vec::new();
    let mut pos = start;
    while pos < end {
        if end - pos < 8 {
            // 末尾少于一个盒子头的填充字节，忽略
            break;
        }
        let header = read_box_header(r, pos, end)?;
        pos = header.end;
        boxes.push(header);
        if boxes.len() > 1_000_000 {
            return Err("文件结构无效：盒子数量异常".into());
        }
    }
    Ok(boxes)
}

/// 在整个文件（长度 `len`）中找到 `moov/mvhd`。
pub fn find_mvhd<R: Read + Seek>(r: &mut R, len: u64) -> Result<Mvhd, String> {
    let top = boxes_in(r, 0, len)?;
    let mut moovs = top.iter().filter(|b| &b.kind == b"moov");
    let moov = moovs
        .next()
        .ok_or("没有找到 moov（不是有效的 MP4 / MOV 文件，或文件不完整）")?;
    if moovs.next().is_some() {
        return Err("文件中有多个 moov，无法确定要修改哪一个".into());
    }
    let children = boxes_in(r, moov.start + moov.header_len, moov.end)?;
    if children.iter().any(|b| &b.kind == b"mvex") {
        return Err("这是分片 MP4（fMP4），播放器不按 mvhd 计算时长，无法修改".into());
    }
    let mvhd = children
        .iter()
        .find(|b| &b.kind == b"mvhd")
        .ok_or("moov 中没有 mvhd")?;
    let body = mvhd.start + mvhd.header_len;
    let body_len = mvhd.end - body;
    let mut version = [0u8; 1];
    if body_len < 4 {
        return Err("mvhd 不完整".into());
    }
    read_exact_at(r, body, &mut version)?;
    let (timescale_at, duration_at, duration_len) = match version[0] {
        0 => (body + 12, body + 16, 4u64),
        1 => (body + 20, body + 24, 8u64),
        v => return Err(format!("不支持的 mvhd 版本：{v}")),
    };
    if duration_at + duration_len > mvhd.end {
        return Err("mvhd 不完整".into());
    }
    let mut ts = [0u8; 4];
    read_exact_at(r, timescale_at, &mut ts)?;
    let timescale = u32::from_be_bytes(ts);
    if timescale == 0 {
        return Err("mvhd 的 timescale 为 0".into());
    }
    let duration = if duration_len == 4 {
        let mut d = [0u8; 4];
        read_exact_at(r, duration_at, &mut d)?;
        u64::from(u32::from_be_bytes(d))
    } else {
        let mut d = [0u8; 8];
        read_exact_at(r, duration_at, &mut d)?;
        u64::from_be_bytes(d)
    };
    Ok(Mvhd {
        version: version[0],
        timescale,
        duration,
        duration_offset: duration_at,
    })
}

/// 目标秒数换算为 mvhd 单位；32 位 mvhd 放不下时报错。
pub fn duration_units(mvhd: &Mvhd, seconds: f64) -> Result<u64, String> {
    let units = (seconds * f64::from(mvhd.timescale)).round();
    if !(units.is_finite() && units >= 1.0) {
        return Err("目标时长无效".into());
    }
    let limit = if mvhd.version == 0 {
        f64::from(u32::MAX)
    } else {
        u64::MAX as f64
    };
    if units > limit {
        return Err("目标时长太长，超出了该文件 mvhd（32 位）能表示的范围".into());
    }
    Ok(units as u64)
}

/// 改写 mvhd 的 duration。
pub fn write_duration<W: Write + Seek>(w: &mut W, mvhd: &Mvhd, units: u64) -> Result<(), String> {
    w.seek(SeekFrom::Start(mvhd.duration_offset))
        .map_err(io_err)?;
    let result = if mvhd.version == 0 {
        let v = u32::try_from(units).map_err(|_| "目标时长超出 32 位 mvhd 的范围".to_owned())?;
        w.write_all(&v.to_be_bytes())
    } else {
        w.write_all(&units.to_be_bytes())
    };
    result.map_err(|e| format!("写入文件出错：{e}"))
}

/// 在文件副本上定位并改写 mvhd，返回改写前后的 mvhd 单位数。
pub fn patch_file(path: &std::path::Path, seconds: f64) -> Result<(Mvhd, u64), String> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("无法打开输出文件：{e}"))?;
    let len = file.metadata().map_err(io_err)?.len();
    let mvhd = find_mvhd(&mut file, len)?;
    let units = duration_units(&mvhd, seconds)?;
    write_duration(&mut file, &mvhd, units)?;
    file.sync_all().map_err(|e| format!("写入文件出错：{e}"))?;
    Ok((mvhd, units))
}

/// 读取文件的 mvhd（只读）。
pub fn read_mvhd(path: &std::path::Path) -> Result<Mvhd, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("无法打开文件：{e}"))?;
    let len = file.metadata().map_err(io_err)?.len();
    find_mvhd(&mut file, len)
}

fn stream_counts(info: &MediaInfo) -> (bool, usize, usize) {
    (info.video.is_some(), info.audio.len(), info.subtitles.len())
}

impl Tool for ChangeDurationParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let info = ctx.info(0)?;
        let input = ctx.input_path(0)?.to_path_buf();
        if !is_mp4_family(&info.format_name) {
            return Err(format!(
                "只支持 MP4 / MOV 容器（当前为 {}）：其它格式没有可以安全改写的总时长字段，可先用“无损转换”换成 mp4",
                if info.format_name.is_empty() { "未知格式" } else { &info.format_name }
            )
            .into());
        }
        let target = self.target(info.duration, &mut ctx.rng)?;
        ctx.set_steps(&[1.0, 3.0]);

        let ext = input
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| "mp4".into());
        let output = ctx.output(&OutputSpec::file(&ext))?;
        ctx.check_cancelled()?;
        std::fs::copy(&input, &output).map_err(|e| format!("无法复制源文件：{e}"))?;
        let (mvhd, units) = patch_file(&output, target)?;
        ctx.skip_step();
        ctx.note(format!(
            "容器时长 {} 秒 → {} 秒（mvhd v{}，timescale {}：{} → {}）",
            fmt_num(mvhd.duration as f64 / f64::from(mvhd.timescale)),
            fmt_num(target),
            mvhd.version,
            mvhd.timescale,
            mvhd.duration,
            units
        ));

        // 校验：mvhd 已是目标值、ffprobe 能读、流没变、能完整解码
        let reread = read_mvhd(&output)?;
        if reread.duration != units {
            return Err("修改后的 mvhd 时长与目标不符".into());
        }
        let written = ctx.probe(&output)?;
        if stream_counts(&written) != stream_counts(&info) {
            return Err("修改后的文件流数量与源文件不同".into());
        }
        let args = vec![
            "-xerror".into(),
            "-i".into(),
            path_arg(&output)?,
            "-map".into(),
            "0:v?".into(),
            "-map".into(),
            "0:a?".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ];
        ctx.run_ffmpeg("校验（完整解码）", args, info.duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SeededRng;
    use std::io::Cursor;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend(kind);
        out.extend(body);
        out
    }

    fn mvhd_v0(timescale: u32, duration: u32) -> Vec<u8> {
        let mut body = vec![0, 0, 0, 0]; // version + flags
        body.extend(1u32.to_be_bytes()); // creation
        body.extend(2u32.to_be_bytes()); // modification
        body.extend(timescale.to_be_bytes());
        body.extend(duration.to_be_bytes());
        body.extend([0u8; 80]); // rate、volume、matrix、next_track_id…
        boxed(b"mvhd", &body)
    }

    fn mvhd_v1(timescale: u32, duration: u64) -> Vec<u8> {
        let mut body = vec![1, 0, 0, 0];
        body.extend(1u64.to_be_bytes());
        body.extend(2u64.to_be_bytes());
        body.extend(timescale.to_be_bytes());
        body.extend(duration.to_be_bytes());
        body.extend([0u8; 80]);
        boxed(b"mvhd", &body)
    }

    /// ftyp + moov(mvhd, trak) + mdat，moov 在前或在后。
    fn file(mvhd: Vec<u8>, moov_first: bool) -> Vec<u8> {
        let ftyp = boxed(b"ftyp", b"isom\0\0\0\x01isom");
        let mut moov_body = mvhd;
        moov_body.extend(boxed(b"trak", &boxed(b"tkhd", &[0u8; 84])));
        let moov = boxed(b"moov", &moov_body);
        let mdat = boxed(b"mdat", &[0xAB; 100]);
        let mut out = ftyp;
        if moov_first {
            out.extend(moov);
            out.extend(mdat);
        } else {
            out.extend(mdat);
            out.extend(moov);
        }
        out
    }

    fn find(bytes: &[u8]) -> Result<Mvhd, String> {
        find_mvhd(&mut Cursor::new(bytes.to_vec()), bytes.len() as u64)
    }

    fn patch(bytes: &[u8], seconds: f64) -> Result<Vec<u8>, String> {
        let mut cursor = Cursor::new(bytes.to_vec());
        let mvhd = find_mvhd(&mut cursor, bytes.len() as u64)?;
        let units = duration_units(&mvhd, seconds)?;
        write_duration(&mut cursor, &mvhd, units)?;
        Ok(cursor.into_inner())
    }

    #[test]
    fn version0_moov_first_and_last() {
        for moov_first in [true, false] {
            let bytes = file(mvhd_v0(1000, 10_000), moov_first);
            let mvhd = find(&bytes).unwrap();
            assert_eq!(
                (mvhd.version, mvhd.timescale, mvhd.duration),
                (0, 1000, 10_000)
            );
            let patched = patch(&bytes, 25.5).unwrap();
            assert_eq!(patched.len(), bytes.len());
            assert_eq!(find(&patched).unwrap().duration, 25_500);
            // 只有 duration 的 4 个字节变化
            let changed: Vec<usize> = (0..bytes.len())
                .filter(|&i| bytes[i] != patched[i])
                .collect();
            let at = mvhd.duration_offset as usize;
            assert!(
                changed.iter().all(|i| (at..at + 4).contains(i)),
                "{changed:?}"
            );
        }
    }

    #[test]
    fn version1_and_large_box_headers() {
        let bytes = file(mvhd_v1(90_000, 900_000), false);
        let mvhd = find(&bytes).unwrap();
        assert_eq!(
            (mvhd.version, mvhd.timescale, mvhd.duration),
            (1, 90_000, 900_000)
        );
        // 64 位才放得下：100 小时 × 90000
        let patched = patch(&bytes, 360_000.0).unwrap();
        assert_eq!(find(&patched).unwrap().duration, 32_400_000_000);

        // mdat 用 64 位盒子头（size == 1）也能跳过
        let mut big = boxed(b"ftyp", b"isom");
        let mdat_body = [0u8; 40];
        big.extend(1u32.to_be_bytes());
        big.extend(b"mdat");
        big.extend(((mdat_body.len() + 16) as u64).to_be_bytes());
        big.extend(mdat_body);
        big.extend(boxed(b"moov", &mvhd_v0(600, 6000)));
        assert_eq!(find(&big).unwrap().duration, 6000);
    }

    #[test]
    fn version0_overflow_is_rejected() {
        let bytes = file(mvhd_v0(90_000, 1), true);
        let err = patch(&bytes, 100_000.0).unwrap_err();
        assert!(err.contains("32 位"), "{err}");
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        let good = file(mvhd_v0(1000, 1000), true);
        let mut cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            vec![0, 0, 0],
            boxed(b"ftyp", b"isom"),                     // 没有 moov
            boxed(b"moov", &boxed(b"trak", &[])),        // 没有 mvhd
            boxed(b"moov", &boxed(b"mvhd", &[0, 0, 0])), // mvhd 不完整
            boxed(b"moov", &boxed(b"mvhd", &[2; 40])),   // 未知版本
            boxed(b"moov", &mvhd_v0(0, 100)),            // timescale 为 0
            [
                boxed(b"moov", &mvhd_v0(1, 1)),
                boxed(b"moov", &mvhd_v0(1, 1)),
            ]
            .concat(), // 两个 moov
        ];
        let mut too_big = good.clone();
        too_big[2] = 0xFF; // ftyp 大小超出文件
        cases.push(too_big);
        let mut too_small = good.clone();
        too_small[0..4].copy_from_slice(&4u32.to_be_bytes());
        cases.push(too_small);
        let mut fragmented_body = mvhd_v0(1000, 0);
        fragmented_body.extend(boxed(b"mvex", &boxed(b"trex", &[0; 24])));
        cases.push(boxed(b"moov", &fragmented_body));
        // 截断到每一个长度都不能 panic
        for n in 0..good.len() {
            cases.push(good[..n].to_vec());
        }
        for (i, bytes) in cases.iter().enumerate() {
            let result = std::panic::catch_unwind(|| find(bytes));
            let result = result.unwrap_or_else(|_| panic!("第 {i} 个输入 panic"));
            if i < 11 {
                assert!(result.is_err(), "第 {i} 个输入应报错：{result:?}");
            }
        }
        assert!(find(&boxed(b"moov", &fragmented_body_for_message()))
            .unwrap_err()
            .contains("分片"));
    }

    fn fragmented_body_for_message() -> Vec<u8> {
        let mut body = mvhd_v0(1000, 0);
        body.extend(boxed(b"mvex", &[]));
        body
    }

    #[test]
    fn targets_are_seeded_and_validated() {
        let params = ChangeDurationParams::default();
        let mut rng = SeededRng::new(1);
        assert_eq!(params.target(12.5, &mut rng).unwrap(), 25.0);
        assert!(params.target(0.0, &mut rng).is_err(), "倍数需要实际时长");
        let range = ChangeDurationParams {
            mode: DurationMode::Absolute,
            seconds: Range::new(100.0, 200.0),
            ..ChangeDurationParams::default()
        };
        let a = range.target(5.0, &mut SeededRng::new(42)).unwrap();
        let b = range.target(5.0, &mut SeededRng::new(42)).unwrap();
        assert_eq!(a, b, "同一种子结果相同");
        assert!((100.0..=200.0).contains(&a));
        assert_eq!((a * 1000.0).round() / 1000.0, a, "保留 3 位小数");
        for bad in [
            ChangeDurationParams {
                multiplier: Range::fixed(0.0),
                ..ChangeDurationParams::default()
            },
            ChangeDurationParams {
                multiplier: Range::new(1.0, 101.0),
                ..ChangeDurationParams::default()
            },
            ChangeDurationParams {
                mode: DurationMode::Absolute,
                seconds: Range::fixed(f64::NAN),
                ..ChangeDurationParams::default()
            },
            ChangeDurationParams {
                mode: DurationMode::Absolute,
                seconds: Range::fixed(0.05),
                ..ChangeDurationParams::default()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn container_family() {
        assert!(is_mp4_family("mov,mp4,m4a,3gp,3g2,mj2"));
        assert!(!is_mp4_family("matroska,webm"));
        assert!(!is_mp4_family("avi"));
        assert!(!is_mp4_family(""));
    }
}
