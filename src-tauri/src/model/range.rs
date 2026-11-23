//! 范围值：`[min, max]`，每个视频（每次裂变）在范围内随机取值；两端相等即固定值。

use serde::{Deserialize, Serialize};

use super::rng::SeededRng;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Range<T> {
    pub min: T,
    pub max: T,
}

/// 能在 [`Range`] 里随机取值的数值类型。
pub trait RangeValue: Copy + PartialOrd {
    fn sample_between(rng: &mut SeededRng, lo: Self, hi: Self) -> Self;
}

impl RangeValue for f64 {
    fn sample_between(rng: &mut SeededRng, lo: f64, hi: f64) -> f64 {
        rng.f64_in(lo, hi)
    }
}

macro_rules! int_range_value {
    ($($t:ty),*) => {$(
        impl RangeValue for $t {
            fn sample_between(rng: &mut SeededRng, lo: $t, hi: $t) -> $t {
                rng.i64_in(lo as i64, hi as i64) as $t
            }
        }
    )*};
}
int_range_value!(i32, i64, u32);

impl RangeValue for u64 {
    fn sample_between(rng: &mut SeededRng, lo: u64, hi: u64) -> u64 {
        rng.u64_in(lo, hi)
    }
}

impl<T: RangeValue> Range<T> {
    pub const fn new(min: T, max: T) -> Self {
        Self { min, max }
    }

    /// 固定值。
    pub const fn fixed(value: T) -> Self {
        Self {
            min: value,
            max: value,
        }
    }

    pub fn is_fixed(&self) -> bool {
        self.min == self.max
    }

    /// 两端按大小排好（用户把 min/max 填反时容错）。
    pub fn ordered(&self) -> (T, T) {
        if self.min <= self.max {
            (self.min, self.max)
        } else {
            (self.max, self.min)
        }
    }

    /// 在范围内取一个值；固定值不消耗随机数。
    pub fn sample(&self, rng: &mut SeededRng) -> T {
        if self.is_fixed() {
            return self.min;
        }
        let (lo, hi) = self.ordered();
        T::sample_between(rng, lo, hi)
    }
}

impl Range<f64> {
    /// 取值后保留 `decimals` 位小数。
    pub fn sample_rounded(&self, rng: &mut SeededRng, decimals: u32) -> f64 {
        let factor = 10f64.powi(decimals as i32);
        (self.sample(rng) * factor).round() / factor
    }
}

impl<T: Default> Default for Range<T> {
    fn default() -> Self {
        Self {
            min: T::default(),
            max: T::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_range_returns_value_without_consuming_rng() {
        let mut rng = SeededRng::new(3);
        let before = rng.clone();
        assert_eq!(Range::fixed(30.0).sample(&mut rng), 30.0);
        assert_eq!(rng, before);
    }

    #[test]
    fn sampling_is_reproducible_and_in_bounds() {
        let range = Range::new(24.0, 30.0);
        let a: Vec<f64> = {
            let mut rng = SeededRng::new(9);
            (0..20).map(|_| range.sample_rounded(&mut rng, 2)).collect()
        };
        let b: Vec<f64> = {
            let mut rng = SeededRng::new(9);
            (0..20).map(|_| range.sample_rounded(&mut rng, 2)).collect()
        };
        assert_eq!(a, b);
        assert!(a.iter().all(|v| (24.0..=30.0).contains(v)));
    }

    #[test]
    fn swapped_bounds_are_tolerated() {
        let mut rng = SeededRng::new(5);
        for _ in 0..100 {
            let v = Range::new(10u32, 5u32).sample(&mut rng);
            assert!((5..=10).contains(&v));
        }
    }

    #[test]
    fn serde_shape() {
        let json = serde_json::to_string(&Range::new(1, 2)).unwrap();
        assert_eq!(json, r#"{"min":1,"max":2}"#);
        let parsed: Range<f64> = serde_json::from_str(r#"{"min":0.5,"max":1}"#).unwrap();
        assert_eq!(parsed, Range::new(0.5, 1.0));
    }
}
