#!/usr/bin/env node
// 把 @seankit/ffmpeg 与 @seankit/ffprobe 的 manifest.json 合并写入 src-tauri/resources/ffmpeg-manifest.json。
// 程序用 include_str! 内嵌这个文件：应用内下载 FFmpeg 时，下载地址、大小与 SHA-256 全部来自它（编译时固定），
// 运行时从不联网获取清单。
//
// 用法：npm run sync-ffmpeg-manifest            # 写入
//       npm run sync-ffmpeg-manifest -- --check # 只检查是否一致（不一致时退出码 1）
//
// 升级 FFmpeg：`npm install --save-dev --save-exact @seankit/ffmpeg@<版本> @seankit/ffprobe@<版本>`，
// 再运行本脚本并提交生成的文件（cargo test 的 managed::tests 会检查两者一致）。

import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const REPO = path.resolve(import.meta.dirname, "..");
const OUT = path.join(REPO, "src-tauri", "resources", "ffmpeg-manifest.json");
const require = createRequire(path.join(REPO, "package.json"));

const read = (pkg) => JSON.parse(fs.readFileSync(require.resolve(`${pkg}/manifest.json`), "utf8"));

const ffmpeg = read("@seankit/ffmpeg");
const ffprobe = read("@seankit/ffprobe");
if (ffmpeg.ffmpegVersion !== ffprobe.ffmpegVersion || ffmpeg.packageVersion !== ffprobe.packageVersion) {
  console.error(
    `@seankit/ffmpeg（${ffmpeg.packageVersion} → FFmpeg ${ffmpeg.ffmpegVersion}）与 @seankit/ffprobe（${ffprobe.packageVersion} → FFmpeg ${ffprobe.ffmpegVersion}）版本不一致`,
  );
  process.exit(1);
}

const combined = {
  $comment: "由 scripts/sync-ffmpeg-manifest.mjs 从 @seankit/ffmpeg 与 @seankit/ffprobe 的 manifest.json 生成，不要手改",
  ffmpegVersion: ffmpeg.ffmpegVersion,
  packageVersion: ffmpeg.packageVersion,
  tools: { ffmpeg, ffprobe },
};
const text = `${JSON.stringify(combined, null, 2)}\n`;

if (process.argv.includes("--check")) {
  const current = fs.existsSync(OUT) ? fs.readFileSync(OUT, "utf8").replace(/\r\n/g, "\n") : "";
  if (current !== text) {
    console.error(`${path.relative(REPO, OUT)} 与已安装的 npm 包不一致，请运行 npm run sync-ffmpeg-manifest`);
    process.exit(1);
  }
  console.log(`${path.relative(REPO, OUT)} 已是最新（FFmpeg ${combined.ffmpegVersion}）`);
} else {
  fs.mkdirSync(path.dirname(OUT), { recursive: true });
  fs.writeFileSync(OUT, text);
  console.log(`已写入 ${path.relative(REPO, OUT)}（FFmpeg ${combined.ffmpegVersion}，包 ${combined.packageVersion}）`);
}
