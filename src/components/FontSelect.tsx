// 字体选择（文本模块、字幕样式共用）：默认（自动选择中文字体）/ 系统字体列表 / 其它字体文件。
// 值为字体文件路径，空字符串表示默认。

import { useEffect, useState } from "react";
import { FileInput } from "lucide-react";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { Select, SelectContent, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from "@/components/ui/select";
import Hint from "@/components/Hint";
import { isTauri, listSystemFonts, pickFile, type SystemFont } from "@/lib/backend";

const FONT_EXTENSIONS = ["ttf", "ttc", "otf", "otc"];

const DEFAULT_FONT = "__default";
const CUSTOM_FONT = "__custom";

let fontList: Promise<SystemFont[]> | null = null;

/** 系统字体（只在桌面应用中加载一次）。 */
function useSystemFonts(): SystemFont[] {
  const [fonts, setFonts] = useState<SystemFont[]>([]);
  useEffect(() => {
    if (!isTauri) return;
    fontList ??= listSystemFonts().catch(() => []);
    let alive = true;
    void fontList.then((list) => alive && setFonts(list));
    return () => {
      alive = false;
    };
  }, []);
  return fonts;
}

export default function FontSelect({ value, onChange }: { value: string; onChange: (fontFile: string) => void }) {
  const fonts = useSystemFonts();
  const [customChosen, setCustomChosen] = useState(false);
  const inList = fonts.some((f) => f.path === value);
  const custom = customChosen || (value !== "" && !inList);
  const selected = custom ? CUSTOM_FONT : value === "" ? DEFAULT_FONT : value;

  const browse = async () => {
    const path = await pickFile("字体", FONT_EXTENSIONS);
    if (path) onChange(path);
  };

  return (
    <div className="space-y-2">
      <Select
        value={selected}
        onValueChange={(next) => {
          setCustomChosen(next === CUSTOM_FONT);
          if (next === DEFAULT_FONT) onChange("");
          else if (next !== CUSTOM_FONT) onChange(next);
        }}
      >
        <SelectTrigger aria-label="字体" className="w-full max-w-56">
          <SelectValue />
        </SelectTrigger>
        <SelectContent className="max-h-72">
          <SelectItem value={DEFAULT_FONT}>
            默认（自动选择中文字体）
          </SelectItem>
          <SelectItem value={CUSTOM_FONT}>
            其它字体文件…
          </SelectItem>
          {fonts.length > 0 && <SelectSeparator />}
          {fonts.map((font) => (
            <SelectItem key={font.path} value={font.path}>
              {font.name}
              {font.cjk && <span className="text-xs text-muted-foreground">中文</span>}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {custom && (
        <InputGroup>
          <InputGroupInput
            value={value}
            onChange={(e) => onChange(e.target.value)}
            placeholder="字体文件路径（.ttf / .ttc / .otf）"
            aria-label="字体文件路径"
          />
          <InputGroupAddon align="inline-end">
            <Hint label="浏览">
              <InputGroupButton size="icon-xs" onClick={() => void browse()} disabled={!isTauri} aria-label="浏览">
                <FileInput />
              </InputGroupButton>
            </Hint>
          </InputGroupAddon>
        </InputGroup>
      )}
    </div>
  );
}

