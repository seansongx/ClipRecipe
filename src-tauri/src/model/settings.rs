//! 应用设置（docs/specs/settings.md），以 JSON 保存在应用配置目录。
//!
//! 新增设置项：在结构体里加字段并在 `Default` 里给默认值即可；
//! 因为整个结构体带 `#[serde(default)]`，旧的配置文件仍能读取。删掉的设置项（例如 0.1.0 的 `theme`）
//! 直接从结构体去掉：读取时忽略未知字段（不要加 `deny_unknown_fields`），下次保存时不再写出。
//! 前端 `src/lib/backend.ts` 的 `Settings` / `DEFAULT_SETTINGS` 必须同步（`tests/recipe_defaults.rs` 会检查）。

use std::path::Path;

use serde::{Deserialize, Serialize};

pub const SETTINGS_VERSION: u32 = 1;
pub const SETTINGS_FILE: &str = "settings.json";
/// `set.name-template` 的默认值：输出文件名取（去除后的）源文件名。
pub const DEFAULT_NAME_TEMPLATE: &str = "{name}";

/// `set.format`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetFormat {
    #[default]
    Mp4,
    /// 保持源文件的容器格式（不支持 H.264/H.265 的容器会改用 mp4）
    Source,
}

/// `set.exists`
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OnExists {
    #[default]
    Rename,
    Overwrite,
}

/// `set.after-all`：全部任务结束后的动作。只对一次运行有效：每次启动时重置为 `None`，触发后前端也会改回 `None`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AfterAll {
    #[default]
    None,
    /// 30 秒倒计时（可取消）后关机
    Shutdown,
    /// 退出程序
    Quit,
}

/// 烧录字幕（`set.auto-subtitle` 与队列里手动关联的字幕）的样式。
/// 尺寸都以 1080 高的画面为基准，按实际画面高度等比缩放。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleStyle {
    /// 字体文件；空时自动选择支持中文的系统字体
    pub font: String,
    /// 字号
    pub size: u32,
    /// 文字颜色 `#rrggbb`
    pub color: String,
    /// 描边颜色 `#rrggbb`
    pub outline_color: String,
    /// 描边宽度；0 为不描边
    pub outline: f64,
    /// 距底部的距离
    pub margin_bottom: u32,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font: String::new(),
            size: 48,
            color: "#ffffff".into(),
            outline_color: "#000000".into(),
            outline: 2.0,
            margin_bottom: 60,
        }
    }
}

/// `set.name-strip`：从输出文件名中去掉的内容。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NameStrip {
    /// 要去掉的文本；多个用换行分隔（正则模式下每行一个表达式）
    pub text: String,
    pub regex: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub version: u32,

    // ---- 4.1 转换 ----
    pub target_format: TargetFormat,
    pub on_exists: OnExists,
    pub name_strip: NameStrip,
    /// `set.name-template`：批量处理输出文件名的格式（变量见 `jobs::output::render_name`）
    pub name_template: String,
    /// `set.parallel`：并行任务数，实际运行时限制在 1..=CPU 核心数
    pub parallel: u32,
    /// `set.hevc`
    pub hevc: bool,
    /// `set.skip-done`
    pub skip_done: bool,
    /// `set.auto-subtitle`：导入时自动关联同目录同名字幕（srt/ass/ssa/vtt），处理时烧录进画面
    pub auto_subtitle: bool,
    /// 烧录字幕的样式
    pub subtitle_style: SubtitleStyle,
    /// `set.auto-cover`：导入时自动关联同目录同名图片作封面
    pub auto_cover: bool,
    /// 封面嵌入帧数：不超过帧率时盖住开头这几帧；超过帧率时用图片生成这么多帧的片段接在开头
    pub cover_frames: u32,
    /// 封面拉伸至视频尺寸（false 时等比缩放、空白处填黑）
    pub cover_stretch: bool,
    /// `set.hdr`：HDR 源转为 SDR（false 时保留 HDR：10 bit H.265 并保留色彩信息）
    pub hdr_to_sdr: bool,
    /// `set.blur-center`：分辨率 / 画中画模块虚化背景的默认取法（模块参数 `blurCenter` 为空时使用）
    pub blur_center: bool,

    // ---- 4.2 通用 ----
    /// `set.gpu`
    pub gpu: bool,
    /// `set.gpu`：有多个可用硬件编码器时核显（Intel QSV、AMD AMF）优先
    pub prefer_integrated_gpu: bool,
    /// `set.gpu`：不再提示“硬件编码器试编码失败（可能驱动过旧）”
    pub gpu_hint_dismissed: bool,
    /// `set.number-prefix`
    pub number_prefix: bool,
    /// `set.recursive`：导入文件夹时包含子目录；也是新建“目录”素材时 `recursive` 的初始值
    pub recursive: bool,
    /// `set.out-by-folder`
    pub out_by_folder: bool,
    /// `set.out-by-name`
    pub out_by_name: bool,
    /// `set.after-all`
    pub after_all: AfterAll,
    /// `set.open-output`
    pub open_output: bool,
    /// `set.sound`
    pub sound: bool,
    /// `set.sound`：自定义提示音（wav）；空时使用内置提示音
    pub sound_path: String,
    /// `set.delete-source`：成功后删除源视频
    pub delete_source: bool,
    /// `set.delete-source`：移入回收站（false 时永久删除）
    pub delete_to_trash: bool,
    /// `set.move-back`：输出移回源视频所在目录
    pub move_back: bool,
    /// `set.remove-task`：完成后从队列移除
    pub remove_finished: bool,
    /// `set.remove-task`：手动删除任务时确认
    pub confirm_remove: bool,
    /// `set.keep-queue`：保留未完成的任务，下次启动时载入
    pub keep_queue: bool,
    /// `set.auto-update`：启动时检查新版本
    pub auto_update: bool,
    /// `set.tray`：关闭窗口时最小化到托盘
    pub tray: bool,
    /// `set.mute-preview`
    pub mute_preview: bool,

    // ---- 4.3 高级开关 ----
    /// `set.fission-folder`：裂变次数 >1 时，每次裂变输出到“第 x 次裂变”目录
    pub fission_folder: bool,
    /// `set.speed-fit`：独立工具的输出时长与目标时长不一致时变速适应（见 `pipeline::speed_fit`）
    pub speed_fit: bool,
    /// `set.reduce-size`：体积压缩（“瘦身”），见 `VideoEncoder::slim_args`
    pub reduce_size: bool,
    /// `set.copyright`：写入输出元数据的版权信息；空为不写
    pub copyright: String,

    /// `pipe.output`：上次选择的输出目录
    pub output_dir: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            target_format: TargetFormat::Mp4,
            on_exists: OnExists::Rename,
            name_strip: NameStrip::default(),
            name_template: DEFAULT_NAME_TEMPLATE.into(),
            parallel: 2,
            hevc: false,
            skip_done: true,
            auto_subtitle: false,
            subtitle_style: SubtitleStyle::default(),
            auto_cover: false,
            cover_frames: 1,
            cover_stretch: true,
            hdr_to_sdr: true,
            blur_center: true,
            gpu: true,
            prefer_integrated_gpu: false,
            gpu_hint_dismissed: false,
            number_prefix: false,
            recursive: true,
            out_by_folder: false,
            out_by_name: false,
            after_all: AfterAll::None,
            open_output: false,
            sound: true,
            sound_path: String::new(),
            delete_source: false,
            delete_to_trash: true,
            move_back: false,
            remove_finished: false,
            confirm_remove: true,
            keep_queue: true,
            auto_update: false,
            tray: false,
            mute_preview: false,
            fission_folder: false,
            speed_fit: false,
            reduce_size: false,
            copyright: String::new(),
            output_dir: String::new(),
        }
    }
}

pub fn cpu_cores() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

impl Settings {
    /// 实际使用的并行数：1..=CPU 核心数。
    pub fn effective_parallel(&self) -> usize {
        (self.parallel.max(1) as usize).min(cpu_cores().max(1))
    }

    /// 读取设置；文件不存在时返回默认值，内容损坏时返回错误（调用方可决定是否用默认值）。
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("设置文件损坏（{}）：{e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("无法读取设置文件（{}）：{e}", path.display())),
        }
    }

    /// 启动时读取设置：“全部完成后”只对一次运行有效，每次启动都恢复为“无操作”。
    pub fn load_for_startup(path: &Path) -> Result<Self, String> {
        let mut settings = Self::load(path)?;
        settings.after_all = AfterAll::None;
        Ok(settings)
    }

    /// 保存设置（先写临时文件再替换，避免写一半时崩溃损坏配置）。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("无法创建配置目录：{e}"))?;
        }
        let mut value = self.clone();
        value.version = SETTINGS_VERSION;
        let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("无法写入设置：{e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("无法保存设置：{e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_missing_fields_use_defaults() {
        let dir = std::env::temp_dir().join(format!("cr-settings-{}", std::process::id()));
        let path = dir.join(SETTINGS_FILE);
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());

        let settings = Settings {
            parallel: 3,
            on_exists: OnExists::Overwrite,
            name_strip: NameStrip {
                text: "_raw".into(),
                regex: false,
            },
            copyright: "© 2026".into(),
            name_template: "{date}_{name}".into(),
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), settings);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains(r#""nameTemplate": "{date}_{name}""#));

        std::fs::write(&path, r#"{"hevc":true,"targetFormat":"source"}"#).unwrap();
        let partial = Settings::load(&path).unwrap();
        assert!(partial.hevc);
        assert_eq!(partial.target_format, TargetFormat::Source);
        assert_eq!(partial.parallel, Settings::default().parallel);

        std::fs::write(&path, "not json").unwrap();
        assert!(Settings::load(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_files_load_and_after_all_resets_on_startup() {
        let dir = std::env::temp_dir().join(format!("cr-settings-old-{}", std::process::id()));
        let path = dir.join(SETTINGS_FILE);
        std::fs::create_dir_all(&dir).unwrap();
        // M1 时代的设置文件：没有任何 M4 字段（外加两个 M4 字段的部分内容）
        std::fs::write(
            &path,
            r#"{"version":1,"targetFormat":"mp4","onExists":"rename","parallel":3,"gpu":false,
               "afterAll":"shutdown","subtitleStyle":{"size":30}}"#,
        )
        .unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.parallel, 3);
        assert!(!loaded.gpu);
        assert_eq!(loaded.after_all, AfterAll::Shutdown);
        assert_eq!(loaded.subtitle_style.size, 30);
        assert_eq!(loaded.subtitle_style.color, "#ffffff");
        assert!(loaded.delete_to_trash && !loaded.delete_source);
        assert_eq!(loaded.cover_frames, 1);
        // 输出文件名格式（set.name-template）出现之前的文件：取默认值，输出名与以前一样
        assert_eq!(loaded.name_template, DEFAULT_NAME_TEMPLATE);
        assert_eq!(loaded.name_template, "{name}");
        let startup = Settings::load_for_startup(&path).unwrap();
        assert_eq!(startup.after_all, AfterAll::None);
        assert_eq!(startup.parallel, 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 已删除的设置项（0.1.0 的 `theme`：跟随系统 / 浅色 / 深色）留在旧文件里时照常读取、其它设置不丢，保存后不再写出。
    #[test]
    fn removed_fields_are_ignored() {
        let dir = std::env::temp_dir().join(format!("cr-settings-removed-{}", std::process::id()));
        let path = dir.join(SETTINGS_FILE);
        std::fs::create_dir_all(&dir).unwrap();
        let expected = Settings {
            parallel: 3,
            mute_preview: true,
            copyright: "x".into(),
            ..Settings::default()
        };
        for theme in ["light", "system", "dark"] {
            std::fs::write(
                &path,
                format!(
                    r#"{{"version":1,"parallel":3,"theme":"{theme}","mutePreview":true,"copyright":"x"}}"#
                ),
            )
            .unwrap();
            assert_eq!(Settings::load(&path).unwrap(), expected);
        }
        Settings::load(&path).unwrap().save(&path).unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("\"theme\""));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parallel_is_clamped() {
        let s = Settings {
            parallel: 0,
            ..Settings::default()
        };
        assert_eq!(s.effective_parallel(), 1);
        let s = Settings {
            parallel: 10_000,
            ..Settings::default()
        };
        assert_eq!(s.effective_parallel(), cpu_cores());
    }
}
