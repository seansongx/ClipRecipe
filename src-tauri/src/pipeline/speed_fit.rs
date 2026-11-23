//! `set.speed-fit`（变速适应）：输出时长与目标时长不一致时，整体变速使两者一致。
//!
//! 供独立工具使用的共用助手（开关在 `Settings::speed_fit`）：工具算出“实际时长”和“目标时长”后调用
//! [`speed_fit`]，把返回的视频 / 音频滤镜接在自己滤镜链的末尾即可。批量流水线不使用（时长由各模块决定）。

use super::fmt_num;

/// 相对误差在此以内视为已经一致，不变速。
pub const TOLERANCE: f64 = 0.005;
/// 允许的变速范围（目标 / 实际）；超出时不变速，由调用方提示。
pub const MIN_FACTOR: f64 = 0.25;
pub const MAX_FACTOR: f64 = 4.0;

/// 变速适应的滤镜。
#[derive(Debug, Clone, PartialEq)]
pub struct SpeedFit {
    /// 时长倍数（目标 / 实际）：>1 放慢，<1 加快
    pub factor: f64,
    /// 视频滤镜：`setpts=倍数*PTS`
    pub video: String,
    /// 音频滤镜：一个或多个 `atempo`（每个限制在 0.5～2）
    pub audio: String,
}

/// 计算把 `actual` 秒变为 `target` 秒的滤镜。已经一致（或参数无效）时返回 `Ok(None)`；
/// 变速倍数超出 [`MIN_FACTOR`]～[`MAX_FACTOR`] 时返回错误。
pub fn speed_fit(actual: f64, target: f64) -> Result<Option<SpeedFit>, String> {
    if !(actual > 0.0 && target > 0.0) || !actual.is_finite() || !target.is_finite() {
        return Ok(None);
    }
    let factor = target / actual;
    if (factor - 1.0).abs() <= TOLERANCE {
        return Ok(None);
    }
    if !(MIN_FACTOR..=MAX_FACTOR).contains(&factor) {
        return Err(format!(
            "变速适应需要把 {} 秒变为 {} 秒（{:.2} 倍），超出允许范围",
            fmt_num(actual),
            fmt_num(target),
            factor
        ));
    }
    // atempo 是速度倍数（= 1 / 时长倍数），单个滤镜限制在 0.5～2，超出时串联
    let mut tempo = 1.0 / factor;
    let mut parts = Vec::new();
    while tempo > 2.0 {
        parts.push("atempo=2".to_owned());
        tempo /= 2.0;
    }
    while tempo < 0.5 {
        parts.push("atempo=0.5".to_owned());
        tempo /= 0.5;
    }
    parts.push(format!("atempo={}", fmt_num(tempo)));
    Ok(Some(SpeedFit {
        factor,
        video: format!("setpts={}*PTS", fmt_num(factor)),
        audio: parts.join(","),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_setpts_and_chained_atempo() {
        let fit = speed_fit(10.0, 20.0).unwrap().unwrap();
        assert_eq!(fit.video, "setpts=2*PTS");
        assert_eq!(fit.audio, "atempo=0.5");
        let fast = speed_fit(30.0, 10.0).unwrap().unwrap();
        assert_eq!(fast.video, "setpts=0.3333*PTS");
        assert_eq!(fast.audio, "atempo=2,atempo=1.5");
        let slow = speed_fit(10.0, 35.0).unwrap().unwrap();
        assert_eq!(slow.audio, "atempo=0.5,atempo=0.5714");
    }

    #[test]
    fn no_change_invalid_and_out_of_range() {
        assert_eq!(speed_fit(10.0, 10.02).unwrap(), None);
        assert_eq!(speed_fit(0.0, 10.0).unwrap(), None);
        assert_eq!(speed_fit(10.0, f64::NAN).unwrap(), None);
        assert!(speed_fit(10.0, 50.0).is_err());
        assert!(speed_fit(10.0, 2.0).is_err());
    }
}
