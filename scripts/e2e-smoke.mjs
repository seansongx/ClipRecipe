#!/usr/bin/env node
// 端到端冒烟测试：启动真正的 ClipRecipe 桌面窗口（Tauri 2 + WebView2），经 Chrome DevTools Protocol 操作界面，
// 检查前后端命令接线、事件、生产 CSP、asset 协议播放、拖放路由、对话框流程、设置与队列持久化。
//
// 用法：
//   npm run e2e                     # 生产形态（内嵌前端、正式 CSP）：先 `tauri build --debug --no-bundle`，跑全部场景
//   npm run e2e -- --mode dev       # 开发形态（`tauri dev`：vite + 调试程序），跑基础场景
//   npm run e2e -- --mode both
//   npm run e2e -- --no-build       # 生产形态不重新构建，直接用 src-tauri/target/debug/cliprecipe.exe
//   npm run e2e -- --keep           # 保留临时目录（素材、输出、隔离的配置）便于排查
//
// 需要：Windows + WebView2；FFmpeg（环境变量 CLIPRECIPE_FFMPEG 指向 ffmpeg.exe，或在 PATH 中）。
// 应用窗口会出现在桌面上，测试结束（含失败、Ctrl+C）时关闭。
//
// 隔离：CLIPRECIPE_CONFIG_DIR（设置、队列）与 WEBVIEW2_USER_DATA_FOLDER（localStorage：配方、工具参数、主题）
// 都指向临时目录，不碰用户的真实数据；启动后用 data_dir 命令确认，不一致时立即停止。
// 系统文件对话框无法自动操作：探针（scripts/e2e-cdp.mjs 的 PROBE_SCRIPT）在 fetch 层用预先排队的路径代替其结果。
// 拖放：通过 Tauri 事件 tauri://drag-enter / drag-over / drag-drop 模拟（与系统拖放进入前端的路径相同，
// 但不经过操作系统的 OLE 拖放）。
//
// 不会触发：关机 / 退出倒计时、删除源视频、移回源位置、永久删除、托盘、检查更新（联网）、打开文件夹 / 网页。

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, execSync } from "node:child_process";
import { Cdp, Page, sleep, waitForPage } from "./e2e-cdp.mjs";
import {
  DEBUG_EXE,
  E2E_TARGET_DIR,
  PROD_EXE,
  REPO,
  bandStddev,
  ffmpegPaths,
  freePort,
  grayFrame,
  killAllChildren,
  killTree,
  listFiles,
  makeMedia,
  portInUse,
  probe,
  processesOf,
  removeDir,
  startProc,
} from "./e2e-env.mjs";

// ------------------------------------------------------------------ 参数与目录

const argv = process.argv.slice(2);
const opt = (name) => argv.includes(name);
const modeArg = argv[argv.indexOf("--mode") + 1];
const MODES = argv.includes("--mode") ? (modeArg === "both" ? ["prod", "dev"] : [modeArg]) : ["prod"];
if (MODES.some((m) => m !== "prod" && m !== "dev")) {
  console.error("--mode 只能是 prod、dev 或 both");
  process.exit(2);
}
if (process.platform !== "win32") {
  console.error("e2e 冒烟测试目前只支持 Windows（WebView2 远程调试）");
  process.exit(2);
}

const stamp = new Date().toISOString().replace(/[-:T]/g, "").slice(0, 14);
const WORK = path.join(os.tmpdir(), `cliprecipe-e2e-${stamp}`);
const REPORT = path.join(os.tmpdir(), `cliprecipe-e2e-report-${stamp}`);
fs.mkdirSync(WORK, { recursive: true });
fs.mkdirSync(REPORT, { recursive: true });
const w = (...p) => path.join(WORK, ...p);

const NO_PROXY = "127.0.0.1,localhost,::1";
process.env.NO_PROXY = NO_PROXY;
process.env.no_proxy = NO_PROXY;

// ------------------------------------------------------------------ 结果记录

const results = [];
let shotIndex = 0;

function log(...args) {
  console.log(`[${new Date().toLocaleTimeString()}]`, ...args);
}

class Skip extends Error {}

async function saveArtifacts(session, label) {
  const files = [];
  const base = `${session.mode}-${String(++shotIndex).padStart(2, "0")}-${label.replace(/[^\w一-龥-]+/g, "_")}`;
  if (session.page && !session.page.cdp.closed) {
    try {
      const png = path.join(REPORT, `${base}.png`);
      fs.writeFileSync(png, await session.page.screenshot());
      files.push(png);
    } catch (e) {
      log("  截图失败：", e.message);
    }
  }
  try {
    const probeState = session.page && !session.page.cdp.closed
      ? await session.page.evaluate(() => ({ csp: window.__e2e?.csp, errors: window.__e2e?.errors, ipc: window.__e2e?.ipc?.slice(-80), unexpectedDialogs: window.__e2e?.unexpectedDialogs, body: document.body.innerText.slice(0, 4000) }))
      : null;
    const json = path.join(REPORT, `${base}.console.json`);
    fs.writeFileSync(json, JSON.stringify({ console: session.page?.console ?? [], probe: probeState }, null, 2));
    files.push(json);
  } catch (e) {
    log("  保存控制台记录失败：", e.message);
  }
  return files;
}

/** 本场景开始以来新增的错误：控制台错误、异常、CSP 违规、IPC 失败、意外的对话框。 */
async function problemsSince(session, mark, allow) {
  const page = session.page;
  const consoleErrors = page.console
    .slice(mark.console)
    .filter((c) => c.kind === "console.error" || c.kind === "exception" || /^log\..*\.error$/.test(c.kind))
    .filter((c) => !allow.console?.some((re) => re.test(c.text)));
  const state = await page.evaluate(
    (m) => ({
      csp: window.__e2e.csp.slice(m.csp),
      errors: window.__e2e.errors.slice(m.errors),
      ipcFailed: window.__e2e.ipc.slice(m.ipc).filter((r) => r.ok === false),
      dialogs: window.__e2e.unexpectedDialogs.slice(m.dialogs),
    }),
    mark,
  );
  const csp = state.csp.filter((v) => !allow.csp?.some((re) => re.test(v.blocked ?? "")));
  const ipcFailed = state.ipcFailed.filter((r) => !allow.ipc?.includes(r.cmd));
  const errors = state.errors.filter((e) => !allow.console?.some((re) => re.test(e.message)));
  return { consoleErrors, csp, ipcFailed, errors, dialogs: state.dialogs };
}

async function markOf(session) {
  const counts = await session.page.evaluate(() => ({
    csp: window.__e2e.csp.length,
    errors: window.__e2e.errors.length,
    ipc: window.__e2e.ipc.length,
    dialogs: window.__e2e.unexpectedDialogs.length,
  }));
  return { ...counts, console: session.page.console.length };
}

/**
 * 运行一个场景。fn 返回说明文字；抛出 Skip 记为跳过。
 * allow：{ console: RegExp[], csp: RegExp[], ipc: 命令名[] } —— 场景里故意制造的错误。
 */
async function scenario(session, name, fn, allow = {}) {
  log(`▶ [${session.mode}] ${name}`);
  const started = Date.now();
  let status = "pass";
  let details = "";
  let artifacts = [];
  let mark = null;
  try {
    if (!session.page) throw new Error("应用未运行");
    await session.page.evaluate(() => (window.__e2e.dialogQueue.length = 0));
    mark = await markOf(session);
    details = (await fn()) ?? "";
    const p = await problemsSince(session, mark, allow);
    const issues = [];
    const unused = await session.page.evaluate(() => window.__e2e.dialogQueue.splice(0));
    if (unused.length) issues.push(`预先排队的对话框结果没有被用到（对应的按钮没有打开对话框）：${JSON.stringify(unused)}`);
    if (p.csp.length) issues.push(`CSP 违规：${JSON.stringify(p.csp)}`);
    if (p.consoleErrors.length) issues.push(`控制台错误：${p.consoleErrors.map((c) => c.text).join(" | ").slice(0, 800)}`);
    if (p.errors.length) issues.push(`页面异常：${JSON.stringify(p.errors).slice(0, 800)}`);
    if (p.ipcFailed.length) issues.push(`命令失败：${p.ipcFailed.map((r) => `${r.cmd} → ${r.error}`).join(" | ").slice(0, 800)}`);
    if (p.dialogs.length) issues.push(`意外的系统对话框：${JSON.stringify(p.dialogs)}`);
    if (issues.length) throw new Error(issues.join("\n"));
  } catch (e) {
    if (e instanceof Skip) {
      status = "skip";
      details = e.message;
    } else {
      status = "fail";
      details = `${details ? `${details}\n` : ""}${e.message}`;
      artifacts = await saveArtifacts(session, name);
      await recover(session);
    }
  }
  const secs = ((Date.now() - started) / 1000).toFixed(1);
  results.push({ mode: session.mode, name, status, details, artifacts, secs });
  const icon = status === "pass" ? "✔" : status === "skip" ? "–" : "✘";
  log(`${icon} [${session.mode}] ${name} (${secs}s)${details ? `\n    ${details.split("\n").join("\n    ")}` : ""}`);
  for (const a of artifacts) log(`    → ${a}`);
  return status === "pass";
}

/** 场景失败后尽量回到干净的界面：关掉对话框、回到批量处理页，避免影响后面的场景。 */
async function recover(session) {
  const page = session.page;
  if (!page || page.cdp.closed) return;
  try {
    await page.evaluate(() => (window.__e2e.dialogQueue.length = 0));
    for (let i = 0; i < 4; i++) {
      const open = await page.evaluate(() => document.querySelectorAll("[role=dialog],[role=alertdialog]").length);
      if (!open) break;
      await page.escape();
      await sleep(400);
    }
    await page.click({ selector: "#nav-batch" }, { timeout: 3000 });
  } catch {
    // 尽力而为
  }
}

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

// ------------------------------------------------------------------ 应用会话

class Session {
  constructor(mode, exe) {
    this.mode = mode;
    this.exe = exe;
    this.proc = null;
    this.page = null;
    this.cdp = null;
    this.appPids = new Set();
    this.launches = 0;
    // 每种形态一个隔离的配置目录；关闭完成提示音（测试时不出声），其余为默认设置
    this.configDir = w(`config-${mode}`);
    fs.mkdirSync(this.configDir, { recursive: true });
    fs.writeFileSync(path.join(this.configDir, "settings.json"), JSON.stringify({ version: 1, sound: false }));
  }

  env(port) {
    const { ffmpeg } = ffmpegPaths();
    return {
      ...process.env,
      CLIPRECIPE_FFMPEG: ffmpeg,
      CLIPRECIPE_CONFIG_DIR: this.configDir,
      WEBVIEW2_USER_DATA_FOLDER: w(`webview-${this.mode}`),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
      NO_PROXY,
      no_proxy: NO_PROXY,
      // tauri dev 也编译到独立目录，不与 cargo test 争用 target/debug（生产形态的程序已另存为 PROD_EXE）
      CARGO_TARGET_DIR: E2E_TARGET_DIR,
    };
  }

  async start() {
    const port = await freePort();
    const logFile = path.join(REPORT, `${this.mode}-app-${++this.launches}.log`);
    if (this.mode === "prod") {
      this.proc = startProc("app", this.exe, [], this.env(port), logFile);
    } else {
      if (await portInUse(1420)) throw new Error("端口 1420 已被占用（Vite 需要它），请先结束占用它的进程");
      this.proc = startProc("tauri-dev", "npm run tauri dev", null, this.env(port), logFile);
    }
    log(`  启动 ${this.mode}（调试端口 ${port}，日志 ${logFile}）`);
    const accept = this.mode === "prod" ? (u) => u.startsWith("http://tauri.localhost") : (u) => /^http:\/\/(localhost|127\.0\.0\.1):1420/.test(u);
    let target;
    try {
      // tauri dev 编译失败时不会退出（等待文件变化），从日志里发现后立即停止等待
      const compileFailed = () =>
        this.mode === "dev" &&
        fs.existsSync(logFile) &&
        /error: could not compile|Error failed to build/i.test(fs.readFileSync(logFile, "utf8").replace(/\x1b\[[0-9;]*m/g, ""));
      target = await waitForPage(port, accept, this.mode === "prod" ? 60000 : 900000, () => !this.proc.exited && !compileFailed());
    } catch (e) {
      if (this.mode === "prod" && e.message.includes(":1420")) {
        throw new Error(`${e.message}\n程序加载的是开发服务器地址：${this.exe} 是开发版（没有内嵌前端），去掉 --no-build 重新构建`);
      }
      throw e;
    }
    for (const pid of processesOf(this.exe)) this.appPids.add(pid);
    this.cdp = await Cdp.connect(target.webSocketDebuggerUrl);
    this.page = new Page(this.cdp);
    await this.page.init();
    // 重新加载，让探针（CSP、异常、IPC 记录）从文档开始生效
    await this.page.reload();
    await this.page.waitFor("主界面", () => !!document.querySelector("#nav-batch") && document.body.innerText.includes("视频队列"), [], { timeout: 30000 });
    // 隔离检查：配置目录必须是测试目录
    const dir = await this.page.invoke("data_dir");
    if (!dir.ok || path.resolve(dir.value) !== path.resolve(this.configDir)) {
      throw new Error(`配置目录未隔离（data_dir = ${JSON.stringify(dir)}），停止测试以免改动用户设置`);
    }
    await sleep(800); // 设置读取、队列恢复
  }

  /** 关闭应用：graceful 时调用 quit_app（等同托盘“退出”），否则直接结束进程树。 */
  async stop({ graceful = true } = {}) {
    if (!this.proc) return;
    if (graceful && this.page && !this.cdp.closed) {
      try {
        await this.page.evaluate(() => {
          setTimeout(() => void window.__TAURI_INTERNALS__.invoke("quit_app").catch(() => {}), 50);
          return true;
        });
      } catch {
        // 连接可能已断开
      }
      const deadline = Date.now() + 15000;
      while (Date.now() < deadline && processesOf(this.exe).length > 0) await sleep(300);
    }
    this.cdp?.close();
    this.page = null;
    // 启动失败时还没记录进程号：先记下来，结束后才能删掉它们的预览目录
    for (const pid of processesOf(this.exe)) this.appPids.add(pid);
    this.proc.kill();
    for (const pid of this.appPids) killTree(pid);
    this.proc = null;
    await sleep(1000);
  }

  /** 删除被强制结束的应用留下的预览目录。 */
  cleanupPreviewDirs() {
    for (const pid of this.appPids) {
      for (const name of ["cliprecipe-preview", "cliprecipe-build", "cliprecipe-tools"]) {
        const dir = path.join(os.tmpdir(), name, String(pid));
        if (fs.existsSync(dir)) fs.rmSync(dir, { recursive: true, force: true });
      }
    }
  }
}

// ------------------------------------------------------------------ 界面操作

const ui = {
  async nav(page, id, heading) {
    await page.click({ selector: `#nav-${id}` });
    await page.waitFor(`页面 ${heading}`, (h) => [...document.querySelectorAll("main h1")].some((e) => e.textContent.trim() === h), [heading]);
  },

  rows(page) {
    return page.evaluate(() =>
      [...document.querySelectorAll("li[data-row-index]")].map((li) => {
        const title = li.querySelector("p[title]");
        const body = title?.closest("div.min-w-0");
        const state = [...li.children].find((c) => c.tagName === "SPAN" && c.classList.contains("tabular-nums") && !c.classList.contains("w-6"));
        return {
          name: title?.textContent.trim(),
          path: title?.getAttribute("title"),
          summary: body?.querySelector(":scope > p")?.textContent.trim() ?? "",
          state: state?.textContent.trim() ?? "",
          badges: [...li.querySelectorAll("[data-slot=badge]")].map((b) => ({ text: b.textContent.trim(), title: b.getAttribute("title") })),
          text: li.innerText,
        };
      }),
    );
  },

  async waitRows(page, what, pred, timeout = 30000) {
    const deadline = Date.now() + timeout;
    let rows = [];
    while (Date.now() < deadline) {
      rows = await ui.rows(page);
      if (pred(rows)) return rows;
      await sleep(250);
    }
    throw new Error(`等待超时：${what}；当前队列：${JSON.stringify(rows.map((r) => [r.name, r.state, r.summary]))}`);
  },

  async dropOnQueue(page, paths) {
    await page.dropFiles(paths, { selector: "main h1", text: "视频队列" });
  },

  async clearQueue(page) {
    if ((await ui.rows(page)).length === 0) return;
    await page.click({ selector: "button", textIncludes: "清空", within: "main" });
    await page.click({ selector: "button", text: "删除", within: "[role=alertdialog]" });
    await ui.waitRows(page, "队列清空", (r) => r.length === 0, 10000);
  },

  async setOutputDir(page, dir, within = "main") {
    fs.mkdirSync(dir, { recursive: true });
    await page.queueDialog(dir);
    await page.click({ selector: "button", textIncludes: "输出到", within });
    await page.waitFor("输出位置已设置", (d) => [...document.querySelectorAll("input[aria-label=输出文件夹]")].some((i) => i.value === d), [dir]);
  },

  switchState(page, label, within = "aside") {
    return page.evaluate((l, wi) => window.__e2e.find({ selector: "[role=switch]", label: l, within: wi })?.getAttribute("data-state") ?? null, label, within);
  },

  async setSwitch(page, label, on, within = "aside") {
    const state = await ui.switchState(page, label, within);
    if (state === null) throw new Error(`找不到开关：${label}`);
    if ((state === "checked") !== on) await page.click({ selector: "[role=switch]", label, within });
    await page.waitFor(`开关 ${label} = ${on}`, (l, wi, want) => window.__e2e.find({ selector: "[role=switch]", label: l, within: wi })?.getAttribute("data-state") === want, [label, within, on ? "checked" : "unchecked"]);
  },

  async expandModule(page, name) {
    const spec = { selector: "aside button[aria-expanded]", textStarts: name };
    const expanded = await page.evaluate((s) => window.__e2e.find(s)?.getAttribute("aria-expanded"), spec);
    if (expanded !== "true") await page.click(spec);
    await page.waitFor(`展开 ${name}`, (s) => window.__e2e.find(s)?.getAttribute("aria-expanded") === "true", [spec]);
  },

  async choose(page, label, within = "aside") {
    await page.click({ selector: "[role=radio]", text: label, within });
    await page.waitFor(`选项 ${label}`, (l, wi) => window.__e2e.find({ selector: "[role=radio]", text: l, within: wi })?.getAttribute("data-state") === "on", [label, within]);
  },

  runSummary(page) {
    return page.evaluate(() => [...document.querySelectorAll("main footer span")].map((s) => s.textContent.trim()).find((t) => t.startsWith("上次") || t.startsWith("处理中")) ?? "");
  },

  async startBatch(page) {
    await page.click({ selector: "main footer button", text: "开始处理" });
    await page.waitFor("处理开始", () => [...document.querySelectorAll("main footer button")].some((b) => b.textContent.trim() === "停止") || [...document.querySelectorAll("main footer span")].some((s) => s.textContent.startsWith("上次")), [], { timeout: 15000 });
  },

  async waitBatchFinished(page, timeout = 240000) {
    await page.waitFor("批量处理结束", () => [...document.querySelectorAll("main footer span")].some((s) => s.textContent.trim().startsWith("上次")) && ![...document.querySelectorAll("main footer button")].some((b) => b.textContent.trim() === "停止"), [], { timeout, interval: 500 });
    return ui.runSummary(page);
  },

  logText(page) {
    return page.evaluate(() => {
      const section = [...document.querySelectorAll("main section")].find((s) => s.textContent.includes("处理记录"));
      return section?.innerText ?? "";
    });
  },

  /** 预览窗口里的 <video> 能加载并播放（asset 协议、CSP）。 */
  async checkVideo(page, what, timeout = 120000) {
    const state = await page.waitFor(
      `${what}：视频加载`,
      () => {
        const dialog = [...document.querySelectorAll("[role=dialog]")].pop();
        if (!dialog) return null;
        const v = dialog.querySelector("video");
        if (v?.error) return { error: `MediaError ${v.error.code} ${v.error.message}`, src: v.src };
        if (v && v.readyState >= 2 && v.videoWidth > 0) return { src: v.src, width: v.videoWidth, height: v.videoHeight, duration: v.duration };
        const text = dialog.innerText;
        if (!v && !text.includes("正在") && dialog.querySelector("svg.lucide-triangle-alert")) return { error: text.slice(0, 300) };
        return null;
      },
      [],
      { timeout, interval: 300 },
    );
    if (state.error) throw new Error(`${what}：${state.error}（${state.src ?? ""}）`);
    const played = await page.evaluate(async () => {
      const v = [...document.querySelectorAll("[role=dialog]")].pop().querySelector("video");
      v.muted = true;
      try {
        await v.play();
      } catch (e) {
        return { error: String(e) };
      }
      const t0 = v.currentTime;
      await new Promise((r) => setTimeout(r, 900));
      return { advanced: v.currentTime > t0, currentTime: v.currentTime };
    });
    if (played.error || !played.advanced) throw new Error(`${what}：无法播放 ${JSON.stringify(played)}`);
    return `${state.width}×${state.height}，${state.duration?.toFixed?.(1)}s，${state.src.slice(0, 60)}…`;
  },

  async closeDialog(page) {
    const before = await page.evaluate(() => document.querySelectorAll("[role=dialog],[role=alertdialog]").length);
    await page.escape();
    await page.waitFor("对话框关闭", (n) => document.querySelectorAll("[role=dialog],[role=alertdialog]").length < n, [before], { timeout: 5000 });
  },

  async openTool(page, name) {
    await ui.nav(page, "toolbox", "工具箱");
    await page.click({ selector: "[role=button]", label: `打开 ${name}` });
    await page.waitFor(`工具对话框 ${name}`, (n) => {
      const d = document.querySelector("[role=dialog]");
      return d && d.querySelector("h2")?.textContent.includes(n) && [...d.querySelectorAll("button")].some((b) => b.textContent.trim() === "开始");
    }, [name]);
  },

  async runTool(page, timeout = 180000) {
    await page.click({ selector: "[role=dialog] button", text: "开始" });
    const summary = await page.waitFor(
      "工具运行结束",
      () => {
        const d = document.querySelector("[role=dialog]");
        const s = [...d.querySelectorAll("span")].map((x) => x.textContent.trim()).find((t) => t.startsWith("上次："));
        const running = [...d.querySelectorAll("button")].some((b) => b.textContent.trim() === "停止");
        return s && !running ? s : null;
      },
      [],
      { timeout, interval: 500 },
    );
    const m = /完成 (\d+) · 失败 (\d+) · 取消 (\d+)/.exec(summary);
    const items = await page.evaluate(() => [...document.querySelectorAll("[role=dialog] ul li")].map((li) => li.innerText.replace(/\s+/g, " ")));
    return { done: Number(m?.[1]), failed: Number(m?.[2]), cancelled: Number(m?.[3]), summary, items };
  },

  async settingsSwitch(page, label, on) {
    await ui.nav(page, "settings", "设置");
    for (const tab of ["转换", "字幕与封面", "通用", "完成后", "高级"]) {
      await page.click({ selector: "[role=tab]", text: tab });
      await sleep(150);
      const found = await page.evaluate((l) => !!window.__e2e.find({ selector: "[role=switch]", label: l, within: "main" }), label);
      if (found) {
        await ui.setSwitch(page, label, on, "main");
        return tab;
      }
    }
    throw new Error(`设置中找不到开关：${label}`);
  },
};

// ------------------------------------------------------------------ 输出检查

function videoOutputs(dir) {
  return listFiles(dir).filter((f) => /\.(mp4|mkv|mov)$/i.test(f));
}

function partials(dir) {
  return listFiles(dir).filter((f) => f.includes(".part.") || f.includes(".cliprecipe-"));
}

// ------------------------------------------------------------------ 场景

async function bootScenarios(s, media) {
  const { page } = s;

  await scenario(
    s,
    "启动：无错误、CSP 生效、三个页面、主题切换",
    async () => {
      const info = await page.evaluate(() => ({ origin: location.origin, ua: navigator.userAgent.match(/Edg\/[\d.]+/)?.[0] }));
      // 启动阶段（重新加载前后）的控制台错误
      const bootErrors = page.console.filter((c) => c.kind === "console.error" || c.kind === "exception" || /^log\..*\.error$/.test(c.kind));
      assert(bootErrors.length === 0, `启动时的控制台错误：${bootErrors.map((c) => c.text).join(" | ")}`);
      // CSP：生产形态下 connect-src 只允许 ipc:，向别处 fetch 必须被拦截（不会真正发出请求）
      const probe = await page.evaluate(async () => {
        const before = window.__e2e.csp.length;
        let blocked = false;
        try {
          await fetch("http://127.0.0.1:9/e2e-csp-probe");
        } catch {
          blocked = true;
        }
        await new Promise((r) => setTimeout(r, 300));
        return { blocked, violations: window.__e2e.csp.slice(before) };
      });
      let cspNote;
      if (s.mode === "prod") {
        assert(info.origin === "http://tauri.localhost", `页面来源应为内嵌前端 http://tauri.localhost，实际 ${info.origin}`);
        assert(probe.violations.some((v) => v.directive.startsWith("connect-src")), `生产 CSP 未生效：${JSON.stringify(probe)}`);
        cspNote = "CSP 生效（connect-src 拦截了探测请求）";
      } else {
        cspNote = `开发形态 devCsp 为 null，不检查 CSP（探测请求${probe.violations.length ? "被拦截" : "未被 CSP 拦截"}）`;
      }
      await ui.nav(page, "toolbox", "工具箱");
      const cards = await page.evaluate(() => document.querySelectorAll("[role=button][aria-label^='打开 ']").length);
      assert(cards >= 30, `工具箱卡片只有 ${cards} 个`);
      await ui.nav(page, "settings", "设置");
      await page.waitFor("设置分组", () => document.querySelectorAll("[role=tab]").length >= 6);
      await ui.nav(page, "batch", "视频队列");
      const modules = await page.evaluate(() => document.querySelectorAll("aside [role=switch]").length);
      assert(modules === 18, `配方面板应有 18 个模块开关，实际 ${modules}`);
      const theme0 = await page.evaluate(() => document.documentElement.dataset.theme);
      await page.click({ selector: "nav button", label: theme0 === "dark" ? "切换到浅色" : "切换到深色" });
      const theme1 = await page.evaluate(() => document.documentElement.dataset.theme);
      assert(theme1 && theme1 !== theme0, `主题未切换：${theme0} → ${theme1}`);
      const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
      s.theme = theme1;
      return `${info.origin}，${info.ua ?? "WebView2"}；${cspNote}；工具卡片 ${cards} 个；主题 ${theme0} → ${theme1}（背景 ${bg}）`;
    },
    { csp: [/e2e-csp-probe/], console: [/e2e-csp-probe/, /Content Security Policy/] },
  );
  await saveShot(s, "boot");

  await scenario(s, "环境命令：app_info / ffmpeg_status / encoder_info", async () => {
    const app = await page.invoke("app_info");
    assert(app.ok && app.value.name === "ClipRecipe" && app.value.cpuCores > 0, `app_info：${JSON.stringify(app)}`);
    const ff = await page.invoke("ffmpeg_status");
    assert(ff.ok && ff.value.available, `ffmpeg_status：${JSON.stringify(ff)}`);
    const enc = await page.invoke("encoder_info", { refresh: false });
    assert(enc.ok && enc.value.h264 && enc.value.hevc, `encoder_info：${JSON.stringify(enc)}`);
    // 设置页“关于 · 运行环境”也能显示出来（同样的命令经界面调用）
    await ui.nav(page, "settings", "设置");
    await page.click({ selector: "[role=tab]", text: "关于" });
    await page.waitFor("运行环境显示 FFmpeg 版本", (v) => document.querySelector("main").innerText.includes(v), [ff.value.version.split(" ")[0]], { timeout: 60000 });
    await page.waitFor("硬件编码器检测完成", () => document.querySelector("main").innerText.includes("开启 GPU 加速时使用"), [], { timeout: 120000 });
    await ui.nav(page, "batch", "视频队列");
    return `v${app.value.version}，${app.value.cpuCores} 核；${ff.value.version}；硬件：${enc.value.hardware.join("、") || "无"}，H.264 → ${enc.value.h264}`;
  });

  await scenario(s, "导入：拖放 + 添加视频按钮 → 行显示媒体信息；拖到输出位置设为输出目录", async () => {
    await ui.clearQueue(page);
    await ui.dropOnQueue(page, [media.a, media.b]);
    await ui.waitRows(page, "两个视频导入", (r) => r.length === 2 && r.every((x) => x.summary.includes("640×360")));
    // “添加视频”按钮：系统对话框由探针返回预先排队的路径
    await page.queueDialog([media.portrait]);
    await page.click({ selector: "main header button", textIncludes: "添加视频" });
    const rows = await ui.waitRows(page, "竖屏视频导入", (r) => r.length === 3 && r.some((x) => x.name === "portrait.mp4" && x.summary.includes("360×640")));
    // 拖一个文件到“输出到”输入框：设为该文件所在的文件夹
    await page.dropFiles([media.extra], { selector: "input[aria-label=输出文件夹]", within: "main" });
    const extraDir = path.dirname(media.extra);
    await page.waitFor("拖放设置输出目录", (d) => document.querySelector("main input[aria-label=输出文件夹]")?.value === d, [extraDir]);
    assert((await ui.rows(page)).length === 3, "拖到输出位置的文件不应导入队列");
    const ipc = await page.evaluate(() => window.__e2e.ipc.filter((r) => r.cmd === "import_paths").map((r) => r.args));
    assert(ipc.length === 2 && ipc.every((a) => Array.isArray(a.paths) && typeof a.recursive === "boolean"), `import_paths 参数：${JSON.stringify(ipc)}`);
    return rows.map((r) => `${r.name}: ${r.summary}`).join("；");
  });
}

async function batchScenarios(s, media) {
  const { page } = s;
  const full = s.mode === "prod";

  await scenario(s, full ? "批量处理：裁剪(百分比) + 文本 + 帧率 + 码率" : "批量处理：帧率（默认配方）", async () => {
    const out = w("out", s.mode, "batch");
    await ui.setOutputDir(page, out);
    assert((await ui.switchState(page, "帧率")) === "checked", "默认配方应启用帧率");
    if (full) {
      await ui.setSwitch(page, "裁剪", true);
      await ui.expandModule(page, "裁剪");
      await ui.choose(page, "百分比");
      await ui.setSwitch(page, "文本", true);
      await ui.setSwitch(page, "码率", true);
      await page.waitFor("已启用 4 个步骤", () => document.querySelector("aside header").innerText.includes("已启用 4 /"));
    }
    const logBefore = (await ui.logText(page)).length;
    await ui.startBatch(page);
    const summary = await ui.waitBatchFinished(page);
    const rows = await ui.rows(page);
    assert(rows.every((r) => r.state.startsWith("完成")), `有行未完成：${JSON.stringify(rows.map((r) => [r.name, r.state, r.text]))}`);
    assert(/完成 3 · 失败 0 · 取消 0/.test(summary), `运行汇总：${summary}`);
    const files = videoOutputs(out);
    assert(files.length === 3, `输出文件数 ${files.length}：${files}`);
    assert(partials(out).length === 0, `留下了临时文件：${partials(out)}`);
    const probes = {};
    for (const f of files) probes[f] = await probe(path.join(out, f));
    const expect = full ? { landscape: [576, 324], portrait: [324, 576] } : { landscape: [640, 360], portrait: [360, 640] };
    for (const [f, p] of Object.entries(probes)) {
      const [ew, eh] = f.startsWith("portrait") ? expect.portrait : expect.landscape;
      assert(p.width === ew && p.height === eh, `${f} 尺寸 ${p.width}×${p.height}，应为 ${ew}×${eh}`);
      assert(Math.abs(p.fps - 30) < 0.1, `${f} 帧率 ${p.fps}`);
      assert(p.acodec, `${f} 没有音频`);
      assert(Math.abs(p.duration - (f.startsWith("portrait") ? 3 : 4)) < 0.3, `${f} 时长 ${p.duration}`);
    }
    const logText = await ui.logText(page);
    assert(logText.length > logBefore && /条/.test(logText), "处理记录没有新增");
    const jobs = await page.evaluate(() => window.__e2e.ipc.filter((r) => r.cmd === "start_jobs").pop());
    assert(jobs?.ok && jobs.args.request.tasks.length === 3, `start_jobs：${JSON.stringify(jobs).slice(0, 300)}`);
    const recordCount = /处理记录\s*(\d+) 条/.exec(logText)?.[1];
    return `${summary}；${Object.entries(probes).map(([f, p]) => `${f} ${p.width}×${p.height}@${p.fps.toFixed(0)} ${p.vcodec}/${p.acodec} ${p.duration.toFixed(2)}s`).join("；")}；处理记录 ${recordCount} 条`;
  });
  await saveShot(s, "batch-done");

  await scenario(s, "预览：生成预览（整条流水线）在 <video> 中播放", async () => {
    await page.click({ selector: "aside footer button", textIncludes: "生成预览" });
    const r = await ui.checkVideo(page, "流水线预览");
    const info = await page.evaluate(() => [...document.querySelectorAll("[role=dialog] p")].map((p) => p.textContent).join(" ").slice(0, 160));
    await saveShot(s, "preview-pipeline");
    // 改片段范围（从 1 秒起 2 秒）后重新生成：旧文件释放、新文件同样能播放
    const firstSrc = await page.evaluate(() => document.querySelector("[role=dialog] video")?.src);
    await page.fill({ selector: "[role=dialog] input", label: "起点（秒）" }, 1);
    await page.fill({ selector: "[role=dialog] input", label: "时长（秒）" }, 2);
    await page.click({ selector: "[role=dialog] button", text: "应用" });
    await page.waitFor("新的预览片段", (old) => {
      const v = document.querySelector("[role=dialog] video");
      return v && v.src !== old && v.readyState >= 1 && Math.abs(v.duration - 2) < 0.3;
    }, [firstSrc], { timeout: 60000 });
    const r2 = await ui.checkVideo(page, "调整范围后的预览");
    await ui.closeDialog(page);
    return `${r}；${info}；调整范围后 ${r2}`;
  });

  await scenario(s, "预览：双击队列行播放源视频（allow_media_file + asset 协议）", async () => {
    await page.click({ selector: "li[data-row-index] p[title]", nth: 0 }, { count: 2 });
    const r = await ui.checkVideo(page, "源视频播放", 30000);
    const allow = await page.evaluate(() => window.__e2e.ipc.filter((x) => x.cmd === "allow_media_file").pop());
    assert(allow?.ok, `allow_media_file：${JSON.stringify(allow)}`);
    await ui.closeDialog(page);
    return r;
  });

  if (!full) return;

  await scenario(s, "预览：单模块预览（裁剪 · 预览此步骤）", async () => {
    await ui.expandModule(page, "裁剪");
    await page.click({ selector: "aside button", textIncludes: "预览此步骤" });
    const r = await ui.checkVideo(page, "单模块预览");
    const steps = await page.evaluate(() => [...document.querySelectorAll("[role=dialog] p")].map((p) => p.textContent).find((t) => t.includes("参与预览的步骤")));
    assert(steps?.includes("裁剪") && !steps.includes("文本"), `参与预览的步骤：${steps}`);
    await ui.closeDialog(page);
    return `${r}；${steps}`;
  });

  await scenario(s, "模块表单：加水印选图片（素材文件对话框）+ 单步预览；文本字体列表（list_system_fonts）", async () => {
    await ui.setSwitch(page, "加水印", true);
    await ui.expandModule(page, "加水印");
    await page.queueDialog(media.watermark);
    await page.click({ selector: "aside [data-slot=input-group]:has(input[aria-label=水印文件路径]) button", label: "浏览" });
    await page.waitFor("水印路径", (p) => document.querySelector("aside input[aria-label=水印文件路径]")?.value === p, [media.watermark]);
    await page.click({ selector: "aside button", textIncludes: "预览此步骤" });
    const r = await ui.checkVideo(page, "加水印预览");
    const steps = await page.evaluate(() => [...document.querySelectorAll("[role=dialog] p")].map((p) => p.textContent).find((t) => t.includes("参与预览的步骤")));
    assert(steps?.includes("加水印"), `参与预览的步骤：${steps}`);
    // 预览画面与源画面不同（水印确实画上了）
    // （预览片段从“起点”秒开始，与源视频同一时刻比较；按 8×8 块取平均差，水印所在的块差别很大，编码噪声很小）
    const { previewPath, start } = await page.evaluate(() => ({
      previewPath: decodeURIComponent(new URL(document.querySelector("[role=dialog] video").src).pathname.slice(1)),
      start: Number(document.querySelector("[role=dialog] input[aria-label='起点（秒）']")?.value ?? 0),
    }));
    const [a, b] = [await grayFrame(previewPath, 0.5, 160, 88), await grayFrame(media.a, start + 0.5, 160, 88)];
    let diff = 0;
    for (let by = 0; by < 88; by += 8) {
      for (let bx = 0; bx < 160; bx += 8) {
        let sum = 0;
        for (let y = by; y < by + 8; y++) for (let x = bx; x < bx + 8; x++) sum += Math.abs(a.data[y * 160 + x] - b.data[y * 160 + x]);
        diff = Math.max(diff, Math.round(sum / 64));
      }
    }
    assert(diff > 30, `水印预览与源画面几乎相同（块平均差最大 ${diff}）`);
    await ui.closeDialog(page);
    await ui.setSwitch(page, "加水印", false);
    await ui.expandModule(page, "文本");
    await page.click({ selector: "aside button", label: "字体" });
    const fonts = await page.waitFor("字体列表", () => {
      const n = document.querySelectorAll("[role=option]").length;
      return n > 2 ? n : null;
    });
    await page.escape();
    await page.waitFor("字体列表关闭", () => document.querySelectorAll("[role=option]").length === 0);
    return `${r}；${steps}；最大像素差 ${diff}；系统字体 ${fonts - 2} 个`;
  });

  await scenario(s, "区域框选：对话框显示取到的画面（extract_frame + asset 协议）", async () => {
    await ui.expandModule(page, "裁剪");
    await ui.choose(page, "指定四边");
    await page.click({ selector: "aside button", textIncludes: "框选保留区域" });
    const img = await page.waitFor(
      "画面图片加载",
      () => {
        const i = document.querySelector("[role=dialog] img[alt=视频画面]");
        return i && i.complete && i.naturalWidth > 0 ? { w: i.naturalWidth, h: i.naturalHeight, src: i.src } : null;
      },
      [],
      { timeout: 30000 },
    );
    const size = await page.evaluate(() => document.querySelector("[role=dialog]").innerText.match(/源尺寸 \S+/)?.[0]);
    await saveShot(s, "region-picker");
    await page.click({ selector: "[role=dialog] button", text: "取消" });
    await page.waitFor("框选对话框关闭", () => !document.querySelector("[role=dialog] img[alt=视频画面]"));
    await ui.choose(page, "百分比");
    return `图片 ${img.w}×${img.h}（${img.src.slice(0, 50)}…）；${size}`;
  });

  await scenario(s, "队列交互：媒体信息、查看命令、右键菜单打开工具（范围为选中）、工具栏工具菜单", async () => {
    // 媒体信息对话框
    await page.click({ selector: "li[data-row-index] button", label: "媒体信息", nth: 0 });
    const infoText = await page.waitFor("媒体信息对话框", () => {
      const d = document.querySelector("[role=dialog]");
      return d && /h264/.test(d.innerText) && /aac/.test(d.innerText) ? d.innerText.replace(/\s+/g, " ").slice(0, 120) : null;
    });
    await ui.closeDialog(page);
    // 查看命令：preview_command → 处理记录里出现 FFmpeg 命令行
    await page.click({ selector: "aside header button", textIncludes: "查看命令" });
    await page.waitFor("处理记录中的命令", () => [...document.querySelectorAll("main section")].some((sec) => sec.innerText.includes("的处理命令")));
    // 右键第二行 → 独立工具 → 视频倒放：对话框打开，范围为选中的 1 个
    await page.rightClick({ selector: "li[data-row-index] p[title]", nth: 1 });
    await page.click({ selector: "[role=menuitem]", textIncludes: "独立工具" });
    await page.click({ selector: "[role=menu] [role=menuitem]", text: "视频倒放" });
    await page.waitFor("视频倒放对话框", () => document.querySelector("[role=dialog] h2")?.textContent.includes("视频倒放"));
    const scope = await page.evaluate(() => window.__e2e.find({ selector: "[role=radio]", textStarts: "队列中选中的", within: "[role=dialog]" })?.getAttribute("data-state") + " " + window.__e2e.find({ selector: "[role=radio]", textStarts: "队列中选中的", within: "[role=dialog]" })?.textContent);
    assert(scope.startsWith("on 队列中选中的 1 个"), `处理范围：${scope}`);
    await ui.closeDialog(page);
    // 工具栏“工具”菜单：选中全部 → 生成 GIF，范围为选中的 3 个
    await page.click({ selector: "main button", text: "全选" });
    await page.click({ selector: "main button", text: "工具" });
    await page.click({ selector: "[role=menuitem]", text: "生成 GIF" });
    await page.waitFor("生成 GIF 对话框", () => document.querySelector("[role=dialog] h2")?.textContent.includes("生成 GIF"));
    const scope2 = await page.evaluate(() => window.__e2e.find({ selector: "[role=radio][data-state=on]", within: "[role=dialog]" })?.textContent);
    assert(scope2 === "队列中选中的 3 个", `处理范围：${scope2}`);
    await ui.closeDialog(page);
    // 拖动把手排序（指针事件，系统拖放开启时 HTML5 拖放不可用）：第 1 行拖到第 3 行下方
    const names = async () => (await ui.rows(page)).map((r) => r.name);
    const before = await names();
    await page.drag({ selector: "li[data-row-index] span[title=拖动调整顺序]", nth: 0 }, { selector: "li[data-row-index]", nth: 2 }, 12);
    const after = await names();
    assert(JSON.stringify(after) === JSON.stringify([before[1], before[2], before[0]]), `拖动排序：${before} → ${after}`);
    // 排序菜单：名称 Z→A
    await page.click({ selector: "main button", text: "排序" });
    await page.click({ selector: "[role=menuitem]", text: "名称 Z→A" });
    const sorted = await names();
    const expected = [...before].sort((a, b) => new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" }).compare(b, a));
    assert(JSON.stringify(sorted) === JSON.stringify(expected), `排序：${sorted}，应为 ${expected}`);
    return `媒体信息：${infoText}…；命令已写入处理记录；右键菜单与工具栏菜单都打开了工具对话框；拖动排序 ${before.join(",")} → ${after.join(",")}；名称 Z→A：${sorted.join(",")}`;
  });

  await scenario(s, "提示音：自定义 wav（allow_sound_file + asset 协议播放）", async () => {
    const wav = w("media", "silence.wav");
    const { ffmpeg: bin } = ffmpegPaths();
    execFileSync(bin, ["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "anullsrc=r=22050:cl=mono", "-t", "0.3", wav]);
    await ui.settingsSwitch(page, "完成提示音", true);
    await page.queueDialog(wav);
    await page.click({ selector: "main button", label: "选择 wav 文件" });
    await page.waitFor("提示音路径", (p) => document.querySelector("main input[aria-label=提示音文件]")?.value === p, [wav]);
    // 记录 <audio> 播放结果
    await page.evaluate(() => {
      window.__e2e.audio = [];
      const play = HTMLMediaElement.prototype.play;
      HTMLMediaElement.prototype.play = function () {
        const p = play.call(this);
        p.then(() => window.__e2e.audio.push({ ok: true, src: this.src }), (e) => window.__e2e.audio.push({ ok: false, src: this.src, error: String(e) }));
        return p;
      };
    });
    await page.click({ selector: "main button", text: "试听" });
    const audio = await page.waitFor("提示音播放", () => window.__e2e.audio[0], [], { timeout: 10000 });
    assert(audio.ok && audio.src.startsWith("http://asset.localhost/"), `提示音：${JSON.stringify(audio)}`);
    await ui.settingsSwitch(page, "完成提示音", false);
    await ui.nav(page, "batch", "视频队列");
    return `播放 ${audio.src.slice(0, 60)}…`;
  });

  await scenario(s, "裂变 2 次：每个视频两个输出", async () => {
    await ui.clearQueue(page);
    await ui.dropOnQueue(page, [media.a]);
    await ui.waitRows(page, "导入一个视频", (r) => r.length === 1 && r[0].summary.includes("640×360"));
    const out = w("out", s.mode, "fission");
    await ui.setOutputDir(page, out);
    await page.fill({ selector: "aside footer input", label: "裂变次数" }, 2);
    await page.waitFor("裂变说明", () => document.querySelector("aside footer").innerText.includes("每个视频输出 2 个"));
    await ui.startBatch(page);
    const summary = await ui.waitBatchFinished(page);
    const rows = await ui.rows(page);
    const files = videoOutputs(out);
    assert(files.length === 2, `输出 ${files.length} 个：${files}`);
    assert(rows[0].state.includes("2 个"), `行状态：${rows[0].state}`);
    for (const f of files) {
      const p = await probe(path.join(out, f));
      assert(p.width === 576 && p.height === 324, `${f} ${p.width}×${p.height}`);
    }
    await page.fill({ selector: "aside footer input", label: "裂变次数" }, 1);
    return `${summary}；${files.join("、")}`;
  });

  await scenario(s, "配方：导出 → 重置 → 导入（按钮与拖放）", async () => {
    const file = w("recipe-export.json");
    await page.queueDialog(file);
    await page.click({ selector: "aside header button", textIncludes: "导出" });
    await page.waitFor("配方文件写出", () => true, [], { timeout: 1000 });
    const deadline = Date.now() + 10000;
    while (!fs.existsSync(file) && Date.now() < deadline) await sleep(200);
    assert(fs.existsSync(file), "导出的配方文件不存在");
    const json = JSON.parse(fs.readFileSync(file, "utf8"));
    assert(json.modules?.["pipe.crop"]?.enabled && json.modules["pipe.crop"].mode === "percent", `导出内容：${JSON.stringify(json.modules?.["pipe.crop"])}`);
    assert(json.modules?.["pipe.text"]?.enabled && json.modules?.["pipe.bitrate"]?.enabled, "导出的配方缺少启用的文本 / 码率");

    const reset = async () => {
      await page.click({ selector: "aside header button", textIncludes: "重置" });
      await page.click({ selector: "[role=alertdialog] button", text: "确定" });
      await page.waitFor("重置后裁剪关闭", () => window.__e2e.find({ selector: "[role=switch]", label: "裁剪", within: "aside" })?.getAttribute("data-state") === "unchecked");
      assert((await ui.switchState(page, "文本")) === "unchecked" && (await ui.switchState(page, "帧率")) === "checked", "重置后应只启用帧率");
    };
    const restored = async (how) => {
      await page.waitFor(`${how}后裁剪开启`, () => window.__e2e.find({ selector: "[role=switch]", label: "裁剪", within: "aside" })?.getAttribute("data-state") === "checked");
      assert((await ui.switchState(page, "文本")) === "checked" && (await ui.switchState(page, "码率")) === "checked", `${how}后文本 / 码率未恢复`);
      await ui.expandModule(page, "裁剪");
      const mode = await page.evaluate(() => window.__e2e.find({ selector: "[role=radio]", text: "百分比", within: "aside" })?.getAttribute("data-state"));
      assert(mode === "on", `${how}后裁剪方式不是百分比`);
    };
    await reset();
    await page.queueDialog(file);
    await page.click({ selector: "aside header button", textIncludes: "导入" });
    await restored("按钮导入");
    await reset();
    await ui.dropOnQueue(page, [file]);
    await restored("拖放导入");
    const rows = await ui.rows(page);
    assert(rows.length === 1, "拖入配方文件不应导入队列");
    return `导出 ${path.basename(file)}（${Object.values(json.modules).filter((m) => m.enabled).length} 个步骤启用）；按钮导入、拖放导入都恢复了参数`;
  });

  await scenario(s, "工具：无损转换（remux）处理队列视频", async () => {
    await ui.nav(page, "batch", "视频队列");
    await ui.dropOnQueue(page, [media.b]);
    await ui.waitRows(page, "两个视频", (r) => r.length === 2);
    await ui.openTool(page, "无损转换");
    await ui.choose(page, "队列中全部 2 个", "[role=dialog]");
    const out = w("out", s.mode, "tool-remux");
    await ui.setOutputDir(page, out, "[role=dialog]");
    const r = await ui.runTool(page);
    assert(r.done === 2 && r.failed === 0, `${r.summary}：${r.items.join(" | ")}`);
    const files = videoOutputs(out);
    assert(files.length === 2, `输出：${files}`);
    for (const f of files) {
      const p = await probe(path.join(out, f));
      assert(p.width === 640 && p.vcodec === "h264" && p.acodec, `${f}：${JSON.stringify(p)}`);
    }
    await ui.closeDialog(page);
    return `${r.summary}；${files.join("、")}`;
  });

  await scenario(s, "工具：进度条（重编码）运行 + 工具预览", async () => {
    await ui.openTool(page, "进度条");
    await ui.choose(page, "队列中全部 2 个", "[role=dialog]");
    const out = w("out", s.mode, "tool-progress");
    await ui.setOutputDir(page, out, "[role=dialog]");
    const r = await ui.runTool(page);
    assert(r.done === 2 && r.failed === 0, `${r.summary}：${r.items.join(" | ")}`);
    const files = videoOutputs(out);
    assert(files.length === 2, `输出：${files}`);
    const p = await probe(path.join(out, files[0]));
    assert(p.width === 640 && p.height === 360 && Math.abs(p.duration - 4) < 0.3, `${files[0]}：${JSON.stringify(p)}`);
    // 工具预览（preview: true）：在预览窗口播放
    await page.click({ selector: "[role=dialog] button", text: "预览" });
    const v = await ui.checkVideo(page, "工具预览");
    await saveShot(s, "tool-preview");
    await ui.closeDialog(page); // 预览窗口
    await ui.closeDialog(page); // 工具对话框
    return `${r.summary}；${files.join("、")}；预览 ${v}`;
  });

  await scenario(s, "工具：图转视频（文件夹输入）", async () => {
    await ui.openTool(page, "图转视频");
    // 拖一个图片文件到文件夹输入框：取其所在文件夹（对话框打开时不导入队列）
    const rowsBefore = (await ui.rows(page)).length;
    await page.dropFiles([path.join(media.imageDir, "one.png")], { selector: "[role=dialog] input[aria-label=图片文件夹]" });
    await page.waitFor("拖放设置图片文件夹", (d) => document.querySelector("[role=dialog] input[aria-label=图片文件夹]")?.value === d, [media.imageDir]);
    assert((await ui.rows(page)).length === rowsBefore, "工具对话框打开时拖入的文件不应导入队列");
    await page.click({ selector: "[role=dialog] button", label: "清除图片文件夹" });
    await page.waitFor("清除图片文件夹", () => document.querySelector("[role=dialog] input[aria-label=图片文件夹]")?.value === "");
    await page.queueDialog(media.imageDir);
    await page.click({ selector: "[role=dialog] button", textIncludes: "图片文件夹" });
    await page.waitFor("图片文件夹已设置", (d) => document.querySelector("[role=dialog] input[aria-label=图片文件夹]")?.value === d, [media.imageDir]);
    const out = w("out", s.mode, "tool-images");
    await ui.setOutputDir(page, out, "[role=dialog]");
    const r = await ui.runTool(page);
    assert(r.done >= 1 && r.failed === 0, `${r.summary}：${r.items.join(" | ")}`);
    const files = videoOutputs(out);
    assert(files.length === 2, `输出：${files}`);
    const p = await probe(path.join(out, files[0]));
    assert(p.width > 0 && p.duration > 0.5, `${files[0]}：${JSON.stringify(p)}`);
    const start = await page.evaluate(() => window.__e2e.ipc.filter((x) => x.cmd === "start_tool").pop()?.args?.request);
    assert(start?.folders?.images === media.imageDir && start.videos.length === 0, `start_tool 参数：${JSON.stringify(start).slice(0, 300)}`);
    await ui.closeDialog(page);
    await ui.nav(page, "batch", "视频队列");
    return `${r.summary}；${files.join("、")}（${p.width}×${p.height}，${p.duration.toFixed(1)}s）`;
  });

  await scenario(s, "设置 + 同名字幕：自动加载显示 CC，烧录的字幕改变画面", async () => {
    await ui.settingsSwitch(page, "自动加载同名字幕", true);
    await ui.settingsSwitch(page, "文件名加队列序号", true);
    await ui.settingsSwitch(page, "静音预览", true);
    await ui.nav(page, "batch", "视频队列");
    // 只留帧率，避免文本模块也画在底部
    await page.click({ selector: "aside header button", textIncludes: "重置" });
    await page.click({ selector: "[role=alertdialog] button", text: "确定" });
    await ui.clearQueue(page);
    await ui.dropOnQueue(page, [media.subbed]);
    const rows = await ui.waitRows(page, "带字幕的视频导入", (r) => r.length === 1 && r[0].badges.some((b) => b.text === "CC" && b.title?.startsWith("烧录字幕")));
    const out = w("out", s.mode, "subtitle");
    await ui.setOutputDir(page, out);
    await ui.startBatch(page);
    const summary = await ui.waitBatchFinished(page);
    const files = videoOutputs(out);
    // 序号按队列长度补零（1 个任务时为 “1_”）
    assert(files.length === 1 && /^0*1_subbed\.mp4$/.test(files[0]), `输出（应带队列序号）：${files}`);
    // 原尺寸灰度帧：源是纯色（标准差≈0），烧录的字幕（白字黑边，底部）让底部一带的标准差明显变大
    const frame = await grayFrame(path.join(out, files[0]), 1.5, 640, 360);
    const src = await grayFrame(media.subbed, 1.5, 640, 360);
    const bottom = bandStddev(frame, 290, 356);
    const top = bandStddev(frame, 10, 120);
    const srcBottom = bandStddev(src, 290, 356);
    const framePng = path.join(REPORT, `${s.mode}-subtitle-output-frame.png`);
    execFileSync(ffmpegPaths().ffmpeg, ["-hide_banner", "-loglevel", "error", "-y", "-ss", "1.5", "-i", path.join(out, files[0]), "-frames:v", "1", framePng]);
    s.shots = [...(s.shots ?? []), framePng];
    assert(bottom > 12 && top < 4 && srcBottom < 3, `底部标准差 ${bottom.toFixed(1)}（源 ${srcBottom.toFixed(1)}），顶部 ${top.toFixed(1)}：字幕没有烧录进画面`);
    await saveShot(s, "subtitle-cc");
    return `${rows[0].badges.map((b) => b.title).join("、")}；${summary}；${files[0]}；底部标准差 ${bottom.toFixed(1)}（源 ${srcBottom.toFixed(1)}），顶部 ${top.toFixed(1)}`;
  });

  await scenario(s, "添加文件夹：12 个视频（文件夹对话框 + import-progress 事件）", async () => {
    await ui.clearQueue(page);
    const dir = w("media", "folder12");
    fs.mkdirSync(path.join(dir, "sub"), { recursive: true });
    for (let i = 1; i <= 12; i++) fs.copyFileSync(media.extra, path.join(dir, i > 10 ? "sub" : "", `v${String(i).padStart(2, "0")}.mp4`));
    fs.writeFileSync(path.join(dir, "notes.txt"), "not a video");
    // 订阅 import-progress（与前端 listen() 相同的 IPC）
    await page.evaluate(async () => {
      window.__e2e.importProgress = [];
      const handler = window.__TAURI_INTERNALS__.transformCallback((e) => window.__e2e.importProgress.push(e.payload));
      await window.__TAURI_INTERNALS__.invoke("plugin:event|listen", { event: "import-progress", target: { kind: "Any" }, handler });
    });
    await page.queueDialog(dir);
    await page.click({ selector: "main header button", textIncludes: "添加文件夹" });
    const rows = await ui.waitRows(page, "12 个视频导入", (r) => r.length === 12 && r.every((x) => x.summary.includes("320×240")));
    const progress = await page.evaluate(() => window.__e2e.importProgress);
    assert(progress.length > 0 && progress.at(-1).done === 12 && progress.at(-1).total === 12, `import-progress：${JSON.stringify(progress)}`);
    const logText = await ui.logText(page);
    assert(logText.includes("导入 12 个视频"), "处理记录没有导入汇总");
    await ui.clearQueue(page);
    return `${rows.length} 行（含子目录 2 个，notes.txt 被忽略）；import-progress ${progress.length} 次，最后 ${JSON.stringify(progress.at(-1))}`;
  });

  await scenario(s, "取消：处理中停止 → 行为已取消，没有残留文件", async () => {
    await ui.clearQueue(page);
    await ui.dropOnQueue(page, [media.long]);
    await ui.waitRows(page, "长视频导入", (r) => r.length === 1 && r[0].summary.includes("1920×1080"));
    const out = w("out", s.mode, "cancel");
    await ui.setOutputDir(page, out);
    await ui.startBatch(page);
    await ui.waitRows(page, "开始处理长视频", (r) => /处理中 [1-9]/.test(r[0].state) || r[0].state.startsWith("完成"), 60000);
    const rowsMid = await ui.rows(page);
    assert(!rowsMid[0].state.startsWith("完成"), "长视频在停止前就处理完了（测试素材太短）");
    const partialSeen = partials(out);
    await page.click({ selector: "main footer button", text: "停止" });
    const summary = await ui.waitBatchFinished(page, 30000);
    const rows = await ui.rows(page);
    assert(rows[0].state.startsWith("已取消"), `行状态：${rows[0].state}`);
    assert(/取消 1/.test(summary), `运行汇总：${summary}`);
    await sleep(1000);
    const left = listFiles(out);
    assert(left.length === 0, `取消后输出目录残留：${left}`);
    return `停止时 ${rowsMid[0].state}（临时文件 ${partialSeen.join("、") || "无"}）；${summary}；输出目录为空`;
  });

  await scenario(s, "取消：工具运行中停止；预览生成中关闭（cancel_preview）", async () => {
    // 从队列工具栏的“工具”菜单打开（批量页面保持显示，可以看到行上的工具进度）
    await page.click({ selector: "main button", text: "工具" });
    await page.click({ selector: "[role=menuitem]", text: "进度条" });
    await page.waitFor("进度条对话框", () => document.querySelector("[role=dialog] h2")?.textContent.includes("进度条"));
    await ui.choose(page, "队列中全部 1 个", "[role=dialog]");
    const out = w("out", s.mode, "tool-cancel");
    await ui.setOutputDir(page, out, "[role=dialog]");
    await page.click({ selector: "[role=dialog] button", text: "开始" });
    await page.waitFor("工具开始处理", () => /处理中|\d+%/.test(document.querySelector("[role=dialog] ul li")?.innerText ?? ""), [], { timeout: 60000 });
    const mid = await page.evaluate(() => document.querySelector("[role=dialog] ul li")?.innerText.replace(/\s+/g, " "));
    // 队列行上同时显示工具进度
    const rowTool = await page.waitFor("队列行显示工具进度", () => document.querySelector("li[data-row-index]")?.innerText.match(/进度条\s*\S*/)?.[0]);
    await page.click({ selector: "[role=dialog] button", text: "停止" });
    const summary = await page.waitFor("工具停止", () => [...document.querySelectorAll("[role=dialog] span")].map((x) => x.textContent.trim()).find((t) => t.startsWith("上次：")), [], { timeout: 30000 });
    assert(/取消 1/.test(summary), `工具汇总：${summary}`);
    await sleep(1000);
    assert(listFiles(out).length === 0, `工具取消后输出目录残留：${listFiles(out)}`);
    await ui.closeDialog(page);
    // 预览：开始生成后马上关闭窗口
    await ui.nav(page, "batch", "视频队列");
    await page.click({ selector: "aside footer button", textIncludes: "生成预览" });
    await page.waitFor("预览窗口", () => !!document.querySelector("[role=dialog]"));
    await ui.closeDialog(page);
    await sleep(500);
    const cancel = await page.evaluate(() => window.__e2e.ipc.filter((r) => r.cmd === "cancel_preview").pop());
    const rowAfter = await page.evaluate(() => document.querySelector("li[data-row-index]")?.innerText.replace(/\s+/g, " "));
    assert(!/失败|错误/.test(rowAfter), `关闭预览后队列行：${rowAfter}`);
    return `停止时 ${mid}；队列行显示“${rowTool}”；${summary}；关闭预览：cancel_preview ${cancel ? (cancel.ok ? "成功" : `失败 ${cancel.error}`) : "未调用（预览已完成）"}`;
  }, { ipc: ["render_preview"] });

  await scenario(
    s,
    "系统命令接线（无副作用）：open_project_page / open_folder 拒绝非法参数",
    async () => {
      const page2 = await page.invoke("open_project_page", { url: "https://example.invalid/" });
      assert(!page2.ok && page2.error.includes("只能打开本项目的页面"), `open_project_page：${JSON.stringify(page2)}`);
      const folder = await page.invoke("open_folder", { path: w("definitely-missing", "x") });
      assert(!folder.ok && folder.error.includes("文件夹不存在"), `open_folder：${JSON.stringify(folder)}`);
      return "两个命令都已注册、参数名正确，并拒绝了非法参数（没有打开任何窗口）；check_update 会联网、schedule_shutdown 会关机，未调用";
    },
    { ipc: ["open_project_page", "open_folder"] },
  );
}

async function persistenceScenarios(s, media) {
  const { page } = s;
  // 准备：队列里留一个已取消（上一场景）+ 一个未处理的视频
  await scenario(s, "重启前：队列与设置已保存到隔离的配置目录", async () => {
    await ui.dropOnQueue(page, [media.extra]);
    await ui.waitRows(page, "extra 导入", (r) => r.some((x) => x.name === "extra.mp4"));
    await sleep(1800); // 队列保存防抖 800ms、设置 300ms
    const queue = JSON.parse(fs.readFileSync(path.join(s.configDir, "queue.json"), "utf8"));
    const settings = JSON.parse(fs.readFileSync(path.join(s.configDir, "settings.json"), "utf8"));
    const queued = (Array.isArray(queue) ? queue : queue.tasks ?? []).map((t) => path.basename(t.path));
    assert(queued.includes("extra.mp4"), `queue.json：${JSON.stringify(queue).slice(0, 300)}`);
    assert(settings.numberPrefix === true && settings.mutePreview === true && settings.autoSubtitle === true, `settings.json：${JSON.stringify(settings).slice(0, 300)}`);
    s.expectQueue = queued;
    return `queue.json：${queued.join("、")}；settings.json：numberPrefix / mutePreview / autoSubtitle = true，theme = ${settings.theme}`;
  });

  log("  重启应用（quit_app）…");
  const oldPids = [...s.appPids];
  await s.stop({ graceful: true });
  // 正常退出时（RunEvent::Exit）应删除本进程的预览目录
  const leftPreview = oldPids.map((pid) => path.join(os.tmpdir(), "cliprecipe-preview", String(pid))).filter((d) => fs.existsSync(d));
  s.appPids.clear();
  await s.start();
  await scenario(s, "正常退出：删除本进程的预览目录", async () => {
    assert(oldPids.length > 0, "没有记录到应用进程号");
    assert(leftPreview.length === 0, `退出后预览目录仍在：${leftPreview}`);
    return `进程 ${oldPids.join("、")} 的预览目录已删除`;
  });

  await scenario(s, "重启后：设置保留、未完成的任务恢复、配方保留", async () => {
    const loaded = await s.page.invoke("load_settings");
    assert(loaded.ok && loaded.value.numberPrefix && loaded.value.mutePreview && loaded.value.autoSubtitle, `load_settings：${JSON.stringify(loaded).slice(0, 300)}`);
    const rows = await ui.waitRows(s.page, "队列恢复", (r) => (s.expectQueue ?? []).every((n) => r.some((x) => x.name === n)), 20000);
    assert(rows.every((r) => r.summary.includes("×")), "恢复的任务缺少媒体信息");
    const theme = await s.page.evaluate(() => document.documentElement.dataset.theme);
    assert(!s.theme || theme === s.theme, `主题未保留：${theme}，应为 ${s.theme}`);
    const fps = await ui.switchState(s.page, "帧率");
    assert(fps === "checked", "配方（localStorage）未保留");
    const shown = await (async () => {
      await ui.nav(s.page, "settings", "设置");
      await s.page.click({ selector: "[role=tab]", text: "字幕与封面" });
      return ui.switchState(s.page, "自动加载同名字幕", "main");
    })();
    assert(shown === "checked", "设置页没有显示保存的开关状态");
    await ui.nav(s.page, "batch", "视频队列");
    await saveShot(s, "after-restart");
    return `恢复 ${rows.length} 个任务（${rows.map((r) => `${r.name} ${r.state}`).join("、")}）；主题 ${theme}；设置页显示已保存的开关`;
  });
}

async function saveShot(s, label) {
  if (!s.page) return;
  try {
    const file = path.join(REPORT, `${s.mode}-shot-${label}.png`);
    fs.writeFileSync(file, await s.page.screenshot());
    s.shots = [...(s.shots ?? []), file];
  } catch {
    // 截图失败不影响结果
  }
}

// ------------------------------------------------------------------ 主流程

const sessions = [];
let cleaned = false;

async function cleanup() {
  if (cleaned) return;
  cleaned = true;
  for (const s of sessions) {
    try {
      await s.stop({ graceful: false });
    } catch {
      // 尽力而为
    }
    s.cleanupPreviewDirs();
  }
  killAllChildren();
  if (opt("--keep")) {
    log(`保留临时目录：${WORK}`);
  } else if (!(await removeDir(WORK))) {
    log(`无法删除临时目录（稍后可手动删除）：${WORK}`);
  }
}

process.on("SIGINT", () => {
  log("收到 Ctrl+C，正在清理…");
  void cleanup().finally(() => process.exit(130));
});
process.on("exit", () => killAllChildren());

function build() {
  log("构建生产形态（tauri build --debug --no-bundle：内嵌前端、正式 CSP）…");
  execSync("npm run tauri -- build --debug --no-bundle", {
    cwd: REPO,
    stdio: "inherit",
    env: { ...process.env, NO_PROXY, no_proxy: NO_PROXY, CARGO_TARGET_DIR: E2E_TARGET_DIR },
  });
}

async function runMode(mode, media) {
  let exe = DEBUG_EXE; // dev：tauri dev 编译出的程序
  if (mode === "prod") {
    if (!opt("--no-build")) {
      build();
      // 立即保留一份：之后的 `tauri dev` 会把 DEBUG_EXE 覆盖成开发版
      fs.copyFileSync(DEBUG_EXE, PROD_EXE);
    }
    if (!fs.existsSync(PROD_EXE)) throw new Error(`找不到 ${PROD_EXE}，先去掉 --no-build 构建一次`);
    // 运行临时目录里的副本（运行中不锁住构建产物）
    fs.mkdirSync(w("bin"), { recursive: true });
    exe = w("bin", "cliprecipe.exe");
    fs.copyFileSync(PROD_EXE, exe);
  }
  const s = new Session(mode, exe);
  sessions.push(s);
  try {
    await s.start();
  } catch (e) {
    results.push({ mode, name: "启动应用", status: "fail", details: e.message, artifacts: await saveArtifacts(s, "start"), secs: "-" });
    log(`✘ [${mode}] 启动应用：${e.message}`);
    await s.stop({ graceful: false });
    return;
  }
  await bootScenarios(s, media);
  await batchScenarios(s, media);
  if (mode === "prod") await persistenceScenarios(s, media);
  await saveShot(s, "final");
  await s.stop({ graceful: true });
}

function printReport() {
  console.log("\n================ ClipRecipe e2e 冒烟测试 ================");
  for (const r of results) {
    const icon = r.status === "pass" ? "PASS" : r.status === "skip" ? "SKIP" : "FAIL";
    console.log(`${icon}  [${r.mode}] ${r.name}  (${r.secs}s)`);
    if (r.status !== "pass" && r.details) console.log(`        ${r.details.split("\n").join("\n        ")}`);
    for (const a of r.artifacts) console.log(`        → ${a}`);
  }
  const shots = sessions.flatMap((s) => s.shots ?? []);
  if (shots.length) console.log(`\n截图：\n  ${shots.join("\n  ")}`);
  const failed = results.filter((r) => r.status === "fail").length;
  console.log(`\n${results.length - failed}/${results.length} 通过；报告目录：${REPORT}`);
  fs.writeFileSync(path.join(REPORT, "results.json"), JSON.stringify(results, null, 2));
  return failed;
}

async function main() {
  log(`临时目录：${WORK}`);
  log(`报告目录：${REPORT}`);
  const { ffmpeg: bin } = ffmpegPaths();
  log(`生成测试素材（${bin}）…`);
  const media = await makeMedia(w("media"));
  for (const mode of MODES) {
    try {
      await runMode(mode, media);
    } catch (e) {
      results.push({ mode, name: "运行", status: "fail", details: e.stack ?? String(e), artifacts: [], secs: "-" });
    }
  }
}

let failed = 1;
try {
  await main();
} catch (e) {
  console.error(e);
} finally {
  await cleanup();
  failed = printReport();
}
process.exit(failed > 0 ? 1 : 0);
