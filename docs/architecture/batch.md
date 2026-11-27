# 批量处理：流水线、任务与设置

## 实现一个流水线模块（`pipe.*`）

18 个模块已全部注册（后端 `RecipeModules` 字段、`pipeline::ordered_modules` 应用顺序、
前端 `features/pipeline/index.ts`）。实现一个模块**只改下面这些文件**：

| 文件 | 改什么 |
|---|---|
| `src-tauri/src/pipeline/modules/<模块>.rs` | 参数结构体加字段（`camelCase`、`#[serde(default)]`，`Default` 给默认值）；实现 `apply`；**删除** `implemented()` 的覆盖；写单元测试 |
| `src/features/pipeline/<模块>.tsx` | 参数类型与后端一致；`defaults` 与后端 `Default` 一致；`Form` 换成真正的表单 |
| `src/features/pipeline/icons.ts` | 在这里给模块加一个 lucide 图标（配方栏步骤行左侧的图标块；没有时显示通用图标） |
| `src-tauri/tests/pipeline_<模块>.rs` | 新建集成测试文件（不要改别人的测试文件） |
| `src/features/registry.ts` | 该模块调用 `f(...)` 末尾加 `"done"` |
| `docs/specs/pipeline.md` | 在对应分组的表里加上该模块 |

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

**前端表单**：只用 shadcn/ui 组件和下面的通用控件，不要手写按钮、输入框、开关等（规则见 [frontend.md](frontend.md)）。
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

**当前取值摘要**：`defineModule` 可加可选的 `summary: (params) => string`，配方面板的模块行在名称下一行显示它（已停用时显示“已停用”）；
尽量不超过 24 个汉字宽，没写或出错时退回 `registry.ts` 里的静态说明。数字、范围值、素材来源的格式化用 `features/pipeline/summary.ts`
的 `num` / `range` / `asset` / `join`（例：`summary: (p) => range(p.fps, " fps")`）。

**默认值一致性检查**：`cargo test` 里的 `tests/recipe_defaults.rs` 会运行 `node scripts/dump-recipe-defaults.mjs`
（用 Vite 直接执行前端源码得到 `defaultRecipe()`），与 `Recipe::default()` 逐字段比较（数字按数值比较，前端缺省的键等同后端 `null`）。
`.tsx` 的 `defaults` 与 `.rs` 的 `Default` 不一致时测试失败并列出差异；不需要维护任何共享快照文件。需要 Node.js 和 `npm install`。

**模块作者自动获得的能力（不用写任何代码）**

- **单模块预览**：配方面板里展开模块后，表单下方有“预览此步骤”按钮——只启用本模块、按当前参数对“当前视频”生成
  5 秒低清片段并在预览窗口播放。前提是模块有 `Form`（不为 null）且不在 `PREVIEW_EXCLUDED`（变速、背景音、加头尾、帧率、码率）中。
  预览走的就是 `apply`，所以模块必须正确更新 `ctx.stream`（预览据此缩小画面、截取时长）。
- **整条流水线预览**（`pipe.preview`）：中栏预览舞台标题栏的“生成预览”，同样排除上述五个模块。
- **预览舞台**（`views/batch/PreviewStage.tsx`、`state/stage.ts`）：常驻显示当前视频套用当前配方后的一帧（单帧，不播放），
  视频、配方、相关设置、取帧时间或随机值变化后防抖自动刷新；批量处理中暂停自动刷新（只能手动刷新），预览窗口打开期间不渲染。
  后端只有一个预览槽、新预览会取消旧预览，所以舞台的主画面与裂变变体（`fission` > 1 时最多 3 个，各用不同随机值）依次渲染，
  这些约束与重试写在 `state/stage.ts` 的注释里。
  舞台走的也是 `pipeline::build_preview`，模块不需要为它额外写代码。

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

## 基础架构的实现说明

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

## 流水线级功能的实现说明

- **裂变**（`pipe.fission`）：次数存在配方的顶层字段 `fission`（1..=100）。调度器对每个任务依次跑 N 轮，
  第 r 轮的种子为 `rng::fission_seed(任务种子, r)`（第 1 轮就是任务种子，所以 N=1 与不裂变完全一致）。
  N>1 时输出到 `<输出目录>/<视频名>/<文件名>_<r>.mp4`；开启高级开关 `fissionFolder`（`set.fission-folder`）时改为
  `<输出目录>/<视频名>/第 r 次裂变/<文件名>.mp4`（视频名是去除后的源文件名，文件名是套用输出文件名格式后的结果，见下文“输出命名”）。进度事件带 `round` / `rounds`，`percent` 是全部轮次的总进度；
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
- **队列**：拖动把手排序（因为窗口启用了系统拖放，不能用 HTML5 拖放，改用指针事件；按状态筛选不是“全部”时不能拖动排序）；“置顶”在处理中会调用
  `prioritize_task` 让等待中的任务提前；排序按名称（自然顺序）/ 时长 / 大小。

## 设置与收尾的实现说明

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
- **输出命名**（`jobs/output.rs` 的 `plan_output`，纯函数）：源文件名 → 文件名去除（`set.name-strip`）→ 输出文件名格式
  （`set.name-template`，`render_name`）→ 队列序号前缀 → 裂变编号 → 扩展名 → 重名时重命名 / 覆盖（不覆盖源文件、不抢本批已占用的名字）。
  `{date}` / `{time}` 用 `OutputRequest::started`：前端点“开始”（或“查看命令”）时把本地时间作为 `startedAt` 传给
  `start_jobs` / `preview_command`，经 `RunExtras::started_at` 给整批共用；没有传时用开始时的 UTC 时间。设置页的示例由
  `lib/nameTemplate.ts` 按同样规则计算。收尾（移回源位置）与队列显示都使用调度器回报的实际输出路径，不自己拼文件名。
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
  检查更新 `system::check_update(当前版本, fetch)`（正式用 `ureq` 发一个不带身份信息的 GET 到 GitHub 公开 API，测试注入假的 fetch；
  后来改为优先用 Tauri updater 自动更新，这里成为不能自动更新时的回退，见 [update.md](update.md)）。
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
