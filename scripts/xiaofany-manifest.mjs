#!/usr/bin/env node
// 由 GitHub Release 下载下来的文件生成 xiaofany.com 发布清单（scripts/publish-xiaofany.mjs --manifest）。
//
//   node scripts/xiaofany-manifest.mjs --dir <下载目录> --tag v0.1.0 --upstream-base <Release 下载地址前缀> [--out <清单路径>]
//
// 每个文件写明 platform、arch、format、kind；有同名 .sig 时写进 signature；upstreamUrl 指向 GitHub
// Release 上的同名文件，作为备用线路。认不出的文件、版本号不符的文件、没有对应文件的 .sig、
// 缺了必需文件或签名的，一律报错，不会悄悄漏掉或多传。
// 清单默认写到 <下载目录>/xiaofany-release.json，文件路径相对清单所在目录。

import { readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { parseArgs } from "node:util";

const { values: args } = parseArgs({
  options: {
    dir: { type: "string" },
    tag: { type: "string" },
    "upstream-base": { type: "string" },
    out: { type: "string" },
  },
});

function fail(message) {
  console.error(`✗ ${message}`);
  process.exit(1);
}

if (!args.dir || !args.tag || !args["upstream-base"]) {
  fail("用法：--dir <目录> --tag v<版本> --upstream-base <url> [--out <清单路径>]");
}
const version = args.tag.replace(/^v(?=\d)/, "");
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) fail(`tag 不是 v<semver>：${args.tag}`);

const v = version.replace(/[.+]/g, "\\$&");
// 文件名是 tauri-action 按 productName 生成的默认名字（见 docs/RELEASING.md「发布哪些文件」）。
// signed：自动更新用的包，正式构建一定带 .sig（缺了说明 Release 上的文件不全）。
// arch 没写的规则取文件名里的第一个捕获组。
// optional：实验性的 macOS dmg（release.yml 的 build-macos 任务，没有更新包与签名）。有就一起发布，
// 供 website/install.sh 从 xiaofany.com 下载；缺了只给出提示，不拦住 Windows 与 Linux 的发布。
const RULES = [
  { re: new RegExp(`^ClipRecipe_${v}_x64-setup\\.exe$`), platform: "win", arch: "x64", format: "exe", signed: true },
  { re: new RegExp(`^ClipRecipe_${v}_amd64\\.AppImage$`), platform: "linux", arch: "x64", format: "appimage", signed: true },
  { re: new RegExp(`^ClipRecipe_${v}_amd64\\.deb$`), platform: "linux", arch: "x64", format: "deb" },
  { re: new RegExp(`^ClipRecipe_${v}_macos_(arm64)\\.dmg$`), platform: "mac", format: "dmg", optional: true },
  { re: new RegExp(`^ClipRecipe_${v}_macos_(x64)\\.dmg$`), platform: "mac", format: "dmg", optional: true },
];
// latest.json 是 GitHub 兜底更新清单，不上传；发布清单自己也在目录里。
const IGNORED = new Set(["latest.json", "xiaofany-release.json"]);

const dir = path.resolve(args.dir);
const out = path.resolve(args.out ?? path.join(dir, "xiaofany-release.json"));
const names = (await readdir(dir)).filter((name) => !IGNORED.has(name)).sort();
const present = new Set(names);
const base = args["upstream-base"].replace(/\/+$/, "");

const files = [];
for (const name of names) {
  if (name.endsWith(".sig")) {
    if (!present.has(name.slice(0, -4))) fail(`签名文件没有对应的安装包：${name}`);
    continue;
  }
  const rule = RULES.find((item) => item.re.test(name));
  if (!rule) fail(`认不出的文件（文件名规则或版本号不符）：${name}`);
  const entry = {
    path: path.relative(path.dirname(out), path.join(dir, name)).split(path.sep).join("/"),
    platform: rule.platform,
    arch: rule.arch ?? rule.re.exec(name)[1],
    format: rule.format,
    kind: "installer",
    upstreamUrl: `${base}/${encodeURIComponent(name)}`,
  };
  if (present.has(`${name}.sig`)) {
    entry.signature = `${entry.path}.sig`;
  } else if (rule.signed) {
    fail(`缺少更新签名：${name}.sig`);
  }
  files.push(entry);
}
// 每个版本都应 Windows 与 Linux 的三个包齐全：少了哪个，xiaofany.com 上该平台就会停在旧版本。
for (const rule of RULES) {
  if (names.some((name) => rule.re.test(name))) continue;
  if (rule.optional) {
    console.warn(`！没有 ${rule.platform} ${rule.format} 安装包（${rule.re.source}），该平台在 xiaofany.com 上停在旧版本`);
  } else {
    fail(`缺少 ${rule.platform} ${rule.format} 安装包（${rule.re.source}）`);
  }
}

await writeFile(out, `${JSON.stringify({ version, files }, null, 2)}\n`);
console.log(`→ ${path.basename(out)}：${version}，${files.length} 个文件`);
for (const f of files) {
  console.log(`  ${path.basename(f.path)}  [${f.platform} ${f.arch} ${f.format} ${f.kind}${f.signature ? " 已签名" : ""}]`);
}
