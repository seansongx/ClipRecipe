#!/usr/bin/env node
// 发布辅助（流程见 docs/RELEASING.md）。
//
// 用法：
//   npm run release:version -- 0.2.0       # 把版本号同步写入 package.json、package-lock.json、src-tauri/Cargo.toml、
//                                          # src-tauri/Cargo.lock、src-tauri/tauri.conf.json
//   npm run release:check                  # 检查上面几处版本一致（不一致时退出码 1）
//   npm run release:check -- v0.2.0        # 另外检查：tag 与版本一致、CHANGELOG 有该版本的小节且已写日期、
//                                          # tauri.conf.json 里填了真正的 updater 公钥（发布流水线 release.yml 运行）
//   node scripts/release.mjs notes 0.2.0   # 输出该版本的发布说明（CHANGELOG 小节 + 下载说明），供草稿 Release 使用
//
// 只用 Node 自带模块，不联网。

import fs from "node:fs";
import path from "node:path";

const REPO = path.resolve(import.meta.dirname, "..");
const FILES = {
  packageJson: "package.json",
  packageLock: "package-lock.json",
  cargoToml: "src-tauri/Cargo.toml",
  cargoLock: "src-tauri/Cargo.lock",
  tauriConf: "src-tauri/tauri.conf.json",
  changelog: "CHANGELOG.md",
};
/** 与 src-tauri/src/system.rs 的 UPDATER_PUBKEY_PLACEHOLDER 相同 */
const PUBKEY_PLACEHOLDER = "REPLACE_WITH_UPDATER_PUBLIC_KEY (see docs/RELEASING.md)";
const REPO_URL = "https://github.com/seansongx/ClipRecipe";
const SEMVER =/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

const read = (file) => fs.readFileSync(path.join(REPO, file), "utf8");
const write = (file, text) => fs.writeFileSync(path.join(REPO, file), text);

function fail(message) {
  console.error(`✗ ${message}`);
  process.exit(1);
}

/** Cargo.toml 中 [package] 的 version 行。 */
const CARGO_TOML_VERSION = /^(\[package\][\s\S]*?^version\s*=\s*")([^"]+)(")/m;
/** Cargo.lock 中本程序（cliprecipe）的 version 行。 */
const CARGO_LOCK_VERSION = /(\[\[package\]\]\r?\nname = "cliprecipe"\r?\nversion = ")([^"]+)(")/;

/** 各处记录的版本号。 */
export function readVersions() {
  const pkg = JSON.parse(read(FILES.packageJson));
  const lock = JSON.parse(read(FILES.packageLock));
  const conf = JSON.parse(read(FILES.tauriConf));
  return {
    [FILES.packageJson]: pkg.version,
    [FILES.packageLock]: lock.version === lock.packages?.[""]?.version ? lock.version : `${lock.version} / ${lock.packages?.[""]?.version}`,
    [FILES.cargoToml]: read(FILES.cargoToml).match(CARGO_TOML_VERSION)?.[2],
    [FILES.cargoLock]: read(FILES.cargoLock).match(CARGO_LOCK_VERSION)?.[2],
    [FILES.tauriConf]: conf.version,
  };
}

function checkVersions() {
  const versions = readVersions();
  const distinct = new Set(Object.values(versions));
  if (distinct.size !== 1 || distinct.has(undefined)) {
    const lines = Object.entries(versions).map(([file, v]) => `  ${file}: ${v ?? "（没找到）"}`);
    fail(`版本号不一致，用 npm run release:version -- <版本> 同步：\n${lines.join("\n")}`);
  }
  return [...distinct][0];
}

function setVersion(version) {
  if (!SEMVER.test(version)) fail(`不是合法的版本号：${version}（例如 0.2.0）`);
  const pkg = JSON.parse(read(FILES.packageJson));
  pkg.version = version;
  write(FILES.packageJson, `${JSON.stringify(pkg, null, 2)}\n`);

  const lock = JSON.parse(read(FILES.packageLock));
  lock.version = version;
  if (lock.packages?.[""]) lock.packages[""].version = version;
  write(FILES.packageLock, `${JSON.stringify(lock, null, 2)}\n`);

  const conf = JSON.parse(read(FILES.tauriConf));
  conf.version = version;
  write(FILES.tauriConf, `${JSON.stringify(conf, null, 2)}\n`);

  for (const [file, pattern] of [
    [FILES.cargoToml, CARGO_TOML_VERSION],
    [FILES.cargoLock, CARGO_LOCK_VERSION],
  ]) {
    const text = read(file);
    if (!pattern.test(text)) fail(`${file} 里没找到版本号`);
    write(file, text.replace(pattern, `$1${version}$3`));
  }
  checkVersions();
  console.log(`✓ 版本号已改为 ${version}。接着在 CHANGELOG.md 写好 [${version}] 小节，见 docs/RELEASING.md`);
}

/** CHANGELOG.md 中某个版本的小节：标题行与正文。 */
export function changelogSection(changelog, version) {
  const lines = changelog.replace(/\r\n/g, "\n").split("\n");
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const heading = new RegExp(`^## \\[${escaped}\\]`);
  const start = lines.findIndex((l) => heading.test(l));
  if (start < 0) return null;
  let end = lines.findIndex((l, i) => i > start && (/^## /.test(l) || /^\[[^\]]+\]:\s/.test(l)));
  if (end < 0) end = lines.length;
  return { heading: lines[start], body: lines.slice(start + 1, end).join("\n").trim() };
}

/** 草稿 Release 的说明：CHANGELOG 小节 + 固定的下载说明。 */
export function releaseNotes(changelog, version) {
  const section = changelogSection(changelog, version);
  // Release 页面上相对链接无效：改为指向该 tag 下的文件
  const body = (section?.body || `请查看 CHANGELOG.md 中 ${version} 的说明。`).replace(
    /\]\((?!https?:|#|mailto:)([^)\s]+)\)/g,
    `](${REPO_URL}/blob/v${version}/$1)`,
  );
  return `${body}

---

### 下载

- **Windows 10 / 11（x64）**：\`ClipRecipe_${version}_x64-setup.exe\`。安装包没有代码签名，首次运行时 Windows SmartScreen 可能提示“已保护你的电脑”，点“更多信息”→“仍要运行”即可。
- **Linux（x64）**：\`ClipRecipe_${version}_amd64.AppImage\`（加上可执行权限后直接运行，支持程序内自动更新），或 \`ClipRecipe_${version}_amd64.deb\`（Debian / Ubuntu，更新时请重新下载安装）。
- 暂不提供 macOS 版本。
- 已安装的 Windows 安装包版与 AppImage 版会在“设置 → 检查更新”中提示并可在程序内更新；其余文件（\`.sig\`、\`latest.json\`）供自动更新使用，不需要手动下载。
`;
}

/** 公钥是否像真正的 minisign 公钥（base64，解开后以 untrusted comment 开头）。 */
export function looksLikePubkey(pubkey) {
  if (typeof pubkey !== "string" || !pubkey.trim() || pubkey.trim() === PUBKEY_PLACEHOLDER) return false;
  try {
    return Buffer.from(pubkey.trim(), "base64").toString("utf8").startsWith("untrusted comment:");
  } catch {
    return false;
  }
}

function checkRelease(tag) {
  const version = checkVersions();
  console.log(`✓ 各处版本号一致：${version}`);
  if (!tag) return;
  if (tag !== `v${version}`) fail(`tag ${tag} 与版本号 ${version} 不一致（应为 v${version}）`);
  console.log(`✓ tag ${tag} 与版本号一致`);

  const section = changelogSection(read(FILES.changelog), version);
  if (!section) fail(`CHANGELOG.md 里没有 ## [${version}] 小节`);
  if (!/\d{4}-\d{2}-\d{2}/.test(section.heading))
    fail(`CHANGELOG.md 的 ${section.heading} 还没有写发布日期（改为 ## [${version}] - YYYY-MM-DD）`);
  if (!section.body) fail(`CHANGELOG.md 的 [${version}] 小节是空的`);
  console.log(`✓ CHANGELOG.md 有 ${section.heading}`);

  const pubkey = JSON.parse(read(FILES.tauriConf)).plugins?.updater?.pubkey;
  if (!looksLikePubkey(pubkey))
    fail("src-tauri/tauri.conf.json 的 plugins.updater.pubkey 还是占位值或不是 minisign 公钥，见 docs/RELEASING.md“首次配置”");
  console.log("✓ 已配置 updater 公钥");
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === path.resolve(import.meta.filename);
if (isMain) {
  const [command, arg] = process.argv.slice(2);
  if (command === "version") {
    if (!arg) fail("用法：npm run release:version -- <版本号>");
    setVersion(arg.replace(/^v/, ""));
  } else if (command === "check") {
    checkRelease(arg);
  } else if (command === "notes") {
    process.stdout.write(releaseNotes(read(FILES.changelog), (arg ?? checkVersions()).replace(/^v/, "")));
  } else {
    fail("用法：node scripts/release.mjs version <版本号> | check [tag] | notes [版本号]");
  }
}
