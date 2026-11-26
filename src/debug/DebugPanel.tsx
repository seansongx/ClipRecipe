// 开发环境的调试面板：状态栏右端的小按钮（或 Ctrl+Shift+D / ⇧⌘D）打开右侧抽屉，内容是平台模拟、主题切换、
// 界面状态模拟与只读的调试信息。只在 import.meta.env.DEV 时由 App.tsx 动态导入；生产构建里没有这段代码。
// 说明见 docs/architecture/frontend.md“开发环境的调试面板”。

import { useEffect, useRef, useState } from "react";
import { Bug, Copy, RotateCcw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { FieldDescription, FieldLegend, FieldSet, Field, FieldContent, FieldLabel, FieldGroup } from "@/components/ui/field";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Sheet, SheetContent, SheetDescription, SheetFooter, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import ChoiceGroup from "@/components/ChoiceGroup";
import Hint from "@/components/Hint";
import { getAppInfo, isTauri, type ThemeSetting } from "@/lib/backend";
import { getRealPlatform, PLATFORM_LABELS, PLATFORMS, usePlatform, type Platform } from "@/lib/platform";
import { formatCombo, matchShortcut, shortcutLabel } from "@/lib/shortcuts";
import { useFfmpeg } from "@/state/ffmpeg";
import { useSettings } from "@/state/settings";
import { realFfmpegStatus, SIMULATIONS } from "./simulations";
import { restoreSavedChoices, useDebug } from "./store";
import { version as packageVersion } from "../../package.json";

const REAL = "real";

const THEME_OPTIONS: { value: ThemeSetting; label: string }[] = [
  { value: "system", label: "跟随系统" },
  { value: "light", label: "浅色" },
  { value: "dark", label: "深色" },
];

/**
 * Ctrl+Shift+D（macOS ⇧⌘D）开关面板。按真实平台匹配（模拟 macOS 时在 Windows 键盘上仍按 Ctrl+Shift+D，
 * Win 键组合会被系统拦截）。对话框打开时也可用（面板本身就是对话框，需要能用同一组键关掉）。
 */
function useToggleShortcut() {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat || !matchShortcut(e, "debugPanel", getRealPlatform().platform)) return;
      e.preventDefault();
      const { open, setOpen } = useDebug.getState();
      setOpen(!open);
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, []);
}

function useWindowSize() {
  const [size, setSize] = useState(() => ({ width: innerWidth, height: innerHeight, dpr: devicePixelRatio }));
  useEffect(() => {
    const onResize = () => setSize({ width: innerWidth, height: innerHeight, dpr: devicePixelRatio });
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return size;
}

function useAppVersion(): string {
  const [version, setVersion] = useState(
    isTauri ? "读取中…" : `${packageVersion}（package.json；浏览器预览没有后端 app_info）`,
  );
  useEffect(() => {
    if (!isTauri) return;
    getAppInfo()
      .then((info) => setVersion(info.version))
      .catch((e) => setVersion(`读取失败：${String(e)}`));
  }, []);
  return version;
}

/** 调试信息的各行（显示与“复制全部”共用）。 */
function useDebugInfo(): [string, string][] {
  const size = useWindowSize();
  const platform = usePlatform();
  const simulated = useDebug((s) => s.platform);
  const version = useAppVersion();
  const simulations = useDebug((s) => s.simulations);
  // 订阅 FFmpeg 状态：真实状态变化（重新检测、下载）时刷新
  useFfmpeg((s) => s.status);
  const real = getRealPlatform();
  const ffmpeg = realFfmpegStatus();
  const checking = useFfmpeg((s) => s.checking);

  const ffmpegRows = (): [string, string][] => {
    if (!isTauri) {
      const reason = "取不到：浏览器预览没有 Tauri 后端（ffmpeg_status 需要桌面应用）";
      return [
        ["FFmpeg", reason],
        ["FFprobe", reason],
      ];
    }
    if (!ffmpeg) return [["FFmpeg / FFprobe", checking ? "正在检测…" : "还没有检测结果"]];
    const reason = ffmpeg.error ? `取不到：${ffmpeg.error}` : "取不到：后端没有返回版本";
    return [
      ["FFmpeg 路径", ffmpeg.path],
      ["FFmpeg 版本", ffmpeg.version ?? reason],
      ["FFprobe 路径", ffmpeg.ffprobePath],
      ["FFprobe 版本", ffmpeg.ffprobeVersion ?? (ffmpeg.version ? reason : "取不到：ffmpeg 检测失败，没有继续检测 ffprobe")],
      ["FFmpeg 来源", ffmpeg.source],
    ];
  };

  return [
    ["窗口尺寸", `${size.width} × ${size.height} CSS 像素（设备像素比 ${size.dpr}）`],
    ["真实平台", `${PLATFORM_LABELS[real.platform]}（${real.source === "tauri" ? "后端 app_info" : "User-Agent 推断"}）`],
    ["当前平台", simulated ? `${PLATFORM_LABELS[platform]}（模拟）` : `${PLATFORM_LABELS[platform]}（未模拟）`],
    ["运行环境", isTauri ? "桌面应用（Tauri）" : "浏览器预览（npm run dev）"],
    ["应用版本", version],
    ...ffmpegRows(),
    ["打开的状态模拟", simulations.length ? simulations.join(", ") : "无"],
  ];
}

function PlatformSection() {
  const simulated = useDebug((s) => s.platform);
  const setPlatform = useDebug((s) => s.setPlatform);
  const real = getRealPlatform().platform;
  const options: { value: Platform | typeof REAL; label: string }[] = [
    { value: REAL, label: `真实（${PLATFORM_LABELS[real]}）` },
    ...PLATFORMS.map((p) => ({ value: p, label: PLATFORM_LABELS[p] })),
  ];
  return (
    <FieldSet>
      <FieldLegend>平台模拟</FieldLegend>
      <FieldDescription>
        只模拟前端表现：快捷键的显示与按键（macOS 用 ⌘）、界面字体、标题栏着色（仅 Windows 调用）、示例路径。
        后端行为（编码器、字体目录、回收站、文件对话框等）仍是真实平台 {PLATFORM_LABELS[real]} 的。
      </FieldDescription>
      <ChoiceGroup
        label="模拟的平台"
        value={simulated ?? REAL}
        options={options}
        onChange={(value) => setPlatform(value === REAL ? null : value)}
      />
    </FieldSet>
  );
}

function ThemeSection() {
  const theme = useSettings((s) => s.settings.theme);
  const setTheme = useDebug((s) => s.setTheme);
  return (
    <FieldSet>
      <FieldLegend>主题</FieldLegend>
      <FieldDescription>
        与设置 → 通用里的“主题”是同一个设置{isTauri ? "（会保存到设置文件）" : ""}，不在“全部恢复默认”的范围内。
      </FieldDescription>
      <ChoiceGroup label="主题" value={theme} options={THEME_OPTIONS} onChange={setTheme} />
    </FieldSet>
  );
}

function SimulationSection() {
  const active = useDebug((s) => s.simulations);
  const setSimulation = useDebug((s) => s.setSimulation);
  return (
    <FieldSet>
      <FieldLegend>状态模拟</FieldLegend>
      <FieldDescription>把模拟数据注入前端状态层；关掉即恢复，不影响自己导入的视频与已有的记录。</FieldDescription>
      <FieldGroup>
        {SIMULATIONS.map((sim) => {
          const id = `debug-sim-${sim.id}`;
          return (
            <Field key={sim.id} orientation="horizontal">
              <FieldContent>
                <FieldLabel htmlFor={id}>{sim.label}</FieldLabel>
                <FieldDescription>{sim.description}</FieldDescription>
              </FieldContent>
              <Switch
                id={id}
                checked={active.includes(sim.id)}
                onCheckedChange={(on) => setSimulation(sim.id, on)}
              />
            </Field>
          );
        })}
      </FieldGroup>
    </FieldSet>
  );
}

function InfoSection() {
  const rows = useDebugInfo();
  const [copied, setCopied] = useState<string | null>(null);
  const list = useRef<HTMLDListElement>(null);
  /** 剪贴板 API 被拒绝时（某些内嵌浏览器）：选中整段信息再复制选区；仍不行就保持选中，提示手动复制。 */
  const copySelection = (reason: unknown) => {
    const selection = getSelection();
    if (!list.current || !selection) return setCopied(`复制失败：${String(reason)}`);
    selection.selectAllChildren(list.current);
    const ok = document.execCommand("copy");
    setCopied(ok ? "已复制" : `剪贴板不可用，已选中全部信息，请按 ${formatCombo({ mod: true, label: "C" })} 复制`);
  };
  const copyAll = () => {
    const text = rows.map(([k, v]) => `${k}：${v}`).join("\n");
    navigator.clipboard
      .writeText(text)
      .then(() => setCopied("已复制"))
      .catch(copySelection);
  };
  return (
    <FieldSet>
      <FieldLegend>调试信息</FieldLegend>
      <dl ref={list} className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 select-text">
        {rows.map(([k, v]) => (
          <div key={k} className="contents">
            <dt className="text-muted-foreground">{k}</dt>
            <dd className="min-w-0 break-all whitespace-pre-wrap">{v}</dd>
          </div>
        ))}
      </dl>
      <div className="flex items-center gap-2">
        <Button variant="outline" size="sm" onClick={copyAll}>
          <Copy /> 复制全部
        </Button>
        {copied && <span role="status" className="text-muted-foreground">{copied}</span>}
      </div>
    </FieldSet>
  );
}

export default function DebugPanel() {
  const open = useDebug((s) => s.open);
  const setOpen = useDebug((s) => s.setOpen);
  const resetAll = useDebug((s) => s.resetAll);
  useToggleShortcut();
  useEffect(() => restoreSavedChoices(), []);

  return (
    <>
      <Hint label="调试面板（仅开发环境）" keys={shortcutLabel("debugPanel", getRealPlatform().platform)} side="top" align="end">
        <Button variant="ghost" size="icon-xs" aria-label="调试面板" onClick={() => setOpen(true)} className="ml-2">
          <Bug />
        </Button>
      </Hint>
      <Sheet open={open} onOpenChange={setOpen}>
        <SheetContent side="right" aria-label="调试面板">
          <SheetHeader>
            <SheetTitle>调试面板</SheetTitle>
            <SheetDescription>
              仅开发环境存在，生产构建不含。选择保存在本机 localStorage，刷新后保留；“全部恢复默认”只恢复平台模拟与状态模拟，不改设置。
            </SheetDescription>
          </SheetHeader>
          <ScrollArea className="min-h-0 flex-1">
            <div className="flex flex-col gap-6 px-6 pb-6">
              <PlatformSection />
              <Separator />
              <ThemeSection />
              <Separator />
              <SimulationSection />
              <Separator />
              <InfoSection />
            </div>
          </ScrollArea>
          <SheetFooter>
            <Button variant="outline" onClick={resetAll}>
              <RotateCcw /> 全部恢复默认
            </Button>
          </SheetFooter>
        </SheetContent>
      </Sheet>
    </>
  );
}
