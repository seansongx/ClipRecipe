import { useId } from "react";
import { FileInput, FolderOpen } from "lucide-react";
import { Checkbox } from "@/components/ui/checkbox";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { Label } from "@/components/ui/label";
import Hint from "@/components/Hint";
import { isTauri, pickFile, pickFolder, type AssetSource, type PickOrder } from "@/lib/backend";
import { useSettings } from "@/state/settings";
import ChoiceGroup from "./ChoiceGroup";

/**
 * 素材来源选择：单文件，或目录（随机 / 依次取，可含子目录）。
 * `extensions` 用于文件对话框过滤，如 ["png", "jpg"]。
 */
export default function AssetSourceInput({
  value,
  onChange,
  extensions,
  kindLabel = "素材",
}: {
  value: AssetSource;
  onChange: (value: AssetSource) => void;
  extensions: string[];
  kindLabel?: string;
}) {
  const recursiveId = useId();

  const browse = async () => {
    const path =
      value.mode === "file" ? await pickFile(kindLabel, extensions) : await pickFolder();
    if (path) onChange({ ...value, path });
  };

  const setMode = (mode: AssetSource["mode"]) => {
    if (mode === value.mode) return;
    onChange(
      mode === "file"
        ? { mode: "file", path: "" }
        : // set.recursive 作为新建目录素材“包含子目录”的初始值；之后以素材自己的设置为准
          { mode: "directory", path: "", recursive: useSettings.getState().settings.recursive, order: "random" },
    );
  };

  return (
    <div className="space-y-2">
      <ChoiceGroup
        label={`${kindLabel}来源`}
        value={value.mode}
        onChange={setMode}
        options={[
          { value: "file", label: "单个文件" },
          { value: "directory", label: "目录" },
        ]}
      />
      <InputGroup>
        <InputGroupInput
          value={value.path}
          onChange={(e) => onChange({ ...value, path: e.target.value })}
          placeholder={value.mode === "file" ? `${kindLabel}文件路径` : `${kindLabel}目录路径`}
          aria-label={value.mode === "file" ? `${kindLabel}文件路径` : `${kindLabel}目录路径`}
        />
        <InputGroupAddon align="inline-end">
          <Hint label="浏览">
            <InputGroupButton size="icon-xs" onClick={() => void browse()} disabled={!isTauri} aria-label="浏览">
              {value.mode === "file" ? <FileInput /> : <FolderOpen />}
            </InputGroupButton>
          </Hint>
        </InputGroupAddon>
      </InputGroup>
      {value.mode === "directory" && (
        <div className="flex flex-wrap items-center gap-3">
          <ChoiceGroup<PickOrder>
            label="取用顺序"
            value={value.order}
            onChange={(order) => onChange({ ...value, order })}
            options={[
              { value: "random", label: "随机取" },
              { value: "sequential", label: "依次取" },
            ]}
          />
          <div className="flex items-center gap-1.5">
            <Checkbox
              id={recursiveId}
              checked={value.recursive}
              onCheckedChange={(checked) => onChange({ ...value, recursive: checked === true })}
            />
            <Label htmlFor={recursiveId}>
              包含子目录
            </Label>
          </div>
        </div>
      )}
    </div>
  );
}
