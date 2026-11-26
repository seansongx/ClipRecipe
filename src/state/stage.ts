// 批量处理页中栏“预览舞台”：当前视频套用当前配方后的一帧（以及裂变变体的小图）。
//
// 后端只有一个预览槽：render_preview / render_tool_preview 开始时会取消尚未完成的上一个，cancel_preview 取消当前那个
// （src-tauri/src/commands/preview.rs）。所以这里：
// - 同一轮刷新里主画面与各变体依次渲染（并发会互相取消）；新一轮直接开始，旧的一轮由后端取消、结果按 generation 丢弃；
// - 预览窗口（state/preview.ts）打开期间不渲染（由视图判断），以免取消它的片段；被它取消的这一轮标记为 interrupted，
//   窗口关闭后由视图重新触发；窗口没开却被取消（例如关闭窗口时的 cancel_preview 晚到）时最多自动重试几次。
// 单帧输出不发 preview-progress（预计时长为 0），不会串到预览窗口的进度上。

import { create } from "zustand";
import { PREVIEW_EXCLUDED } from "../features/pipeline";
import { mediaUrl, releasePreviewFile, renderPreview, type PreviewResult, type Recipe, type Settings } from "../lib/backend";
import { usePreview } from "./preview";
import { useRecipe } from "./recipe";
import { getSelectedVideo } from "./selection";
import { useSettings } from "./settings";

export type StageView = "effect" | "source";

/** 最多显示几个裂变变体。 */
export const MAX_VARIANTS = 3;
/** 主画面与变体小图的最大高度（像素）。 */
const MAIN_HEIGHT = 720;
const THUMB_HEIGHT = 180;
/** 不是因为预览窗口而被取消时，最多自动重试几次。 */
const MAX_RETRIES = 3;
/** 后端取消预览时的错误文字（preview.rs 的 PreviewError::Cancelled）。 */
const CANCELLED = "预览已取消";

export interface StageImage {
  path: string;
  url: string;
}

export interface StageThumb {
  image: StageImage | null;
  error: string | null;
}

interface StageState {
  view: StageView;
  /** 取帧时间（秒，预览输出的时间轴） */
  time: number;
  /** time 属于哪个视频（换视频时重置） */
  timeFor: string | null;
  /** 裂变变体的种子；不裂变时主画面固定用第一个 */
  seeds: number[];
  /** 选中的变体（主画面用它的种子） */
  variant: number;

  /** 最近一次开始渲染时的输入（视图据此判断是否需要刷新） */
  key: string | null;
  loading: boolean;
  image: StageImage | null;
  result: PreviewResult | null;
  error: string | null;
  thumbs: StageThumb[];
  thumbsLoading: boolean;
  /** 这一轮被其它预览取消了，需要重新渲染 */
  interrupted: boolean;

  setView: (view: StageView) => void;
  setTime: (time: number, itemKey?: string) => void;
  pickVariant: (index: number) => void;
  shuffle: () => void;
  /** 按当前视频、配方、设置与舞台状态渲染（视图防抖后调用；手动刷新直接调用） */
  render: () => Promise<void>;
  /** 卸载时：丢弃进行中的结果并删除全部文件 */
  dispose: () => void;
}

const randomSeed = () => Math.floor(Math.random() * 0xffff_ffff) + 1;
const randomSeeds = () => Array.from({ length: MAX_VARIANTS }, randomSeed);

/** 变体个数：fission > 1 时为 min(fission, 3)，否则 0。 */
export const variantCount = (fission: number) => (fission > 1 ? Math.min(fission, MAX_VARIANTS) : 0);

/** 影响预览画面的配方部分：不参与预览的模块、裂变次数、关闭的模块的参数都不算。 */
function recipeKey(recipe: Recipe): unknown {
  const modules: Record<string, unknown> = {};
  for (const [id, params] of Object.entries(recipe.modules)) {
    if (PREVIEW_EXCLUDED.includes(id)) continue;
    modules[id] = params.enabled ? params : false;
  }
  return modules;
}

/** 影响预览画面的设置（流水线里读到的、单帧预览里有效的那几项）。 */
function settingsKey(settings: Settings): unknown {
  return { blurCenter: settings.blurCenter, subtitleStyle: settings.subtitleStyle };
}

/** 一次渲染的全部输入；视图用它判断要不要刷新，render() 用它记录这一轮。 */
export function stageKey(
  video: { path: string; subtitle: string | null; number: number } | null,
  recipe: Recipe,
  settings: Settings,
  time: number,
  seeds: number[],
  variant: number,
): string | null {
  if (!video) return null;
  const count = variantCount(recipe.fission);
  return JSON.stringify([
    video.path,
    video.subtitle,
    video.number,
    recipeKey(recipe),
    settingsKey(settings),
    Math.round(time * 100) / 100,
    count > 0 ? seeds.slice(0, count) : seeds[0],
    count > 0 ? Math.min(variant, count - 1) : 0,
  ]);
}

function currentKey(state: StageState): string | null {
  const selected = getSelectedVideo();
  return stageKey(
    selected && { path: selected.item.path, subtitle: selected.item.subtitle, number: selected.number },
    useRecipe.getState().recipe,
    useSettings.getState().settings,
    state.time,
    state.seeds,
    state.variant,
  );
}

const errorText = (e: unknown) => String(e).replace(/^Error: /, "");

/** 换图后稍等再删旧文件：新图片加载出来之前 <img> 仍显示旧的。 */
function releaseLater(image: StageImage | null) {
  if (image) setTimeout(() => releasePreviewFile(image.path), 1000);
}

let generation = 0;
let retries = 0;

export const useStage = create<StageState>((set, get) => ({
  view: "effect",
  time: 0,
  timeFor: null,
  seeds: randomSeeds(),
  variant: 0,
  key: null,
  loading: false,
  image: null,
  result: null,
  error: null,
  thumbs: [],
  thumbsLoading: false,
  interrupted: false,

  setView: (view) => set({ view }),
  setTime: (time, itemKey) => set(itemKey === undefined ? { time } : { time, timeFor: itemKey }),
  pickVariant: (variant) => set({ variant }),
  shuffle: () => set({ seeds: randomSeeds() }),

  render: async () => {
    const selected = getSelectedVideo();
    const state = get();
    const key = currentKey(state);
    if (!selected || !key) {
      get().dispose();
      return;
    }
    const current = ++generation;
    if (key !== state.key) retries = 0;
    const recipe = useRecipe.getState().recipe;
    const settings = useSettings.getState().settings;
    const count = variantCount(recipe.fission);
    const seeds = state.seeds.slice(0, Math.max(1, count));
    const mainSeed = count > 0 ? seeds[Math.min(state.variant, count - 1)] : seeds[0];
    const base = {
      path: selected.item.path,
      info: selected.item.info,
      recipe,
      settings,
      onlyModule: null,
      output: { kind: "frame" as const, time: state.time },
      number: selected.number,
      subtitle: selected.item.subtitle,
    };

    /** 被取消：是本模块的新一轮取消的就忽略，否则标记为需要重新渲染。 */
    const onCancelled = () => {
      if (current !== generation) return;
      // 预览窗口打开时取消的不算重试次数：窗口关闭后视图会重新触发
      if (!usePreview.getState().open) retries += 1;
      if (retries > MAX_RETRIES) {
        set({ loading: false, thumbsLoading: false, interrupted: false, error: "预览被其它预览打断，请点刷新重试" });
        return;
      }
      set({ loading: false, thumbsLoading: false, interrupted: true });
    };

    const previousThumbs = get().thumbs;
    for (const dropped of previousThumbs.slice(count)) releaseLater(dropped.image);
    set({
      key,
      loading: true,
      error: null,
      interrupted: false,
      thumbsLoading: count > 0,
      thumbs: previousThumbs.slice(0, count),
    });

    let result: PreviewResult;
    try {
      result = await renderPreview({ ...base, seed: mainSeed, maxHeight: MAIN_HEIGHT });
    } catch (e) {
      const message = errorText(e);
      if (message === CANCELLED) return onCancelled();
      if (current !== generation) return;
      set({ loading: false, thumbsLoading: false, error: message });
      return;
    }
    if (current !== generation) {
      releasePreviewFile(result.path);
      return;
    }
    releaseLater(get().image);
    set({ loading: false, image: { path: result.path, url: mediaUrl(result.path) }, result });

    // 主画面之后依次渲染变体（并发会互相取消）
    for (let i = 0; i < count; i++) {
      let thumb: StageThumb;
      try {
        const r = await renderPreview({ ...base, seed: seeds[i], maxHeight: THUMB_HEIGHT });
        if (current !== generation) {
          releasePreviewFile(r.path);
          return;
        }
        thumb = { image: { path: r.path, url: mediaUrl(r.path) }, error: null };
      } catch (e) {
        const message = errorText(e);
        if (message === CANCELLED) return onCancelled();
        if (current !== generation) return;
        thumb = { image: null, error: message };
      }
      const thumbs = get().thumbs.slice();
      releaseLater(thumbs[i]?.image ?? null);
      thumbs[i] = thumb;
      set({ thumbs });
    }
    retries = 0;
    set({ thumbsLoading: false });
  },

  dispose: () => {
    generation++;
    retries = 0;
    const { image, thumbs } = get();
    if (image) releasePreviewFile(image.path);
    for (const t of thumbs) if (t.image) releasePreviewFile(t.image.path);
    set({ key: null, loading: false, image: null, result: null, error: null, thumbs: [], thumbsLoading: false, interrupted: false });
  },
}));
