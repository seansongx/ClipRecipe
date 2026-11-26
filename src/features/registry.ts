// 功能注册表：ID 与 docs/specs/ 一一对应。
// 界面根据这里的元数据渲染；实现某个功能后把 status 改为 "done"。

export type FeatureStatus = "planned" | "wip" | "done";

export interface Feature {
  id: string;
  name: string;
  summary: string;
  group: string;
  status: FeatureStatus;
}

const f = (
  group: string,
  id: string,
  name: string,
  summary: string,
  status: FeatureStatus = "planned",
): Feature => ({
  id,
  name,
  summary,
  group,
  status,
});

/** 批量流水线模块：对队列里的每个视频套用。 */
export const PIPELINE_MODULES: Feature[] = [
  f("叠加", "pipe.remove-watermark", "去水印", "模糊指定区域，可限定时段", "done"),
  f("叠加", "pipe.add-watermark", "加水印", "图片 / GIF / 视频水印，支持子水印", "done"),
  f("叠加", "pipe.pip", "画中画", "自身或外部素材作背景，可虚化、抠像", "done"),
  f("叠加", "pipe.text", "文本", "最多 3 处，文件名 / 目录 / 自定义", "done"),

  f("画面", "pipe.crop", "裁剪", "区域、百分比、随机、居中比例", "done"),
  f("画面", "pipe.resolution", "分辨率", "标准档位或自定义，背景填充、倒影", "done"),
  f("画面", "pipe.rotate-flip", "旋转与翻转", "固定角度、翻转、随机角度", "done"),
  f("画面", "pipe.adjust", "画面调整", "亮度、对比度、饱和度、LUT、更多效果", "done"),
  f("画面", "pipe.grid", "宫格分屏", "同一画面重复排布", "done"),
  f("画面", "pipe.zoom", "动态缩放", "画面往复放大缩小", "done"),

  f("时间", "pipe.trim", "去头尾", "按时长去除或截取一段", "done"),
  f("时间", "pipe.intro-outro", "加头尾与边框", "片头、片尾、相框", "done"),
  f("时间", "pipe.speed", "变速", "固定倍率或分段变速，可变调", "done"),
  f("时间", "pipe.drop-frames", "抽帧", "每隔 N 帧丢 1 帧", "done"),
  f("时间", "pipe.dedupe-frames", "重复帧移除", "去掉连续重复帧", "done"),

  f("音频", "pipe.bgm", "背景音", "单文件或目录随机，淡入淡出、循环", "done"),

  f("编码", "pipe.fps", "帧率", "在范围内取值", "done"),
  f("编码", "pipe.bitrate", "码率", "倍率或固定值", "done"),
];

/** 独立工具：单一用途的批处理。 */
export const TOOLS: Feature[] = [
  f("合并与分割", "tool.merge", "视频合并", "全部 / 选中 / 自定义规则，可加转场", "done"),
  f("合并与分割", "tool.split", "视频分割", "按时长、段数或场景", "done"),
  f("合并与分割", "tool.folder-merge", "文件夹合并", "按规则合并文件夹内的视频", "done"),
  f("合并与分割", "tool.clip-merge", "截取合并", "每个视频截一段后拼接", "done"),
  f("合并与分割", "tool.hls", "M3U8 切片", "批量切为 HLS", "done"),

  f("重组", "tool.mirror-remix", "镜像重组", "按场景拆分，部分片段翻转后重组", "done"),
  f("重组", "tool.transition-remix", "转场重组", "按场景拆分，加转场后重组", "done"),
  f("重组", "tool.split-remix", "拆分重组", "指定位置插入一段视频", "done"),
  f("重组", "tool.remove-remix", "移除重组", "指定位置移除一段", "done"),
  f("重组", "tool.simple-mix", "简单混剪", "主视频 + 副片段 + 音频 + 字幕", "done"),

  f("画面效果", "tool.freeze-anim", "定帧动画", "画面停在指定帧上", "done"),
  f("画面效果", "tool.freeze-play", "定格播放", "指定位置暂停一段时间", "done"),
  f("画面效果", "tool.camera-move", "运镜效果", "裁取中间区域往复平移", "done"),
  f("画面效果", "tool.remove-black-bars", "黑边去除", "自动检测并裁掉黑边", "done"),
  f("画面效果", "tool.insert-frame", "插帧", "指定位置插入图片", "done"),
  f("画面效果", "tool.progress-bar", "进度条", "在画面上绘制播放进度", "done"),
  f("画面效果", "tool.orientation", "横竖互转", "空白处填充颜色或图片", "done"),
  f("画面效果", "tool.stitch", "画面拼接", "多个视频拼成分屏", "done"),
  f("画面效果", "tool.reverse", "视频倒放", "可去除音频", "done"),
  f("画面效果", "tool.image-to-video", "图转视频", "图片批量转 mp4", "done"),
  f("画面效果", "tool.gif", "生成 GIF", "可选高质量模式", "done"),
  f("画面效果", "tool.screenshots", "批量截图", "可绘入文件名", "done"),

  f("时间与静止", "tool.remove-still", "静止帧移除", "移除画面静止的片段", "done"),
  f("时间与静止", "tool.remove-silence", "静音移除", "移除或加速静音片段", "done"),
  f("时间与静止", "tool.smart-trim", "智能去头尾", "依据场景切换判断头尾", "done"),
  f("时间与静止", "tool.change-duration", "改变视频时长", "只改时长元数据", "done"),

  f("音频与转换", "tool.extract-av", "音视频提取", "提取音轨或纯视频", "done"),
  f("音频与转换", "tool.av-mux", "音视频合成", "音频与视频文件夹批量配对", "done"),
  f("音频与转换", "tool.volume-pitch", "音量音调", "批量调整，可统一响度", "done"),
  f("音频与转换", "tool.remux", "无损转换", "只换封装不重编码", "done"),

  f("字幕与封面", "tool.subtitle-asr", "语音生成字幕", "识别语音生成 SRT", "done"),
  f("字幕与封面", "tool.subtitle-extract", "字幕提取", "导出内嵌字幕轨", "done"),
  f("字幕与封面", "tool.subtitle-erase", "字幕擦除", "擦除画面中的硬字幕", "done"),
  f("字幕与封面", "tool.cover", "封面处理", "嵌入封面、截取封面", "done"),
];

export function groupBy(features: Feature[]): [string, Feature[]][] {
  const map = new Map<string, Feature[]>();
  for (const feature of features) {
    const list = map.get(feature.group);
    if (list) list.push(feature);
    else map.set(feature.group, [feature]);
  }
  return [...map.entries()];
}
