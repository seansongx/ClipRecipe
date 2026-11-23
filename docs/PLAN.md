# 实施计划

功能范围见 [FEATURES.md](FEATURES.md)。本文定义里程碑、架构约定和验收标准。

## 里程碑

| 里程碑 | 内容 | 方式 |
|---|---|---|
| **M1 基础架构** | FFmpeg/FFprobe 定位与媒体探测；导入（文件、文件夹、递归、拖放）；配方数据模型（范围值、素材来源）；滤镜图生成框架；任务调度（并行、进度、取消、日志）；硬件编码探测与软件回退；输出命名规则；设置持久化；前端状态、队列、参数面板框架、处理记录、设置页。先接入 `pipe.fps`、`pipe.bitrate` 两个模块打通全链路 | 单任务 |
| **M2 流水线模块** | 其余 16 个 `pipe.*` 模块，加 `pipe.fission`、`pipe.preview`、`pipe.reset`、`pipe.recipe` | 按分组并行，各任务只改自己模块的文件 |
| **M3 独立工具** | 工具运行框架 + 34 个 `tool.*` | 先框架（单任务），后按分组并行 |
| **M4 设置与收尾** | FEATURES 第 4 节剩余设置项、托盘、完成后动作、队列恢复、文档更新 | 单任务 |

## 架构约定

### 后端 `src-tauri/src/`

```
ffmpeg/      定位可执行文件、probe（MediaInfo）、运行与进度解析、编码器探测；manifest / managed：应用内下载 FFmpeg
model/       Range（范围值）、AssetSource（文件/目录素材）、Recipe（配方）、Settings
pipeline/    mod.rs 负责把 MediaInfo + Recipe + 随机种子 变成一条 FFmpeg 命令
  modules/   每个 pipe.* 模块一个文件：参数结构体 + 向滤镜图追加自己的滤镜
jobs/        任务队列、调度、事件推送
tools/       每个 tool.* 一个文件：参数结构体 + 执行逻辑；框架：context / runner / plan / output / media / analysis/
commands/    暴露给前端的命令，按领域分文件
```

- FFmpeg 命令行只在后端拼装，前端只传结构化参数。
- 所有随机取值（范围值、目录随机素材、裂变）在后端用**可指定种子**的随机数完成，保证单元测试可复现。
- 参数结构体用 serde，字段 `camelCase`，都有 `Default`；配方文件带版本号。
- 模块之间互不依赖；应用顺序在 `pipeline/mod.rs` 里集中定义。
- 新增模块/工具不应需要改动别的模块文件。公共注册点（模块列表、命令注册）在 M1 一次性把全部 ID 占位好，后续任务只填自己的文件。

### 前端 `src/`

```
features/registry.ts     功能元数据（ID、名称、分组、状态）
features/pipeline/<id>   每个流水线模块的参数表单
features/tools/<id>      每个工具的参数表单
components/ui/           shadcn/ui 组件（用 CLI 生成，见下）
components/              基于 shadcn 组合的业务控件：范围输入、素材来源选择、区域框选、颜色等
state/                   队列、配方、设置、任务进度
lib/backend.ts           命令与事件封装，类型与后端 serde 结构一致
lib/utils.ts             cn()（合并 className）、scrollViewport()
```

- 界面文字用中文。
- **控件一律用 shadcn/ui**（Radix 版，`components.json`，导入别名 `@/` → `src/`）。shadcn 有的控件**不准手写**：
  Button、Switch、Input、Textarea、Label、Select、Slider、Checkbox、RadioGroup、Tabs、ToggleGroup、Dialog、AlertDialog、
  Tooltip、Popover、DropdownMenu、ScrollArea、Progress、Separator、Badge、Card、Collapsible、InputGroup、Field、Spinner 等；
  也不要写原生 `<button>` / `<input>` / `<select>` / `<textarea>`、原生取色器、自制遮罩/弹窗/提示框，或 `window.confirm`（用 AlertDialog）。
  需要的组件还没有时用 CLI 添加：`npx shadcn@latest add <组件>`（生成到 `components/ui/`，不要凭记忆手抄）。
  只有 shadcn 没有的业务控件才自己写，并且必须由 shadcn 组件组合而成（见“模块/工具编写指南”的控件表）。
- 样式用 Tailwind 和 `index.css` 里的主题变量（shadcn 标准命名：`background`、`foreground`、`card`、`popover`、`primary`（琥珀色强调）、
  `secondary`、`muted`、`muted-foreground`、`accent`、`border`、`input`、`ring`、`destructive`，另加 `success`、`warning`），
  如 `bg-card`、`text-muted-foreground`、`text-primary`、`text-warning`；不要写死颜色。深色模式由 App.tsx 在 `<html>` 上设置
  `data-theme="dark"`，`index.css` 把 Tailwind 的 `dark:` 变体绑定到它，深浅色都要正常。
- 功能完成后把 `registry.ts` 的 `status` 改为 `done`，并更新 FEATURES.md 的状态标记。

## 验收标准

每个任务交付时必须全部通过：

```
npm run typecheck
npm run build
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- **单元测试**：每个模块/工具对给定参数和种子生成的滤镜串或命令行有断言。
- **集成测试**（`src-tauri/tests/`）：用 FFmpeg 的 `lavfi` 源现场生成测试视频，真实跑一遍，再用 ffprobe 断言输出（分辨率、时长、帧率、流数量等）。不提交任何媒体文件。
- 不留 `todo!()`、`unimplemented!()`、空实现或被注释掉的测试；做不到的点在交付说明里明确列出。
- 不提交、不推送；由主会话验收后统一处理。

## 模块/工具编写指南

### 实现一个流水线模块（`pipe.*`）

18 个模块已在 M1 全部注册（后端 `RecipeModules` 字段、`pipeline::ordered_modules` 应用顺序、
前端 `features/pipeline/index.ts`）。实现一个模块**只改下面这些文件**：

| 文件 | 改什么 |
|---|---|
| `src-tauri/src/pipeline/modules/<模块>.rs` | 参数结构体加字段（`camelCase`、`#[serde(default)]`，`Default` 给默认值）；实现 `apply`；**删除** `implemented()` 的覆盖；写单元测试 |
| `src/features/pipeline/<模块>.tsx` | 参数类型与后端一致；`defaults` 与后端 `Default` 一致；`Form` 换成真正的表单 |
| `src-tauri/tests/pipeline_<模块>.rs` | 新建集成测试文件（不要改别人的测试文件） |
| `src/features/registry.ts` | 该模块调用 `f(...)` 末尾加 `"done"` |
| `docs/FEATURES.md` | 该模块状态改为 ✅ |

文件名：后端为 ID 去掉 `pipe.`、连字符换下划线（`pipe.rotate-flip` → `rotate_flip.rs`），前端保留连字符（`rotate-flip.tsx`）。

**参考实现**

- `fps.rs` / `fps.tsx`：只追加一进一出滤镜的模块（`ctx.graph.video("...")`），以及范围值取样。
- `bitrate.rs` / `bitrate.tsx`：只改编码参数的模块（`ctx.encode`），带模式切换与参数校验。

**`apply` 里可用的东西**（`pipeline::BuildContext`）

- `ctx.info`：源视频的 `MediaInfo`；`ctx.settings`：设置；`ctx.task_index`：队列序号（从 0 起，用于素材“依次取”）。
- `ctx.rng`：本模块专用的随机数（种子 + 模块 ID 派生），配合 `Range::sample` / `sample_rounded`、`AssetSource::pick` 使用。不要自己建随机数。
- `ctx.graph`：滤镜图。简单滤镜用 `video(..)` / `audio(..)`；多输入滤镜先 `take_video()` 取当前标签，
  `label("前缀")` 生成新标签，`chain(&[输入..], "滤镜", &[输出..])` 写整条链，再 `set_video(新标签)`
  （音频同理，`set_audio(None)` 表示去掉音频）。示例见 `pipeline/graph.rs` 的测试。
- `ctx.add_input(前置参数, 路径)`：加水印图、背景音等额外输入，返回 FFmpeg 输入序号（如 `1` → 在滤镜里用 `1:v`）。
- `ctx.main_input_args`：主输入的前置参数（如去头尾的 `-ss`）。
- `ctx.encode.output_args`：额外输出参数；`ctx.encode.video_kbps`：目标码率。
- `ctx.stream`：当前宽高 / 帧率 / 预计时长。**改变尺寸、帧率或时长的模块必须更新它**——后面的模块据此计算坐标，进度条据此计算百分比。
- `ctx.warn("...")`：非致命问题（会显示在处理记录里）；参数无效时 `return Err("中文原因".into())`。
- 数字写进滤镜时用 `pipeline::fmt_num`（`30.0` → `30`）。
- `ctx.probe(路径)`：读取素材（片头片尾、音乐等）的 `MediaInfo`，使用本次运行的 ffprobe（`RunConfig.paths`）。
  不要自己调用 `ffmpeg::locate()`。单元测试里没有注入时返回错误（`ctx.can_probe()` 为 false）；
  需要时用 `pipeline::testing::build_with_probe(.., &假函数)` 或 `ctx.set_probe(&假函数)`。
- `ctx.rate_text()`：当前帧率在滤镜里的写法（未改动时用源视频的分数原文，如 `30000/1001`；`25/1` 写作 `25`），帧率未知为 `None`。
- `pipeline::escape_filter_path(路径)`：把文件路径写进滤镜参数（`lut3d=file=`、`subtitles=` 等）时做两级转义。
- `ctx.graph.discard_audio()`：丢弃当前音频（未连接的音频链自动接入 `anullsink`），之后可再 `set_audio`。
- 可变帧率：只丢帧不重排时间戳的模块把 `ctx.stream.vfr` 设为 true；按帧序号计算时间的模块（抽帧、动态缩放等）
  在开头调用 `ctx.ensure_cfr()`（必要时插入 `fps=当前帧率`）。
- `ctx.later_modules()`：本次构建中排在当前模块之后、会被应用的模块 ID，用于提示模块间的冲突（例：重复帧移除
  在后面有帧率 / 变速 / 抽帧 / 动态缩放时给出警告）。
- `ctx.stream.audio_channels`：输出音频声道数。改变声道布局的模块必须更新（背景音统一为立体声 → 2）；
  AAC 码率由它决定（`pipeline::aac_kbps`：单声道 96k、立体声 192k。FFmpeg 9.1 自带的 AAC 编码器在单声道
  约 176k～208k 遇到“数字静音后接纯音”会死循环，所以不要自己写 `-b:a`，数字静音可以直接用 `anullsrc`）。
- **奇数尺寸**：构建最后若 `ctx.stream` 的宽或高为奇数，会统一 `crop` 到偶数（yuv420p 编码要求）。模块不必自己取偶，
  但必须让 `ctx.stream` 反映真实尺寸。
- **卡死保护**：`ffmpeg::run` 在 180 秒（`DEFAULT_STALL_TIMEOUT`）内输出没有任何进展时结束 ffmpeg，任务失败并提示“卡死”。
- `ctx.is_preview()`：是否在构建预览（`pipe.preview` / 单模块预览）。只在预览里显示的辅助标记据此添加，正式输出里绝不能出现
  （例：去水印在预览里用 `drawbox` 标出区域，见 `remove_watermark.rs`；单元测试可用 `pipeline::testing::build_preview_with`）。
- **临时文件**：`ctx.temp_file("名字")` 返回本次构建临时目录（`<系统临时目录>/cliprecipe-build/<进程号>-<序号>/`，
  第一次调用时创建）下的一个路径，模块自己写入；`ctx.register_temp(路径)` 登记模块在别处创建的文件或目录。
  登记的一切由 `BuiltCommand::temp_files` 持有，在任务的这一轮（或这次预览、“查看命令”）结束时删除——成功、构建失败、
  运行失败、取消都一样，模块不用自己清理；调度器启动时清理其它进程超过一天的残留。构建过程默认不写文件，只在确实需要时使用
  （例：文本模块的超长文字改用 `textfile=`，见 `text.rs`）。集成测试见 `tests/build_temp.rs`。

**集成测试共用工具**（`tests/common`）：`make_clip`（宽或高为奇数时生成真正的奇数尺寸视频，yuv444p）、
`make_odd_clip`、`streams`（容器 / 视频 / 音频时长、帧数、帧率、采样率）、`stream_duration`、`avg_rgb`（某帧某区域的平均颜色；
FFmpeg 9.1 对单张图片加 `-ss` 时不输出帧，图片请不要用它）、`assert_close`。
`tests/all_modules.rs` 同时开启全部 18 个模块跑一遍（横屏有声、竖屏无声、奇数尺寸、去掉背景音、裂变 3 次、画中画改画布、
整条流水线预览），检查任务成功、音视频时长一致、实际时长与进度所用的预计时长接近、每个模块的滤镜确实出现在命令里。
修改模块后若参数含义变了，同步更新其中 `everything()` 的参数（该测试会检查所有已实现模块都已开启）。
`tests/hw_fallback.rs` 在本机挑一个试编码失败的硬件编码器假装可用，经真实调度器验证 H.264 / H.265 的软件编码回退。

**单元测试**（写在模块文件末尾的 `#[cfg(test)] mod tests`）

```rust
use crate::pipeline::testing::{build_with, sample_info}; // 1280x720、25fps、10s、4000kbps、AAC
let mut recipe = crate::model::Recipe::disabled();       // 全部关闭
recipe.modules.crop = CropParams { enabled: true, /* ... */ ..Default::default() };
let built = build_with(&sample_info(), &recipe, 42).unwrap();
assert_eq!(built.arg_after("-filter_complex"), Some("[0:v:0]crop=...[v0]"));
```

对给定参数和种子断言完整的滤镜串；范围值要断言同一种子结果相同、取值在范围内；无效参数断言返回 `Err`。

**集成测试**（`src-tauri/tests/pipeline_<模块>.rs`）

```rust
mod common;
use common::*; // ffmpeg_paths()、TempDir、make_clip、probe、collector
```

用 `make_clip` 现场生成 lavfi 视频，构造 `Recipe::disabled()` 只开本模块，用 `jobs::start(RunConfig { encoders: EncoderCaps::software_only(), seed: 固定值, .. }, ..).wait()`
真实跑一遍，再用 `probe` 断言输出（分辨率、时长、帧率、流数量等）。写法见 `tests/jobs.rs` 的
`fps_and_bitrate_job_produces_expected_output`。`npm install` 后测试自动使用 `node_modules` 里的 FFmpeg 9.0.2 essentials；
也可设置 `CLIPRECIPE_FFMPEG`（或把 ffmpeg/ffprobe 放进 PATH）用别的构建。测试必须在 essentials 与 full 构建上都通过：
不要依赖 essentials 没有的滤镜（`whisper`、`flite`…，确需时先用 `ffmpeg::has_filter` 检查并打印跳过原因）；
用 ffprobe 数流时只读 JSON 顶层的 `streams`（ffprobe 9.0 对 MPEG-TS 还会输出 `programs[].streams`，见 `tool_support::probe_entries`）。

**前端表单**：只用 shadcn/ui 组件和下面的通用控件，不要手写按钮、输入框、开关等（规则见“架构约定 · 前端”）。
表单通过 `onChange(局部参数)` 提交修改，不要直接访问状态仓库（只读的 `useSelectedVideo()` 除外，见下）。

| 用途 | 控件 | 导入 |
|---|---|---|
| 一行参数（左标签、右控件、下方说明） | `ParamField`（`label`、`hint`） | `import ParamField from "@/components/ParamField"` |
| 范围值（最小 ~ 最大，相等即固定） | `RangeInput`（`value: Range`、`min`、`max`、`step`、`integer`、`unit`） | `import RangeInput from "@/components/RangeInput"` |
| 单个数字（失焦/回车时取整、限幅） | `NumberInput`（`value`、`min`、`max`、`step`、`integer`、`label`、`className` 控制宽度） | `import NumberInput from "@/components/NumberInput"` |
| 少量互斥选项 | `ChoiceGroup<T>`（`value`、`options: {value,label}[]`、`onChange`） | `import ChoiceGroup from "@/components/ChoiceGroup"` |
| 开关 | shadcn `Switch`（`checked`、`onCheckedChange`、`aria-label`） | `import { Switch } from "@/components/ui/switch"` |
| 下拉选择 | shadcn `Select` 系列 | `@/components/ui/select` |
| 复选框 + 文字 | shadcn `Checkbox` + `Label`（`htmlFor`，id 用 `useId()`） | `@/components/ui/checkbox`、`@/components/ui/label` |
| 多段可选按钮（可换行） | shadcn `ToggleGroup` / `ToggleGroupItem` | `@/components/ui/toggle-group` |
| 文件 / 目录素材 | `AssetSourceInput`（`value: AssetSource`、`extensions`、`kindLabel`） | `import AssetSourceInput from "@/components/AssetSourceInput"` |
| 颜色（#rrggbb） | `ColorField`（`value`、`onChange`、`label`）：Popover + 色板 + 十六进制输入 | `import ColorField from "@/components/ColorField"` |
| 区域框选 / 位置选取 | `RegionPicker` / `PointPicker`（见下） | `@/components/RegionPicker` |
| 图标按钮的提示 | `Hint`（包一个 shadcn Tooltip；图标按钮仍需 `aria-label`） | `import Hint from "@/components/Hint"` |
| 其它（按钮、文本框、对话框…） | 直接用 `@/components/ui/*` 里的 shadcn 组件 | 如 `import { Button } from "@/components/ui/button"` |

参数区宽度约 240px：控件用紧凑尺寸（输入框 `h-7 text-xs`，按钮 `size="xs"`），说明文字放进 `ParamField` 的 `hint`。

**默认值一致性检查**：`cargo test` 里的 `tests/recipe_defaults.rs` 会运行 `node scripts/dump-recipe-defaults.mjs`
（用 Vite 直接执行前端源码得到 `defaultRecipe()`），与 `Recipe::default()` 逐字段比较（数字按数值比较，前端缺省的键等同后端 `null`）。
`.tsx` 的 `defaults` 与 `.rs` 的 `Default` 不一致时测试失败并列出差异；不需要维护任何共享快照文件。需要 Node.js 和 `npm install`。

**模块作者自动获得的能力（不用写任何代码）**

- **单模块预览**：配方面板里展开模块后，表单下方有“预览此步骤”按钮——只启用本模块、按当前参数对“当前视频”生成
  5 秒低清片段并在预览窗口播放。前提是模块有 `Form`（不为 null）且不在 `PREVIEW_EXCLUDED`（变速、背景音、加头尾、帧率、码率）中。
  预览走的就是 `apply`，所以模块必须正确更新 `ctx.stream`（预览据此缩小画面、截取时长）。
- **整条流水线预览**（`pipe.preview`）：配方面板底部“生成预览”，同样排除上述五个模块。

**当前视频**：`import { useSelectedVideo } from "../../state/selection"`，返回 `{ item: QueueItem, number } | null`
——队列中第一个选中的视频，没有选中时为第一个。可用来显示源尺寸（`item.info.video`）等；只读。
表单直接静态导入它和 `RegionPicker` 即可。为此 `state/queue`、`state/selection`、`state/log`、`state/settings` 是最底层的状态，
**不能**导入 `state/recipe` 或 `features/*`（否则 `features/pipeline` → 表单 → … → `features/pipeline` 成环，
加载顺序一变就报 “Cannot access 'DEFS' before initialization”）；需要配方的操作由调用方传入（如 `useQueue.start(recipe)`）。

**当前 FFmpeg 的能力**：`import { useFfmpegStatus } from "@/state/ffmpeg"`（只读，未检测时为 null），如 `status.whisper`
表示有无 whisper 滤镜。依赖特定构建才有的滤镜的表单应据此提前说明（见 `subtitle-asr.tsx`），不要等到运行时才失败。

**区域框选 / 位置选取**（`components/RegionPicker.tsx`）：一个按钮 + 对话框（shadcn Button / Dialog / Slider），对话框显示当前视频某一帧（可拖动时间轴换帧），
坐标一律是**源视频像素**（已按旋转信息转正，与 FFmpeg 滤镜看到的画面一致）。

```tsx
import RegionPicker, { PointPicker, type Rect, type Point } from "@/components/RegionPicker";

<RegionPicker
  value={value.regions}               // Rect[]：{ x, y, width, height }
  onChange={(regions) => onChange({ regions })}  // 点“确定”时回传
  max={2}                             // 最多几个矩形，默认 1（满了再画会替换当前选中的）
  aspect={4 / 3}                      // 可选：锁定宽高比
  label="框选水印区域"                 // 可选：按钮文字
/>
<PointPicker value={value.position /* Point | null */} onChange={(position) => onChange({ position })} />
```

拖拽画矩形、拖动移动、拖四角缩放、选中后点 × 删除；小于 2 像素的误点会被丢弃。队列为空时按钮禁用。
需要自己取帧时可用 `components/useVideoFrame.ts` 的 `useVideoFrame(item, time)`（返回图片地址与源宽高）。

### 独立工具（`tool.*`）

34 个工具已在 M3 第一个任务中全部注册（后端 `tools/mod.rs` 的 `registry!`、命令 `start_tool` 等、
前端 `features/tools/index.ts`、工具箱与队列菜单）。实现一个工具**只改下面这些文件**：

| 文件 | 改什么 |
|---|---|
| `src-tauri/src/tools/<工具>.rs` | 参数结构体加字段（`camelCase`、`#[serde(default)]`，`Default` 给默认值）；需要时调整 `inputs()` / 覆盖 `plan()`；实现 `run()`；**删除** `implemented()` 的覆盖；写单元测试 |
| `src/features/tools/<工具>.tsx` | 参数类型（用 `type X = {..}`，不要用 `interface`）与后端一致；`defaults`、`inputs` 与后端一致；`Form` 换成真正的表单 |
| `src-tauri/tests/tool_<工具>.rs` | 新建集成测试文件 |
| `src/features/registry.ts` | 该工具调用 `f(...)` 末尾加 `"done"`（工具箱与队列菜单据此启用它） |
| `docs/FEATURES.md` | 该工具状态改为 ✅ |

文件名：后端为 ID 去掉 `tool.`、连字符换下划线（`tool.extract-av` → `extract_av.rs`），前端保留连字符（`extract-av.tsx`）。
`cargo test` 的 `tests/recipe_defaults.rs` 会检查：前端 `defaults` 与后端 `Default` 逐字段一致、`inputs` 与 `inputs()` 一致
（文件夹只比较键）、registry.ts 中为 `done` 的工具正是后端 `implemented()` 为 true 的工具。不需要改任何共享文件。

**框架概览**（`src-tauri/src/tools/`）

- `Tool` trait（实现者是参数结构体）：`id()`；`implemented()`；`inputs()`——`ToolInputs::VIDEOS`（队列视频，默认）、
  `ToolInputs::folders(&[FolderSlot { key, label }])`（文件夹）、`ToolInputs::videos_and_folders(..)`（两者）；
  `plan(&PlanRequest) -> Vec<WorkItem>`——把输入分成可并行的**工作项**（默认每个队列视频一项）；`run(&mut ToolContext)`——执行一个工作项；
  `supports_preview()`——是否支持预览（默认否，见下“工具预览”）。
- 规划阶段的随机（打乱顺序、随机配对）用 `req.rng(工具 ID)`（`PlanRequest::seed` 就是本次运行的种子，与执行时相同、可复现）。
  这样规划出的工作项就是真正的输入与分组：界面上显示正确，各组可以并行（合并 / 文件夹合并的打乱、截取合并的打乱、音视频合成的随机配对都这样做）。
  测试里用 `runner::plan_seeded(.., 种子)`（`runner::plan` 等同种子 0，保留兼容）。
- 调度（`runner.rs`）：工作项按设置的并行数执行；事件与批量处理相同（`JobEvent`，Tauri 事件名 `tool-event`），
  进度事件的 `round`/`rounds` 表示第几步/共几步。成功时输出统一从临时名改为最终名，失败或取消时删除；临时目录总会删除；
  工具里 panic 会变成“工具内部错误”。同一时间只运行一个工具，与批量处理互不影响。
- 只沿用主界面的“码率”（`pipe.bitrate` 参数随请求传入）与“输出位置”；命名规则与批量处理相同（见下）。

**选哪个参考实现**

| 形态 | 参考 | 要点 |
|---|---|---|
| 一进一出、不重编码（换封装、改元数据、提取字幕…） | `remux.rs` / `remux.tsx` | 默认 `plan`；`ctx.output(&OutputSpec::file(ext))`；`ctx.run_ffmpeg(..)`；命令拼装写成纯函数单独测试 |
| 一进多出（提取音轨、截图、字幕轨…） | `extract_av.rs` / `extract-av.tsx` | 一条命令多个输出：每个输出一组参数 + 一个 `ctx.output(..)` 路径 |
| 一进多出 + 先分析后处理 + 重编码（分割、去黑边、智能去头尾…） | `split.rs` / `split.tsx` | `ctx.set_steps` / `ctx.analyze_input` / `ctx.split_next_step`；每段 `ctx.run_encode`；`OutputSpec::..in_stem_folder()`；无损模式把 segment 结果写进 `ctx.temp_dir()` 再 `ctx.keep_file` |
| 多进一出、多步、中间文件（合并、重组、静音 / 静止帧移除、拼接…） | `tests/tool_framework.rs` 的 `SceneConcat` | `plan::all_videos_as_one`（或文件夹 `plan::files_as_one`）；中间结果写 `ctx.temp_file(..)`；`media::write_concat_list` + concat 分离器 |
| 文件夹输入 | `orientation.rs` 等占位文件的 `inputs()` | `plan` 里 `req.folder_files(key, 扩展名)` + `plan::per_file` / `files_as_one`；两个文件夹配对时自己组 `WorkItem::new(..)` |
| 输出一个文件夹（HLS） | — | `OutputSpec::folder()` 返回已创建的临时文件夹，成功后整体改名；文件夹从不覆盖，重名时加 “ (n)” |
| 一进一出的滤镜效果（叠加、定帧、运镜、去黑边、进度条…） | `camera_move.rs`、`progress_bar.rs` | `common::filter_job::FilterJob` + `run_job`；声明 `supports_preview` |
| 片段列表（剪掉 / 重排 / 插入 / 转场） | `remove_remix.rs`、`mirror_remix.rs` | 算出 `common::cutlist::Timeline`，`cutlist::render` 单遍编码 |
| 合并多个文件 | `merge.rs`、`folder_merge.rs` | `common::merge_engine`：`merge_segments` / `run_groups`；分组与打乱在 `plan` 里做 |

**共用模块**（`tools/common/`，详见 `common/mod.rs` 开头的表）。多个工具要用的逻辑放这里，**不要从别的工具文件导入**
（原来放在工具文件里的同名项仍从原位置再导出，旧代码可以继续编译，新代码请从 `common` 导入）：

| 模块 | 内容 |
|---|---|
| `common::filter_job` | 一进一出命令 `FilterJob`（输入、`[v]`/`[a]` 滤镜图、音频复制或 AAC）与 `run_job`；帧率 `Rate`（`of`、`frame_at`、`frames_for`、`total_frames`）；插入空档 `gap_video_filter` / `gap_audio_filter`；`even_crop`、`display_size`、`check_duration`、`float_text`、`color_hex`、`source_audio`；图片输入 `image_input_args`、`IMAGE_EXTENSIONS` |
| `common::positions` | `parse_time` / `parse_positions`（秒、分:秒、百分比的列表）、`position_frames`、`windows_expr`；位置参数 `PositionMode` / `resolve_positions`（时间点 / 百分比 / 均分） |
| `common::cutlist` | `Piece`（主视频区间：倍速、翻转；插入其它视频）、`Join`（直接拼接 / 转场）、`Timeline`、`SourceFormat`（帧网格对齐 `snap`）、`build_graph`（纯函数）、`render(ctx, &时间线, 插入文件, 说明)`；`SQUARE_PIXELS` |
| `common::merge_engine` | `Segment`（整个文件或其中一段）、`MergeOptions`、`merge_segments`、`encode_segments`、`concat_copy`、`run_groups`、`conform_target` / `conform_graph`、`insert_middles`、`shuffle`、`chunk_items`、转场选择 |
| `common::scenes` | 场景分段 `check_scene_params`、`scene_segments`、`merge_short` |

前端同理：工具表单共用的部件在 `features/tools/shared/`（`WithUnit`、`SceneFields`、`PositionFields` / `NumberListInput`、
`MergeOptionsFields`），原文件仍再导出。

**非方形像素**（SAR ≠ 1 的变形源）：`VideoStream::sar` / `sar_ratio()` / `square_display_size()`。拼接不同来源时先换算成方形像素再统一：
`cutlist` 的主视频在滤镜图开头缩放一次（`SourceFormat` 的宽高就是方形像素的显示尺寸，`anamorphic` 为真时），插入的视频与合并的每个片段先接
`SQUARE_PIXELS`（`scale=trunc(iw*sar/2)*2:ih,setsar=1`，方形像素时是空操作）；合并的目标尺寸用第一个片段的 `square_display_size()`；
无损合并要求各片段 SAR 相同。新工具拼接多个来源时同样先接 `SQUARE_PIXELS`，不要只写 `setsar=1`（那会改变变形源的显示比例）。

**`run` 里可用的东西**（`ToolContext`，详见 `tools/context.rs` 开头的表）

- 输入：`ctx.inputs()`、`ctx.input_path(i)`、`ctx.info(i)`（没有时用 ffprobe 探测并缓存）、`ctx.probe(路径)`、`ctx.stem(i)`（去除后的文件名）、
  `ctx.video_extension(i)`（按设置“目标格式”为 mp4 或保持源格式）、`ctx.settings()`、`ctx.number()`。
- 输出：`ctx.output(&OutputSpec)` 返回**写入用的临时路径**（同目录下 `.<名称>.cliprecipe-tool-<n>.part.<扩展名>`）。
  `OutputSpec::file("mp3").suffix("_audio")` → `<输出目录>/<名称>_audio.mp3`；`.in_stem_folder()` → `<输出目录>/<名称>/…`；
  `.stem("自定义名")` 代替输入名；`.from_input(i)` 以第 i 个输入为准。规则与批量处理相同：输出目录（空则源目录）、按来源文件夹分目录、
  输出到视频同名文件夹、文件名去除、队列序号、已存在时重命名或覆盖；永远不覆盖工作项的任何输入，并行时不会抢同一个名字。
  `ctx.keep_file(已生成的文件, &spec)`：把临时目录里生成的文件（如 segment 结果）移动为一个输出。
- 运行：`ctx.run_ffmpeg(说明, 参数, 预计输出时长)`（参数不含开头的通用参数，自动加上 `-hide_banner -nostdin -y -loglevel warning -progress pipe:1 -nostats`）。
  需要视频编码时用 `ctx.run_encode(说明, 预计时长, |enc| Ok(参数))`：`enc.video_args(容器)` 已含编码器（按设置选择 GPU / H.265）与码率
  （沿用 `pipe.bitrate`；未启用时见下“默认码率”，源码率未知时恒定质量）；`Encoding::audio_args(声道数)` 为 AAC 参数。硬件编码失败自动改用软件编码重试，
  之后本工作项都用软件编码。卡死保护（180 秒无进展）与取消对每次运行都生效。
  默认码率以第 0 个输入为准、输出画面与它相同；输出由别的输入组成（合并的分组）或画面尺寸变了（拼接、缩放）时用
  `ctx.run_encode_from(说明, 预计时长, &EncodeBasis::inputs([..]).frame(宽, 高, 帧率), |enc| ..)`（`ctx.encoding_from(&basis)` 只取设置）。
- 进度：每次运行 ffmpeg（含分析）占一步。`ctx.set_steps(&[权重..])` 设定剩余各步；分析后才知道段数时 `ctx.split_next_step(&[..])`
  把下一步再细分；不需要运行时 `ctx.skip_step()`。未设定时只有一步。
- 临时文件：`ctx.temp_dir()`、`ctx.temp_file("名字")`（`<系统临时目录>/cliprecipe-tools/<进程号>/<序号>-<n>/`），工作项结束时整体删除；
  启动运行时清理其它进程超过一天的残留。
- 取消：ffmpeg 运行中自动响应；自己的循环里调用 `ctx.check_cancelled()?`。
- 日志：`ctx.note(..)`（信息）、`ctx.warn(..)`（同样内容只记一次）、`ctx.log(级别, ..)`。前端会在消息前加工具名，写进共用的处理记录。
- 随机数：`ctx.rng`（种子 + 序号 + 工具 ID 派生，可复现）；需要多个独立序列时 `SeededRng::derive(ctx.seed(), "键")`。
- 错误：`run` 返回 `Result<(), ToolError>`，`String` / `&str` 可以直接 `?` / `.into()`（中文原因会显示给用户）。
- 小工具（`tools/media.rs`）：`args(&[..])`、`secs(秒)`（写进命令行的时间）、`container_accepts(容器, 流类型, 编码)`（能否直接复制）、
  `copy_container(..)`（不重编码输出视频时选容器）、`audio_copy_extension(编码)`、`is_text_subtitle`、`write_concat_list`。

**分析助手**（`tools/analysis/`；工具里用 `ctx.analyze_input(i, &检测)` 或 `ctx.analyze(路径, &info, &检测)`，各占一步进度；
测试或别处用 `analysis::run_analysis(ffmpeg, 路径, &info, &检测, &取消, 进度回调)`）

| 检测 | 参数（`Default`） | 结果 |
|---|---|---|
| `SceneDetect` 场景切换 | `threshold: 0.3`（0～1，越小越敏感）、`min_gap: 1.0` 秒（与前一个切点、开头、结尾的最小间隔） | `Vec<f64>` 切点（秒，升序） |
| `SilenceDetect` 静音（第一条音轨） | `noise_db: -30.0`、`min_duration: 0.5` 秒 | `Vec<Interval>`；没有音轨时报错 |
| `BlackBarDetect` 黑边 | `limit: 24`（0～255）、`round: 2`、`sample_fps: 2.0`、`min_bar: 4` 像素 | `Option<CropRect>`（显示坐标，`rect.filter()` → `crop=w:h:x:y`，奇数偏移加 `:exact=1`）；没有黑边为 None。用 `cropdetect` 的精确边界 `x1..y2`，奇数偏移也不多裁；宽高取偶时向黑边扩 1 像素 |
| `FreezeDetect` 静止画面 | `noise_db: -60.0`、`min_duration: 1.0` 秒 | `Vec<Interval>` |

区间工具：`Interval { start, end }.duration()`、`normalize_intervals`（限幅、排序、合并）、`complement(区间, 时长, 最短)`
（例如去掉静音后要保留的片段）、`segments_from_cuts(切点, 时长)`。检测结果来自 `metadata=mode=print` 打印到日志的逐帧元数据
（`MetadataParser`），全部帧都会经过滤镜，所以进度与卡死保护正常。新增检测时实现 `Analysis` trait（`args` + `parse`）。

**单元测试**（工具文件末尾）：把命令拼装 / 规划写成纯函数，用 `crate::pipeline::testing::sample_info()` 断言完整参数；
无效参数断言 `Err`。参考 `remux.rs`、`extract_av.rs`、`split.rs` 的测试。

**集成测试**（`src-tauri/tests/tool_<工具>.rs`）

```rust
mod common;
mod tool_support;
use common::*;        // ffmpeg_paths()、TempDir、make_clip、probe、streams、avg_rgb、assert_close
use tool_support::*;  // 运行：tool_config、fixed_bitrate、registered、run_tool、start_tool、done_outputs、log_messages
                      // 素材：make_lavfi、make_scene_clip、colour_scenes、split_colour_scenes、color_clip、
                      //       make_luma_counter + counter_index（亮度计数器）、tone
                      // 画面：frame_lumas、frame_colours、frame_pixels、frame_row、assert_colour、RED…BLACK、CENTRE、psnr
                      // 声音：audio_samples、rms、tone_db
                      // 流与文件：codecs、stream_count、video_duration、video_packets_md5、all_files

let (summary, c) = run_tool(
    tool_config(&paths, &out_dir, &dir.join("tmp")),           // 软件编码、固定种子、临时目录放在测试目录下
    registered("tool.split", serde_json::json!({"mode": "count", "count": 3})),
    &[video_task("t1", &input)],
);
assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
let outputs = done_outputs(&c, "t1");                            // 最终输出路径
```

用 lavfi 现场生成素材（`make_lavfi` 可写任意滤镜图，`make_scene_clip` 生成在已知位置硬切换的视频），真实跑一遍，用 ffprobe 断言输出；
同时断言失败 / 取消后输出目录与临时目录里没有残留（`all_files`）。文件夹输入用 `start_tool(.., &folders, &c)` 传入。

**前端表单**：规则与流水线模块相同（只用 shadcn/ui 与通用控件，见上面的控件表；表单通过 `onChange(局部参数)` 提交）。
表单区宽约 300px。文件夹输入、处理范围（队列中选中 / 全部）、输出位置、开始 / 停止、逐项进度与处理记录都由工具对话框
（`views/tools/ToolDialog.tsx`）统一提供，工具只写参数表单。参数自动记在本机（localStorage），对话框里可“恢复默认”。
文件夹输入框与输出位置可以拖入文件夹（拖入文件取其所在文件夹）；运行中每个视频的状态与进度也显示在批量队列对应的行上。
每个 `<工具>.tsx` 是单独按需加载的代码块（`features/tools/index.ts` 用 `import.meta.glob`）：组件里用 `useToolDef(id)`，
别处用 `loadToolDef(id)`；工具文件的写法不变（`defineTool({ id, defaults, inputs, Form, preview? })`）。

**工具预览**：一进一出、处理队列视频、几秒片段就能看出效果的工具，后端实现 `fn supports_preview(&self) -> bool { true }`，
前端 `defineTool` 加 `preview: true`（`cargo test` 的 `recipe_defaults` 检查两边一致）。对话框底栏出现“预览”按钮：
框架把当前视频的一段（默认 5 秒、可在预览窗口调整起点 / 时长）截成低清片段（高度 ≤ 480），作为唯一输入用工具自己的 `run`
处理（同一个调度器、同样的取消与临时目录清理），取第一个视频输出在预览窗口播放（`tools::preview::render_tool_preview`，
命令 `render_tool_preview`，与流水线预览共用取消、进度事件与预览目录）。按时间点写的参数以片段的时间为准。
已开启：运镜、进度条、定格播放、定帧动画、插帧、去黑边、镜像重组、转场重组。

**工具自动获得的入口**：工具箱卡片；处理队列视频的工具（`inputs.videos`）在 registry 标记为 `done` 后，还会出现在队列工具栏的
“工具”菜单和队列行右键菜单的“独立工具”子菜单里（打开时处理范围为选中的视频）。

### M3 独立工具框架的实现说明

- **一次一个工具**：`start_tool` 先在后台规划工作项（校验输入、列出文件夹），返回工作项清单与种子，再开始执行；
  已有工具在运行时拒绝。停止 `stop_tool`、取消单项 `cancel_tool_item`。
- **分析类运行**：`ffmpeg::run_with_stderr` 在原有运行函数（进度、取消、卡死保护）之上把 stderr 每一行交给回调；
  `run` / `run_with_stall_timeout` 的行为不变。
- **修订**：默认码率改为“每像素码率 + 质量下限”（见 M1 说明）并用于工具与批量处理；`EncodeBasis` / `run_encode_from`；
  `PlanRequest::seed`（规划阶段随机）；`tools/common/` 与 `features/tools/shared/`；工具预览；黑边精确检测；SAR 统一；
  队列行显示工具进度、对话框拖入文件夹；工具表单按需加载。
- **共享改动**：`jobs::output::folder_relative` 改为公开；`fsutil::move_file`（跨分区时复制后删除）；`components/OutputPicker.tsx`
  从 BatchView 中提取出来供工具对话框复用；`LogPanel` 可传 `filter` / `className` / `bodyClassName`；
  `scripts/dump-recipe-defaults.mjs tools` 输出工具声明。

### M1 的实现说明

- **队列归前端**：队列顺序、选择、媒体信息由前端 `state/queue.ts` 维护；点“开始”时把任务（含已探测的
  `MediaInfo`）、配方、设置一并交给后端 `start_jobs`，后端只负责执行并通过 `job-event` 回报。
- **输出写临时文件**：每个任务先写到输出目录下的 `.<名称>.cliprecipe-<序号>.part.<扩展名>`，成功后改名；
  失败或取消时删除，因此不会留下半截文件，“覆盖”模式也只在成功后才替换旧文件，且永远不会覆盖源文件。
- **硬件编码**：启动处理时（GPU 开启）对 NVENC / QSV / AMF 的 H.264、H.265 各做一次试编码并缓存；
  运行时硬件编码失败会自动用 libx264 / libx265 重试一次。
- **默认码率**（未启用 `pipe.bitrate` 时，批量处理与独立工具相同，`ffmpeg::encoders::default_video_kbps`）：
  **源视频的每像素码率 × 输出画面的每秒像素数，且不低于质量下限**（H.264 每像素每帧 0.04 bit、至少 300 kbit/s；H.265 为 0.028 bit、
  至少 200 kbit/s；帧率按 1～60 计）。输出与源同尺寸时就是源码率；多个来源（合并）按时长加权。源码率未知时用恒定质量（CRF 23 / 28）。
  取舍：批量工具的用户期望“与源差不多的画质和体积”，纯恒定质量（CRF）会让高码率源变小很多、低码率源变大很多，体积不可预期；
  单纯沿用源码率时，低码率源（屏幕录制、纯色画面、高压缩的下载视频）在叠加文字 / 进度条 / 转场 / 拼接其它素材后内容比源复杂，
  画面会糊成一片。每像素下限只抬高那些“码率低得不正常”的源（1080p30 约 2.5 Mbit/s、720p25 约 0.9 Mbit/s），正常码率的源仍与源一致。
  仍是目标码率（VBR）模式，软件与硬件编码器（NVENC / QSV / AMF）行为一致。
- **音频**：音频没有经过滤镜且编码与目标容器兼容时直接复制，否则转 AAC（码率按声道数，见 `pipeline::aac_kbps`）；只保留第一条视频流和第一条音频流。

### M2 流水线级功能的实现说明

- **裂变**（`pipe.fission`）：次数存在配方的顶层字段 `fission`（1..=100）。调度器对每个任务依次跑 N 轮，
  第 r 轮的种子为 `rng::fission_seed(任务种子, r)`（第 1 轮就是任务种子，所以 N=1 与不裂变完全一致）。
  N>1 时输出到 `<输出目录>/<视频名>/<视频名>_<r>.mp4`；开启高级开关 `fissionFolder`（`set.fission-folder`）时改为
  `<输出目录>/<视频名>/第 r 次裂变/<视频名>.mp4`。进度事件带 `round` / `rounds`，`percent` 是全部轮次的总进度；
  完成事件的 `outputs` 列出全部输出。某一轮失败或取消时，已完成的轮次保留。
- **预览**：`pipeline::build_preview` 与正式构建共用模块应用逻辑，只是输出改为 libx264 ultrafast 的低清片段
  （默认高 480，`-ss`/`-t` 放在输出端，因此时间是输出时间轴）或单帧 jpg，不用硬件编码。文件写在
  `<系统临时目录>/cliprecipe-preview/<进程号>/`：换片段 / 关窗口时前端通知删除，目录最多保留 40 个文件，
  退出时整体删除，启动时清理其它进程超过一天的残留。新的预览会取消未完成的旧预览。
- **本地媒体加载**：WebView 用 Tauri asset 协议（`convertFileSrc`）读取本地文件。配置中的静态范围为空，
  启动时只把本进程的预览目录加入范围；双击播放源视频时由 `allow_media_file` 把那一个视频文件加入范围。
  `security.csp` 只允许 `self`、`ipc:` 与 `asset:`/`http://asset.localhost` 媒体；开发模式（`devCsp`）不限制，便于 Vite 热更新。
- **配方文件**：JSON（`*.json`），与 `Recipe` 的 serde 结构相同。导入走 `Recipe::from_json_lenient`：版本高于当前程序时拒绝；
  缺失的模块 / 字段取默认值；未知模块、类型不对的字段逐个改用默认值并在处理记录里提示。拖进窗口时，`.json` 文件按配方加载，
  其余按视频导入；拖到“输出到”输入框上的文件夹（或文件所在文件夹）设为输出目录。
- **队列**：拖动把手排序（因为窗口启用了系统拖放，不能用 HTML5 拖放，改用指针事件）；“置顶”在处理中会调用
  `prioritize_task` 让等待中的任务提前；排序按名称（自然顺序）/ 时长 / 大小。

### M4 设置与收尾的实现说明

- **设置**：`model/settings.rs` 的 `Settings` 整体 `#[serde(default)]`，旧的 settings.json 照常读取（见单元测试
  `old_files_load_and_after_all_resets_on_startup`）。前端 `lib/backend.ts` 的 `Settings` / `DEFAULT_SETTINGS` 必须同步：
  `tests/recipe_defaults.rs` 的 `frontend_settings_defaults_match_backend` 运行 `node scripts/dump-recipe-defaults.mjs settings`
  逐字段比较。设置页按“转换 / 字幕与封面 / 通用 / 完成后 / 高级 / 关于”分组（竖向 Tabs）。`load_settings` 会把
  `afterAll` 重置为 `none`（只对一次运行有效），并让“核显优先”（进程级，独立工具也生效）与托盘即时生效。
- **流水线级步骤**（`pipeline/steps.rs`，在 `pipeline::build` 中固定调用，不属于任何模块）：
  - HDR（`set.hdr`）：`VideoStream` 新增 `colorTransfer` / `colorPrimaries` / `colorSpace`，`is_hdr()` 认 PQ / HLG。转 SDR 时在所有模块之前
    用 zscale + tonemap(hable) 转为 BT.709 并标记色彩；保留 HDR 时改用 10 bit H.265（软件 `yuv420p10le`、硬件 `p010le`），沿用源的色彩标记，
    选了 H.264 时警告。预览总是转 SDR。
  - 字幕（`queue.sidecar` / `set.auto-subtitle`）：在所有模块之前用 `subtitles` 滤镜烧录，字幕跟随源视频的画面与时间，之后的模块一并作用；
    去头尾的输入端 `-ss` 起点在模块应用完后才知道，所以先写占位标记，最后用 `FilterGraph::replace_marker` 换成
    `setpts=PTS+起点/TB,subtitles=…,setpts=PTS-起点/TB`。样式（`subtitleStyle`）以 PlayRes 1080 为基准写进 `force_style`，只用于 srt / vtt；
    选中的字体文件链接（或复制）到构建临时目录作为 `fontsdir`（中文可正常显示）。预览同样烧录。
  - 封面（`set.auto-cover`）：在所有模块与偶数裁剪之后。帧数 N ≤ 帧率时 `overlay` 盖住开头 N 帧（时长不变）；N > 帧率时用
    `-loop 1` 的图片生成 N 帧片段 `concat` 在开头，音频 `adelay` 相应延后、预计时长增加。`coverStretch` 为 false 时等比缩放补黑边。
    mp4 / mov / m4v 另把图片以 mjpeg 写为第二条视频流（`attached_pic`），此时 `-tag:v` 改为 `-tag:v:0`。预览不嵌入封面。
  - 体积压缩（`set.reduce-size`）：未启用码率模块时，用 `VideoEncoder::slim_args`（CRF 28 / 32、硬件 CQ / QP 32，软件预设 slow，
    知道源码率时以它为 `-maxrate`）。启用码率模块时以模块为准并警告。版权（`set.copyright`）：`-metadata copyright=…`。
  - 独立工具不经过这些步骤（字幕 / 封面 / 体积压缩只作用于批量处理；版权信息见下文，工具也会写入）。
- **虚化背景**（`set.blur-center`）：分辨率、画中画的 `blurCenter` 改为 `Option<bool>`，默认 `null` = 跟随设置；旧配方里的 true / false 仍然有效且优先。
  表单为“跟随设置 / 取中 / 拉伸”（`components/BlurCenterChoice.tsx`）。
- **目录素材**（`set.recursive`）：`AssetSourceInput` 新建“目录”素材时以设置为“包含子目录”的初始值，之后以素材自己的勾选为准。
- **附加信息不进 `TaskSpec`**（独立工具也构造它）：`start_jobs` 的任务是 `TaskSpec` + `subtitle` / `cover`（serde flatten），
  按任务 ID 放进 `RunExtras::attachments`，经 `jobs::start_with` 交给调度器；`jobs::start` 不变（等于空的附加信息）。
- **收尾**（`jobs/finish.rs`，`set.move-back` / `set.delete-source`）：只在任务全部轮次成功、输出都已改为最终文件后执行；先校验每个输出存在且非空；
  移回源目录时按 `onExists` 重命名或覆盖，但永远不覆盖源视频——只有同时开启删除源视频，才在源视频删除之后让输出占用它的名字；
  任何移动失败都不删除源视频；输出与源同路径时不删除。删除默认移入回收站（`trash` crate），可改为永久删除；`RunExtras::dispose` 供测试替换。
  集成测试 `tests/m4_finish.rs` 覆盖成功、永久删除、失败、取消、移回（重命名 / 覆盖）、移回 + 删除。
- **队列保存**（`queue.persist`）：前端在队列变化时（防抖）把未完成的任务交给 `save_queue`，写到配置目录 `queue.json`；启动时先 `restore_queue`
  （丢弃不存在的视频并记录、取消已不存在的字幕 / 封面关联、缺少媒体信息的重新探测）再开始保存。关闭 `keepQueue` 时删除保存的内容。
- **系统副作用**都在小函数里：`system::shutdown_now`（只在 30 秒倒计时 `system::Countdown` 结束时调用，倒计时 / 取消 / 丢弃即取消有单元测试，
  测试不会关机）、`commands::system::sync_tray`（托盘只在开启 `set.tray` 时创建；关闭窗口是否改为隐藏由纯函数 `should_hide_on_close` 决定）、
  `tauri_plugin_opener::open_path / open_url`（只在 Rust 端调用，不开放给前端，因此不需要新增 capability；网页只允许本项目 GitHub 地址）、
  检查更新 `system::check_update(当前版本, fetch)`（正式用 `ureq` 发一个不带身份信息的 GET 到 GitHub 公开 API，测试注入假的 fetch）。
  提示音在前端：自定义 wav 经 `allow_sound_file` 加入 asset 范围后播放，内置提示音用 WebAudio 合成（不需要音频文件）。
- **新依赖**：`trash`（回收站）、`ureq`（rustls，检查更新）、`tauri-plugin-opener`（只用其 Rust 函数）、tauri 的 `tray-icon` 特性。CSP 未改动。
- **变速适应**（`set.speed-fit`）：共用助手 `pipeline::speed_fit::speed_fit(实际, 目标)`（`setpts` + 串联 `atempo`，0.25～4 倍，
  误差 0.5% 以内不变速）。接入的工具：`tool.av-mux`（视频变速到配对音频的时长，重编码视频，保留的原声同步变速，
  `av_mux::fit_graph` / `fit_args`）；`tool.clip-merge`（有目标时长时用 `merge_engine::OutputEnd::SpeedFit`，
  `append_speed_fit` 把滤镜接在合并图的 `[vout]` / `[aout]` 之后，再截到目标保证帧级准确）。其它工具没有外部“目标时长”
  （输出时长由自身参数决定），不使用这个设置。测试：`tests/tool_settings.rs`。
- **独立工具的版权信息**：`tools::context::with_copyright` 在 `run_ffmpeg` / `run_encode(_from)` 中统一处理：最后一个参数是视频文件时
  在它前面加 `-metadata copyright=…`（图片、GIF、HLS、分析命令不受影响）。
- **变形（非方形像素）源**：`steps::normalize_pixels` 在所有模块之前把 SAR ≠ 1 的源缩放为方形像素
  （尺寸为 `VideoStream::square_display_size`），`ctx.stream` 与 `ctx.source_size()` 都是换算后的尺寸；区域框选的取帧
  （`preview::extract_frame`）同样输出方形像素，坐标一致。片头片尾素材是变形视频时先换算再缩放。测试：`tests/pipeline_anamorphic.rs`、
  `all_modules_on_anamorphic_clip`。
- **工具小函数整理**：颜色 / 缩放方式 / 取偶 / 尺寸检查移到 `tools/common/frame.rs`（`orientation` 再导出）；图转视频用
  `EncodeBasis::output_only(宽, 高, 帧率)`（不参照图片的“码率”）。批量截图的文件名绘制样式是它自己的参数（封面处理不绘制文字）。

### 应用内下载 FFmpeg 的实现说明

- **固定清单**：开发依赖 `@seankit/ffmpeg`、`@seankit/ffprobe`（package.json 中写死精确版本）各带一份 `manifest.json`
  （每个平台的 tgz 地址——npmmirror 在前、npmjs 在后、内容相同——以及 `size`、`sha256`、包内路径 `binary`、`binarySha256`、
  `binarySize`、Rust 目标三元组、别名）。`npm run sync-ffmpeg-manifest`（`scripts/sync-ffmpeg-manifest.mjs`）把两份原样合并为
  `src-tauri/resources/ffmpeg-manifest.json` 并提交；`ffmpeg/manifest.rs` 用 `include_str!` 内嵌、解析并校验（只允许 HTTPS、
  哈希格式、包内路径安全、两个工具版本一致、版本号可作目录名）。**程序只下载内嵌清单列出的文件**，运行时从不联网取清单。
  `manifest::tests::committed_manifest_matches_installed_npm_packages` 在提交的清单与 node_modules 中的包不一致时失败
  （没有 node_modules 时打印原因并跳过）。平台：先按编译目标三元组（build.rs 写入 `CLIPRECIPE_TARGET`）匹配 `rustTargets`，
  再按 `win32-x64` 式的平台键，最后按别名（Windows on ARM → x64）；都没有时给出中文说明，界面不显示下载按钮。
- **升级 FFmpeg**：`npm install --save-dev --save-exact @seankit/ffmpeg@<新版本> @seankit/ffprobe@<新版本>` →
  `npm run sync-ffmpeg-manifest` → 用新的 node_modules 跑一遍 `cargo test`（会用到新构建）并检查 essentials 缺少的功能是否变化 →
  提交 package.json、package-lock.json 与清单。已下载旧版本的用户在设置页看到“可更新”（旧版本仍然可用），点击后下载新版本，
  成功后删除旧版本目录。
- **查找顺序**（`ffmpeg::locate_detailed`，每次调用重新查找、不缓存）：`CLIPRECIPE_FFMPEG` → 托管副本（固定版本；只有更旧的
  版本时也使用并标记 `outdated`）→ 程序所在目录 → `PATH` → 仅调试构建与测试：仓库 `node_modules/@seankit/ffmpeg-<平台>/bin`
  与 `ffprobe-<平台>/bin`（两个目录，`npm install` 即可运行程序与测试）。`ffmpeg_status` 报告 `source`（env / managed /
  bundled / path / dev / missing）、版本、`whisper`（有无 whisper 滤镜）与 `managed`（能否下载、大小、已下载的版本、可否更新）。
- **托管目录**：`<应用本地数据目录>/ffmpeg/<FFmpeg 版本>/`（Windows：`%LOCALAPPDATA%\io.github.cliprecipe\ffmpeg\9.0.2\`），
  ffmpeg 与 ffprobe 在同一目录，另有 `receipt.json`（版本、平台、实际下载地址、tgz 与可执行文件的哈希和大小、安装时间）。
  设置了 `CLIPRECIPE_CONFIG_DIR` 时改为 `<它>/ffmpeg`；`cargo test` 不登记根目录，因此测试永远不碰真实的数据目录。
  版本目录要有安装记录、版本与目录名一致、两个可执行文件的大小与记录相符才算完整；以点开头的临时目录不算。
- **下载与安装**（`ffmpeg/managed.rs`，只在用户点击时由 `download_ffmpeg` 调用）：按清单顺序尝试地址，失败换下一个；
  边下载边写入 `<根目录>/.staging-*` 并计算 SHA-256，超过清单大小立即停止，结束时必须正好相等；**先**核对 tgz 的 SHA-256，
  一致后才解压；只解出 `package/<binary>` 这一个条目，其它条目不写入磁盘；任何条目是绝对路径 / 盘符 / 含 `..` / 链接或特殊文件 /
  大于可执行文件，整个压缩包拒绝；核对可执行文件的 SHA-256 与大小；Unix 上设置 0755；两个工具都通过后写安装记录，把临时目录整体改名为
  版本目录（已有同版本时先移开、成功后删除、失败移回），成功后删除旧版本。失败、取消都删除临时目录，下次安装时清理残留
  （一小时内其它进程的临时目录保留）。未经校验的文件不会出现在版本目录中，也就不会被找到、执行。删除（`remove_managed_ffmpeg`）
  先改名再删除：正在使用的程序在 Windows 上无法改名，因此不会删掉一半；处理或下载进行中时拒绝。下载与删除同一时间只允许一个。
- **网络**（`net.rs`，检查更新共用）：只发普通 GET，`User-Agent: ClipRecipe`，不带任何用户标识；正式构建只允许 HTTPS
  （ureq `https_only`，跳转到 http 也拒绝）；代理按环境变量 `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY`，`NO_PROXY` 中的主机与本机地址直连
  （此前检查更新不读代理，现在两者一致）；连接超时 15 秒，30 秒收不到任何数据判定卡住并换下一个地址。取消在两次读取之间生效，
  网络卡住时最多等到读取超时。
- **新依赖**：`sha2`、`flate2`（此前已是间接依赖）、`tar`（`default-features = false`，只用其读取器，不用 `unpack`）；
  开发依赖 `@seankit/ffmpeg`、`@seankit/ffprobe`。
- **命令与事件**（`commands/ffmpeg.rs`）：`download_ffmpeg`（返回 `{ cancelled, status }`，失败时返回错误）、`cancel_ffmpeg_download`、
  `remove_managed_ffmpeg`、`redetect_ffmpeg`；进度事件 `ffmpeg-download`（阶段、工具、来源主机、已下载 / 总字节、速度）。
  安装或删除后清空编码器探测缓存；因为 `locate` 不缓存，新副本立即生效，无需重启。没有新增 capability，CSP 不变（下载在 Rust 中进行）。
- **界面**：`state/ffmpeg.ts`（状态、下载进度、`requireFfmpeg`：导入 / 开始处理 / 运行工具前发现没有 FFmpeg 时在处理记录里指向下载入口并
  重新显示提示）、`components/FfmpegDownload.tsx`（下载 / 进度 / 速度 / 取消 / 失败重试、下载说明、批量处理页“未找到 FFmpeg”提示，
  可关闭）；设置 → 关于 → 运行环境：来源、版本、路径、重新检测、下载 / 更新 / 删除（AlertDialog 确认）。
- **essentials 构建缺少的功能**（与 gyan.dev 9.1 full 对比 `-filters` / `-encoders` / `-decoders`）：`whisper`（语音生成字幕的内置引擎：
  `ffmpeg_status.whisper` 为 false 时表单提前说明，运行时报错指向 whisper-cli 或完整构建）、`flite`（只有测试用来合成语音：
  没有时真实识别的测试打印原因跳过）、libdav1d（AV1 改用 libaom 解码）、SVT-AV1 / rav1e / libjxl 等编码器、frei0r、libplacebo、
  OpenCL / Vulkan 滤镜、bs2b / sofalizer 等——代码都没有用到。rubberband、zscale、libass、drawtext、vidstab、NVENC / QSV / AMF、
  libx264 / libx265 / libmp3lame / libopus 都有。
- **ffprobe 9.0 与 HLS 测试**：ffprobe 9.0.2 在 `-show_entries stream=…` 时对带节目的容器（MPEG-TS）还输出 `programs[].streams`，
  CSV 输出因此把每条流列两次（9.1 不会）。`tool.hls` 的输出本身在两个版本下相同且正确（每个分片一条 H.264、一条 AAC），
  所以只改了测试的探测：`tool_support::probe_entries` / `stream_count` 改为读 JSON 顶层的 `streams`。
- **端到端测试**：`npm run e2e` 以 `--features e2e-hooks` 构建调试版。这个特性（只能用于调试构建，否则 `compile_error!`）多出两个
  环境变量：`CLIPRECIPE_E2E_FFMPEG_MANIFEST`（用测试清单代替内嵌清单，允许 `http://` 本机地址）与 `CLIPRECIPE_E2E_ISOLATE_FFMPEG=1`
  （不查找程序目录、PATH、node_modules）。正式构建里这些代码不存在，所以无法被触发。`[ffmpeg]` 场景用
  `scripts/e2e-ffmpeg-server.mjs` 的本机假服务器（把 node_modules 里的真 ffmpeg / ffprobe 打成 npm 结构的 tgz，第一个地址返回 503）
  走完整界面流程：缺少提示与导入拦截 → 下载中取消（临时文件清理）→ 下载完成后立即可导入 → 设置页删除。
