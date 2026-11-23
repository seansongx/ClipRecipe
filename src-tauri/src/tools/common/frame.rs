//! 画面规格小函数：颜色参数、缩放到目标框的方式、宽高取偶与检查。横竖互转、画面拼接、图转视频、批量截图共用。

use serde::{Deserialize, Serialize};

/// 目标宽高的范围（像素）。
pub const SIZE_RANGE: (u32, u32) = (16, 7680);

/// `#rrggbb`（或 `#rgb`）→ FFmpeg 颜色 `0xrrggbb`。
pub fn color_arg(text: &str, what: &str) -> Result<String, String> {
    let hex = text.trim().trim_start_matches('#');
    let full = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => hex.to_owned(),
        _ => String::new(),
    };
    if full.len() == 6 && full.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(format!("0x{}", full.to_ascii_lowercase()))
    } else {
        Err(format!("{what}不是有效的颜色：{text}"))
    }
}

/// 向下取偶数（至少 2）。
pub fn even(value: u32) -> u32 {
    (value & !1).max(2)
}

/// 画面放进目标框的方式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Fit {
    /// 完整放入（保持比例，可能留边）
    #[default]
    Fit,
    /// 铺满（保持比例，居中裁掉多余部分）
    Fill,
    /// 拉伸到目标宽高（不保持比例）
    Stretch,
}

/// 缩放到 `w × h` 框的滤镜：`Fit` 的结果可能小于框（宽高取偶），`Fill` / `Stretch` 正好等于框。
pub fn fit_scale(fit: Fit, w: u32, h: u32) -> String {
    match fit {
        Fit::Fit => {
            format!("scale={w}:{h}:force_original_aspect_ratio=decrease:force_divisible_by=2")
        }
        Fit::Fill => format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}"),
        Fit::Stretch => format!("scale={w}:{h}"),
    }
}

/// 检查目标宽高并取偶。
pub fn target_size(width: u32, height: u32) -> Result<(u32, u32), String> {
    let (min, max) = SIZE_RANGE;
    if !(min..=max).contains(&width) || !(min..=max).contains(&height) {
        return Err(format!("宽高必须在 {min}～{max} 像素之间"));
    }
    Ok((even(width), even(height)))
}
