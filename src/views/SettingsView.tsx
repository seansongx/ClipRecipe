import { useEffect, useState } from "react";
import {
  getAppInfo,
  getFfmpegStatus,
  isTauri,
  type AppInfo,
  type FfmpegStatus,
} from "../lib/backend";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start gap-4 border-b border-line px-4 py-3 last:border-b-0">
      <span className="w-28 shrink-0 text-muted">{label}</span>
      <div className="min-w-0 flex-1 select-text break-words">{children}</div>
    </div>
  );
}

export default function SettingsView() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [ffmpeg, setFfmpeg] = useState<FfmpegStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauri) return;
    getAppInfo().then(setInfo).catch((e) => setError(String(e)));
    getFfmpegStatus().then(setFfmpeg).catch((e) => setError(String(e)));
  }, []);

  return (
    <div className="h-full overflow-y-auto">
      <header className="border-b border-line px-5 py-3">
        <h1 className="text-base font-semibold">设置</h1>
      </header>

      <div className="max-w-2xl px-5 py-4">
        <h2 className="mb-2 text-xs font-medium text-muted">运行环境</h2>
        <div className="rounded-xl border border-line bg-surface">
          {!isTauri ? (
            <Row label="状态">当前在浏览器中预览，环境信息需要在桌面应用中查看。</Row>
          ) : (
            <>
              <Row label="版本">{info ? `${info.name} ${info.version}` : "读取中…"}</Row>
              <Row label="CPU 核心">{info ? info.cpuCores : "读取中…"}</Row>
              <Row label="FFmpeg">
                {!ffmpeg && "检测中…"}
                {ffmpeg?.available && (
                  <>
                    <p>{ffmpeg.version}</p>
                    <p className="text-xs text-muted">{ffmpeg.path}</p>
                  </>
                )}
                {ffmpeg && !ffmpeg.available && (
                  <>
                    <p>未找到可用的 FFmpeg</p>
                    <p className="text-xs text-muted">
                      {ffmpeg.error}。请安装 FFmpeg 并加入 PATH，或设置环境变量 CLIPRECIPE_FFMPEG。
                    </p>
                  </>
                )}
              </Row>
              {error && <Row label="错误">{error}</Row>}
            </>
          )}
        </div>

        <p className="mt-6 text-xs text-muted">
          转换与通用设置项尚未实现，规划见 docs/FEATURES.md 第 4 节。
        </p>
      </div>
    </div>
  );
}
