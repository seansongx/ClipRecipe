// 虚化背景的取法（分辨率、画中画模块共用）：跟随设置 set.blur-center / 取中间 / 整幅拉伸。
// 参数值为 null 时跟随设置；旧配方里的 true / false 仍然有效且优先于设置。

import ChoiceGroup from "@/components/ChoiceGroup";

type Choice = "settings" | "center" | "stretch";

const toChoice = (value: boolean | null): Choice =>
  value === null ? "settings" : value ? "center" : "stretch";

const fromChoice = (choice: Choice): boolean | null =>
  choice === "settings" ? null : choice === "center";

export default function BlurCenterChoice({
  value,
  onChange,
}: {
  value: boolean | null;
  onChange: (value: boolean | null) => void;
}) {
  return (
    <ChoiceGroup<Choice>
      label="虚化背景取法"
      value={toChoice(value ?? null)}
      onChange={(choice) => onChange(fromChoice(choice))}
      options={[
        { value: "settings", label: "跟随设置" },
        { value: "center", label: "取中" },
        { value: "stretch", label: "拉伸" },
      ]}
    />
  );
}
