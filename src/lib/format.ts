// 显示用的格式化函数。

export const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

export const dirName = (path: string) => {
  const i = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return i >= 0 ? path.slice(0, i) : "";
};

/** 比较路径用的键：不区分大小写、统一分隔符。 */
export const pathKey = (path: string) => path.replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();

export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "--:--";
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

export function formatBitrate(bps: number | null | undefined): string {
  if (!bps) return "";
  return bps >= 1_000_000 ? `${(bps / 1_000_000).toFixed(1)} Mbps` : `${Math.round(bps / 1000)} kbps`;
}

export function formatSize(bytes: number): string {
  if (!bytes) return "";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

export function formatFps(fps: number): string {
  if (!fps) return "";
  return `${Number(fps.toFixed(2))} fps`;
}

export function formatTime(date: Date): string {
  return date.toLocaleTimeString("zh-CN", { hour12: false });
}
