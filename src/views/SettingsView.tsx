// 设置页：左侧分组列表 + 右侧当前分组。每个设置项对应 docs/specs/settings.md的一个 ID（set.*），
// 字段定义见 lib/backend.ts 的 Settings（与后端 model/settings.rs 一致）。

import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { BookOpen, FileInput, FolderOpen, Play, RefreshCw, Trash2, TriangleAlert, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Card } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import ChoiceGroup from "@/components/ChoiceGroup";
import ColorField from "@/components/ColorField";
import { FfmpegDownloadControl, FfmpegDownloadNote } from "@/components/FfmpegDownload";
import FontSelect from "@/components/FontSelect";
import Hint from "@/components/Hint";
import NumberInput from "@/components/NumberInput";
import VerticalTabsList from "@/components/VerticalTabsList";
import {
  getAppInfo,
  getDataDir,
  isTauri,
  openDataDir,
  openProjectPage,
  pickFile,
  type AfterAll,
  type AppInfo,
  type EncoderInfo,
  type FfmpegStatus,
  type OnExists,
  type Settings,
  type SubtitleStyle,
  type TargetFormat,
} from "@/lib/backend";
import { formatSize } from "@/lib/format";
import { batchTime, DEFAULT_NAME_TEMPLATE, NAME_VARIABLES, renderName, timeVars } from "@/lib/nameTemplate";
import { EXAMPLE_PATHS, usePlatform } from "@/lib/platform";
import { cn } from "@/lib/utils";
import { shortcutGroups } from "@/lib/shortcuts";
import { useEncoders } from "@/state/encoders";
import { SOURCE_LABELS, useFfmpeg } from "@/state/ffmpeg";
import { playCompletionSound } from "@/state/finish";
import { useLog } from "@/state/log";
import { useSettings } from "@/state/settings";
import { useUpdate } from "@/state/update";

type BooleanSettingKey = {
  [K in keyof Settings]: Settings[K] extends boolean ? K : never;
}[keyof Settings];

const GROUPS = [
  { id: "convert", label: "转换" },
  { id: "subtitle", label: "字幕与封面" },
  { id: "general", label: "通用" },
  { id: "finish", label: "完成后" },
  { id: "advanced", label: "高级" },
  { id: "about", label: "关于" },
] as const;
type GroupId = (typeof GROUPS)[number]["id"];

/**
 * 设置项一行：左边名称 + 次要色说明，右边控件（固定宽度的一列，各行的控件左对齐）。
 * `stacked` 用于内容很长的信息行（路径、版本、说明文字）：名称在上，内容占满整行。
 */
function Row({
  label,
  hint,
  stacked = false,
  children,
}: {
  label: string;
  hint?: ReactNode;
  stacked?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={cn("flex gap-x-6 gap-y-2 px-4 py-3", stacked ? "flex-col" : "items-start")}>
      <div className={cn("min-w-0", !stacked && "flex-1")}>
        <p className="font-medium">{label}</p>
        {hint && <p className="mt-0.5 text-xs text-muted-foreground">{hint}</p>}
      </div>
      <div className={cn("min-w-0 select-text break-words", stacked ? "w-full" : "w-[22rem] max-w-[50%] shrink-0")}>
        {children}
      </div>
    </div>
  );
}

/** 一组设置项：圆角 card 容器，行间细分隔线；可带小标题。 */
function Section({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <section className="mb-6">
      {title && <h3 className="mb-2 px-1 text-xs font-medium text-muted-foreground">{title}</h3>}
      <Card className="gap-0 divide-y py-0">{children}</Card>
    </section>
  );
}

/** 左侧分组栏的样式，与工具箱的分类栏一致（一列平铺条目，选中是 muted 底 + 前景色，未选中是次要色）。 */
const NAV_LIST = "h-fit w-full gap-0.5 rounded-none bg-transparent p-0";
const NAV_TAB =
  "h-8 flex-none rounded-md px-2.5 text-muted-foreground hover:bg-muted/60 data-active:bg-muted data-active:text-foreground dark:text-muted-foreground dark:data-active:border-transparent dark:data-active:bg-muted dark:data-active:text-foreground";

function useToggle() {
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  return (key: BooleanSettingKey, label: string, disabled = false) => (
    <Switch
      aria-label={label}
      checked={settings[key]}
      disabled={disabled}
      onCheckedChange={(v) => {
        const patch: Partial<Settings> = {};
        patch[key] = v;
        update(patch);
      }}
    />
  );
}

function useCpuCores() {
  const [cpuCores, setCpuCores] = useState(16);
  useEffect(() => {
    if (isTauri) getAppInfo().then((i) => setCpuCores(i.cpuCores)).catch(() => {});
  }, []);
  return cpuCores;
}

// ---------------------------------------------------------------- 转换

/** set.name-template：输出文件名格式。变量按钮插入到光标处；示例用固定的文件名与当前时间。 */
function NameTemplateRow() {
  const template = useSettings((s) => s.settings.nameTemplate);
  const update = useSettings((s) => s.update);
  const inputRef = useRef<HTMLInputElement>(null);

  const insert = (token: string) => {
    const input = inputRef.current;
    const start = input?.selectionStart ?? template.length;
    const end = input?.selectionEnd ?? start;
    update({ nameTemplate: template.slice(0, start) + token + template.slice(end) });
    const caret = start + token.length;
    requestAnimationFrame(() => {
      input?.focus();
      input?.setSelectionRange(caret, caret);
    });
  };

  const example = renderName(template, {
    name: "示例视频",
    index: "01",
    folder: "素材",
    ...timeVars(batchTime()),
  });

  return (
    <Row label="输出文件名格式" hint="批量处理输出的文件名。可用变量：文件名（已去除）、队列序号、开始处理的日期与时间、源文件所在文件夹；其它文字原样保留">
      <div className="flex items-center gap-2">
        <Input
          ref={inputRef}
          aria-label="输出文件名格式"
          value={template}
          onChange={(e) => update({ nameTemplate: e.target.value })}
          placeholder={DEFAULT_NAME_TEMPLATE}
        />
        <Button
          variant="outline"
          size="sm"
          disabled={template === DEFAULT_NAME_TEMPLATE}
          onClick={() => update({ nameTemplate: DEFAULT_NAME_TEMPLATE })}
        >
          恢复默认
        </Button>
      </div>
      <div className="mt-1.5 flex flex-wrap gap-1">
        {NAME_VARIABLES.map((v) => (
          <Button key={v.token} variant="secondary" size="xs" onClick={() => insert(v.token)}>
            {v.label} {v.token}
          </Button>
        ))}
      </div>
      <p className="mt-1.5 truncate text-xs text-muted-foreground">示例：{example}.mp4</p>
    </Row>
  );
}

function ConvertGroup() {
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const toggle = useToggle();
  const cpuCores = useCpuCores();
  const regexId = useId();

  return (
    <>
      <Section>
        <Row label="目标格式" hint="不支持 H.264/H.265 的源格式会改用 MP4">
          <ChoiceGroup<TargetFormat>
            label="目标格式"
            value={settings.targetFormat}
            onChange={(targetFormat) => update({ targetFormat })}
            options={[
              { value: "mp4", label: "MP4" },
              { value: "source", label: "保持源格式" },
            ]}
          />
        </Row>
        <Row label="目标已存在时" hint="也用于“输出移回源位置”；永远不会覆盖源视频本身">
          <ChoiceGroup<OnExists>
            label="目标已存在时"
            value={settings.onExists}
            onChange={(onExists) => update({ onExists })}
            options={[
              { value: "rename", label: "重命名" },
              { value: "overwrite", label: "覆盖" },
            ]}
          />
        </Row>
        <Row label="文件名去除" hint="从输出文件名中去掉这些内容，每行一项">
          <Textarea
            aria-label="文件名去除"
            value={settings.nameStrip.text}
            onChange={(e) => update({ nameStrip: { ...settings.nameStrip, text: e.target.value } })}
            rows={2}
            placeholder="例如：_原片"
            className="min-h-14 resize-y"
          />
          <div className="mt-1.5 flex items-center gap-1.5">
            <Checkbox
              id={regexId}
              checked={settings.nameStrip.regex}
              onCheckedChange={(v) => update({ nameStrip: { ...settings.nameStrip, regex: v === true } })}
            />
            <Label htmlFor={regexId}>
              按正则表达式匹配
            </Label>
          </div>
        </Row>
        <NameTemplateRow />
        <Row label="并行任务数" hint={`上限为 CPU 核心数（${cpuCores}）`}>
          <NumberInput
            label="并行任务数"
            value={settings.parallel}
            min={1}
            max={cpuCores}
            integer
            onChange={(parallel) => update({ parallel })}
          />
        </Row>
        <Row label="使用 H.265" hint="体积更小；部分平台和旧设备可能无法播放">
          {toggle("hevc", "使用 H.265")}
        </Row>
        <Row
          label="HDR 转 SDR"
          hint="源视频为 HDR（PQ / HLG）时转换为普通 SDR 画面。关闭则保留 HDR：输出 10 bit H.265 并保留色彩信息（选了 H.264 时会自动改用 H.265）"
        >
          {toggle("hdrToSdr", "HDR 转 SDR")}
        </Row>
        <Row
          label="虚化背景取画面中间"
          hint="分辨率、画中画的虚化背景：开为等比放大后取中间，关为整幅拉伸。模块里选了“取中 / 拉伸”时以模块为准"
        >
          {toggle("blurCenter", "虚化背景取画面中间")}
        </Row>
        <Row label="跳过已完成" hint="重新开始时跳过队列中已完成的视频；改了参数想重跑时关闭">
          {toggle("skipDone", "跳过已完成")}
        </Row>
      </Section>
    </>
  );
}

// ---------------------------------------------------------------- 字幕与封面

function SubtitleGroup() {
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const toggle = useToggle();
  const style = settings.subtitleStyle;
  const setStyle = (patch: Partial<SubtitleStyle>) => update({ subtitleStyle: { ...style, ...patch } });

  return (
    <>
      <Section title="字幕">
        <Row
          label="自动加载同名字幕"
          hint="导入时自动关联视频同目录下的同名字幕（srt / ass / ssa / vtt），处理时烧录进画面。也可在队列右键菜单中手动设置"
        >
          {toggle("autoSubtitle", "自动加载同名字幕")}
        </Row>
        <Row label="字体" hint="默认自动选择支持中文的系统字体">
          <FontSelect value={style.font} onChange={(font) => setStyle({ font })} />
        </Row>
        <Row label="字号" hint="以 1080 高的画面为基准，按实际画面等比缩放（下同）">
          <NumberInput label="字号" value={style.size} min={8} max={400} integer onChange={(size) => setStyle({ size })} />
        </Row>
        <Row label="文字颜色">
          <ColorField label="文字颜色" value={style.color} onChange={(color) => setStyle({ color })} />
        </Row>
        <Row label="描边" hint="描边宽度，0 为不描边">
          <div className="flex items-center gap-2">
            <NumberInput
              label="描边宽度"
              value={style.outline}
              min={0}
              max={20}
              step={0.5}
              onChange={(outline) => setStyle({ outline })}
            />
            <ColorField label="描边颜色" value={style.outlineColor} onChange={(outlineColor) => setStyle({ outlineColor })} />
          </div>
        </Row>
        <Row label="底部边距">
          <NumberInput
            label="底部边距"
            value={style.marginBottom}
            min={0}
            max={1000}
            integer
            onChange={(marginBottom) => setStyle({ marginBottom })}
          />
        </Row>
        <p className="border-t px-4 py-2 text-xs text-muted-foreground">
          样式用于 srt / vtt 字幕；ass / ssa 字幕保留文件自带的样式（只使用这里的字体）。字幕跟随源视频的画面与时间烧录，
          之后的裁剪、缩放、变速等模块一并作用于字幕；片头片尾上没有字幕。
        </p>
      </Section>
      <Section title="封面">
        <Row
          label="自动加载同名封面"
          hint="导入时自动关联同目录下的同名图片（jpg / png / webp / bmp），处理时嵌入输出"
        >
          {toggle("autoCover", "自动加载同名封面")}
        </Row>
        <Row
          label="封面帧数"
          hint="不超过视频帧率时，开头这几帧显示封面；超过帧率时，用图片生成这么多帧的片段接在视频开头（音频相应延后）。MP4 还会写入封面图"
        >
          <NumberInput
            label="封面帧数"
            value={settings.coverFrames}
            min={1}
            max={100000}
            integer
            onChange={(coverFrames) => update({ coverFrames })}
          />
        </Row>
        <Row label="拉伸至视频尺寸" hint="关闭时等比缩放，空白处填黑">
          {toggle("coverStretch", "封面拉伸至视频尺寸")}
        </Row>
      </Section>
    </>
  );
}

// ---------------------------------------------------------------- 通用

function GpuHint() {
  const dismissed = useSettings((s) => s.settings.gpuHintDismissed);
  const gpu = useSettings((s) => s.settings.gpu);
  const update = useSettings((s) => s.update);
  const [failed, setFailed] = useState<string[]>([]);
  const ffmpegPath = useFfmpeg((s) => (s.status?.available ? s.status.path : null));
  useEffect(() => {
    if (!isTauri || !gpu || !ffmpegPath) return;
    let alive = true;
    useEncoders
      .getState()
      .detect(false)
      .then((info) => alive && setFailed(info.failedVendors))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [gpu, ffmpegPath]);
  if (dismissed || !gpu || failed.length === 0) return null;
  return (
    <div className="mt-2 flex items-start gap-2 rounded-md border border-warning/40 bg-warning/10 px-2 py-1.5 text-xs">
      <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-warning" />
      <p className="flex-1">
        检测到 {failed.join("、")} 的显卡驱动，但硬件编码试编码失败，可能是驱动版本过低或显卡不支持编码，已改用软件编码。
        更新显卡驱动后可在“关于”中重新检测。
      </p>
      <Hint label="不再提示">
        <Button variant="ghost" size="icon-sm" aria-label="不再提示" onClick={() => update({ gpuHintDismissed: true })}>
          <X />
        </Button>
      </Hint>
    </div>
  );
}

function GeneralGroup() {
  const settings = useSettings((s) => s.settings);
  const toggle = useToggle();
  const checking = useUpdate((s) => s.checking);
  const updateInfo = useUpdate((s) => s.info);
  const updateError = useUpdate((s) => s.error);

  return (
    <>
      <Section title="编码与导入">
        <Row label="GPU 硬件加速" hint="使用 NVENC / QSV / AMF；失败时自动改用软件编码">
          {toggle("gpu", "GPU 硬件加速")}
          <GpuHint />
        </Row>
        <Row label="核显优先" hint="有多个可用的硬件编码器时，优先用核显（Intel QSV、AMD AMF），独显留给其它工作">
          {toggle("preferIntegratedGpu", "核显优先", !settings.gpu)}
        </Row>
        <Row label="文件名加队列序号" hint="如 01_视频名.mp4">
          {toggle("numberPrefix", "文件名加队列序号")}
        </Row>
        <Row
          label="目录包含子目录"
          hint="导入文件夹时包含子目录；也是新建“目录”素材（水印、背景音等）时“包含子目录”的初始值，之后以素材自己的勾选为准"
        >
          {toggle("recursive", "目录包含子目录")}
        </Row>
        <Row label="按来源文件夹输出" hint="文件夹方式导入时，输出到与来源文件夹同名的目录，保持分类结构">
          {toggle("outByFolder", "按来源文件夹输出")}
        </Row>
        <Row label="输出到视频同名文件夹" hint="裂变次数大于 1 时总是如此">
          {toggle("outByName", "输出到视频同名文件夹")}
        </Row>
      </Section>
      <Section title="队列">
        <Row label="保留未完成任务" hint="退出时保存未完成的任务，下次启动自动载入（已不存在的文件会被移除）">
          {toggle("keepQueue", "保留未完成任务")}
        </Row>
        <Row label="完成后从队列移除" hint="处理成功的任务自动移出队列">
          {toggle("removeFinished", "完成后从队列移除")}
        </Row>
        <Row label="删除任务时确认" hint="从队列删除任务（不删除文件）前弹出确认">
          {toggle("confirmRemove", "删除任务时确认")}
        </Row>
      </Section>
      <Section title="界面">
        <Row label="关闭窗口时最小化到托盘" hint="点托盘图标显示窗口；托盘菜单里可退出">
          {toggle("tray", "关闭窗口时最小化到托盘")}
        </Row>
        <Row label="静音预览" hint="预览与播放时默认静音">
          {toggle("mutePreview", "静音预览")}
        </Row>
        <Row label="启动时检查更新" hint="只读取 GitHub 上公开的发布信息；发现新版本时先询问，确认后才下载并安装（不支持自动安装的版本改为打开发布页）">
          {toggle("autoUpdate", "启动时检查更新")}
          <div className="mt-1.5 flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={!isTauri || checking}
              onClick={() => void useUpdate.getState().check(true)}
            >
              <RefreshCw /> {checking ? "检查中…" : "立即检查"}
            </Button>
            <span className="text-xs text-muted-foreground">
              {updateError
                ? `检查失败：${updateError}`
                : updateInfo && !updateInfo.newer
                  ? `已是最新版本（${updateInfo.current}）`
                  : ""}
            </span>
          </div>
        </Row>
      </Section>
    </>
  );
}

// ---------------------------------------------------------------- 完成后

function SoundRow() {
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const toggle = useToggle();
  const browse = async () => {
    const path = await pickFile("提示音", ["wav"]);
    if (path) update({ soundPath: path });
  };
  return (
    <Row label="完成提示音" hint="全部任务结束时播放；不选文件时使用内置提示音">
      {toggle("sound", "完成提示音")}
      <div className="mt-1.5 flex items-center gap-2">
        <InputGroup className="max-w-96">
          <InputGroupInput
            value={settings.soundPath}
            onChange={(e) => update({ soundPath: e.target.value })}
            placeholder="内置提示音（可选择 wav 文件）"
            aria-label="提示音文件"
            disabled={!settings.sound}
          />
          <InputGroupAddon align="inline-end">
            <Hint label="选择 wav 文件">
              <InputGroupButton
                size="icon-xs"
                onClick={() => void browse()}
                disabled={!isTauri || !settings.sound}
                aria-label="选择 wav 文件"
              >
                <FileInput />
              </InputGroupButton>
            </Hint>
          </InputGroupAddon>
        </InputGroup>
        <Button
          variant="outline"
          size="sm"
          disabled={!settings.sound}
          onClick={() => void playCompletionSound(settings.soundPath)}
        >
          <Play /> 试听
        </Button>
      </div>
    </Row>
  );
}

function DeleteSourceRow() {
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const [confirming, setConfirming] = useState(false);
  const trashId = useId();
  return (
    <Row
      label="处理后删除源视频"
      hint="任务成功、输出文件已写完并校验后才删除；失败或取消时不动源视频；输出与源视频是同一个文件时不删除"
    >
      <Switch
        aria-label="处理后删除源视频"
        checked={settings.deleteSource}
        onCheckedChange={(v) => (v ? setConfirming(true) : update({ deleteSource: false }))}
      />
      <div className="mt-1.5 flex items-center gap-1.5">
        <Checkbox
          id={trashId}
          checked={settings.deleteToTrash}
          onCheckedChange={(v) => update({ deleteToTrash: v === true })}
        />
        <Label htmlFor={trashId}>
          移入回收站（取消勾选则永久删除，无法恢复）
        </Label>
      </div>
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>开启“处理后删除源视频”？</AlertDialogTitle>
            <AlertDialogDescription>
              每个任务处理成功后，源视频会被
              {settings.deleteToTrash ? "移入回收站" : "永久删除（无法恢复）"}。
              只在输出文件完整写入后才会删除；失败或取消的任务不受影响。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                update({ deleteSource: true });
                setConfirming(false);
              }}
            >
              开启
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Row>
  );
}

function FinishGroup() {
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const toggle = useToggle();
  return (
    <>
      <Section title="全部完成后">
        <Row label="全部完成后" hint="只对下一次运行有效，触发后（以及每次启动时）自动恢复为“无操作”；关机前有 30 秒倒计时可取消">
          <ChoiceGroup<AfterAll>
            label="全部完成后"
            value={settings.afterAll}
            onChange={(afterAll) => update({ afterAll })}
            options={[
              { value: "none", label: "无操作" },
              { value: "shutdown", label: "关机" },
              { value: "quit", label: "退出程序" },
            ]}
          />
        </Row>
        <Row label="打开输出文件夹" hint="全部结束后打开输出位置（未设置输出目录时为第一个输出所在的文件夹）">
          {toggle("openOutput", "完成后打开输出文件夹")}
        </Row>
        <SoundRow />
      </Section>
      <Section title="每个任务成功后">
        <Row label="输出移回源位置" hint="把输出移到源视频所在的文件夹；同名时按“目标已存在时”重命名或覆盖（不会覆盖源视频本身）">
          {toggle("moveBack", "输出移回源位置")}
        </Row>
        <DeleteSourceRow />
      </Section>
    </>
  );
}

// ---------------------------------------------------------------- 高级

function AdvancedGroup() {
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const toggle = useToggle();
  return (
    <Section>
      <Row label="裂变独立文件夹" hint="裂变次数大于 1 时，每次裂变输出到视频同名文件夹下的“第 x 次裂变”目录">
        {toggle("fissionFolder", "裂变独立文件夹")}
      </Row>
      <Row
        label="体积压缩"
        hint="“瘦身”：不再沿用源码率，改用更高的恒定质量值（H.264 CRF 28、H.265 CRF 32，软件编码用 slow 预设），并以源码率为上限。启用码率模块时以码率模块为准"
      >
        {toggle("reduceSize", "体积压缩")}
      </Row>
      <Row label="变速适应" hint="音视频合成：视频变速到配对音频的时长（重编码视频）；截取合并：有目标时长时整体变速到目标时长，代替截断。变速 0.25～4 倍，超出时按原方式处理">
        {toggle("speedFit", "变速适应")}
      </Row>
      <Row label="版权信息" hint="写入输出视频的元数据（copyright），批量处理与独立工具都会写入；留空不写">
        <Input
          aria-label="版权信息"
          value={settings.copyright}
          onChange={(e) => update({ copyright: e.target.value })}
          placeholder="例如：© 2026 某某工作室"
          className="max-w-96"
        />
      </Row>
    </Section>
  );
}

// ---------------------------------------------------------------- 关于

function FfmpegRow({ ffmpeg }: { ffmpeg: FfmpegStatus | null }) {
  const checking = useFfmpeg((s) => s.checking);
  const platform = usePlatform();
  return (
    <Row stacked label="FFmpeg" hint="查找顺序：环境变量 CLIPRECIPE_FFMPEG → 应用内下载 → 程序目录 → 系统 PATH">
      {!ffmpeg && "检测中…"}
      {ffmpeg?.available && (
        <>
          <p>{ffmpeg.version}</p>
          <p className="text-xs text-muted-foreground">来源：{SOURCE_LABELS[ffmpeg.source]}</p>
          <p className="text-xs text-muted-foreground">{ffmpeg.path}</p>
          <p className="text-xs text-muted-foreground">{ffmpeg.ffprobePath}</p>
          {!ffmpeg.whisper && (
            <p className="text-xs text-muted-foreground">
              没有 whisper 滤镜：“语音生成字幕”需改用 whisper-cli，或使用带 whisper 的完整版 FFmpeg。
            </p>
          )}
        </>
      )}
      {ffmpeg && !ffmpeg.available && (
        <>
          <p>未找到可用的 FFmpeg</p>
          <p className="text-xs text-muted-foreground">
            {ffmpeg.error}。可以在下方下载（启动时会自动下载一次），或自行安装 FFmpeg 并加入 PATH、设置环境变量 CLIPRECIPE_FFMPEG 指向
            ffmpeg 可执行文件（如 {EXAMPLE_PATHS[platform].ffmpeg}，ffprobe 需在同一目录）。
          </p>
        </>
      )}
      <Button
        variant="outline"
        size="sm"
        onClick={() => void useFfmpeg.getState().redetect()}
        disabled={checking}
        className="mt-1"
      >
        <RefreshCw /> {checking ? "检测中…" : "重新检测 FFmpeg"}
      </Button>
    </Row>
  );
}

function ManagedFfmpegRow({ ffmpeg }: { ffmpeg: FfmpegStatus }) {
  const download = useFfmpeg((s) => s.download);
  const removing = useFfmpeg((s) => s.removing);
  const [confirming, setConfirming] = useState(false);
  const managed = ffmpeg.managed;
  const installed = managed.installedVersion;
  const running = download.kind === "running";
  return (
    <Row stacked label="应用内下载" hint="下载固定版本的 FFmpeg 到本机数据目录，不影响系统里已安装的 FFmpeg">
      {!managed.supported ? (
        <p className="text-xs text-muted-foreground">{managed.unsupportedReason}</p>
      ) : (
        <div className="flex flex-col gap-2">
          {installed && (
            <div>
              <p>
                已下载 FFmpeg {installed}
                {managed.updateAvailable && <span className="text-warning">（可更新到 {managed.pinnedVersion}）</span>}
              </p>
              <p className="text-xs text-muted-foreground">{managed.installedDir}</p>
              {ffmpeg.source === "env" && (
                <p className="text-xs text-muted-foreground">
                  当前使用环境变量 CLIPRECIPE_FFMPEG 指定的 FFmpeg（优先于应用内下载的副本）。
                </p>
              )}
            </div>
          )}
          {(!installed || managed.updateAvailable || download.kind !== "idle") && (
            <FfmpegDownloadControl
              idleLabel={
                installed
                  ? `更新到 FFmpeg ${managed.pinnedVersion}（${formatSize(managed.downloadSize)}）`
                  : `下载 FFmpeg ${managed.pinnedVersion}（${formatSize(managed.downloadSize)}）`
              }
            />
          )}
          {installed && !running && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => setConfirming(true)}
              disabled={removing}
              className="self-start"
            >
              <Trash2 /> {removing ? "删除中…" : "删除已下载的 FFmpeg"}
            </Button>
          )}
          <FfmpegDownloadNote />
        </div>
      )}
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>删除已下载的 FFmpeg？</AlertDialogTitle>
            <AlertDialogDescription>
              删除 {managed.installedDir}。之后会改用系统中的 FFmpeg（如果有）；需要时可以重新下载。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setConfirming(false);
                void useFfmpeg.getState().remove();
              }}
            >
              删除
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Row>
  );
}

function EnvironmentSection() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const ffmpeg = useFfmpeg((s) => s.status);
  const [encoders, setEncoders] = useState<EncoderInfo | null>(null);
  const [detecting, setDetecting] = useState(false);
  const [dataDir, setDataDir] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const detect = (refresh: boolean) => {
    setDetecting(true);
    useEncoders
      .getState()
      .detect(refresh)
      .then(setEncoders)
      .catch((e) => setError(String(e)))
      .finally(() => setDetecting(false));
  };

  useEffect(() => {
    if (!isTauri) return;
    getAppInfo().then(setInfo).catch((e) => setError(String(e)));
    getDataDir().then(setDataDir).catch(() => {});
    void useFfmpeg.getState().refresh();
  }, []);

  // FFmpeg 换了（下载、删除、重新检测）时重新读取编码器信息
  const ffmpegPath = ffmpeg?.available ? ffmpeg.path : null;
  useEffect(() => {
    if (!isTauri) return;
    if (ffmpegPath) detect(false);
    else setEncoders(null);
  }, [ffmpegPath]);

  return (
    <Section title="运行环境">
      {!isTauri ? (
        <>
          <Row stacked label="状态">当前在浏览器中预览，环境信息需要在桌面应用中查看。</Row>
          {/* 浏览器里没有检测结果；状态层里有 FFmpeg 状态时（开发环境的调试面板注入）照常显示 */}
          {ffmpeg && <FfmpegRow ffmpeg={ffmpeg} />}
          {ffmpeg && <ManagedFfmpegRow ffmpeg={ffmpeg} />}
        </>
      ) : (
        <>
          <Row label="CPU 核心">{info ? info.cpuCores : "读取中…"}</Row>
          <FfmpegRow ffmpeg={ffmpeg} />
          {ffmpeg && <ManagedFfmpegRow ffmpeg={ffmpeg} />}
          <Row stacked label="硬件编码器" hint="通过实际试编码检测">
            {detecting && <p className="text-muted-foreground">检测中…</p>}
            {!detecting && encoders && (
              <>
                <p>{encoders.hardware.length > 0 ? encoders.hardware.join("、") : "未检测到可用的硬件编码器"}</p>
                <p className="text-xs text-muted-foreground">
                  开启 GPU 加速时使用：H.264 → {encoders.h264}，H.265 → {encoders.hevc}
                </p>
                {encoders.failedVendors.length > 0 && (
                  <p className="text-xs text-warning">
                    {encoders.failedVendors.join("、")}：已安装驱动但试编码失败，可能需要更新显卡驱动
                  </p>
                )}
              </>
            )}
            <Button
              variant="outline"
              size="sm"
              onClick={() => detect(true)}
              disabled={detecting || !ffmpeg?.available}
              className="mt-1"
            >
              <RefreshCw /> 重新检测
            </Button>
          </Row>
          {dataDir && (
            <Row stacked label="数据目录" hint="设置、保存的队列">
              <p>{dataDir}</p>
              <Button
                variant="outline"
                size="sm"
                onClick={() => openDataDir().catch((e) => setError(String(e)))}
                className="mt-1"
              >
                <FolderOpen /> 打开数据目录
              </Button>
            </Row>
          )}
          {error && <Row stacked label="错误">{error}</Row>}
        </>
      )}
    </Section>
  );
}

function AboutGroup() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  useEffect(() => {
    if (isTauri) getAppInfo().then(setInfo).catch(() => {});
  }, []);
  const openDocs = () =>
    openProjectPage().catch((e) => useLog.getState().add("error", `无法打开使用文档：${String(e)}`));
  return (
    <>
      <Section>
        <Row label="ClipRecipe" stacked>
          <p>{info ? `版本 ${info.version}` : "开源的视频批量处理桌面工具"}</p>
          <p className="mt-1 text-xs text-muted-foreground">以 MIT 许可证开源，免费使用，无需注册。</p>
          <Button
            variant="outline"
            size="sm"
            disabled={!isTauri}
            onClick={() => void openDocs()}
            className="mt-1.5"
          >
            <BookOpen /> 使用文档（GitHub）
          </Button>
        </Row>
        <Row label="FFmpeg 说明" stacked>
          <p className="text-xs text-muted-foreground">
            本程序调用 FFmpeg / FFprobe（作为独立程序运行）完成全部音视频处理，本身不包含也不分发 FFmpeg：可以使用系统中安装的
            FFmpeg，或在下方“运行环境”中点击下载（启动时找不到 FFmpeg 会自动下载未经修改的上游构建）。FFmpeg 是 FFmpeg 项目的商标，
            依其构建方式按 LGPL 或 GPL 授权（应用内下载的构建为 GPL-3.0-or-later）；请遵守相应许可。
          </p>
        </Row>
      </Section>
      <ShortcutsSection />
      <EnvironmentSection />
    </>
  );
}

/** 键盘快捷键一览（内容见 lib/shortcuts.ts）。 */
function ShortcutsSection() {
  const platform = usePlatform();
  return (
    <>
      {shortcutGroups(platform).map((group) => (
        <Section key={group.title} title={`键盘快捷键 · ${group.title}`}>
          {group.items.map((item) => (
            <div key={item.label} className="flex items-center gap-4 px-4 py-2">
              <KbdGroup className="w-56 shrink-0 flex-wrap">
                {item.keys.map((k) => (
                  <Kbd key={k}>{k}</Kbd>
                ))}
              </KbdGroup>
              <span className="text-xs">{item.label}</span>
            </div>
          ))}
        </Section>
      ))}
    </>
  );
}

const GROUP_VIEWS: Record<GroupId, () => ReactNode> = {
  convert: () => <ConvertGroup />,
  subtitle: () => <SubtitleGroup />,
  general: () => <GeneralGroup />,
  finish: () => <FinishGroup />,
  advanced: () => <AdvancedGroup />,
  about: () => <AboutGroup />,
};

export default function SettingsView() {
  const [group, setGroup] = useState<GroupId>("convert");
  return (
    <Tabs
      value={group}
      onValueChange={(v) => setGroup(v as GroupId)}
      orientation="vertical"
      className="h-full min-h-0 gap-0"
    >
      <nav className="w-[200px] shrink-0 border-r bg-card px-2 py-3">
        {/* 页面标题只给读屏和端到端测试（main h1）用：顶栏导航已经写着“设置”，不再显示一遍 */}
        <h1 className="sr-only">设置</h1>
        <VerticalTabsList aria-label="设置分组" className={NAV_LIST}>
          {GROUPS.map((g) => (
            <TabsTrigger key={g.id} value={g.id} className={NAV_TAB}>
              {g.label}
            </TabsTrigger>
          ))}
        </VerticalTabsList>
      </nav>
      {GROUPS.map((g) => (
        <TabsContent key={g.id} value={g.id} className="min-h-0 min-w-0 bg-background">
          <ScrollArea className="h-full">
            <div className="max-w-4xl px-8 py-6">
              <h2 className="mb-5 text-lg font-semibold">{g.label}</h2>
              {!isTauri && (
                <p className="mb-4 rounded-lg border bg-card px-3 py-2 text-xs text-muted-foreground">
                  浏览器预览模式：设置只在本次页面中生效，不会保存。
                </p>
              )}
              {GROUP_VIEWS[g.id]()}
            </div>
          </ScrollArea>
        </TabsContent>
      ))}
    </Tabs>
  );
}
