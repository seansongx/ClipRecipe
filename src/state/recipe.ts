// 当前配方。为方便起见在本机 localStorage 里记住上次的参数（读写失败时忽略）。
// 配方文件的导入导出与参数重置见 pipe.recipe / pipe.reset。

import { create } from "zustand";
import { clampFission, defaultRecipe, normalizeRecipe } from "../features/pipeline";
import {
  loadRecipeFile,
  pickRecipeFile,
  pickRecipeSavePath,
  saveRecipeFile,
  type ModuleParams,
  type Recipe,
} from "../lib/backend";
import { fileName } from "../lib/format";
import { useLog } from "./log";

const STORAGE_KEY = "cliprecipe.recipe";

function loadSaved(): Recipe {
  try {
    const text = localStorage.getItem(STORAGE_KEY);
    if (text) return normalizeRecipe(JSON.parse(text));
  } catch {
    // 忽略：使用默认配方
  }
  return defaultRecipe();
}

function persist(recipe: Recipe) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(recipe));
  } catch {
    // 忽略：仅本次生效
  }
}

interface RecipeState {
  recipe: Recipe;
  /** 局部更新某个模块的参数 */
  updateModule: (id: string, patch: Partial<ModuleParams>) => void;
  setEnabled: (id: string, enabled: boolean) => void;
  replace: (recipe: unknown) => void;
  /** pipe.fission：裂变次数 */
  setFission: (n: number) => void;
  /** pipe.reset：恢复默认（仅启用帧率）。二次确认由界面（RecipePanel 的 AlertDialog）负责 */
  reset: () => void;
  /** pipe.recipe：从文件导入（path 为空时弹出选择框）；返回是否成功 */
  importFile: (path?: string) => Promise<boolean>;
  /** pipe.recipe：保存为文件 */
  exportFile: () => Promise<void>;
}

export const useRecipe = create<RecipeState>((set, get) => {
  const commit = (recipe: Recipe) => {
    persist(recipe);
    set({ recipe });
  };
  return {
    recipe: loadSaved(),
    updateModule: (id, patch) => {
      const { recipe } = get();
      const current = recipe.modules[id];
      if (!current) return;
      commit({ ...recipe, modules: { ...recipe.modules, [id]: { ...current, ...patch } } });
    },
    setEnabled: (id, enabled) => get().updateModule(id, { enabled }),
    replace: (input) => commit(normalizeRecipe(input)),
    setFission: (n) => commit({ ...get().recipe, fission: clampFission(n) }),

    reset: () => {
      commit(defaultRecipe());
      useLog.getState().add("info", "已恢复默认参数");
    },

    importFile: async (path) => {
      const log = useLog.getState();
      const file = path ?? (await pickRecipeFile());
      if (!file) return false;
      try {
        const loaded = await loadRecipeFile(file);
        commit(normalizeRecipe(loaded.recipe));
        log.add("info", `已导入配方：${fileName(file)}`);
        for (const warning of loaded.warnings) log.add("warn", `配方：${warning}`);
        return true;
      } catch (e) {
        log.add("error", `导入配方失败：${String(e)}`);
        return false;
      }
    },

    exportFile: async () => {
      const log = useLog.getState();
      const file = await pickRecipeSavePath("配方.json");
      if (!file) return;
      try {
        await saveRecipeFile(file, get().recipe);
        log.add("info", `已导出配方：${file}`);
      } catch (e) {
        log.add("error", `导出配方失败：${String(e)}`);
      }
    },
  };
});
