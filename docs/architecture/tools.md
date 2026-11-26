# 独立工具（`tool.*`）

34 个工具已全部注册（后端 `tools/mod.rs` 的 `registry!`、命令 `start_tool` 等、
前端 `features/tools/index.ts`、工具箱与队列菜单）。实现一个工具**只改下面这些文件**：

| 文件 | 改什么 |
|---|---|
| `src-tauri/src/tools/<工具>.rs` | 参数结构体加字段（`camelCase`、`#[serde(default)]`，`Default` 给默认值）；需要时调整 `inputs()` / 覆盖 `plan()`；实现 `run()`；**删除** `implemented()` 的覆盖；写单元测试 |
| `src/features/tools/<工具>.tsx` | 参数类型（用 `type X = {..}`，不要用 `interface`）与后端一致；`defaults`、`inputs` 与后端一致；`Form` 换成真正的表单 |
| `src-tauri/tests/tool_<工具>.rs` | 新建集成测试文件 |
| `src/features/registry.ts` | 该工具调用 `f(...)` 末尾加 `"done"`（工具箱与队列菜单据此启用它） |
| `docs/specs/tools.md` | 在对应分组的表里加上该工具 |

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
  （沿用 `pipe.bitrate`；未启用时见 [batch.md](batch.md) 的“默认码率”，源码率未知时恒定质量）；`Encoding::audio_args(声道数)` 为 AAC 参数。硬件编码失败自动改用软件编码重试，
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

**前端表单**：规则与流水线模块相同（只用 shadcn/ui 与通用控件，见 [batch.md](batch.md) 的控件表；表单通过 `onChange(局部参数)` 提交）。
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

## 独立工具框架的实现说明

- **一次一个工具**：`start_tool` 先在后台规划工作项（校验输入、列出文件夹），返回工作项清单与种子，再开始执行；
  已有工具在运行时拒绝。停止 `stop_tool`、取消单项 `cancel_tool_item`。
- **分析类运行**：`ffmpeg::run_with_stderr` 在原有运行函数（进度、取消、卡死保护）之上把 stderr 每一行交给回调；
  `run` / `run_with_stall_timeout` 的行为不变。
- **修订**：默认码率改为“每像素码率 + 质量下限”（见 [batch.md](batch.md) 的“基础架构的实现说明”）并用于工具与批量处理；`EncodeBasis` / `run_encode_from`；
  `PlanRequest::seed`（规划阶段随机）；`tools/common/` 与 `features/tools/shared/`；工具预览；黑边精确检测；SAR 统一；
  队列行显示工具进度、对话框拖入文件夹；工具表单按需加载。
- **共享改动**：`jobs::output::folder_relative` 改为公开；`fsutil::move_file`（跨分区时复制后删除）；`components/OutputPicker.tsx`
  从 BatchView 中提取出来供工具对话框复用；`LogPanel` 可传 `filter` / `className` / `bodyClassName`；
  `scripts/dump-recipe-defaults.mjs tools` 输出工具声明。
