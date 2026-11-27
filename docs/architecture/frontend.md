# 前端约定

- 界面文字用中文。
- **控件一律用 shadcn/ui**（Base UI 版，`components.json`，导入别名 `@/` → `src/`）。shadcn 有的控件**不准手写**：
  Button、Switch、Input、Textarea、Label、Select、Slider、Checkbox、RadioGroup、Tabs、ToggleGroup、Dialog、AlertDialog、
  Tooltip、Popover、DropdownMenu、ScrollArea、Progress、Separator、Badge、Card、Collapsible、InputGroup、Field、Spinner 等；
  也不要写原生 `<button>` / `<input>` / `<select>` / `<textarea>`、原生取色器、自制遮罩/弹窗/提示框，或 `window.confirm`（用 AlertDialog）。
  需要的组件还没有时用 CLI 添加：`npx shadcn@latest add <组件>`（生成到 `components/ui/`，不要凭记忆手抄）。
  只有 shadcn 没有的业务控件才自己写，并且必须由 shadcn 组件组合而成（见 [batch.md](batch.md) 的控件表）。
- **shadcn 风格是 Mira 的 Base UI 版**（`components.json` 的 `"style": "base-mira"`：组件基于 `@base-ui/react`、lucide 图标、
  CSS 变量、基础色 neutral；等同于 `npx shadcn@latest init --base base --preset b1D0dv72`，预设码 `b1D0dv72` = Mira 样式 + lucide + 中性色，
  见 `npx shadcn preset decode`）。**`components/ui/` 是 CLI 生成后按本应用的设计改过的**，不再与注册表原版一致：可以改组件本身，
  也可以在使用处写外观类。约定：
  - 添加组件：`npx shadcn@latest add <组件>`，生成后按本应用的设计调整。更新已有组件时不要直接 `--overwrite`（会丢掉本应用的改动）：
    先 `npx shadcn@latest add <组件> --diff` 看上游差异，再手动合并。没有“与原版一致”的检查（`check:shadcn` 已删除）。
  - 已改过的组件及原因：`button`（有只给页面唯一主操作用的 `xl` 尺寸）；`switch`（更大；未选中为透明底 + 边框 + 小滑块，因为在琥珀主色下
    选中与未选中的轨道亮度太接近）；`slider`（粗轨道、竖条滑块头、已选段用前景色）；`toggle`（有 `pill` 变体，队列状态筛选用）；
    `toggle` / `toggle-group` / `tabs`（全应用的单选分段只有一种长相，见下方“选中态与对比度”：`toggle` 用 `segmented` 变体代替原来的
    `outline`，`toggle-group` 按 spacing 把它画成轨道里的段或独立小块，`tabs` 的横向 `default` 变体改成同样的轨道 + 段并加 `size`；
    竖向 Tabs 的样式不变）；`badge` / `field`（`text-primary` 改为 `text-primary-ink`）。再改组件时同样在这里记“改了什么、为什么”，不要抄类名。
  - 组件里个别用不到的导入（如 `scroll-area.tsx` 的 `import * as React`）会撞上 `noUnusedLocals`；`npm run typecheck`
    （`scripts/typecheck.mjs`）只对 `components/ui/` 忽略这一类（TS6133），业务代码照常检查。
  - **应用只有深色主题**（视频工具，设计稿只有深色；没有主题设置、没有切换）。
  - **配色与密度由 `index.css` 的主题变量统一决定**：shadcn 变量以 CLI 为 Mira + neutral 写出的深色（`.dark`）默认值（oklch）为底，
    只有一组、定义在 `:root` 上；中性色取自设计稿（带一点冷色调），调整处在行尾注明默认值，原因与约束写在 `index.css` 头部注释：
    - 为通过界面审计（见下方“选中态与对比度”）：`--muted`（CLI 0.269 0 0）、`--input` 白 15%→45%、`--ring` 0.556→0.6。
    - 主色 `--primary` 是琥珀色（CLI 默认是浅灰），`--primary-foreground` 是深色字；用在主按钮、开关、进度条、选中项的描边与浅底、画面上的框线。
      琥珀色的**文字 / 图标用自有 token `--primary-ink`（`text-primary-ink`），不要写 `text-primary`**：现在它就等于 `--primary`，
      主色以后调到当文字不够 4.5 时只改这一个 token。
    - `--warning` 取偏黄的色相，以便与主色、`--destructive` 区分。
    - `--background` 不要动（`desktop.rs` 的启动底色按它核对）。
    使用处可以写外观类，但颜色只引用 token；不要用全局 CSS（按 `data-slot` 等）覆盖组件外观，也不要在使用处另写字号 / 高度去凑密度（见下方“密度与字体”）。
    Mira 预设的 Inter 是远程 / 打包字体，CSP 不允许，`--font-sans` 用系统界面字体。必要的行为放在 `components/` 的薄封装或 `lib/desktop.ts`（例如输入框关闭拼写检查与自动填充）。
  - **Base UI 的用法**（与 Radix 版不同处）：组合用 `render` 属性（`<PopoverTrigger render={<Button variant="outline" />}>文字</PopoverTrigger>`），
    没有 `asChild`；状态属性是 `data-active`（Tabs）、`data-checked` / `data-unchecked`（Switch、Checkbox、菜单勾选项）、`data-pressed`（Toggle）、
    `data-open` / `data-closed`（浮层），没有 `data-state`；ToggleGroup 的值总是数组（单选取 `[value]`，多选加 `multiple`）；
    Select 要传 `items`（`{ value, label }[]`），否则 SelectValue 显示原始值，`onValueChange` 可能给 `null`；
    菜单项用 `onClick`（没有 `onSelect`，写了不会报错也不会触发）；菜单的 Label 必须放在 `DropdownMenuGroup` / `ContextMenuGroup` 里；
    `AlertDialogAction` 只是按钮，不会关闭对话框（用受控的 `open`，在 onClick 里关闭）；Slider 的值传数组（`[value]`）。
  - **竖向 Tabs（设置分组、工具箱的分类与工具列表）的标签栏用 `components/VerticalTabsList.tsx`**（代替 `TabsList`，外层仍是 `<Tabs orientation="vertical">`）。
    原因：shadcn（base-mira）的 `Tabs` 把 `orientation` 解构出来，只写成 `data-orientation`（决定竖向样式），没有传给 Base UI 的
    `Tabs.Root`，Base UI 因此按横向处理：方向键是 ← / →，没有 `aria-orientation`（2026-11 时注册表里最新的 base-mira `tabs` 仍如此，
    `npx shadcn@latest add tabs --diff` 无差异）。`VerticalTabsList` 在 TabsList 上补 `aria-orientation="vertical"`（Base UI 把使用处的属性
    合并在默认属性之后），并在 onKeyDown 里只改按键映射：它先于 Base UI 的处理执行、拿到同一个事件对象，把 ↑ / ↓ 改成 ← / → 交给
    Base UI，← / → 用 `preventBaseUIHandler` 拦下。移动焦点、首尾循环、Home / End、回车 / 空格选中都仍由 Base UI 完成，所以它记录的
    高亮项、roving tabindex 与焦点始终一致（Tab 离开再回来落在刚才的标签上）。不要改成自己调用 `focus()`：Base UI 只在收到 focus 事件时
    同步高亮项，窗口不在前台时 `focus()` 不发事件，之后 Home 等按键会以旧的高亮项为准。横向 Tabs 照常用 `TabsList`。不采用的做法：用 `render` 再套一层带
    `orientation` 的 `Tabs.Root`（value / onValueChange 写两遍、两层 Root）；直接用 Base UI 的 `Tabs.Root` 重写 shadcn 的 `Tabs`（等于手写
    shadcn 已有的控件）。上游修正后删掉 `VerticalTabsList`、改回 `TabsList`，再按上面“更新已有组件”的做法更新 `tabs`。
- 页面自己的普通元素（队列行、状态栏、日志行、标题、说明文字）只用 `index.css` 里的一套 token：shadcn 的语义变量（`background`、`foreground`、
  `card`、`popover`、`primary`、`secondary`、`muted`、`muted-foreground`、`accent`、`border`、`input`、`ring`、`destructive`），
  另加本应用自有的 `success`、`warning`（状态色）、`primary-ink`（主色的文字 / 图标色）、`titlebar`（系统标题栏底色，= card）、`media` / `media-foreground`（视频画面区的底色与
  压在画面上的文字、框线）。自有 token 和 shadcn 变量写法一样（`:root` 上的 oklch 值），经 `@theme inline`
  暴露成 `bg-success`、`text-warning`、`bg-media` 等；新颜色先找已有 token，确实没有再加在那里。不要写死颜色（十六进制、`rgb()`、`bg-[#…]`、
  `bg-black` 之类）；用户选择的颜色（水印、文字颜色等参数）是数据，不算。shadcn 组件里的 `dark:` 变体要始终生效：`index.html` 写死
  `<html data-theme="dark">`（没有代码改它），`index.css` 的 `@custom-variant dark` 匹配这个属性；不要删掉它。

## 桌面应用体验约定

界面要像桌面程序而不是网页（参考剪映专业版那种深色、紧凑的工具感，但不照抄布局与图标）。新界面照下面做，全局部分已经实现：

- **窗口**：保留系统窗口框架（贴靠布局、缩放边框、阴影、圆角、系统菜单都由系统提供）。主窗口隐藏创建，前端画好第一帧后
  `window_ready` 才显示。窗口主题创建时固定为深色（系统标题栏与 WebView 的 `prefers-color-scheme` 都是深色，不随系统切换）；
  Windows 11 的标题栏 / 标题文字 / 边框颜色经 DWM 设为 `--titlebar` / `--foreground` / `--border`（`state/window-chrome.ts` 在 canvas 上把 oklch 换算成 `#rrggbb`，带透明度的叠在 `--background` 上；
  只在平台为 Windows 时换算并传给后端；其它平台传空串，后端把三个颜色恢复为系统默认 `DWMWA_COLOR_DEFAULT`）；
  窗口显示前的底色是 `desktop.rs` 的 `BOOT_BACKGROUND`（`--background` 换算成的 `#0f1012`，单元测试按 `index.css` 核对），所以首帧不闪白；
  Windows 10 与其它平台只有系统的深色标题栏（`src-tauri/src/desktop.rs`、`state/window-chrome.ts`）。窗口位置、大小、最大化由
  tauri-plugin-window-state 记在配置目录的 `window-state.json`（保存的位置不在任何显示器上时移回主屏中央）；单实例
  （tauri-plugin-single-instance）：再次启动时激活已有窗口（含最小化、托盘隐藏），命令行里的文件经 `open-paths` 事件导入。
  设置了 `CLIPRECIPE_CONFIG_DIR` 时视为另一套实例，不启用单实例（端到端测试另用 `CLIPRECIPE_E2E_SINGLE_INSTANCE` 验证）。
- **没有网页行为**（`lib/desktop.ts`，main.tsx 安装）：空白处右键什么也不弹，应用菜单用 shadcn ContextMenu，输入框保留系统编辑菜单；
  刷新、查找、打印、保存 / 打开网页、查看源码、光标浏览（F7）、缩放（Ctrl +/-/0、Ctrl + 滚轮、双指）、前进后退（Alt + 方向键、鼠标侧键、滑动）
  全部无效，Ctrl+A 只在输入框里全选；正式版另由 WebView2 关闭浏览器快捷键与自动填充（调试版保留 F12 开发者工具）。
  不要写 `<a href>` 导航，外部网址经 `open_project_page`（只允许本项目地址）在系统浏览器打开；不用 HTML5 拖放（拖动排序用指针事件）。
- **光标与选择**：控件（按钮、标签页、开关、菜单项、选项）一律箭头光标，手形只给真正的超链接（`index.css` 已统一，不要加 `cursor-pointer`）。
  界面文字不可选中（`body { user-select: none }`）；值得复制的内容加 `select-text`：处理记录、路径、命令、错误信息、关于 / 运行环境。
  输入框是文本光标。鼠标点击不出焦点框，键盘焦点环（`focus-visible`）保留。
- **提示**：不用原生 `title` 属性（e2e 会检查页面上没有 `[title]`）。一律用 `components/Hint.tsx`（shadcn Tooltip，500 ms 延迟、
  反色提示）；有快捷键时传 `keys`（显示为 Kbd）；按钮可能被禁用时加 `disabledTrigger`。
  `Hint` 用 TooltipTrigger 的 `render` 把触发器渲染成被包的控件。Base UI 各组件的状态属性互不重叠（提示是 `data-popup-open`），
  不会像 Radix 的 `asChild` 那样把选中态改掉；但 shadcn 的 TooltipTrigger 传进来的 `data-slot="tooltip-trigger"` 会覆盖被包控件自己的
  `data-slot`（Mira 的样式与 ButtonGroup、InputGroup 等按 data-slot 选择元素），`Hint` 合并属性时去掉了它。自己用 `render` 把一个
  shadcn 触发器套在另一个 shadcn 控件上时也要注意这一点（e2e 的界面状态审计会检查带状态的控件的 data-slot 没有变成 tooltip-trigger）。
- **选中态与对比度**：用组件自带的选中样式，使用处不另写外观类。“在几个互斥选项间切换”只有一种长相——**分段切换**：一条轨道
  （细 `border` 边框、`background` 底、2px 内边距），里面的段没有描边、圆角小一号，选中段 muted 底 + 前景色 + 加粗，未选中次要色、悬停浅底，
  键盘焦点是 ring 色光晕；轨道总高与同尺寸的按钮 / 输入框相同。它有两种写法：ToggleGroup 的 `segmented` 变体（`ChoiceGroup` 默认用它；
  `spacing={0}` 是轨道，选项放不下时轨道换成多行；`spacing` ≥ 1 是排成栅格的独立小块，块用 `border` 边框、`background` 底，选中态同上；
  预览舞台顶栏用 `size="sm"`）和横向 Tabs 的 `default` 变体（顶栏主导航用 `size="lg"`，文本模块的“文本 1/2/3”用默认尺寸）。
  轨道 / 小块的底总是不透明的 `background`，所以压在 card、secondary、background 哪种底色上，选中段（muted）与未选中段的底色对比都是 1.39。
  不属于这一类的：队列的状态筛选是 ToggleGroup 的 `pill` 变体（筛选标签）；设置分组、工具箱的分类 / 工具列表是竖向 Tabs（导航列表，
  使用处写了选中样式：muted 底，工具列表另有主色描边）。
  规则：选中项与未选中项的底色亮度对比 ≥ 1.2（只改文字颜色不算选中态），文字对比满足 WCAG AA（正文 4.5:1、大字 3:1），
  控件边界（输入框、复选框、开关）与背景 3:1，键盘焦点环描边与背景 3:1，禁用的控件按 WCAG 豁免；e2e 的界面状态审计
  （`scripts/e2e-audit-page.js`）逐组检查，全部是硬失败（`SOFT_RULES` 为空）。shadcn 默认的深色配色达不到其中几条（输入框与开关边界、
  竖向 Tabs 选中项、焦点环），所以 `index.css` 调整了 `--muted` / `--input` / `--ring`
  （取值见上方“组件与配色”；调整后：边界 ≥ 3.26，选中底 ≥ 1.27，焦点环对标签栏底 3.59，占位符 ≥ 4.65）。
  改这些令牌时要同时满足 `index.css` 注释里列的几组对比。
- **快捷键**：`lib/shortcuts.ts` 是唯一来源：每个快捷键定义一次按键组合（主修饰键 `mod`、Shift、Alt、主键），
  按当前平台（`lib/platform.ts`）生成显示文字（macOS 用符号 `⇧⌘O`，其余 `Ctrl+Shift+O`）并匹配按键事件（`matchShortcut`：macOS 的主修饰键是
  metaKey，其余是 ctrlKey，另一个修饰键必须没按）。按钮提示的 `keys`、菜单里的 `ContextMenuShortcut`、设置 → 关于里的快捷键表（`shortcutGroups`）
  都用 `useShortcutLabels()` / `shortcutLabel()` 取文字，不要写死 `Ctrl+`。全局快捷键在 App.tsx 的 `useAppShortcuts`
  （添加视频 / 文件夹 ⌃O / ⌃⇧O、开始 ⌃Enter、Esc 停止、⌃, 设置、⌃1/2/3 切换页面、F1 文档；⌃ 指主修饰键），
  有对话框或菜单打开时让给它们；队列列表（listbox）获得焦点时：方向键 / Home / End、Shift 扩展选择、空格、Enter 播放、全选、移除所选
  （Windows / Linux 为 Delete，macOS 为 ⌫）。新增快捷键时在 `lib/shortcuts.ts` 加定义，按钮提示与处理都引用它的 ID。
  对话框的焦点限制与关闭后焦点回到触发按钮由 Base UI 负责。
- **滚动条与滚动**：长内容用 shadcn ScrollArea；原生滚动条（文本框、对话框）由 `index.css` 设为细的、颜色与 ScrollArea 滑块相同（`--border`，标准的 `scrollbar-width` / `scrollbar-color`），没有回弹滚动。
- **密度与字体**：密度由 `index.css` 的 `@theme { --text-xs; --spacing }` 两个变量统一放大（正文 13 px、间距刻度 0.265rem，控件约 30 px、标题栏约 47 到 51 px），不要在组件上另写字号 / 高度去压缩或放大。
  字体是各平台的系统界面字体（`index.css` 的 `--font-ui` 按 `<html data-platform>` 切换，经 `--font-sans` 生效：Windows 为 Segoe UI / Microsoft YaHei，
  macOS 为系统字体 / PingFang SC，Linux 为 system-ui / Noto Sans CJK SC 等；不要在组件上写字体。Mira 预设的 Inter 是远程 / 打包字体，CSP 不允许），
  `body` 的字号是正文 `text-xs/relaxed`（取上面的变量值）（不要改 `<html>` 的字号，那是 rem 的基准），数字等宽（全局 `tabular-nums`）；
  圆角用 `--radius`（0.625rem）刻度；
  不要做网页式的大号空状态、大圆角卡片、居中的营销文案。
- **动效**：用 Mira 组件自带的动效（浮层淡入 + 轻微缩放 / 位移，不再用全局规则归零），尊重系统的“减少动态效果”（`index.css` 在 `prefers-reduced-motion` 时关闭动画）。
- **状态**：窗口底部状态栏（`views/StatusBar.tsx`）显示队列数量、运行状态与总进度、FFmpeg 与实际会用的编码器（读后端已缓存的探测结果 `encoder_info_cached`，还没探测时如实说明）；批量处理或独立工具运行时
  任务栏按钮显示进度（`state/taskbar.ts` → `set_taskbar_progress`），结束清除，有失败时显示红色直到用户回到窗口。

## 平台来源（`lib/platform.ts`）

前端**只从这里**读当前平台（`"windows" | "macos" | "linux"`），不要在别处读 `navigator.userAgent` / `navigator.platform`，
也不要自己用 `e.ctrlKey || e.metaKey` 判断主修饰键。

- 真实平台：桌面应用里是后端 `app_info` 的 `platform`（Rust 编译目标的 `std::env::consts::OS`），启动时 `state/events.ts` 调用
  `loadRealPlatform()` 读取；读到之前、以及 `npm run dev` 在浏览器里跑（没有 Tauri）时用 User-Agent 推断（WebView 的 UA 与编译目标一致）。
- 用法：组件用 `usePlatform()`（平台变化时重新渲染），事件处理等用 `getPlatform()`；主修饰键用 `isModKey(e)`；
  快捷键用 `lib/shortcuts.ts`；系统文件管理器的名字 `FILE_MANAGER[platform]`；说明文字与占位符里的示例路径 `EXAMPLE_PATHS[platform]`
  （只是示例，真实路径的处理与平台无关）。`<html data-platform>` 跟随当前平台（`main.tsx` 首次渲染前写入），`index.css` 据此切换字体。
- 开发环境下可被调试面板模拟（`simulatePlatform`）；生产构建里 `import.meta.env.DEV` 是常量 false，模拟分支被剔除，永远是真实平台。
- 后端的平台差异仍是编译期 `cfg(target_os)`，模拟不影响。

## 开发环境的调试面板（`src/debug/`）

只在开发环境（`npm run dev`、`npm run tauri dev`）存在，用来在一台机器上检查各平台的前端表现和难复现的界面状态。

- **入口**：状态栏最右端的小虫图标按钮，或快捷键 Ctrl+Shift+D（macOS ⇧⌘D；按**真实**平台匹配，模拟 macOS 时在 Windows 键盘上仍按
  Ctrl+Shift+D）。打开后是右侧抽屉（shadcn Sheet）。面板里的选择存在 localStorage 的 `cliprecipe.debug-panel`，刷新后重新应用；
  “全部恢复默认”只恢复面板自己的东西：关掉所有模拟、取消平台模拟、清除面板的 localStorage；**不改设置**。
- **平台模拟**：真实 / Windows / macOS / Linux。只改前端表现：快捷键的显示与按键、界面字体、标题栏着色（只在 Windows 时调用 DWM 着色）、
  示例路径与文件管理器名称。后端行为（编码器、字体目录、回收站、文件对话框……）仍是真实平台的，面板上有说明。
  在 Windows 上模拟 macOS 时 ⌘ 对应 Win 键，Win+数字等会被系统拦截，验证按键可以在开发者工具里派发带 `metaKey` 的 KeyboardEvent。
- **状态模拟**（`src/debug/simulations.ts`）：把模拟数据注入前端状态层（`state/ffmpeg`、`state/queue`、`state/log` 的 zustand store），
  视图照常从状态层读取，**视图里没有任何调试分支**。每项可单独打开 / 恢复：打开时记下被替换的真实值，恢复时放回；注入的队列行与日志按 ID 移除，
  不影响用户自己的数据。桌面应用里后端异步回来的真实结果（启动检测 FFmpeg、`jobs_running`）由订阅守住：真实值记为恢复值，界面仍显示模拟值。
  - 缺少 FFmpeg：`useFfmpeg.status` 换成“未找到、可下载”，出现批量处理页提示、状态栏与设置 → 关于 → 运行环境的对应状态，导入 / 开始处理被拦下。
  - FFmpeg 下载进行中 / 下载失败（互斥）：`useFfmpeg.download` 为 running（进度循环推进；点“取消”结束模拟）或 failed。
  - 队列里各种状态的任务：处理中（含裂变 2/3）、等待、完成、失败、取消各 2 个，`running` 为 true，状态栏显示总进度。
  - 处理记录里各级别日志：调试 / 信息 / 警告 / 错误各 2 条。
  浏览器里同样可用（设置页的运行环境在浏览器里只在状态层有 FFmpeg 状态时显示 FFmpeg 两行）。
- **模拟数据不写盘、不发给后端**（`npm run tauri dev` 里也不改动、不污染真实数据）：
  - 模拟的队列行的 ID 与路径登记在 `lib/simulated.ts`。状态层保存队列（`unfinishedTasks` → `queue.json`）、开始处理、插队、重载同名字幕封面时
    排除它们；停止、取消在前端把模拟行改为已取消。兜底：`lib/backend.ts` 的命令调用在参数里发现登记过的值时直接拒绝并说明是模拟数据
    （播放、工具、预览、媒体信息等入口因此不会把假路径发给后端）。生产构建里这些检查不存在（`import.meta.env.DEV`）。
  - 任一 FFmpeg 模拟打开期间，`useFfmpeg` 里会触发后端的操作（下载、取消下载、删除、重新检测）换成前端的：点“下载 / 重试下载”进入“下载进行中”模拟，
    “取消”结束它，删除与重新检测只在处理记录里说明；全部 FFmpeg 模拟关闭后换回。处理记录本来就不写盘。
  - 模拟平台时标题栏：切到 macOS / Linux 时后端把 DWM 颜色恢复为系统默认，切回 Windows 重新着色。
- **测试**：`npm run test:debug-panel`（`scripts/test-debug-panel.mjs`，用 Vite 按开发模式加载真实模块，断言保存队列排除模拟行、命令参数识别模拟值、
  停止 / 取消不调用后端、FFmpeg 操作被替换与换回、恢复默认不改设置）；`npm run e2e:debug-panel`（`--mode dev --debug-panel`：在 `tauri dev` 的真实窗口里
  验证 queue.json 不含模拟行、操作不发出后端命令、开着模拟重启没有缺失警告、FFmpeg 模拟不调用下载 / 重新检测、恢复默认不改 settings.json、
  模拟 macOS / Linux 时 DWM 标题栏颜色为系统默认并在切回后恢复）。
- **调试信息**（只读）：窗口尺寸（随缩放更新）、真实平台及来源、当前 / 模拟平台、应用版本（浏览器里读 package.json）、
  FFmpeg / FFprobe 的路径与版本（取自后端 `ffmpeg_status`，其中 `ffprobeVersion` 为本面板新增；“缺少 FFmpeg”模拟打开时仍显示真实值；取不到时写明原因），
  “复制全部”（剪贴板 API 被拒绝时改为选中整段后复制选区）。
- **生产构建不含面板**：`App.tsx` 里 `const DebugPanel = import.meta.env.DEV ? lazy(() => import("./debug/DebugPanel")) : null`，
  生产构建时条件是常量 false，动态导入连同 `src/debug/` 整个被摇树剔除；平台模块里的模拟分支同理。
  检查：`npm run build` 后在 `dist/` 里搜“调试面板”“cliprecipe.debug-panel”“simulatePlatform”等都应搜不到。
  其它代码不要静态导入 `src/debug/` 里的任何东西。

## 界面的端到端检查

- **桌面体验的端到端检查**（见上方“桌面应用体验约定”）：页面与原生窗口标题；首帧（探针在文档创建时记录第一帧的 `data-theme`
  与底色，以及之后每次变化：第一帧就是深色、之后不变；启动时与重启后各查一次，重启前往 settings.json 写入旧版本的 `"theme": "light"`，
  确认旧设置文件照常读取且不影响深色）；空白处右键被拦截、输入框保留编辑菜单；刷新 / 查找 / 打印 /
  缩放 / 后退等按键与 Ctrl+滚轮被拦截且页面没有重新加载；控件光标、界面文字不可选、页面上没有 `title` 属性；Ctrl+1/2/3、Ctrl+,、
  Ctrl+O 与队列列表的 Ctrl+A / End / Shift+↑ / Delete；Esc 停止批量处理；`set_taskbar_progress` 在处理中为 normal、结束后为 none；
  用 Win32（PowerShell）移动窗口后重启，位置大小恢复且 `window-state.json` 在隔离的配置目录里；单实例：主会话带
  `CLIPRECIPE_E2E_SINGLE_INSTANCE=1`（只在 `e2e-hooks` 构建中存在：即使设置了配置目录也启用单实例，并改用应用标识
  `com.xiaofany.cliprecipe.e2e`，不会激活用户正在运行的程序），把窗口最小化后带一个视频路径再启动一次，第二个进程退出、
  原窗口恢复、视频进入队列。报告目录里另有含系统标题栏的真实窗口截图（`*-window-*.png`）。
- **界面状态审计**（`scripts/e2e-audit-page.js` 注入页面，`window.__e2e.audit({ root })`；场景“界面状态审计”）：只有深色，跑一遍，
  覆盖三个页面、设置的六个分组、18 个模块的表单（先经“添加步骤”加入配方再逐个展开）、34 个工具的工作区，另在 FFmpeg 下载提示与设置页各审计一次。检查：
  ① 同组选项（标签页、分段选择、按钮组、列表）里选中项与未选中项明显不同——底色对比 ≥ 1.2，或有只属于选中项的指示条 / 边框 / 阴影，
  只差文字颜色不算；② `aria-selected` / `aria-checked` / `aria-pressed` 与 Base UI 的状态属性（`data-active`、`data-checked` / `data-unchecked`、`data-pressed`）一致，带状态的控件的 `data-slot` 没有被提示改写成 `tooltip-trigger`；③ 开关、复选框选中与未选中不同，
  它们和输入框的边界对比 ≥ 3；④ 每处文字（含占位符）与逐层叠加后的实际背景对比 ≥ 4.5（大字 3），禁用的豁免。
  另外显式断言顶栏导航与设置分组的选中项（aria / data-active / 可见底色）以及顶栏导航的键盘焦点环（对比 ≥ 3）。颜色一律经 canvas 换算
  （oklab、color-mix 也能比较），读取前先把鼠标移开、取消焦点并等过渡结束。
