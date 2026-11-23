//! 配方文件的导入导出（`pipe.recipe`）。

use std::path::Path;

use crate::model::{LoadedRecipe, Recipe, RECIPE_VERSION};

/// 把配方保存为 JSON 文件。
#[tauri::command]
pub fn save_recipe_file(path: String, recipe: Recipe) -> Result<(), String> {
    save(Path::new(&path), &recipe)
}

/// 读取配方文件：版本过高时报错；缺失 / 无效的内容取默认值并给出提示。
#[tauri::command]
pub fn load_recipe_file(path: String) -> Result<LoadedRecipe, String> {
    load(Path::new(&path))
}

pub fn save(path: &Path, recipe: &Recipe) -> Result<(), String> {
    let mut recipe = recipe.clone();
    recipe.version = RECIPE_VERSION;
    std::fs::write(path, recipe.to_json())
        .map_err(|e| format!("无法保存配方到 {}：{e}", path.display()))
}

pub fn load(path: &Path) -> Result<LoadedRecipe, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("无法读取配方文件 {}：{e}", path.display()))?;
    Recipe::from_json_lenient(text.trim_start_matches('\u{feff}'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_then_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("cr-recipe-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("配方.json");
        let mut recipe = Recipe {
            fission: 3,
            ..Recipe::default()
        };
        recipe.modules.bitrate.enabled = true;
        save(&path, &recipe).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.recipe, recipe);
        assert!(loaded.warnings.is_empty());

        // 带 BOM 的文件（记事本保存）也能读
        std::fs::write(&path, "\u{feff}{\"version\":1,\"fission\":2}").unwrap();
        assert_eq!(load(&path).unwrap().recipe.fission, 2);
        assert!(load(&dir.join("missing.json")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
