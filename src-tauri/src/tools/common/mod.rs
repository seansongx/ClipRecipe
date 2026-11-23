//! 多个工具共用的引擎与小函数（不属于某一个工具）。新工具需要的公共逻辑放这里，不要从别的工具文件导入。
//!
//! | 模块 | 内容 | 使用者 |
//! |---|---|---|
//! | [`filter_job`] | 一进一出“滤镜图 + 重编码”命令 `FilterJob` / `run_job`、帧率 `Rate`、插入空档的音视频滤镜、`even_crop`、`color_hex`、图片输入参数 | 定帧动画、定格播放、插帧、运镜、去黑边、进度条 |
//! | [`positions`] | 时间 / 位置列表解析 `parse_time` / `parse_positions`、位置参数 `resolve_positions`（时间点 / 百分比 / 均分）、`position_frames`、`windows_expr` | 定格播放、插帧、进度条、拆分 / 移除重组 |
//! | [`cutlist`] | 片段列表 → 输出：`Timeline` / `Piece` / `SourceFormat` → `build_graph` / `render`（单遍 filter_complex，帧对齐，转场） | 移除 / 镜像 / 转场 / 拆分重组、静止帧 / 静音移除、智能去头尾 |
//! | [`merge_engine`] | 合并：`Segment`、`MergeOptions`、无损 / 重编码、`conform_target` / `conform_graph`、中间视频、转场、分组与打乱 | 合并、文件夹合并、片段合并 |
//! | [`frame`] | 画面规格：`color_arg`、`Fit` / `fit_scale`、`even`、`target_size` | 横竖互转、画面拼接、图转视频、批量截图 |
//! | [`scenes`] | 场景分段：`check_scene_params`、`scene_segments`、`merge_short` | 镜像重组、转场重组 |
//!
//! 原来放在各工具文件里的同名函数 / 类型仍可从原位置导入（再导出），新代码请从这里导入。

pub mod cutlist;
pub mod filter_job;
pub mod frame;
pub mod merge_engine;
pub mod positions;
pub mod scenes;
