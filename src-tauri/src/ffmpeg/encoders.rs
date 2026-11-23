//! 视频编码器：探测可用的硬件编码器（实际试编码一小段 lavfi 画面），
//! 缓存结果，并按设置选择编码器；没有可用硬件编码器时回退到 libx264 / libx265。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::command;

/// 输出视频编码格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    Hevc,
}

/// 硬件编码器厂商；排列顺序即优先顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HwVendor {
    Nvenc,
    Qsv,
    Amf,
}

impl HwVendor {
    pub const ALL: [HwVendor; 3] = [HwVendor::Nvenc, HwVendor::Qsv, HwVendor::Amf];
    /// 核显优先（`set.gpu`）时的顺序：Intel 核显（QSV）→ AMD（AMF，常见于 APU）→ NVIDIA 独显。
    pub const INTEGRATED_FIRST: [HwVendor; 3] = [HwVendor::Qsv, HwVendor::Amf, HwVendor::Nvenc];

    /// 显示名称。
    pub fn label(&self) -> &'static str {
        match self {
            HwVendor::Nvenc => "NVIDIA（NVENC）",
            HwVendor::Qsv => "Intel（QSV）",
            HwVendor::Amf => "AMD（AMF）",
        }
    }

    /// Windows 下该厂商编码运行库（随显卡驱动安装）的文件名。
    fn driver_library(&self) -> &'static str {
        match self {
            HwVendor::Nvenc => "nvEncodeAPI64.dll",
            HwVendor::Qsv => "libmfxhw64.dll",
            HwVendor::Amf => "amfrt64.dll",
        }
    }
}

/// 进程级的“核显优先”偏好（由设置的读取 / 保存更新），供不直接拿到设置的调用方（如独立工具）使用。
static PREFER_INTEGRATED: AtomicBool = AtomicBool::new(false);

/// 更新进程级的“核显优先”偏好。
pub fn set_prefer_integrated(prefer: bool) {
    PREFER_INTEGRATED.store(prefer, Ordering::Relaxed);
}

/// 当前进程级的“核显优先”偏好。
pub fn prefer_integrated() -> bool {
    PREFER_INTEGRATED.load(Ordering::Relaxed)
}

/// 一个具体的视频编码器。`vendor` 为 `None` 时是软件编码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoEncoder {
    pub codec: VideoCodec,
    pub vendor: Option<HwVendor>,
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_owned()).collect()
}

impl VideoEncoder {
    pub const fn software(codec: VideoCodec) -> Self {
        Self {
            codec,
            vendor: None,
        }
    }

    pub const fn hardware(codec: VideoCodec, vendor: HwVendor) -> Self {
        Self {
            codec,
            vendor: Some(vendor),
        }
    }

    pub fn is_hardware(&self) -> bool {
        self.vendor.is_some()
    }

    /// FFmpeg 中的编码器名。
    pub fn name(&self) -> &'static str {
        use HwVendor::*;
        use VideoCodec::*;
        match (self.codec, self.vendor) {
            (H264, None) => "libx264",
            (Hevc, None) => "libx265",
            (H264, Some(Nvenc)) => "h264_nvenc",
            (Hevc, Some(Nvenc)) => "hevc_nvenc",
            (H264, Some(Qsv)) => "h264_qsv",
            (Hevc, Some(Qsv)) => "hevc_qsv",
            (H264, Some(Amf)) => "h264_amf",
            (Hevc, Some(Amf)) => "hevc_amf",
        }
    }

    /// 编码器像素格式：QSV 需要 nv12，其余用 yuv420p。
    pub fn pix_fmt(&self) -> &'static str {
        if self.vendor == Some(HwVendor::Qsv) {
            "nv12"
        } else {
            "yuv420p"
        }
    }

    /// `-c:v`、预设与像素格式。
    pub fn base_args(&self) -> Vec<String> {
        let mut args = s(&["-c:v", self.name()]);
        match self.vendor {
            None => args.extend(s(&["-preset", "medium"])),
            Some(HwVendor::Nvenc) => args.extend(s(&["-preset", "p5"])),
            Some(HwVendor::Qsv) => args.extend(s(&["-preset", "medium"])),
            Some(HwVendor::Amf) => args.extend(s(&["-quality", "balanced"])),
        }
        args.extend(s(&["-pix_fmt", self.pix_fmt()]));
        args
    }

    /// 目标码率（kbit/s）模式的参数。
    pub fn bitrate_args(&self, kbps: u64) -> Vec<String> {
        let kbps = kbps.max(1);
        let b = format!("{kbps}k");
        let max = format!("{}k", kbps * 3 / 2);
        let buf = format!("{}k", kbps * 2);
        let mut args = match self.vendor {
            Some(HwVendor::Nvenc) => s(&["-rc", "vbr"]),
            Some(HwVendor::Amf) => s(&["-rc", "vbr_peak"]),
            _ => Vec::new(),
        };
        args.extend([
            "-b:v".to_owned(),
            b,
            "-maxrate".to_owned(),
            max,
            "-bufsize".to_owned(),
            buf,
        ]);
        args
    }

    /// 不知道目标码率时的恒定质量参数。
    pub fn quality_args(&self) -> Vec<String> {
        match (self.vendor, self.codec) {
            (None, VideoCodec::H264) => s(&["-crf", "23"]),
            (None, VideoCodec::Hevc) => s(&["-crf", "28"]),
            (Some(HwVendor::Nvenc), _) => s(&["-rc", "vbr", "-cq", "23", "-b:v", "0"]),
            (Some(HwVendor::Qsv), _) => s(&["-global_quality", "23"]),
            (Some(HwVendor::Amf), _) => s(&["-rc", "cqp", "-qp_i", "23", "-qp_p", "23"]),
        }
    }

    /// `set.reduce-size`（体积压缩 / “瘦身”）的参数：用更高的恒定质量值（H.264 CRF 28、H.265 CRF 32，
    /// 硬件编码对应的 CQ / QP 32）代替“沿用源码率”（软件编码的预设由调用方改为更慢、压缩率更高的 `slow`）。
    /// 知道源码率时再加码率上限（`-maxrate` = 源码率，`-bufsize` = 2 倍），保证输出不会比源更大。
    /// 返回的参数放在 [`Self::base_args`] 之后，替代码率 / 质量参数。
    pub fn slim_args(&self, source_kbps: Option<u64>) -> Vec<String> {
        let mut args = match (self.vendor, self.codec) {
            (None, VideoCodec::H264) => s(&["-crf", "28"]),
            (None, VideoCodec::Hevc) => s(&["-crf", "32"]),
            (Some(HwVendor::Nvenc), _) => s(&["-rc", "vbr", "-cq", "32", "-b:v", "0"]),
            (Some(HwVendor::Qsv), _) => s(&["-global_quality", "32"]),
            (Some(HwVendor::Amf), _) => s(&["-rc", "cqp", "-qp_i", "32", "-qp_p", "32"]),
        };
        // AMF 的 cqp 模式不接受码率上限
        if let (Some(kbps), false) = (source_kbps, self.vendor == Some(HwVendor::Amf)) {
            let kbps = kbps.max(1);
            args.extend([
                "-maxrate".to_owned(),
                format!("{kbps}k"),
                "-bufsize".to_owned(),
                format!("{}k", kbps * 2),
            ]);
        }
        args
    }

    /// 保留 HDR 时的 10 bit 像素格式：软件编码 `yuv420p10le`，硬件编码 `p010le`。
    pub fn pix_fmt_10bit(&self) -> &'static str {
        if self.is_hardware() {
            "p010le"
        } else {
            "yuv420p10le"
        }
    }
}

/// 探测结果：试编码成功的硬件编码器。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderCaps {
    pub hardware: Vec<VideoEncoder>,
}

impl EncoderCaps {
    /// 不使用任何硬件编码器。
    pub fn software_only() -> Self {
        Self::default()
    }

    /// 按设置选择编码器：`gpu` 打开时取优先级最高的可用硬件编码器，否则软件编码。
    /// 厂商优先顺序取进程级偏好（[`set_prefer_integrated`]）。
    pub fn select(&self, codec: VideoCodec, gpu: bool) -> VideoEncoder {
        self.select_with(codec, gpu, prefer_integrated())
    }

    /// 同 [`Self::select`]，显式指定是否核显优先。
    pub fn select_with(
        &self,
        codec: VideoCodec,
        gpu: bool,
        prefer_integrated: bool,
    ) -> VideoEncoder {
        let order = if prefer_integrated {
            HwVendor::INTEGRATED_FIRST
        } else {
            HwVendor::ALL
        };
        if gpu {
            for vendor in order {
                let candidate = VideoEncoder::hardware(codec, vendor);
                if self.hardware.contains(&candidate) {
                    return candidate;
                }
            }
        }
        VideoEncoder::software(codec)
    }

    pub fn names(&self) -> Vec<String> {
        self.hardware.iter().map(|e| e.name().to_owned()).collect()
    }
}

/// 试编码的超时；驱动异常时 ffmpeg 可能卡住。
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// 用一小段 lavfi 画面试编码，判断编码器是否真的可用。
pub fn test_encoder(ffmpeg: &Path, encoder: VideoEncoder) -> bool {
    let child = command(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i", "color=c=black:s=256x256:r=25:d=0.2"])
        .args(["-frames:v", "3", "-pix_fmt", encoder.pix_fmt()])
        .args(["-c:v", encoder.name(), "-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if start.elapsed() > PROBE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return false,
        }
    }
}

/// 并行试编码全部硬件编码器（不使用缓存）。
pub fn detect(ffmpeg: &Path) -> EncoderCaps {
    let candidates: Vec<VideoEncoder> = HwVendor::ALL
        .iter()
        .flat_map(|&v| {
            [
                VideoEncoder::hardware(VideoCodec::H264, v),
                VideoEncoder::hardware(VideoCodec::Hevc, v),
            ]
        })
        .collect();
    let hardware = std::thread::scope(|scope| {
        let handles: Vec<_> = candidates
            .iter()
            .map(|&enc| scope.spawn(move || (enc, test_encoder(ffmpeg, enc))))
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .filter(|(_, ok)| *ok)
            .map(|(enc, _)| enc)
            .collect()
    });
    EncoderCaps { hardware }
}

fn cache() -> &'static Mutex<HashMap<PathBuf, EncoderCaps>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, EncoderCaps>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

/// 带进程内缓存的探测；同一个 ffmpeg 只探测一次。
pub fn cached(ffmpeg: &Path) -> EncoderCaps {
    if let Some(caps) = cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(ffmpeg)
    {
        return caps.clone();
    }
    let caps = detect(ffmpeg);
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(ffmpeg.to_path_buf(), caps.clone());
    caps
}

/// 清空全部缓存（下载 / 删除 FFmpeg 后，下次使用时重新探测）。
pub fn clear_cache() {
    cache().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// 丢弃缓存重新探测（例如更新驱动后）。
pub fn refresh(ffmpeg: &Path) -> EncoderCaps {
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(ffmpeg);
    cached(ffmpeg)
}

// ---------------------------------------------------------------------------------------------
// 默认码率（未启用 `pipe.bitrate` 时）
// ---------------------------------------------------------------------------------------------

/// H.264 默认码率的质量下限：每像素每帧 0.04 bit（1080p30 ≈ 2.5 Mbit/s，720p25 ≈ 0.9 Mbit/s）。
pub const FLOOR_BPP_H264: f64 = 0.04;
/// H.265 的质量下限（同等画质约为 H.264 的 70%）。
pub const FLOOR_BPP_HEVC: f64 = 0.028;
/// 小画面的绝对下限（kbit/s）：H.264 / H.265。
pub const FLOOR_MIN_KBPS_H264: u64 = 300;
pub const FLOOR_MIN_KBPS_HEVC: u64 = 200;

/// 画面规格（显示宽高、帧率），用来在不同画面之间按像素量换算码率。帧率未知（≤ 0）按 25 算；
/// 超过 60 的按 60 算（高帧率相邻帧更相似，码率不随帧率线性增长）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

impl FrameSize {
    pub fn new(width: u32, height: u32, fps: f64) -> Self {
        Self { width, height, fps }
    }

    /// 每秒像素数。
    pub fn pixel_rate(&self) -> f64 {
        let fps = if self.fps.is_finite() && self.fps > 0.0 {
            self.fps.clamp(1.0, 60.0)
        } else {
            25.0
        };
        f64::from(self.width.max(1)) * f64::from(self.height.max(1)) * fps
    }
}

/// 输出画面的质量下限（kbit/s）。
pub fn quality_floor_kbps(codec: VideoCodec, output: FrameSize) -> u64 {
    let (bpp, min) = match codec {
        VideoCodec::H264 => (FLOOR_BPP_H264, FLOOR_MIN_KBPS_H264),
        VideoCodec::Hevc => (FLOOR_BPP_HEVC, FLOOR_MIN_KBPS_HEVC),
    };
    ((output.pixel_rate() * bpp / 1000.0).round() as u64).max(min)
}

/// 码率的来源：一个源视频的码率（kbit/s）、画面与权重（通常是它在输出里占的时长）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BitrateSource {
    pub kbps: u64,
    pub frame: FrameSize,
    pub weight: f64,
}

/// 未启用“码率”设置（`pipe.bitrate`）时的目标码率：**沿用源视频的每像素码率，并保证不低于质量下限**。
///
/// - 各来源的每像素码率（码率 ÷ 每秒像素数）按权重平均，再乘以输出画面的每秒像素数——输出与源同尺寸时
///   就是源码率（“大致与源相同的画质和体积”）；缩放、拼接不同尺寸的源时按像素量换算。
/// - 取它与 [`quality_floor_kbps`] 的较大者：低码率的源（屏幕录制、纯色画面、高压缩的下载视频）在叠加文字、
///   进度条、转场、拼接其它素材后内容比源复杂，沿用源码率会糊成一片，下限保证可接受的画质。
/// - 没有任何来源的码率时返回 None，由调用方改用恒定质量（`quality_args`）。
///
/// 目标码率模式对软件与硬件编码器（NVENC / QSV / AMF 的 VBR）都适用，所以两者行为一致。
pub fn default_video_kbps(
    codec: VideoCodec,
    sources: &[BitrateSource],
    output: FrameSize,
) -> Option<u64> {
    let usable: Vec<&BitrateSource> = sources.iter().filter(|s| s.kbps > 0).collect();
    if usable.is_empty() {
        return None;
    }
    let weight = |s: &BitrateSource| {
        if s.weight.is_finite() && s.weight > 0.0 {
            s.weight
        } else {
            1.0
        }
    };
    let total: f64 = usable.iter().map(|s| weight(s)).sum();
    let bpp: f64 = usable
        .iter()
        .map(|s| s.kbps as f64 / s.frame.pixel_rate() * weight(s))
        .sum::<f64>()
        / total;
    let scaled = (bpp * output.pixel_rate()).round().max(1.0) as u64;
    Some(scaled.max(quality_floor_kbps(codec, output)))
}

/// 显卡驱动的编码运行库是否存在（只在 Windows 下检查 System32；其它系统返回 false）。
pub fn driver_present(vendor: HwVendor) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let root = std::env::var_os("WINDIR")
        .or_else(|| std::env::var_os("SystemRoot"))
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    root.join("System32")
        .join(vendor.driver_library())
        .is_file()
}

/// 装了某厂商的显卡驱动、但它的硬件编码器全部试编码失败的厂商（通常是驱动版本过低，或该显卡不支持编码）。
/// `present` 判断驱动是否存在（正式使用 [`driver_present`]，测试可替换）。
pub fn failed_vendors(caps: &EncoderCaps, present: impl Fn(HwVendor) -> bool) -> Vec<HwVendor> {
    HwVendor::ALL
        .into_iter()
        .filter(|&v| present(v) && !caps.hardware.iter().any(|e| e.vendor == Some(v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bitrate_keeps_source_rate_with_quality_floor() {
        let hd = FrameSize::new(1280, 720, 25.0);
        let src = |kbps| BitrateSource {
            kbps,
            frame: hd,
            weight: 1.0,
        };
        // 正常码率的源：沿用源码率
        assert_eq!(
            default_video_kbps(VideoCodec::H264, &[src(4000)], hd),
            Some(4000)
        );
        // 低码率的源：提到下限（1280×720×25×0.04 ≈ 922k）
        assert_eq!(
            default_video_kbps(VideoCodec::H264, &[src(20)], hd),
            Some(922)
        );
        assert_eq!(quality_floor_kbps(VideoCodec::Hevc, hd), 645);
        // 小画面的绝对下限
        let tiny = FrameSize::new(320, 240, 25.0);
        assert_eq!(quality_floor_kbps(VideoCodec::H264, tiny), 300);
        // 输出放大一倍（面积 4 倍）：按每像素码率换算
        let big = FrameSize::new(2560, 1440, 25.0);
        assert_eq!(
            default_video_kbps(VideoCodec::H264, &[src(4000)], big),
            Some(16000)
        );
        // 多个来源按权重平均每像素码率
        let mixed = [
            src(2000),
            BitrateSource {
                kbps: 6000,
                frame: hd,
                weight: 3.0,
            },
        ];
        assert_eq!(default_video_kbps(VideoCodec::H264, &mixed, hd), Some(5000));
        // 没有码率 → 恒定质量
        assert_eq!(default_video_kbps(VideoCodec::H264, &[], hd), None);
        // 帧率未知按 25、超过 60 按 60
        assert_eq!(FrameSize::new(10, 10, 0.0).pixel_rate(), 2500.0);
        assert_eq!(FrameSize::new(10, 10, 240.0).pixel_rate(), 6000.0);
    }

    #[test]
    fn integrated_preference_changes_vendor_order() {
        let caps = EncoderCaps {
            hardware: vec![
                VideoEncoder::hardware(VideoCodec::H264, HwVendor::Nvenc),
                VideoEncoder::hardware(VideoCodec::H264, HwVendor::Qsv),
            ],
        };
        assert_eq!(
            caps.select_with(VideoCodec::H264, true, false).name(),
            "h264_nvenc"
        );
        assert_eq!(
            caps.select_with(VideoCodec::H264, true, true).name(),
            "h264_qsv"
        );
        assert_eq!(
            caps.select_with(VideoCodec::H264, false, true).name(),
            "libx264"
        );
        // 只有独显可用时仍然用独显
        let only_nvenc = EncoderCaps {
            hardware: vec![VideoEncoder::hardware(VideoCodec::H264, HwVendor::Nvenc)],
        };
        assert_eq!(
            only_nvenc.select_with(VideoCodec::H264, true, true).name(),
            "h264_nvenc"
        );
    }

    #[test]
    fn failed_vendor_hint_needs_driver_and_no_working_encoder() {
        let caps = EncoderCaps {
            hardware: vec![VideoEncoder::hardware(VideoCodec::Hevc, HwVendor::Qsv)],
        };
        let installed = |v: HwVendor| matches!(v, HwVendor::Nvenc | HwVendor::Qsv);
        assert_eq!(failed_vendors(&caps, installed), [HwVendor::Nvenc]);
        assert!(failed_vendors(&caps, |_| false).is_empty());
        assert_eq!(HwVendor::Nvenc.label(), "NVIDIA（NVENC）");
    }

    #[test]
    fn slim_args_use_higher_crf_and_cap_bitrate() {
        let x264 = VideoEncoder::software(VideoCodec::H264);
        assert_eq!(
            x264.slim_args(Some(4000)),
            ["-crf", "28", "-maxrate", "4000k", "-bufsize", "8000k"]
        );
        assert_eq!(
            VideoEncoder::software(VideoCodec::Hevc).slim_args(None),
            ["-crf", "32"]
        );
        let amf = VideoEncoder::hardware(VideoCodec::H264, HwVendor::Amf);
        assert!(!amf.slim_args(Some(4000)).contains(&"-maxrate".to_owned()));
        assert_eq!(x264.pix_fmt_10bit(), "yuv420p10le");
        assert_eq!(amf.pix_fmt_10bit(), "p010le");
    }

    #[test]
    fn names_and_args() {
        let x264 = VideoEncoder::software(VideoCodec::H264);
        assert_eq!(x264.name(), "libx264");
        assert_eq!(
            x264.base_args(),
            ["-c:v", "libx264", "-preset", "medium", "-pix_fmt", "yuv420p"]
        );
        assert_eq!(
            x264.bitrate_args(2000),
            ["-b:v", "2000k", "-maxrate", "3000k", "-bufsize", "4000k"]
        );
        assert_eq!(x264.quality_args(), ["-crf", "23"]);

        let qsv = VideoEncoder::hardware(VideoCodec::Hevc, HwVendor::Qsv);
        assert_eq!(qsv.name(), "hevc_qsv");
        assert!(qsv
            .base_args()
            .ends_with(&["-pix_fmt".to_owned(), "nv12".to_owned()]));

        let nvenc = VideoEncoder::hardware(VideoCodec::H264, HwVendor::Nvenc);
        assert_eq!(&nvenc.bitrate_args(1000)[..2], ["-rc", "vbr"]);
    }

    #[test]
    fn selection_prefers_hardware_in_vendor_order_and_falls_back() {
        let caps = EncoderCaps {
            hardware: vec![
                VideoEncoder::hardware(VideoCodec::H264, HwVendor::Amf),
                VideoEncoder::hardware(VideoCodec::H264, HwVendor::Qsv),
            ],
        };
        assert_eq!(caps.select(VideoCodec::H264, true).name(), "h264_qsv");
        assert_eq!(caps.select(VideoCodec::H264, false).name(), "libx264");
        assert_eq!(caps.select(VideoCodec::Hevc, true).name(), "libx265");
        assert_eq!(
            EncoderCaps::software_only()
                .select(VideoCodec::H264, true)
                .name(),
            "libx264"
        );
    }
}
