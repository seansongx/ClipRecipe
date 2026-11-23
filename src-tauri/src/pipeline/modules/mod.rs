//! 每个 `pipe.*` 模块一个文件：参数结构体 + `PipelineModule` 实现。
//!
//! 全部 18 个模块已在此注册（M1）；后续实现某个模块只需修改它自己的文件。
//! 应用顺序见 `pipeline::ordered_modules`。

pub mod add_watermark;
pub mod adjust;
pub mod bgm;
pub mod bitrate;
pub mod crop;
pub mod dedupe_frames;
pub mod drop_frames;
pub mod fps;
pub mod grid;
pub mod intro_outro;
pub mod pip;
pub mod remove_watermark;
pub mod resolution;
pub mod rotate_flip;
pub mod speed;
pub mod text;
pub mod trim;
pub mod zoom;

pub use add_watermark::AddWatermarkParams;
pub use adjust::AdjustParams;
pub use bgm::BgmParams;
pub use bitrate::{BitrateMode, BitrateParams};
pub use crop::CropParams;
pub use dedupe_frames::DedupeFramesParams;
pub use drop_frames::DropFramesParams;
pub use fps::FpsParams;
pub use grid::GridParams;
pub use intro_outro::IntroOutroParams;
pub use pip::PipParams;
pub use remove_watermark::RemoveWatermarkParams;
pub use resolution::ResolutionParams;
pub use rotate_flip::RotateFlipParams;
pub use speed::SpeedParams;
pub use text::TextParams;
pub use trim::TrimParams;
pub use zoom::ZoomParams;
