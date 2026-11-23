//! 配方：流水线全部模块的参数，可整体保存 / 导入。
//!
//! JSON 形如 `{"version":1,"modules":{"pipe.fps":{"enabled":true,...},...}}`，
//! `modules` 的键就是功能 ID（与前端 registry.ts 一致）。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::pipeline::modules::*;

pub const RECIPE_VERSION: u32 = 1;

/// `pipe.fission` 裂变次数的上限。
pub const MAX_FISSION: u32 = 100;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Recipe {
    pub version: u32,
    /// `pipe.fission`：每个视频生成几个输出（1 表示不裂变），实际使用时限制在 1..=[`MAX_FISSION`]
    pub fission: u32,
    pub modules: RecipeModules,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            version: RECIPE_VERSION,
            fission: 1,
            modules: RecipeModules::default(),
        }
    }
}

/// 宽松读取配方文件的结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedRecipe {
    pub recipe: Recipe,
    /// 被忽略或改用默认值的内容（显示给用户）
    pub warnings: Vec<String>,
}

/// 全部 18 个流水线模块的参数（字段顺序同 registry.ts）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecipeModules {
    #[serde(rename = "pipe.remove-watermark")]
    pub remove_watermark: RemoveWatermarkParams,
    #[serde(rename = "pipe.add-watermark")]
    pub add_watermark: AddWatermarkParams,
    #[serde(rename = "pipe.pip")]
    pub pip: PipParams,
    #[serde(rename = "pipe.text")]
    pub text: TextParams,

    #[serde(rename = "pipe.crop")]
    pub crop: CropParams,
    #[serde(rename = "pipe.resolution")]
    pub resolution: ResolutionParams,
    #[serde(rename = "pipe.rotate-flip")]
    pub rotate_flip: RotateFlipParams,
    #[serde(rename = "pipe.adjust")]
    pub adjust: AdjustParams,
    #[serde(rename = "pipe.grid")]
    pub grid: GridParams,
    #[serde(rename = "pipe.zoom")]
    pub zoom: ZoomParams,

    #[serde(rename = "pipe.trim")]
    pub trim: TrimParams,
    #[serde(rename = "pipe.intro-outro")]
    pub intro_outro: IntroOutroParams,
    #[serde(rename = "pipe.speed")]
    pub speed: SpeedParams,
    #[serde(rename = "pipe.drop-frames")]
    pub drop_frames: DropFramesParams,
    #[serde(rename = "pipe.dedupe-frames")]
    pub dedupe_frames: DedupeFramesParams,

    #[serde(rename = "pipe.bgm")]
    pub bgm: BgmParams,

    #[serde(rename = "pipe.fps")]
    pub fps: FpsParams,
    #[serde(rename = "pipe.bitrate")]
    pub bitrate: BitrateParams,
}

impl Recipe {
    /// 所有模块都关闭的配方（测试里常用作起点）。
    pub fn disabled() -> Self {
        let mut recipe = Self::default();
        recipe.modules.fps.enabled = false;
        recipe
    }

    /// 解析配方 JSON；拒绝比当前程序更新的版本。
    pub fn from_json(text: &str) -> Result<Self, String> {
        let recipe: Recipe =
            serde_json::from_str(text).map_err(|e| format!("配方文件无法解析：{e}"))?;
        if recipe.version > RECIPE_VERSION {
            return Err(format!(
                "配方版本 {} 高于当前程序支持的 {}，请升级 ClipRecipe",
                recipe.version, RECIPE_VERSION
            ));
        }
        Ok(recipe)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("配方总能序列化")
    }

    /// 实际使用的裂变次数（1..=MAX_FISSION）。
    pub fn fission_count(&self) -> usize {
        self.fission.clamp(1, MAX_FISSION) as usize
    }

    /// 宽松地读取配方文件（导入用）：
    ///
    /// - 版本高于当前程序时报错；不是 JSON 对象时报错；
    /// - 缺失的模块、字段取默认值；
    /// - 未知模块被忽略，类型不对的字段改用默认值，二者都会给出提示。
    pub fn from_json_lenient(text: &str) -> Result<LoadedRecipe, String> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| format!("配方文件不是有效的 JSON：{e}"))?;
        let input = value
            .as_object()
            .ok_or_else(|| "配方文件格式不正确".to_owned())?;
        if let Some(version) = input.get("version") {
            match version.as_u64() {
                Some(v) if v > u64::from(RECIPE_VERSION) => {
                    return Err(format!(
                        "配方版本 {v} 高于当前程序支持的 {RECIPE_VERSION}，请升级 ClipRecipe"
                    ))
                }
                Some(_) => {}
                None => return Err("配方文件的版本号无效".into()),
            }
        }
        let mut warnings = Vec::new();
        let mut recipe = Recipe::default();
        if let Some(fission) = input.get("fission") {
            match serde_json::from_value::<u32>(fission.clone()) {
                Ok(n) => recipe.fission = n.clamp(1, MAX_FISSION),
                Err(_) => warnings.push("裂变次数无效，已使用默认值".to_owned()),
            }
        }
        let mut modules = serde_json::to_value(&recipe.modules).map_err(|e| e.to_string())?;
        match input.get("modules") {
            None => {}
            Some(Value::Object(saved_modules)) => {
                for (id, saved) in saved_modules {
                    let Some(default) = modules.get(id).cloned() else {
                        warnings.push(format!("忽略未知模块 {id}"));
                        continue;
                    };
                    modules[id] = merge_module(id, default, saved, &mut warnings);
                }
            }
            Some(_) => warnings.push("配方中的模块列表格式不正确，已全部使用默认值".to_owned()),
        }
        recipe.modules = serde_json::from_value(modules).map_err(|e| e.to_string())?;
        Ok(LoadedRecipe { recipe, warnings })
    }

    /// 打开或关闭某个模块（按功能 ID）。
    pub fn set_module_enabled(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        let mut modules = serde_json::to_value(&self.modules).map_err(|e| e.to_string())?;
        let params = modules
            .get_mut(id)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("未知模块：{id}"))?;
        params.insert("enabled".into(), Value::Bool(enabled));
        self.modules = serde_json::from_value(modules).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// 某个模块的参数能否被后端接受。
fn module_accepts(id: &str, params: &Value) -> bool {
    let mut modules = Map::new();
    modules.insert(id.to_owned(), params.clone());
    serde_json::from_value::<RecipeModules>(Value::Object(modules)).is_ok()
}

/// 把文件里某个模块的参数合并到默认值上：整体可用就直接用，否则逐个字段检查。
fn merge_module(id: &str, default: Value, saved: &Value, warnings: &mut Vec<String>) -> Value {
    if module_accepts(id, saved) {
        return saved.clone();
    }
    let Some(fields) = saved.as_object() else {
        warnings.push(format!("{id} 的参数格式不正确，已使用默认值"));
        return default;
    };
    let mut merged = default;
    for (key, value) in fields {
        let mut trial = merged.clone();
        trial[key] = value.clone();
        if module_accepts(id, &trial) {
            merged = trial;
        } else {
            warnings.push(format!("{id} 的参数 {key} 无效，已使用默认值"));
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_enables_only_fps() {
        let recipe = Recipe::default();
        let enabled: Vec<&str> = crate::pipeline::ordered_modules(&recipe.modules)
            .iter()
            .filter(|m| m.enabled())
            .map(|m| m.id())
            .collect();
        assert_eq!(enabled, ["pipe.fps"]);
    }

    #[test]
    fn json_keys_are_feature_ids() {
        let value = serde_json::to_value(Recipe::default()).unwrap();
        let mut keys: Vec<&String> = value["modules"].as_object().unwrap().keys().collect();
        keys.sort();
        let mut ids: Vec<&str> = crate::pipeline::MODULE_IDS.to_vec();
        ids.sort();
        assert_eq!(keys, ids);
    }

    #[test]
    fn roundtrip_and_partial_input() {
        let mut recipe = Recipe::default();
        recipe.modules.bitrate.enabled = true;
        assert_eq!(Recipe::from_json(&recipe.to_json()).unwrap(), recipe);

        let partial =
            Recipe::from_json(r#"{"version":1,"modules":{"pipe.bitrate":{"enabled":true}}}"#)
                .unwrap();
        assert!(partial.modules.bitrate.enabled);
        assert!(partial.modules.fps.enabled, "缺失的模块取默认值");

        assert!(Recipe::from_json(r#"{"version":99}"#).is_err());
    }

    #[test]
    fn fission_defaults_to_one_and_is_clamped() {
        let recipe = Recipe::from_json(r#"{"version":1}"#).unwrap();
        assert_eq!(recipe.fission, 1);
        let mut recipe = Recipe {
            fission: 0,
            ..Recipe::default()
        };
        assert_eq!(recipe.fission_count(), 1);
        recipe.fission = 5000;
        assert_eq!(recipe.fission_count(), MAX_FISSION as usize);
    }

    #[test]
    fn lenient_load_tolerates_missing_unknown_and_invalid() {
        let loaded = Recipe::from_json_lenient(
            r#"{
                "version": 1,
                "fission": 3,
                "modules": {
                    "pipe.fps": {"enabled": false, "fps": "fast", "extra": 1},
                    "pipe.bitrate": {"enabled": true, "mode": "fixed"},
                    "pipe.crop": 42,
                    "pipe.unknown": {"enabled": true}
                }
            }"#,
        )
        .unwrap();
        let r = &loaded.recipe;
        assert_eq!(r.fission, 3);
        assert!(!r.modules.fps.enabled, "有效字段保留");
        assert_eq!(
            r.modules.fps.fps,
            FpsParams::default().fps,
            "无效字段取默认值"
        );
        assert!(r.modules.bitrate.enabled);
        assert_eq!(r.modules.bitrate.mode, BitrateMode::Fixed);
        assert_eq!(r.modules.crop, CropParams::default());
        assert_eq!(r.modules.trim, TrimParams::default(), "缺失的模块取默认值");
        let w = loaded.warnings.join("\n");
        assert!(w.contains("pipe.fps 的参数 fps 无效"), "{w}");
        assert!(w.contains("pipe.crop 的参数格式不正确"), "{w}");
        assert!(w.contains("忽略未知模块 pipe.unknown"), "{w}");
        assert_eq!(loaded.warnings.len(), 3, "{w}");
    }

    #[test]
    fn lenient_load_checks_version_and_shape() {
        assert!(Recipe::from_json_lenient(r#"{"version":2}"#)
            .unwrap_err()
            .contains("请升级"));
        assert!(Recipe::from_json_lenient("[1,2]").is_err());
        assert!(Recipe::from_json_lenient("not json").is_err());
        // 没有版本号的旧文件按当前版本读取
        let loaded = Recipe::from_json_lenient(r#"{"fission":"x"}"#).unwrap();
        assert_eq!(loaded.recipe, Recipe::default());
        assert_eq!(loaded.warnings, ["裂变次数无效，已使用默认值"]);
        // 完整往返没有任何提示
        let mut recipe = Recipe {
            fission: 4,
            ..Recipe::default()
        };
        recipe.modules.bitrate.enabled = true;
        let loaded = Recipe::from_json_lenient(&recipe.to_json()).unwrap();
        assert_eq!(loaded.recipe, recipe);
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn set_module_enabled_by_id() {
        let mut recipe = Recipe::disabled();
        recipe.set_module_enabled("pipe.bitrate", true).unwrap();
        assert!(recipe.modules.bitrate.enabled);
        recipe.set_module_enabled("pipe.bitrate", false).unwrap();
        assert_eq!(recipe, Recipe::disabled());
        assert!(recipe.set_module_enabled("pipe.nope", true).is_err());
    }
}
