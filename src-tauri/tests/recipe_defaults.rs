//! 守卫：前端各模块的 `defaults`（src/features/pipeline/<模块>.tsx）必须与后端参数结构体的
//! `Default` 一致。
//!
//! 做法：运行 `node scripts/dump-recipe-defaults.mjs`（借助 Vite 直接执行前端源码，得到
//! `defaultRecipe()`），与 `Recipe::default()` 的 JSON 逐字段比较。实现模块时只需改自己的
//! `.rs` / `.tsx`，不需要维护任何共享的快照文件。
//!
//! 独立工具同理：`node scripts/dump-recipe-defaults.mjs tools` 输出每个工具的 `defaults` 与 `inputs`，
//! 与后端 `tools::default_params()`、`tools::input_declarations()` 比较。
//!
//! 需要 Node.js 并已执行 `npm install`。确实无法运行 Node 时可设置环境变量
//! `CLIPRECIPE_SKIP_TS_DEFAULTS=1` 跳过（会打印提示，CI 与交付验收时不要设置）。

use std::path::Path;
use std::process::Command;

use cliprecipe_lib::model::Recipe;
use cliprecipe_lib::tools;
use serde_json::Value;

fn frontend_defaults() -> Option<Value> {
    run_dump_script(&[])
}

/// 运行导出脚本；`args` 为空时输出默认配方，`["tools"]` 时输出全部独立工具的默认参数与输入声明。
fn run_dump_script(args: &[&str]) -> Option<Value> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let script = root.join("scripts").join("dump-recipe-defaults.mjs");
    let output = match Command::new("node")
        .arg(&script)
        .args(args)
        .current_dir(&root)
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            if std::env::var_os("CLIPRECIPE_SKIP_TS_DEFAULTS").is_some() {
                println!("跳过前端默认值检查：无法运行 node（{e}）");
                return None;
            }
            panic!(
                "无法运行 node（{e}）。此测试需要 Node.js 与 npm install；\
                 确实无法运行时可设置 CLIPRECIPE_SKIP_TS_DEFAULTS=1 跳过"
            );
        }
    };
    assert!(
        output.status.success(),
        "运行 {} 失败：\n{}",
        script.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(serde_json::from_slice(&output.stdout).expect("脚本应输出 JSON"))
}

/// 递归比较；数字按数值比较（前端 `30` 等于后端 `30.0`），
/// 前端缺少的键视为与后端的 `null` 相同（反序列化时会取默认值 None）。
fn diff(path: &str, backend: &Value, frontend: Option<&Value>, out: &mut Vec<String>) {
    match (backend, frontend) {
        (Value::Null, None) => {}
        (_, None) => out.push(format!("{path}：前端缺少，后端默认为 {backend}")),
        (Value::Number(a), Some(Value::Number(b))) => {
            let (a, b) = (
                a.as_f64().unwrap_or(f64::NAN),
                b.as_f64().unwrap_or(f64::NAN),
            );
            if (a - b).abs() > 1e-9 {
                out.push(format!("{path}：后端 {a}，前端 {b}"));
            }
        }
        (Value::Object(a), Some(Value::Object(b))) => {
            for (key, value) in a {
                diff(&format!("{path}.{key}"), value, b.get(key), out);
            }
            for key in b.keys().filter(|k| !a.contains_key(*k)) {
                out.push(format!("{path}.{key}：前端有，后端参数结构体没有"));
            }
        }
        (Value::Array(a), Some(Value::Array(b))) => {
            if a.len() != b.len() {
                out.push(format!(
                    "{path}：数组长度 后端 {}，前端 {}",
                    a.len(),
                    b.len()
                ));
            }
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                diff(&format!("{path}[{i}]"), x, Some(y), out);
            }
        }
        (a, Some(b)) => {
            if a != b {
                out.push(format!("{path}：后端 {a}，前端 {b}"));
            }
        }
    }
}

#[test]
fn frontend_defaults_match_backend_defaults() {
    let Some(frontend) = frontend_defaults() else {
        return;
    };
    let backend = serde_json::to_value(Recipe::default()).unwrap();
    let mut problems = Vec::new();
    diff("recipe", &backend, Some(&frontend), &mut problems);
    assert!(
        problems.is_empty(),
        "前端 defaults 与后端 Default 不一致（修改对应模块的 .tsx 或 .rs）：\n{}",
        problems.join("\n")
    );
}

/// 设置：前端 `lib/backend.ts` 的 `DEFAULT_SETTINGS` 必须与 `Settings::default()` 逐字段一致
/// （新增设置项时两边都要加，前端多出或缺少的键都会报出来）。
#[test]
fn frontend_settings_defaults_match_backend() {
    let Some(frontend) = run_dump_script(&["settings"]) else {
        return;
    };
    let backend = serde_json::to_value(cliprecipe_lib::model::Settings::default()).unwrap();
    let mut problems = Vec::new();
    diff("settings", &backend, Some(&frontend), &mut problems);
    assert!(
        problems.is_empty(),
        "前端 DEFAULT_SETTINGS 与后端 Settings::default() 不一致（修改 src/lib/backend.ts 或 model/settings.rs）：\n{}",
        problems.join("\n")
    );
}

/// 独立工具：前端 `features/tools/<工具>.tsx` 的 `defaults`、`inputs` 必须与后端参数结构体的
/// `Default`、`Tool::inputs()` 一致（文件夹输入只比较键），registry.ts 中标记为 done 的工具
/// 必须正是后端 `implemented()` 为 true 的工具；`preview` 与 `Tool::supports_preview()` 一致。实现工具时同样只改自己的文件（加 registry.ts 的状态）。
#[test]
fn frontend_tool_declarations_match_backend() {
    let Some(frontend) = run_dump_script(&["tools"]) else {
        return;
    };
    let defaults = tools::default_params();
    let inputs = tools::input_declarations();
    let backend: serde_json::Map<String, Value> = tools::TOOL_IDS
        .iter()
        .map(|id| {
            let declared = &inputs[*id];
            let folders: Vec<Value> = declared["folders"]
                .as_array()
                .map(|list| list.iter().map(|f| f["key"].clone()).collect())
                .unwrap_or_default();
            let value = serde_json::json!({
                "implemented": tools::default_tool(id).is_some_and(|t| t.implemented()),
                "defaults": defaults[*id],
                "inputs": { "videos": declared["videos"], "folders": folders },
                "preview": tools::default_tool(id).is_some_and(|t| t.supports_preview()),
            });
            ((*id).to_owned(), value)
        })
        .collect();
    let mut problems = Vec::new();
    diff(
        "tools",
        &Value::Object(backend),
        Some(&frontend),
        &mut problems,
    );
    assert!(
        problems.is_empty(),
        "前端工具定义与后端不一致（修改对应工具的 .tsx 或 .rs）：\n{}",
        problems.join("\n")
    );
}

#[test]
fn diff_reports_mismatches() {
    let backend: Value = serde_json::json!({"a": 30.0, "b": {"c": null, "d": [1, 2]}, "e": "x"});
    let mut out = Vec::new();
    diff(
        "r",
        &backend,
        Some(&serde_json::json!({"a": 30, "b": {"d": [1, 2]}, "e": "x"})),
        &mut out,
    );
    assert!(out.is_empty(), "{out:?}");
    diff(
        "r",
        &backend,
        Some(&serde_json::json!({"a": 31, "b": {"d": [1], "z": 0}})),
        &mut out,
    );
    assert_eq!(
        out,
        [
            "r.a：后端 30，前端 31",
            "r.b.d：数组长度 后端 2，前端 1",
            "r.b.z：前端有，后端参数结构体没有",
            "r.e：前端缺少，后端默认为 \"x\"",
        ]
    );
}
