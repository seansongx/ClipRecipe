//! `pipe.grid` 宫格分屏：把同一画面重复排成 2 / 3 / 4 / 6 / 9 宫格。
//!
//! 参数与取舍（docs/specs/pipeline.md 未规定的部分）：
//! - 排布（列 × 行）：2 → 1×2 或 2×1；3 → 1×3 或 3×1；4 → 2×2；6 → 2×3 或 3×2；9 → 3×3。
//! - `direction`：`vertical` 为沿竖直方向排（上下堆叠），`horizontal` 为沿水平方向排；
//!   `auto` 时横屏（宽 ≥ 高）竖排、竖屏横排。方向影响 2、3、6 宫格的排布，以及 9 宫格“两端”的含义。
//! - 输出尺寸：每格保持源画面比例（不拉伸、不裁剪）；整体按源画面拼接后，等比缩小到
//!   “长边不超过源画面的长边”，每格宽高取偶数。例如 1280×720 竖排 2 宫格 → 每格 1136×640，输出 1136×1280；
//!   2×2 → 每格 640×360，输出 1280×720。
//! - `blurEnds`（两端虚化）：沿排布方向的首尾两排（行或列）用 `gblur` 虚化、中间保持清晰，
//!   只对 3、6、9 宫格有效（2、4 宫格没有“中间”，给出警告后忽略）；`blur` 为 sigma（0～100）。

use serde::{Deserialize, Serialize};

use crate::model::Range;
use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.grid";

pub const COUNTS: [u32; 5] = [2, 3, 4, 6, 9];
pub const MAX_BLUR: f64 = 100.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GridDirection {
    /// 横屏竖排、竖屏横排
    #[default]
    Auto,
    /// 沿水平方向排（左右并排）
    Horizontal,
    /// 沿竖直方向排（上下堆叠）
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GridParams {
    pub enabled: bool,
    /// 宫格数：2 / 3 / 4 / 6 / 9
    pub count: u32,
    pub direction: GridDirection,
    /// 两端虚化
    pub blur_ends: bool,
    /// 虚化强度（gblur sigma），取值保留 1 位小数
    pub blur: Range<f64>,
}

impl Default for GridParams {
    fn default() -> Self {
        Self {
            enabled: false,
            count: 3,
            direction: GridDirection::Auto,
            blur_ends: false,
            blur: Range::fixed(20.0),
        }
    }
}

/// 向下取偶数，至少为 2。
fn even_down(x: f64) -> u32 {
    let n = (x + 1e-6).floor().max(2.0) as u32;
    n - n % 2
}

/// 返回 (列数, 行数)。`vertical` 为 true 表示沿竖直方向排。
fn layout(count: u32, vertical: bool) -> (u32, u32) {
    match (count, vertical) {
        (2, true) => (1, 2),
        (2, false) => (2, 1),
        (3, true) => (1, 3),
        (3, false) => (3, 1),
        (6, true) => (2, 3),
        (6, false) => (3, 2),
        (9, _) => (3, 3),
        _ => (2, 2),
    }
}

impl PipelineModule for GridParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        if !COUNTS.contains(&self.count) {
            return Err("宫格数只能是 2、3、4、6、9".into());
        }
        let (lo, hi) = self.blur.ordered();
        if self.blur_ends && (lo < 0.0 || hi > MAX_BLUR) {
            return Err(format!("虚化强度需在 0～{MAX_BLUR} 之间"));
        }

        let (w, h) = (ctx.stream.width, ctx.stream.height);
        let vertical = match self.direction {
            GridDirection::Auto => w >= h,
            GridDirection::Vertical => true,
            GridDirection::Horizontal => false,
        };
        let (cols, rows) = layout(self.count, vertical);

        let (wf, hf) = (f64::from(w), f64::from(h));
        let long_side = wf.max(hf);
        let scale = (long_side / (f64::from(cols) * wf).max(f64::from(rows) * hf)).min(1.0);
        let (cw, ch) = (even_down(wf * scale), even_down(hf * scale));

        // 需要虚化的格子
        let mut blurred = vec![false; self.count as usize];
        if self.blur_ends {
            let along = if vertical { rows } else { cols };
            if along < 3 {
                ctx.warn("两端虚化只对 3、6、9 宫格有效，已忽略");
            } else {
                for (i, b) in blurred.iter_mut().enumerate() {
                    let (col, row) = (i as u32 % cols, i as u32 / cols);
                    let pos = if vertical { row } else { col };
                    *b = pos == 0 || pos == along - 1;
                }
            }
        }
        let sigma = if blurred.iter().any(|b| *b) {
            self.blur.sample_rounded(&mut ctx.rng, 1)
        } else {
            0.0
        };

        let input = ctx.graph.take_video();
        let cells: Vec<String> = (0..self.count).map(|_| ctx.graph.label("g")).collect();
        let cell_refs: Vec<&str> = cells.iter().map(String::as_str).collect();
        ctx.graph.chain(
            &[&input],
            &format!("scale={cw}:{ch},setsar=1,split={}", self.count),
            &cell_refs,
        );
        let mut stack_inputs = Vec::with_capacity(cells.len());
        for (cell, blur) in cells.iter().zip(&blurred) {
            if *blur && sigma > 0.0 {
                let out = ctx.graph.label("g");
                ctx.graph
                    .chain(&[cell], &format!("gblur=sigma={}", fmt_num(sigma)), &[&out]);
                stack_inputs.push(out);
            } else {
                stack_inputs.push(cell.clone());
            }
        }
        let out = ctx.graph.label("grid");
        let stack_refs: Vec<&str> = stack_inputs.iter().map(String::as_str).collect();
        ctx.graph.chain(
            &stack_refs,
            &format!("xstack=inputs={}:grid={cols}x{rows}", self.count),
            &[&out],
        );
        ctx.graph.set_video(out);
        ctx.stream.width = cw * cols;
        ctx.stream.height = ch * rows;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Recipe, SeededRng, Settings};
    use crate::pipeline::testing::{build_with, sample_info};

    fn recipe_with(params: GridParams) -> Recipe {
        let mut recipe = Recipe::disabled();
        recipe.modules.grid = params;
        recipe
    }

    fn filter(params: GridParams, seed: u64) -> String {
        let built = build_with(&sample_info(), &recipe_with(params), seed).unwrap();
        built.arg_after("-filter_complex").unwrap_or("").to_owned()
    }

    fn stream_after(params: &GridParams, portrait: bool) -> (u32, u32) {
        let mut info = sample_info();
        if portrait {
            let v = info.video.as_mut().unwrap();
            (v.width, v.height) = (720, 1280);
        }
        let settings = Settings::default();
        let mut ctx = BuildContext::new(&info, &settings, 0);
        ctx.rng = SeededRng::derive(1, ID);
        params.apply(&mut ctx).unwrap();
        (ctx.stream.width, ctx.stream.height)
    }

    fn grid(count: u32) -> GridParams {
        GridParams {
            enabled: true,
            count,
            ..GridParams::default()
        }
    }

    #[test]
    fn two_grid_auto_stacks_landscape_vertically() {
        assert_eq!(
            filter(grid(2), 1),
            "[0:v:0]scale=1136:640,setsar=1,split=2[g0][g1];[g0][g1]xstack=inputs=2:grid=1x2[grid2]"
        );
        assert_eq!(stream_after(&grid(2), false), (1136, 1280));
        // 竖屏自动横排
        assert_eq!(stream_after(&grid(2), true), (1280, 1136));
        // 强制方向
        let forced = GridParams {
            direction: GridDirection::Horizontal,
            ..grid(2)
        };
        assert_eq!(
            filter(forced.clone(), 1),
            "[0:v:0]scale=640:360,setsar=1,split=2[g0][g1];[g0][g1]xstack=inputs=2:grid=2x1[grid2]"
        );
        assert_eq!(stream_after(&forced, false), (1280, 360));
    }

    #[test]
    fn square_and_six_grids_have_sensible_sizes() {
        assert_eq!(stream_after(&grid(4), false), (1280, 720));
        assert_eq!(stream_after(&grid(9), false), (1278, 720));
        assert_eq!(stream_after(&grid(6), false), (1280, 1080));
        assert_eq!(stream_after(&grid(6), true), (1080, 1280));
        assert_eq!(stream_after(&grid(3), false), (758, 1278));
    }

    #[test]
    fn blur_ends_blurs_outer_cells() {
        let params = GridParams {
            blur_ends: true,
            ..grid(3)
        };
        assert_eq!(
            filter(params, 1),
            "[0:v:0]scale=758:426,setsar=1,split=3[g0][g1][g2];\
             [g0]gblur=sigma=20[g3];[g2]gblur=sigma=20[g4];\
             [g3][g1][g4]xstack=inputs=3:grid=1x3[grid5]"
        );
        // 3x3 竖排：首尾两行虚化，中间一行清晰
        let nine = GridParams {
            blur_ends: true,
            ..grid(9)
        };
        let fc = filter(nine, 1);
        assert_eq!(fc.matches("gblur").count(), 6, "{fc}");
        assert!(
            fc.contains("[g9][g10][g11][g3][g4][g5][g12][g13][g14]xstack"),
            "{fc}"
        );
    }

    #[test]
    fn blur_ends_on_two_grid_warns() {
        let params = GridParams {
            blur_ends: true,
            ..grid(2)
        };
        let built = build_with(&sample_info(), &recipe_with(params), 1).unwrap();
        assert!(!built
            .arg_after("-filter_complex")
            .unwrap()
            .contains("gblur"));
        assert!(built.warnings.iter().any(|w| w.contains("两端虚化")));
    }

    #[test]
    fn blur_range_is_reproducible_and_bounded() {
        let params = GridParams {
            blur_ends: true,
            blur: Range::new(10.0, 30.0),
            ..grid(3)
        };
        assert_eq!(filter(params.clone(), 3), filter(params.clone(), 3));
        for seed in 0..20 {
            let fc = filter(params.clone(), seed);
            let sigma: f64 = fc
                .split("gblur=sigma=")
                .nth(1)
                .unwrap()
                .split('[')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            assert!((10.0..=30.0).contains(&sigma), "{fc}");
        }
    }

    #[test]
    fn invalid_params_are_rejected() {
        assert!(build_with(&sample_info(), &recipe_with(grid(5)), 1).is_err());
        let bad = GridParams {
            blur_ends: true,
            blur: Range::new(0.0, 500.0),
            ..grid(3)
        };
        let err = build_with(&sample_info(), &recipe_with(bad), 1).unwrap_err();
        assert!(err.contains("虚化"), "{err}");
    }
}
