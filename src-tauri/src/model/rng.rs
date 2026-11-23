//! 可指定种子的随机数（SplitMix64）。
//!
//! 不依赖外部 crate，保证同一个种子在任何版本、任何平台上都得到同样的序列，
//! 便于单元测试断言与“裂变”复现。

use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededRng {
    state: u64,
}

/// FNV-1a，用于把字符串键混入种子。
fn fnv1a(key: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

impl SeededRng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// 由种子和一个键派生出独立的随机序列。
    ///
    /// 每个流水线模块用自己的 ID 派生，增删别的模块不会改变它的取值。
    pub fn derive(seed: u64, key: &str) -> Self {
        let mut mixer = Self::new(seed ^ fnv1a(key));
        Self::new(mixer.next_u64())
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// `[0, 1)` 内均匀分布。
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// `[min, max]` 内的实数；`min == max` 时直接返回。
    pub fn f64_in(&mut self, min: f64, max: f64) -> f64 {
        let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
        if lo == hi {
            return lo;
        }
        lo + (hi - lo) * self.next_f64()
    }

    /// `[min, max]` 内的整数（两端都包含）。
    pub fn i64_in(&mut self, min: i64, max: i64) -> i64 {
        let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
        let span = (hi as i128 - lo as i128 + 1) as u128;
        let offset = (u128::from(self.next_u64()) * span) >> 64;
        (lo as i128 + offset as i128) as i64
    }

    /// `[min, max]` 内的无符号整数（两端都包含）。
    pub fn u64_in(&mut self, min: u64, max: u64) -> u64 {
        let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
        let span = u128::from(hi - lo) + 1;
        let offset = (u128::from(self.next_u64()) * span) >> 64;
        lo + offset as u64
    }

    /// `[0, len)` 内的下标；`len` 为 0 时返回 0。
    pub fn index(&mut self, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        self.u64_in(0, len as u64 - 1) as usize
    }

    /// 以概率 `p` 返回 true。
    pub fn chance(&mut self, p: f64) -> bool {
        self.next_f64() < p
    }
}

/// 没有指定种子时用时间生成一个。
pub fn random_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    SeededRng::new(nanos ^ u64::from(std::process::id())).next_u64()
}

/// 把基础种子和序号混合，得到每个任务（每次裂变）各自的种子。
pub fn mix_seed(seed: u64, n: u64) -> u64 {
    SeededRng::new(seed ^ n.wrapping_mul(0x9e37_79b9_7f4a_7c15)).next_u64()
}

/// 第 `round` 次裂变（从 1 起）使用的种子。
///
/// 第 1 次就是任务种子本身，所以裂变次数为 1 时与不裂变完全相同；
/// 之后每次由任务种子和轮次派生，范围值、目录随机素材等因此各不相同。
pub fn fission_seed(task_seed: u64, round: usize) -> u64 {
    if round <= 1 {
        task_seed
    } else {
        SeededRng::derive(task_seed, &format!("fission#{round}")).next_u64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = SeededRng::new(42);
        let mut b = SeededRng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        // 固定参考值，防止算法被无意改动
        assert_eq!(SeededRng::new(0).next_u64(), 0xe220_a839_7b1d_cdaf);
    }

    #[test]
    fn derived_streams_differ_by_key() {
        let mut a = SeededRng::derive(7, "pipe.fps");
        let mut b = SeededRng::derive(7, "pipe.bitrate");
        assert_ne!(a.next_u64(), b.next_u64());
        assert_eq!(SeededRng::derive(7, "x"), SeededRng::derive(7, "x"));
    }

    #[test]
    fn ranges_are_inclusive_and_bounded() {
        let mut rng = SeededRng::new(1);
        let mut seen = [false; 3];
        for _ in 0..1000 {
            let v = rng.i64_in(-1, 1);
            assert!((-1..=1).contains(&v));
            seen[(v + 1) as usize] = true;
            let f = rng.f64_in(2.0, 3.0);
            assert!((2.0..=3.0).contains(&f));
            assert!(rng.index(5) < 5);
        }
        assert_eq!(seen, [true; 3]);
        assert_eq!(rng.u64_in(5, 5), 5);
        assert_eq!(rng.i64_in(i64::MIN, i64::MIN), i64::MIN);
    }

    #[test]
    fn fission_seeds_are_distinct_and_reproducible() {
        let task = mix_seed(7, 3);
        assert_eq!(fission_seed(task, 1), task, "第 1 次裂变与不裂变相同");
        let seeds: Vec<u64> = (1..=20).map(|r| fission_seed(task, r)).collect();
        let mut unique = seeds.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), seeds.len(), "各次裂变的种子互不相同");
        assert_eq!(fission_seed(task, 5), fission_seed(task, 5));
        // 不同任务的同一轮次也不同
        assert_ne!(fission_seed(mix_seed(7, 4), 2), fission_seed(task, 2));
    }
}
