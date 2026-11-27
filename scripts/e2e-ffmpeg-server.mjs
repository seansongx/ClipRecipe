// 端到端测试：应用内下载 FFmpeg 的本机假服务器。
//
// 把 node_modules 里（或 CLIPRECIPE_FFMPEG 指定的）真正的 ffmpeg / ffprobe 打成与 npm 包相同结构的 tgz
// （package/package.json + package/bin/<可执行文件>），在 127.0.0.1 的随机端口上提供：
//   /mirror/<工具>.tgz → 503（验证换下一个地址）
//   /npm/<工具>.tgz    → 正常内容，分块限速发送（界面能看到进度、来得及点“取消”）。
//                        程序启动时缺少 FFmpeg 会自动开始下载：用 slow: true 启动，先慢速发送，测试稳定地看到“自动开始后的
//                        下载中”并取消；之后 setSlow(false) 恢复正常速度（正在进行的发送下一块起生效）
// 并写一份与 src-tauri/resources/ffmpeg-manifest.json 格式相同的测试清单（http://127.0.0.1 地址、真实的哈希）。
//
// 程序只在以 `--features e2e-hooks` 构建的调试版中、设置了 CLIPRECIPE_E2E_FFMPEG_MANIFEST 时才读取这份清单
// （见 src-tauri/src/ffmpeg/manifest.rs 的 active()）；正式构建没有这个入口，只认内嵌清单与 HTTPS。

import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import zlib from "node:zlib";
import { REPO } from "./e2e-env.mjs";

const sha256 = (buf) => crypto.createHash("sha256").update(buf).digest("hex");

/** 一个 ustar 头（512 字节）。 */
function tarHeader(name, size) {
  const h = Buffer.alloc(512, 0);
  const put = (offset, length, text) => h.write(text, offset, Math.min(length, Buffer.byteLength(text)), "latin1");
  const octal = (offset, length, value) => put(offset, length, `${value.toString(8).padStart(length - 1, "0")}\0`);
  put(0, 100, name);
  octal(100, 8, 0o755);
  octal(108, 8, 0);
  octal(116, 8, 0);
  octal(124, 12, size);
  octal(136, 12, Math.floor(Date.now() / 1000));
  h.fill(0x20, 148, 156); // 计算校验和时按空格
  put(156, 1, "0");
  put(257, 6, "ustar\0");
  put(263, 2, "00");
  let sum = 0;
  for (const b of h) sum += b;
  put(148, 8, `${sum.toString(8).padStart(6, "0")}\0 `);
  return h;
}

function tarEntry(name, data) {
  const pad = (512 - (data.length % 512)) % 512;
  return [tarHeader(name, data.length), data, Buffer.alloc(pad, 0)];
}

/** 与 npm 包相同结构的 tgz。 */
function packageTgz(tool, binary) {
  const tar = Buffer.concat([
    ...tarEntry("package/package.json", Buffer.from(JSON.stringify({ name: `@seankit/${tool}-e2e`, version: "0.0.0-e2e" }))),
    ...tarEntry(`package/bin/${path.basename(binary.file)}`, binary.data),
    Buffer.alloc(1024, 0),
  ]);
  return zlib.gzipSync(tar, { level: 1 });
}

/**
 * 启动假服务器并写出测试清单。
 * @returns {{ manifestPath, port, hits: string[], close: () => Promise<void>, sizes }}
 */
export async function startFakeFfmpegServer({ dir, ffmpeg, ffprobe, chunk = 512 * 1024, delayMs = 15, slow = false }) {
  // 慢速：每 250 ms 发 64 KB（约 256 KB/s），进度在动、取消在块之间生效，几十秒内不会下完
  const SLOW = { chunk: 64 * 1024, delayMs: 250 };
  let slowMode = slow;
  const pace = () => (slowMode ? SLOW : { chunk, delayMs });
  fs.mkdirSync(dir, { recursive: true });
  const committed = JSON.parse(fs.readFileSync(path.join(REPO, "src-tauri", "resources", "ffmpeg-manifest.json"), "utf8"));
  const version = committed.ffmpegVersion;
  const files = {};
  for (const [tool, file] of [["ffmpeg", ffmpeg], ["ffprobe", ffprobe]]) {
    const data = fs.readFileSync(file);
    const tgz = packageTgz(tool, { file, data });
    files[tool] = { tgz, binary: data, name: path.basename(file) };
  }

  const hits = [];
  const sockets = new Set();
  const server = http.createServer((req, res) => {
    hits.push(req.url);
    const m = /^\/(mirror|npm)\/(ffmpeg|ffprobe)\.tgz$/.exec(req.url ?? "");
    if (!m) {
      res.writeHead(404).end();
      return;
    }
    if (m[1] === "mirror") {
      res.writeHead(503, { "Content-Type": "text/plain" }).end("mirror down");
      return;
    }
    const body = files[m[2]].tgz;
    res.writeHead(200, { "Content-Type": "application/octet-stream", "Content-Length": body.length });
    let offset = 0;
    let closed = false;
    res.on("close", () => (closed = true));
    const pump = () => {
      if (closed) return;
      if (offset >= body.length) {
        res.end();
        return;
      }
      const p = pace();
      const next = body.subarray(offset, offset + p.chunk);
      offset += next.length;
      res.write(next, () => setTimeout(pump, p.delayMs));
    };
    pump();
  });
  server.on("connection", (s) => {
    sockets.add(s);
    s.on("close", () => sockets.delete(s));
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  const url = (kind, tool) => `http://127.0.0.1:${port}/${kind}/${tool}.tgz`;

  const tool = (name) => {
    const f = files[name];
    return {
      tool: name,
      ffmpegVersion: version,
      packageVersion: "0.0.0-e2e",
      aliases: { "win32-arm64": "win32-x64" },
      platforms: {
        "win32-x64": {
          package: `@seankit/${name}-win32-x64`,
          version: "0.0.0-e2e",
          file: `${name}.tgz`,
          size: f.tgz.length,
          sha256: sha256(f.tgz),
          integrity: "",
          binary: `bin/${f.name}`,
          binarySha256: sha256(f.binary),
          binarySize: f.binary.length,
          license: "GPL-3.0-or-later",
          rustTargets: ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"],
          urls: [url("mirror", name), url("npm", name)],
        },
      },
    };
  };
  const manifest = { ffmpegVersion: version, packageVersion: "0.0.0-e2e", tools: { ffmpeg: tool("ffmpeg"), ffprobe: tool("ffprobe") } };
  const manifestPath = path.join(dir, "ffmpeg-manifest.e2e.json");
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2));

  return {
    port,
    hits,
    version,
    manifestPath,
    sizes: { ffmpeg: files.ffmpeg.tgz.length, ffprobe: files.ffprobe.tgz.length },
    /** 切换慢速发送（见文件头） */
    setSlow: (on) => {
      slowMode = on;
    },
    close: () =>
      new Promise((resolve) => {
        for (const s of sockets) s.destroy();
        server.close(() => resolve());
      }),
  };
}
