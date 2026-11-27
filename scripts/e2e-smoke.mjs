#!/usr/bin/env node
// 端到端冒烟测试：启动真正的 ClipRecipe 桌面窗口（Tauri 2 + WebView2），经 Chrome DevTools Protocol 操作界面，
// 检查前后端命令接线、事件、生产 CSP、asset 协议播放、拖放路由、对话框流程、设置与队列持久化。
//
// 用法：
//   npm run e2e                     # 生产形态（内嵌前端、正式 CSP）：先 `tauri build --debug --no-bundle`，跑全部场景
//   npm run e2e -- --mode dev       # 开发形态（`tauri dev`：vite + 调试程序），跑基础场景
//   npm run e2e -- --mode both
//   npm run e2e:debug-panel         # 开发形态下只跑调试面板的场景（--mode dev --debug-panel；面板只在开发环境存在）
//   npm run e2e -- --no-build       # 生产形态不重新构建，直接用 src-tauri/target/debug/cliprecipe.exe
//   npm run e2e -- --keep           # 保留临时目录（素材、输出、隔离的配置）便于排查
//   npm run e2e -- --skip "审计|ffmpeg"  # 调试用：名称匹配该正则的场景记为跳过（"ffmpeg" 跳过整组 [ffmpeg] 场景）；
//                                   #   后面的场景依赖前面留下的队列与配方，跳过的场景只应是不改变这些状态的（如界面审计）
//
// 需要：Windows + WebView2；FFmpeg：`npm install` 装好的 node_modules/@seankit 包（调试构建的程序自己找到它），
// 或环境变量 CLIPRECIPE_FFMPEG 指向 ffmpeg.exe（设置了就优先使用）。
// 生产形态以 `--features e2e-hooks` 构建：最后一组场景（[ffmpeg]）在一个模拟“没有 FFmpeg”的隔离会话里，
// 通过界面从本机假服务器（scripts/e2e-ffmpeg-server.mjs）真实地下载、取消、安装、删除 FFmpeg。
// 应用窗口会出现在桌面上，测试结束（含失败、Ctrl+C）时关闭。
//
// 隔离：CLIPRECIPE_CONFIG_DIR（设置、队列）与 WEBVIEW2_USER_DATA_FOLDER（localStorage：配方、工具参数）
// 都指向临时目录，不碰用户的真实数据；启动后用 data_dir 命令确认，不一致时立即停止。
// 系统文件对话框无法自动操作：探针（scripts/e2e-cdp.mjs 的 PROBE_SCRIPT）在 fetch 层用预先排队的路径代替其结果。
// 拖放：通过 Tauri 事件 tauri://drag-enter / drag-over / drag-drop 模拟（与系统拖放进入前端的路径相同，
// 但不经过操作系统的 OLE 拖放）。
//
// 不会触发：关机 / 退出倒计时、删除源视频、移回源位置、永久删除、托盘、检查更新（联网）、打开文件夹 / 网页；
// 应用内下载 FFmpeg 只访问本机假服务器，不联网。

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, execSync, spawn } from "node:child_process";
import { Cdp, Page, sleep, waitForPage } from "./e2e-cdp.mjs";
import { startFakeFfmpegServer } from "./e2e-ffmpeg-server.mjs";
import {
  DEBUG_EXE,
  E2E_TARGET_DIR,
  PROD_EXE,
  REPO,
  bandStddev,
  captureNativeWindow,
  titlebarColor,
  ffmpegPaths,
  freePort,
  grayFrame,
  killAllChildren,
  killTree,
  listFiles,
  makeMedia,
  minimizeNativeWindow,
  moveNativeWindow,
  nativeWindow,
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
const SKIP = argv.includes("--skip") ? new RegExp(argv[argv.indexOf("--skip") + 1]) : null;
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
/** 界面审计中降级为警告的规则（scripts/e2e-audit-page.js 的 SOFT_RULES）：每条带审计位置，最后汇总实测值。 */
const auditWarnings = [];

function log(...args) {
  console.log(`[${new Date().toLocaleTimeString()}]`, ...args);
}

class Skip extends Error {}

async function saveArtifacts(session, label) {
  const files = [];
  const base = `${session.label}-${String(++shotIndex).padStart(2, "0")}-${label.replace(/[^\w一-龥-]+/g, "_")}`;
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
  // 后端只有一个预览槽：预览舞台的取帧与预览窗口的片段互相取消是设计行为（src/state/stage.ts），
  // 被取消的那次 render_preview 以“预览已取消”失败，不算问题；其它 render_preview 失败照常报告
  const ipcFailed = state.ipcFailed
    .filter((r) => !allow.ipc?.includes(r.cmd))
    .filter((r) => !(r.cmd === "render_preview" && /预览已取消/.test(r.error ?? "")));
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
  if (SKIP?.test(name)) {
    results.push({ mode: session.label, name, status: "skip", details: `--skip ${SKIP.source}`, artifacts: [], secs: "-" });
    log(`– [${session.label}] ${name}（--skip）`);
    return true;
  }
  log(`▶ [${session.label}] ${name}`);
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
  results.push({ mode: session.label, name, status, details, artifacts, secs });
  const icon = status === "pass" ? "✔" : status === "skip" ? "–" : "✘";
  log(`${icon} [${session.label}] ${name} (${secs}s)${details ? `\n    ${details.split("\n").join("\n    ")}` : ""}`);
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
      const open = await page.evaluate(() => document.querySelectorAll("[role=dialog],[role=alertdialog],[role=menu]").length);
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
  /**
   * @param {"prod"|"dev"} mode 程序形态
   * @param {string} exe
   * @param {{ label?: string, env?: Record<string, string> }} options label：报告里的名字（也区分配置目录）；env：额外的环境变量
   */
  constructor(mode, exe, { label, env } = {}) {
    this.mode = mode;
    this.label = label ?? mode;
    this.extraEnv = env ?? {};
    this.exe = exe;
    this.proc = null;
    this.page = null;
    this.cdp = null;
    this.appPids = new Set();
    this.launches = 0;
    // 每个会话一个隔离的配置目录；关闭完成提示音（测试时不出声），其余为默认设置
    this.configDir = w(`config-${this.label}`);
    fs.mkdirSync(this.configDir, { recursive: true });
    fs.writeFileSync(path.join(this.configDir, "settings.json"), JSON.stringify({ version: 1, sound: false }));
  }

  env(port) {
    // FFmpeg：设置了 CLIPRECIPE_FFMPEG 就传给程序；否则调试构建的程序自己找到 node_modules 里的 @seankit 包。
    // 隔离会话（CLIPRECIPE_E2E_ISOLATE_FFMPEG）模拟没有 FFmpeg 的电脑，不传。
    const env = { ...process.env };
    delete env.CLIPRECIPE_FFMPEG;
    if (process.env.CLIPRECIPE_FFMPEG && !this.extraEnv.CLIPRECIPE_E2E_ISOLATE_FFMPEG) env.CLIPRECIPE_FFMPEG = process.env.CLIPRECIPE_FFMPEG;
    return {
      ...env,
      ...this.extraEnv,
      CLIPRECIPE_CONFIG_DIR: this.configDir,
      WEBVIEW2_USER_DATA_FOLDER: w(`webview-${this.label}`),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
      NO_PROXY,
      no_proxy: NO_PROXY,
      // tauri dev 也编译到独立目录，不与 cargo test 争用 target/debug（生产形态的程序已另存为 PROD_EXE）
      CARGO_TARGET_DIR: E2E_TARGET_DIR,
    };
  }

  async start() {
    const port = await freePort();
    const logFile = path.join(REPORT, `${this.label}-app-${++this.launches}.log`);
    if (this.mode === "prod") {
      this.proc = startProc("app", this.exe, [], this.env(port), logFile);
    } else {
      if (await portInUse(1420)) throw new Error("端口 1420 已被占用（Vite 需要它），请先结束占用它的进程");
      this.proc = startProc("tauri-dev", "npm run tauri dev", null, this.env(port), logFile);
    }
    log(`  启动 ${this.label}（调试端口 ${port}，日志 ${logFile}）`);
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

  /** 应用主进程的进程号。 */
  appPid() {
    return processesOf(this.exe)[0] ?? [...this.appPids][0] ?? null;
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

/** 批量处理页左栏（视频队列）、中栏预览舞台、顶栏主导航。 */
const QUEUE = "main section:has(> header > h1)";
const STAGE = "section[aria-label=效果预览]";
const TOPNAV = "header:has(#nav-batch) nav";
const WORKSPACE = "[data-tool-workspace]";
/** 每个页面加载完成的标志（工具箱没有 h1：以工具分类标签为准）。 */
const PAGE_READY = {
  batch: "main h1",
  toolbox: "main [role=tablist][aria-label=工具分类]",
  settings: "main h1",
};

const ui = {
  /** 点顶栏导航切换页面；heading 只用于提示文字。 */
  async nav(page, id, heading = id) {
    await page.click({ selector: `#nav-${id}` });
    await ui.waitView(page, id, heading);
  },

  /** 等到顶栏选中 id 且页面内容已挂载。 */
  waitView(page, id, heading = id) {
    return page.waitFor(
      `页面 ${heading}`,
      (i, sel) => document.querySelector(`#nav-${i}`)?.getAttribute("aria-selected") === "true" && document.querySelector("main")?.id === `view-${i}` && !!document.querySelector(sel),
      [id, PAGE_READY[id]],
    );
  },

  /** 打开菜单（点触发按钮）并点其中的菜单项；item 是菜单项文字的开头，submenu 是途经的子菜单项文字的开头。 */
  async menu(page, trigger, item, submenu = null) {
    await page.click(trigger);
    await page.waitFor("菜单打开", () => !!document.querySelector("[role=menu]"));
    if (submenu) {
      await page.click({ selector: "[role=menu] [role=menuitem]", textStarts: submenu });
      await page.waitFor(`子菜单 ${submenu}`, (t) => [...document.querySelectorAll("[role=menu] [role=menuitem]")].some((m) => window.__e2e.visible(m) && window.__e2e.text(m).startsWith(t)), [item]);
    }
    // 菜单项文字后面可能带快捷键（如“全选Ctrl+A”），按开头匹配
    await page.click({ selector: "[role=menu] [role=menuitem]", textStarts: item });
    await page.waitFor("菜单关闭", () => !document.querySelector("[role=menu]"), [], { timeout: 5000 }).catch(() => {});
  },

  /** 队列标题行的“更多”菜单（全选、反选、排序、清空）。 */
  queueMore(page, item, submenu = null) {
    return ui.menu(page, { selector: `${QUEUE} header button`, label: "更多" }, item, submenu);
  },

  /** 队列标题行的“添加”菜单（添加视频 / 添加文件夹）。 */
  queueAdd(page, item) {
    return ui.menu(page, { selector: `${QUEUE} header button`, text: "添加" }, item);
  },

  /** 配方标题区的“更多”菜单（查看命令、重置…）。 */
  recipeMore(page, item) {
    return ui.menu(page, { selector: "aside header button", label: "更多" }, item);
  },

  /** 重置配方（“更多 → 重置…”，确认）。 */
  async resetRecipe(page) {
    await ui.recipeMore(page, "重置…");
    await page.click({ selector: "[role=alertdialog] button", text: "确定" });
    await page.waitFor("重置确认框关闭", () => !document.querySelector("[role=alertdialog]"));
  },

  /** 配方面板里列出的步骤开关（名称）。 */
  listedSteps(page) {
    return page.evaluate(() => [...document.querySelectorAll("aside [data-slot=collapsible] > div > [role=switch]")].filter(window.__e2e.visible).map((s) => s.getAttribute("aria-label")));
  },

  /** “添加步骤”菜单里的模块名（菜单项的第一行）；打开后用 Esc 关上。 */
  async addableSteps(page) {
    await page.click({ selector: "aside button", textStarts: "添加步骤" });
    const names = await page.waitFor("添加步骤菜单", () => {
      const items = [...document.querySelectorAll("[role=menu] [role=menuitem]")];
      return items.length ? items.map((m) => m.querySelector("span > span")?.textContent.trim()) : null;
    });
    return names;
  },

  /** 模块不在列表里时经“添加步骤”加进来（直接启用并展开）；已列出时什么也不做。返回是否经菜单添加。 */
  async ensureStep(page, name) {
    if ((await ui.listedSteps(page)).includes(name)) return false;
    await page.click({ selector: "aside button", textStarts: "添加步骤" });
    const nth = await page.waitFor(`添加步骤菜单里的 ${name}`, (n) => {
      const items = [...document.querySelectorAll("[role=menu] [role=menuitem]")].filter(window.__e2e.visible);
      const i = items.findIndex((m) => m.querySelector("span > span")?.textContent.trim() === n);
      return i >= 0 ? i + 1 : null;
    }, [name]);
    await page.click({ selector: "[role=menu] [role=menuitem]", nth: nth - 1 });
    await page.waitFor(`步骤 ${name} 已添加`, (n) => window.__e2e.state(window.__e2e.find({ selector: "[role=switch]", label: n, within: "aside" })) === "checked", [name]);
    return true;
  },

  rows(page) {
    return page.evaluate(() =>
      [...document.querySelectorAll("li[data-row-index]")].map((li) => {
        const title = li.querySelector("p[data-path]");
        const body = title?.closest("div.min-w-0");
        // 短状态（未处理 / 处理中 45% / 完成…）在文件名同一行右侧；细节（“3 个输出”等）在媒体信息下一行
        const state = body?.querySelector(":scope > div:first-child > span.tabular-nums");
        const lines = body ? [...body.querySelectorAll(":scope > p")] : [];
        return {
          name: title?.textContent.trim(),
          path: title?.getAttribute("data-path"),
          summary: lines[0]?.textContent.trim() ?? "",
          state: state?.textContent.trim() ?? "",
          detail: lines[1]?.textContent.trim() ?? "",
          badges: [...li.querySelectorAll("[data-slot=badge]")].map((b) => ({ text: b.textContent.trim(), title: b.getAttribute("aria-label") })),
          text: li.innerText,
        };
      }),
    );
  },

  /** 把鼠标移到第 nth 行上（行内按钮只在悬停时可点）。 */
  async hoverRow(page, nth) {
    const pos = await page.waitLocate({ selector: "li[data-row-index]", nth }, 10000, { enabled: false });
    await page.cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: pos.x, y: pos.y, button: "none", pointerType: "mouse" });
    await sleep(200);
  },

  /** 预览舞台当前显示的图片（kind：effect 效果 / source 原片）；没有或没加载完时为 null。 */
  stageImage(page, kind = "effect") {
    return page.evaluate((sel, k) => {
      const suffix = k === "effect" ? "套用配方后的画面" : "原片画面";
      const img = [...document.querySelectorAll(`${sel} img`)].find((i) => i.alt.endsWith(suffix) && i.alt.startsWith("「"));
      if (!img) return null;
      return { src: img.src, alt: img.alt, complete: img.complete, width: img.naturalWidth, height: img.naturalHeight };
    }, STAGE, kind);
  },

  /** 等预览舞台显示（与 notSrc 不同的）已加载的图片。舞台在预览窗口打开、批量处理中不自动刷新。 */
  async waitStageImage(page, what, { kind = "effect", notSrc = null, timeout = 60000 } = {}) {
    const deadline = Date.now() + timeout;
    let last = null;
    while (Date.now() < deadline) {
      last = await ui.stageImage(page, kind);
      if (last && last.complete && last.width > 0 && last.src !== notSrc) return last;
      await sleep(250);
    }
    const message = await page.evaluate((sel) => document.querySelector(`${sel} [role=alert]`)?.textContent ?? document.querySelector(sel)?.innerText.slice(0, 300), STAGE);
    throw new Error(`等待超时：${what}；舞台图片 ${JSON.stringify(last)}；舞台内容：${message}`);
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
    await ui.queueMore(page, "清空");
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
    return page.evaluate((l, wi) => window.__e2e.state(window.__e2e.find({ selector: "[role=switch]", label: l, within: wi })), label, within);
  },

  async setSwitch(page, label, on, within = "aside") {
    const state = await ui.switchState(page, label, within);
    if (state === null) throw new Error(`找不到开关：${label}`);
    if ((state === "checked") !== on) await page.click({ selector: "[role=switch]", label, within });
    await page.waitFor(`开关 ${label} = ${on}`, (l, wi, want) => window.__e2e.state(window.__e2e.find({ selector: "[role=switch]", label: l, within: wi })) === want, [label, within, on ? "checked" : "unchecked"]);
  },

  async expandModule(page, name) {
    const spec = { selector: "aside button[aria-expanded]", textStarts: name };
    const expanded = await page.evaluate((s) => window.__e2e.find(s)?.getAttribute("aria-expanded"), spec);
    if (expanded !== "true") await page.click(spec);
    await page.waitFor(`展开 ${name}`, (s) => window.__e2e.find(s)?.getAttribute("aria-expanded") === "true", [spec]);
  },

  async choose(page, label, within = "aside") {
    await page.click({ selector: "[data-slot=toggle-group-item]", text: label, within });
    await page.waitFor(`选项 ${label}`, (l, wi) => window.__e2e.state(window.__e2e.find({ selector: "[data-slot=toggle-group-item]", text: l, within: wi })) === "on", [label, within]);
  },

  runSummary(page) {
    return page.evaluate(() => [...document.querySelectorAll("#statusbar span")].map((s) => s.textContent.trim()).find((t) => t.startsWith("上次") || t.startsWith("处理中")) ?? "");
  },

  async startBatch(page) {
    await page.click({ selector: "main footer button", text: "开始处理" });
    await page.waitFor("处理开始", () => [...document.querySelectorAll("main footer button")].some((b) => b.textContent.trim() === "停止") || [...document.querySelectorAll("#statusbar span")].some((s) => s.textContent.startsWith("上次")), [], { timeout: 15000 });
  },

  async waitBatchFinished(page, timeout = 240000) {
    await page.waitFor("批量处理结束", () => [...document.querySelectorAll("#statusbar span")].some((s) => s.textContent.trim().startsWith("上次")) && ![...document.querySelectorAll("main footer button")].some((b) => b.textContent.trim() === "停止"), [], { timeout, interval: 500 });
    return ui.runSummary(page);
  },

  logText(page) {
    return page.evaluate(() => {
      // 处理记录面板是中栏 section 里最内层的那个（中栏外层 section 也包含它）
      const section = [...document.querySelectorAll("main section")].filter((s) => s.textContent.includes("处理记录")).pop();
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
    // 焦点回到按钮时 Base UI 的提示会打开，这时第一次 Esc 只关闭提示：先关掉打开着的提示
    if (await page.evaluate(() => !!document.querySelector("[data-slot=tooltip-content][data-open]"))) await page.escape();
    await page.escape();
    await page.waitFor("对话框关闭", (n) => document.querySelectorAll("[role=dialog],[role=alertdialog]").length < n, [before], { timeout: 5000 });
  },

  /** 工作区标题里的工具名（h2 的第一个文本节点，不含分组徽标）。 */
  workspaceTool(page) {
    return page.evaluate((sel) => {
      const ws = document.querySelector(sel);
      return ws ? { id: ws.getAttribute("data-tool-workspace"), name: ws.querySelector("h2")?.firstChild?.textContent.trim() ?? "" } : null;
    }, WORKSPACE);
  },

  /** 等工作区显示工具 name（定义已加载：有“开始”或“停止”按钮）。 */
  waitWorkspace(page, name) {
    return page.waitFor(`工具工作区 ${name}`, (sel, n) => {
      const ws = document.querySelector(sel);
      if (!ws || ws.querySelector("h2")?.firstChild?.textContent.trim() !== n) return null;
      return [...ws.querySelectorAll("button")].some((b) => ["开始", "停止"].includes(b.textContent.trim())) ? ws.getAttribute("data-tool-workspace") : null;
    }, [WORKSPACE, name]);
  },

  /** 在工具箱里（“全部”分类下）选中工具 name。 */
  async openTool(page, name) {
    if ((await page.evaluate(() => document.querySelector("main")?.id)) !== "view-toolbox") await ui.nav(page, "toolbox", "工具箱");
    const all = { selector: "[role=tablist][aria-label=工具分类] [role=tab]", textStarts: "全部" };
    if ((await page.evaluate((s) => window.__e2e.find(s)?.getAttribute("aria-selected"), all)) !== "true") await page.click(all);
    await page.click({ selector: "[role=tablist][aria-label=工具] [role=tab]", label: `打开 ${name}` });
    return ui.waitWorkspace(page, name);
  },

  async runTool(page, timeout = 180000) {
    await page.click({ selector: `${WORKSPACE} button`, text: "开始" });
    const summary = await page.waitFor(
      "工具运行结束",
      (sel) => {
        const d = document.querySelector(sel);
        const s = [...d.querySelectorAll("span")].map((x) => x.textContent.trim()).find((t) => t.startsWith("上次："));
        const running = [...d.querySelectorAll("button")].some((b) => b.textContent.trim() === "停止");
        return s && !running ? s : null;
      },
      [WORKSPACE],
      { timeout, interval: 500 },
    );
    const m = /完成 (\d+) · 失败 (\d+) · 取消 (\d+)/.exec(summary);
    const items = await page.evaluate((sel) => [...document.querySelectorAll(`${sel} ul li`)].map((li) => li.innerText.replace(/\s+/g, " ")), WORKSPACE);
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

/** 首帧检查：应用只有深色主题，第一帧就是 data-theme="dark" 与深色底色，之后 data-theme 没有变化过。 */
async function checkFirstFrame(page) {
  const boot = await page.evaluate(() => {
    const first = window.__e2e.firstFrame;
    // 颜色变量是 oklch：经 canvas 换算成 sRGB 再比较亮度
    let rgba = null;
    if (first?.background) {
      const ctx = document.createElement("canvas").getContext("2d", { willReadFrequently: true });
      ctx.fillStyle = "rgba(0, 0, 0, 0)";
      ctx.fillStyle = first.background;
      ctx.fillRect(0, 0, 1, 1);
      rgba = [...ctx.getImageData(0, 0, 1, 1).data];
    }
    return { first, rgba, history: window.__e2e.themeHistory, now: document.documentElement.dataset.theme };
  });
  assert(boot.first, "没有记录到首帧");
  assert(boot.first.theme === "dark" && boot.now === "dark", `首帧主题 ${boot.first.theme}、现在 ${boot.now}，应都为 dark（${JSON.stringify(boot)}）`);
  assert(boot.history.length === 0, `启动后 data-theme 变化过：${JSON.stringify(boot.history)}`);
  const rgb = boot.rgba ?? [];
  const transparent = rgb.length === 4 && rgb[3] === 0;
  const luminance = rgb.length >= 3 ? (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]) / 255 : -1;
  // 底色透明时露出的是 WebView 底色（后端创建窗口时设为同一深色），也不会闪白
  assert(transparent || luminance < 0.2, `首帧底色 ${boot.first.background} 不是深色`);
  return `首帧主题 ${boot.first.theme}（底色 ${boot.first.background}），启动后 data-theme 没有变化`;
}

/** 在主窗口截一张包含系统标题栏的真实屏幕截图（尽力而为）。 */
function saveNativeShot(s, label) {
  const pid = s.appPid();
  if (!pid) return;
  try {
    const file = path.join(REPORT, `${s.label}-window-${label}.png`);
    captureNativeWindow(pid, file);
    s.shots = [...(s.shots ?? []), file];
  } catch (e) {
    log("  窗口截图失败：", e.message);
  }
}

/** 桌面应用体验：去掉网页行为、标题、光标、首帧主题（docs/architecture/frontend.md“桌面应用体验约定”）。 */
async function desktopScenarios(s) {
  const { page } = s;
  await scenario(s, "桌面体验：窗口标题、首帧主题、右键菜单、浏览器快捷键与缩放、光标与选择、无原生提示", async () => {
    const notes = [];
    // 标题：页面标题与原生窗口标题都只是程序名
    const docTitle = await page.evaluate(() => document.title);
    assert(docTitle === "ClipRecipe", `页面标题：${docTitle}`);
    const win = nativeWindow(s.appPid());
    assert(win && win.title === "ClipRecipe" && win.visible, `原生窗口：${JSON.stringify(win)}`);
    notes.push(`窗口“${win.title}” ${win.width}×${win.height}`);
    notes.push(await checkFirstFrame(page));
    // 窗口配色（标题栏、底色）：启动时设置，颜色都是 #rrggbb，底色是深色
    const chrome = await page.evaluate(() => window.__e2e.ipc.filter((r) => r.cmd === "set_window_chrome").map((r) => ({ ok: r.ok, ...r.args?.chrome })));
    const hex = /^#[0-9a-f]{6}$/;
    assert(chrome.length > 0 && chrome.every((c) => c.ok && [c.titlebar, c.text, c.border, c.background].every((v) => hex.test(v))), `set_window_chrome：${JSON.stringify(chrome)}`);
    const last = chrome.at(-1);
    const [r, g, b] = [1, 3, 5].map((i) => parseInt(last.background.slice(i, i + 2), 16));
    assert((0.2126 * r + 0.7152 * g + 0.0722 * b) / 255 < 0.2, `窗口底色不是深色：${JSON.stringify(last)}`);
    notes.push(`窗口配色：底色 ${last.background}、标题栏 ${last.titlebar}、文字 ${last.text}、边框 ${last.border}`);

    // 右键空白处：没有菜单（默认行为被取消）；输入框保留系统编辑菜单（不取消）
    const mark = await page.evaluate(() => window.__e2e.events.length);
    const empty = await page.locate({ selector: "main header h1" });
    await page.rightClickAt(empty.x, empty.y);
    const menuEvent = await page.evaluate((m) => window.__e2e.events.slice(m).find((e) => e.type === "contextmenu"), mark);
    assert(menuEvent?.prevented === true, `空白处右键没有被拦截：${JSON.stringify(menuEvent)}`);
    assert(!(await page.evaluate(() => !!document.querySelector("[role=menu]"))), "空白处右键不应弹出菜单");
    const inputMenu = await page.evaluate(() => {
      const input = document.querySelector("main input[aria-label=输出文件夹]");
      const ev = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
      input.dispatchEvent(ev);
      return ev.defaultPrevented;
    });
    assert(inputMenu === false, "输入框应保留系统的编辑菜单");

    // 浏览器快捷键：刷新、查找、打印、保存、查看源码、光标浏览、缩放、后退都不起作用（页面没有重新加载、事件被取消）
    await page.evaluate(() => {
      window.__e2eMarker = "still-here";
      document.activeElement?.blur?.();
    });
    const zoomBefore = await page.evaluate(() => ({ dpr: window.devicePixelRatio, scale: window.visualViewport?.scale ?? 1, width: window.innerWidth }));
    const keys = [
      ["F5", "F5", 116, 0],
      ["r", "KeyR", 82, 2],
      ["R", "KeyR", 82, 10],
      ["f", "KeyF", 70, 2],
      ["g", "KeyG", 71, 2],
      ["p", "KeyP", 80, 2],
      ["s", "KeyS", 83, 2],
      ["u", "KeyU", 85, 2],
      ["F7", "F7", 118, 0],
      ["=", "Equal", 187, 2],
      ["-", "Minus", 189, 2],
      ["0", "Digit0", 48, 2],
      ["ArrowLeft", "ArrowLeft", 37, 1],
      ["a", "KeyA", 65, 2],
    ];
    const keyMark = await page.evaluate(() => window.__e2e.events.length);
    for (const [key, code, keyCode, modifiers] of keys) await page.key(key, code, keyCode, modifiers);
    await page.wheel(400, 300, -240, 2); // Ctrl + 滚轮
    await sleep(600);
    const after = await page.evaluate((m) => ({
      marker: window.__e2eMarker,
      events: window.__e2e.events.slice(m),
      dpr: window.devicePixelRatio,
      scale: window.visualViewport?.scale ?? 1,
      width: window.innerWidth,
      selection: String(window.getSelection()),
      dialogs: document.querySelectorAll("[role=dialog],[role=alertdialog]").length,
    }), keyMark);
    assert(after.marker === "still-here", "按浏览器快捷键后页面被重新加载了");
    const keyEvents = after.events.filter((e) => e.type === "keydown");
    const notPrevented = keyEvents.filter((e) => !e.prevented).map((e) => `${e.ctrl ? "Ctrl+" : ""}${e.alt ? "Alt+" : ""}${e.key}`);
    assert(keyEvents.length === keys.length && notPrevented.length === 0, `没有被拦截的浏览器快捷键：${notPrevented.join("、")}（收到 ${keyEvents.length} 个）`);
    const wheel = after.events.find((e) => e.type === "wheel" && e.ctrl);
    assert(wheel?.prevented === true, `Ctrl+滚轮没有被拦截：${JSON.stringify(wheel)}`);
    assert(after.dpr === zoomBefore.dpr && after.scale === zoomBefore.scale && after.width === zoomBefore.width, `页面被缩放了：${JSON.stringify(zoomBefore)} → ${JSON.stringify(after)}`);
    assert(after.selection === "", `Ctrl+A 选中了页面文字：${after.selection.slice(0, 60)}`);
    assert(after.dialogs === 0, "快捷键打开了对话框");
    notes.push(`${keys.length} 个浏览器快捷键与 Ctrl+滚轮都被拦截`);

    // 光标与选择：控件是箭头光标，界面文字不可选；输入框是文本光标；没有原生 title 提示
    const ui = await page.evaluate(() => {
      const cursor = (sel) => {
        const el = document.querySelector(sel);
        return el ? getComputedStyle(el).cursor : null;
      };
      return {
        button: cursor("main header button"),
        tab: cursor("#nav-batch"),
        switch: cursor("aside [role=switch]"),
        input: cursor("main input[aria-label=输出文件夹]"),
        headingSelect: getComputedStyle(document.querySelector("main h1")).userSelect,
        titled: [...document.querySelectorAll("[title]")].map((e) => `${e.tagName}:${e.getAttribute("title")}`).slice(0, 5),
      };
    });
    assert(ui.button === "default" && ui.tab === "default" && ui.switch === "default", `控件光标：${JSON.stringify(ui)}`);
    assert(ui.input === "text", `输入框光标：${ui.input}`);
    assert(ui.headingSelect === "none", `标题文字可以选中：${ui.headingSelect}`);
    assert(ui.titled.length === 0, `还有原生 title 提示：${ui.titled.join("、")}`);
    notes.push("按钮 / 标签 / 开关为箭头光标，输入框为文本光标，界面文字不可选，没有 title 提示");
    return notes.join("；");
  });
  saveNativeShot(s, "boot");
}

async function bootScenarios(s, media) {
  const { page } = s;

  await desktopScenarios(s);

  await scenario(
    s,
    "启动：无错误、CSP 生效、三个页面、没有主题切换",
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
      // “全部”分类下列出全部工具；未选中工具时工作区显示第一个
      const cards = await page.waitFor("工具列表", () => {
        const n = document.querySelectorAll("[role=tablist][aria-label=工具] [role=tab][aria-label^='打开 ']").length;
        return n > 0 ? n : null;
      });
      assert(cards === 34, `工具箱“全部”分类应列出 34 个工具，实际 ${cards}`);
      const firstTool = await ui.waitWorkspace(page, "视频合并");
      await ui.nav(page, "settings", "设置");
      await page.waitFor("设置分组", () => document.querySelectorAll("main [aria-label=设置分组] [role=tab]").length >= 6);
      await ui.nav(page, "batch", "视频队列");
      // 配方面板只列出启用的步骤（默认只有帧率），其余 17 个在“添加步骤”菜单里
      const listed = await ui.listedSteps(page);
      assert(JSON.stringify(listed) === JSON.stringify(["帧率"]), `默认配方应只列出“帧率”：${listed}`);
      const addable = await ui.addableSteps(page);
      await page.escape();
      await page.waitFor("添加步骤菜单关闭", () => !document.querySelector("[role=menu]"));
      assert(addable.length === 17 && !addable.includes("帧率"), `“添加步骤”菜单应有其余 17 个模块：${addable}`);
      // 只有深色主题：界面上没有主题切换
      const themeControls = await page.evaluate(() =>
        [...document.querySelectorAll("button, [role=radio], [role=tab]")]
          .map((e) => e.getAttribute("aria-label") || e.textContent || "")
          .filter((t) => /主题|浅色|深色/.test(t)),
      );
      assert(themeControls.length === 0, `界面上还有主题控件：${themeControls.join("、")}`);
      const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
      return `${info.origin}，${info.ua ?? "WebView2"}；${cspNote}；工具 ${cards} 个（默认选中 ${firstTool}）；配方列出 ${listed.join("、")}，可添加 ${addable.length} 个；没有主题控件（背景 ${bg}）`;
    },
    { csp: [/e2e-csp-probe/], console: [/e2e-csp-probe/, /Content Security Policy/] },
  );
  await saveShot(s, "boot");

  await scenario(s, "环境命令：app_info / ffmpeg_status / encoder_info", async () => {
    const app = await page.invoke("app_info");
    assert(app.ok && app.value.name === "ClipRecipe" && app.value.cpuCores > 0, `app_info：${JSON.stringify(app)}`);
    const ff = await page.invoke("ffmpeg_status");
    assert(ff.ok && ff.value.available, `ffmpeg_status：${JSON.stringify(ff)}`);
    // 平台来源（src/lib/platform.ts）：后端报告编译目标，前端写到 <html data-platform>（本测试只在 Windows 上运行）
    assert(app.value.platform === "windows", `app_info.platform 应为 windows：${JSON.stringify(app.value)}`);
    await page.waitFor("页面平台为 windows", () => document.documentElement.dataset.platform === "windows");
    assert(/^ffprobe version /.test(ff.value.ffprobeVersion ?? ""), `ffmpeg_status.ffprobeVersion：${ff.value.ffprobeVersion}`);
    const enc = await page.invoke("encoder_info", { refresh: false });
    assert(enc.ok && enc.value.h264 && enc.value.hevc, `encoder_info：${JSON.stringify(enc)}`);
    // 设置页“关于 · 运行环境”也能显示出来（同样的命令经界面调用）
    await ui.nav(page, "settings", "设置");
    await page.click({ selector: "[role=tab]", text: "关于" });
    await page.waitFor("运行环境显示 FFmpeg 版本", (v) => document.querySelector("main").innerText.includes(v), [ff.value.version.split(" ")[0]], { timeout: 60000 });
    await page.waitFor("硬件编码器检测完成", () => document.querySelector("main").innerText.includes("开启 GPU 加速时使用"), [], { timeout: 120000 });
    await ui.nav(page, "batch", "视频队列");
    const expectSource = process.env.CLIPRECIPE_FFMPEG ? "env" : "dev";
    assert(ff.value.source === expectSource, `FFmpeg 来源应为 ${expectSource}：${JSON.stringify(ff.value).slice(0, 400)}`);
    assert(ff.value.managed?.supported && ff.value.managed.downloadSize > 0, `应用内下载状态：${JSON.stringify(ff.value.managed)}`);
    return `v${app.value.version}，${app.value.cpuCores} 核；${ff.value.version}（来源 ${ff.value.source}，whisper 滤镜 ${ff.value.whisper ? "有" : "无"}）；硬件：${enc.value.hardware.join("、") || "无"}，H.264 → ${enc.value.h264}`;
  });

  await scenario(s, "导入：拖放 + 添加视频菜单 → 行显示媒体信息、预览舞台显示效果帧；拖到输出位置设为输出目录", async () => {
    await ui.clearQueue(page);
    await ui.dropOnQueue(page, [media.a, media.b]);
    await ui.waitRows(page, "两个视频导入", (r) => r.length === 2 && r.every((x) => x.summary.includes("640×360")));
    // 预览舞台：当前视频（没有选中时为第一个）套用配方后的一帧，自动渲染，图片能加载（asset 协议、CSP）
    const first = (await ui.rows(page))[0].name;
    const frame = await ui.waitStageImage(page, "预览舞台的效果帧");
    assert(frame.alt === `「${first}」套用配方后的画面` && frame.src.startsWith("http://asset.localhost/"), `舞台图片：${JSON.stringify(frame)}`);
    // “添加 → 添加视频”：系统对话框由探针返回预先排队的路径
    await page.queueDialog([media.portrait]);
    await ui.queueAdd(page, "添加视频");
    const rows = await ui.waitRows(page, "竖屏视频导入", (r) => r.length === 3 && r.some((x) => x.name === "portrait.mp4" && x.summary.includes("360×640")));
    // 拖一个文件到“输出到”输入框：设为该文件所在的文件夹
    await page.dropFiles([media.extra], { selector: "input[aria-label=输出文件夹]", within: "main" });
    const extraDir = path.dirname(media.extra);
    await page.waitFor("拖放设置输出目录", (d) => document.querySelector("main input[aria-label=输出文件夹]")?.value === d, [extraDir]);
    assert((await ui.rows(page)).length === 3, "拖到输出位置的文件不应导入队列");
    const ipc = await page.evaluate(() => window.__e2e.ipc.filter((r) => r.cmd === "import_paths").map((r) => r.args));
    assert(ipc.length === 2 && ipc.every((a) => Array.isArray(a.paths) && typeof a.recursive === "boolean"), `import_paths 参数：${JSON.stringify(ipc)}`);
    return `${rows.map((r) => `${r.name}: ${r.summary}`).join("；")}；舞台效果帧 ${frame.width}×${frame.height}`;
  });

  await shortcutScenario(s, media);
  await auditScenario(s);
}

/**
 * 界面状态审计（scripts/e2e-audit-page.js）：鼠标移开、取消焦点后检查 root 内的选中态、aria 与 Base UI 状态属性一致、对比度。
 * 返回统计；有问题时抛出（列出全部问题）。
 */
async function auditUi(page, label, root, found = null) {
  await page.cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: 2, y: 4000, button: "none", pointerType: "mouse" });
  await page.evaluate(() => document.activeElement?.blur?.());
  await sleep(250);
  const r = await page.evaluate((o) => window.__e2e.audit(o), { root });
  for (const w of r.warnings ?? []) {
    log(`  ⚠ 审计警告（${label}）：${w}`);
    auditWarnings.push({ label, text: w });
  }
  // found：收集问题而不是立即失败（审计场景要覆盖全部页面后再统一报告）
  if (r.problems.length && found) found.push(...r.problems.map((p) => `（${label}）${p}`));
  else if (r.problems.length) throw new Error(`界面审计（${label}）发现 ${r.problems.length} 个问题：\n${r.problems.slice(0, 15).join("\n")}`);
  return r;
}

/** 导航项（顶栏主导航 / 设置分组）的选中态：选中项 aria 与 Base UI 的 data-active 一致，底色与未选中项明显不同，键盘焦点环与选中底色同时可见。 */
async function assertNavSelected(page, listSelector, expectText) {
  const r = await page.evaluate((sel) => {
    const tabs = [...document.querySelectorAll(`${sel} [role=tab]`)];
    const canvas = document.createElement("canvas").getContext("2d", { willReadFrequently: true });
    const rgb = (css) => {
      canvas.clearRect(0, 0, 1, 1);
      canvas.fillStyle = css;
      canvas.fillRect(0, 0, 1, 1);
      return [...canvas.getImageData(0, 0, 1, 1).data];
    };
    return tabs.map((t) => ({
      text: t.textContent.trim(),
      selected: t.getAttribute("aria-selected") === "true",
      state: window.__e2e.state(t),
      bg: rgb(getComputedStyle(t).backgroundColor),
    }));
  }, listSelector);
  const selected = r.filter((t) => t.selected);
  assert(selected.length === 1 && selected[0].text.includes(expectText), `${listSelector} 的选中项应为“${expectText}”：${JSON.stringify(r)}`);
  assert(selected[0].state === "active", `${listSelector} 选中项的状态是 ${selected[0].state}（应为 active，即带 data-active）`);
  const others = r.filter((t) => !t.selected);
  assert(others.every((t) => t.state === "inactive"), `${listSelector} 未选中项的状态：${JSON.stringify(others.map((t) => t.state))}`);
  // 选中项有可见的底色（不透明度 > 20/255），未选中项没有底色。属于“选中项底色对比”规则：
  // 它是硬失败；只有被临时放进 e2e-audit-page.js 的 SOFT_RULES（目前为空）时才只记警告
  const bgVisible = selected[0].bg[3] > 20 && others.every((t) => t.bg[3] < 5);
  if (!bgVisible) {
    const soft = await page.evaluate(() => window.__e2e.softRules?.["selected-distinct"]);
    const message = `${listSelector} 选中项“${expectText}”的底色不透明度 ${selected[0].bg[3]}/255 ≤ 20（rgba(${selected[0].bg.join(",")})）`;
    assert(soft, message);
    const label = "导航选中底色";
    const text = `[selected-distinct] ${message}【降级为警告：${soft}】`;
    log(`  ⚠ 审计警告（${label}）：${text}`);
    auditWarnings.push({ label, text });
  }
  return `${expectText}（底色 rgba(${selected[0].bg.join(",")})）`;
}

/** 全部页面、18 个模块、34 个工具工作区审计一遍（应用只有深色主题）。 */
async function auditScenario(s) {
  const { page } = s;
  await scenario(s, "界面状态审计：三个页面、设置各分组、18 个模块、34 个工具工作区（深色）", async () => {
    const theme = await page.evaluate(() => document.documentElement.dataset.theme);
    assert(theme === "dark", `页面主题应为 dark：${theme}`);
    const counts = { audits: 0, groups: 0, texts: 0, boundaries: 0 };
    const add = (r) => {
      counts.audits++;
      counts.groups += r.stats.groups;
      counts.texts += r.stats.texts;
      counts.boundaries += r.stats.boundaries;
    };
    const notes = [];
    const found = [];
    // 三个页面：顶栏导航的选中项明确
    for (const [id, heading, label] of [["batch", "视频队列", "批量处理"], ["toolbox", "工具箱", "工具箱"], ["settings", "设置", "设置"]]) {
      await ui.nav(page, id, heading);
      if (id === "batch") await ui.waitStageImage(page, "审计前舞台出图");
      add(await auditUi(page, `${theme} ${heading}`, undefined, found));
      await saveShot(s, `audit-${theme}-${id}`);
      notes.push(`${theme} 顶栏导航 ${await assertNavSelected(page, TOPNAV, label)}`);
      if (id === "batch") {
        // 选中一行：选中行的样式与只在有选择时出现的操作条（置顶、工具菜单、移除）也要审计；审计后 Ctrl+点击取消选择，不改变后续场景的状态
        const row = { selector: "li[data-row-index] p[data-path]", nth: 0 };
        await page.click(row);
        await page.waitFor("操作条显示已选 1 个", (sel) => document.querySelector(sel)?.innerText.includes("已选 1 个"), [QUEUE]);
        add(await auditUi(page, `${theme} 视频队列 · 已选 1 个`, QUEUE, found));
        const pos = await page.waitLocate(row, 5000);
        for (const type of ["mousePressed", "mouseReleased"]) {
          await page.cdp.send("Input.dispatchMouseEvent", { type, x: pos.x, y: pos.y, button: "left", buttons: type === "mousePressed" ? 1 : 0, clickCount: 1, modifiers: 2, pointerType: "mouse" });
        }
        await page.waitFor("取消选择后操作条消失", (sel) => !document.querySelector(sel)?.innerText.includes("已选"), [QUEUE]);
      }
    }
    // 顶栏导航的键盘焦点环与选中底色同时可见、互相区分
    const ring = await page.evaluate(() => window.__e2e.focusRing("#nav-settings"));
    assert(ring.ok, `顶栏导航焦点环：${JSON.stringify(ring)}`);
    if (ring.contrast < 3) {
      const soft = await page.evaluate(() => window.__e2e.softRules?.["focus-ring-contrast"]);
      const message = `顶栏导航焦点环描边对比 ${ring.contrast} < 3（${JSON.stringify(ring)}）`;
      if (!soft) found.push(`（${theme}）${message}`);
      else {
        const text = `[focus-ring-contrast] ${message}【降级为警告：${soft}】`;
        log(`  ⚠ 审计警告（${theme} 顶栏导航焦点环）：${text}`);
        auditWarnings.push({ label: `${theme} 顶栏导航焦点环`, text });
      }
    }
    // 设置的每个分组
    for (const tab of ["转换", "字幕与封面", "通用", "完成后", "高级", "关于"]) {
      await page.click({ selector: "[role=tab]", text: tab });
      await sleep(200);
      add(await auditUi(page, `${theme} 设置 · ${tab}`, "main", found));
      await assertNavSelected(page, "main [aria-label=设置分组]", tab);
    }
    // 18 个模块的参数表单：没列出的经“添加步骤”加进来（会启用），马上关掉开关（行仍在，显示“已停用”），逐个展开审计
    await ui.nav(page, "batch", "视频队列");
    const addable = await ui.addableSteps(page);
    await page.escape();
    await page.waitFor("添加步骤菜单关闭", () => !document.querySelector("[role=menu]"));
    if (addable.length > 0) {
      await page.click({ selector: "aside button", textStarts: "添加步骤" });
      add(await auditUi(page, `${theme} 添加步骤菜单`, "[role=menu]", found));
      await page.escape();
      await page.waitFor("添加步骤菜单关闭", () => !document.querySelector("[role=menu]"));
    }
    for (const name of addable) {
      await ui.ensureStep(page, name);
      if (name !== "帧率") await ui.setSwitch(page, name, false);
    }
    const modules = await ui.listedSteps(page);
    assert(modules.length === 18, `配方面板列出的模块数 ${modules.length}：${modules}`);
    const disabledSummary = await page.evaluate(() => [...document.querySelectorAll("aside button[aria-expanded]")].filter((b) => b.textContent.includes("已停用")).length);
    assert(disabledSummary === 17, `关掉开关后应有 17 行显示“已停用”，实际 ${disabledSummary}`);
    for (const name of modules) {
      await ui.expandModule(page, name);
      add(await auditUi(page, `${theme} 模块 ${name}`, "aside", found));
      if (name === "文本") await saveShot(s, `audit-${theme}-module-text`);
    }
    await ui.expandModule(page, "帧率");
    // 34 个工具：依次选中，审计工作区
    await ui.nav(page, "toolbox", "工具箱");
    await ui.openTool(page, "视频合并");
    const tools = await page.evaluate(() => [...document.querySelectorAll("[role=tablist][aria-label=工具] [role=tab][aria-label^='打开 ']")].map((b) => b.getAttribute("aria-label").slice(3)));
    for (const name of tools) {
      await ui.openTool(page, name);
      add(await auditUi(page, `${theme} 工具 ${name}`, WORKSPACE, found));
    }
    notes.push(`${theme}：${modules.length} 个模块、${tools.length} 个工具`);
    await ui.nav(page, "batch", "视频队列");
    // 恢复默认配方（只有帧率；没启用的模块从列表消失），后面的场景从默认配方开始
    await ui.resetRecipe(page);
    await page.waitFor("重置后只列出帧率", () => [...document.querySelectorAll("aside [data-slot=collapsible] > div > [role=switch]")].map((x) => x.getAttribute("aria-label")).join() === "帧率");
    if (found.length) throw new Error(`界面审计发现 ${found.length} 个问题（${counts.audits} 次审计）：\n${[...new Set(found)].join("\n")}`);
    return `${counts.audits} 次审计无问题（${counts.groups} 组选项、${counts.texts} 处文字、${counts.boundaries} 个控件边界）；${notes.join("；")}`;
  });
}

/** 键盘快捷键：切换页面、打开设置、Ctrl+O 添加视频，队列列表的 Ctrl+A / 方向键 / Shift 选择 / Delete。 */
async function shortcutScenario(s, media) {
  const { page } = s;
  await scenario(s, "键盘：Ctrl+1/2/3 与 Ctrl+, 切换页面、Ctrl+O 添加视频、队列 Ctrl+A / 方向键 / Shift / Delete", async () => {
    const heading = () => page.evaluate(() => document.querySelector("main h1")?.textContent.trim());
    const press = async (key, code, keyCode, modifiers, view) => {
      await page.key(key, code, keyCode, modifiers);
      await ui.waitView(page, view);
    };
    await press("2", "Digit2", 50, 2, "toolbox");
    await press("3", "Digit3", 51, 2, "settings");
    await press("1", "Digit1", 49, 2, "batch");
    await press(",", "Comma", 188, 2, "settings");
    await press("1", "Digit1", 49, 2, "batch");
    const pages = await heading();

    // Ctrl+O：系统对话框（探针代答）选中的视频加入队列
    const before = (await ui.rows(page)).length;
    await page.queueDialog([media.extra]);
    await page.key("o", "KeyO", 79, 2);
    await ui.waitRows(page, "Ctrl+O 添加视频", (r) => r.length === before + 1 && r.some((x) => x.name === "extra.mp4"));

    const selectedRows = () =>
      page.evaluate(() => [...document.querySelectorAll("li[data-row-index][aria-selected=true]")].map((li) => li.querySelector("p[data-path]")?.textContent.trim()));
    // 点击第一行：列表获得焦点、只选中这一行
    await page.click({ selector: "li[data-row-index] p[data-path]", nth: 0 });
    const focused = await page.evaluate(() => document.activeElement?.getAttribute("role"));
    assert(focused === "listbox", `点击后焦点应在队列列表上：${focused}`);
    assert((await selectedRows()).length === 1, "点击应只选中一行");
    await page.key("a", "KeyA", 65, 2);
    const all = await selectedRows();
    assert(all.length === before + 1, `Ctrl+A 应全选：${all}`);
    const pageSelection = await page.evaluate(() => String(window.getSelection()));
    assert(pageSelection === "", "Ctrl+A 不应选中页面文字");
    await page.key("End", "End", 35);
    let sel = await selectedRows();
    assert(sel.length === 1 && sel[0] === "extra.mp4", `End 应只选中最后一行：${sel}`);
    await page.key("ArrowUp", "ArrowUp", 38, 8); // Shift+↑
    sel = await selectedRows();
    assert(sel.length === 2 && sel.includes("extra.mp4"), `Shift+↑ 应选中两行：${sel}`);
    await page.key("ArrowDown", "ArrowDown", 40);
    sel = await selectedRows();
    assert(sel.length === 1 && sel[0] === "extra.mp4", `↓ 应回到只选中最后一行：${sel}`);
    // Delete：移除所选（开启了“删除任务时确认”时先确认）
    await page.key("Delete", "Delete", 46);
    await sleep(300);
    if (await page.evaluate(() => !!document.querySelector("[role=alertdialog]"))) {
      await page.click({ selector: "[role=alertdialog] button", text: "删除" });
    }
    const rows = await ui.waitRows(page, "Delete 移除所选", (r) => r.length === before && !r.some((x) => x.name === "extra.mp4"));
    return `页面切换后回到“${pages}”；Ctrl+O 添加 extra.mp4；Ctrl+A 全选 ${all.length} 行；End / Shift+↑ / ↓ 选择正确；Delete 后剩 ${rows.length} 行`;
  });
}

async function batchScenarios(s, media) {
  const { page } = s;
  const full = s.mode === "prod";

  await scenario(s, full ? "批量处理：裁剪(百分比) + 文本 + 帧率 + 码率" : "批量处理：帧率（默认配方）", async () => {
    const out = w("out", s.mode, "batch");
    await ui.setOutputDir(page, out);
    assert((await ui.switchState(page, "帧率")) === "checked", "默认配方应启用帧率");
    let stageNote = "";
    if (full) {
      // 改参与预览的参数（裁剪）后，舞台自动刷新成新的图片
      const before = await ui.waitStageImage(page, "改参数前的舞台效果帧");
      await ui.ensureStep(page, "裁剪");
      await ui.expandModule(page, "裁剪");
      await ui.choose(page, "百分比");
      await ui.ensureStep(page, "文本");
      await ui.ensureStep(page, "码率");
      await page.waitFor("已启用 4 个步骤", () => document.querySelector("aside").innerText.includes("已启用 4 个步骤"));
      await ui.waitStageImage(page, "改参数后舞台刷新", { notSrc: before.src });
      const result = await page.waitFor("舞台的参与预览步骤包含裁剪与文本", (sel) => {
        const t = [...document.querySelectorAll(`${sel} span`)].map((x) => x.textContent).find((x) => x.startsWith("参与预览："));
        return t?.includes("裁剪") && t.includes("文本") ? t : null;
      }, [STAGE], { timeout: 60000 });
      const after = await ui.waitStageImage(page, "舞台最新的效果帧", { notSrc: before.src });
      stageNote = `舞台 ${before.width}×${before.height} → ${after.width}×${after.height}（${result.split("。")[0]}）；`;
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
    // 任务栏进度：处理中为 normal（0～100），结束后清除
    const taskbar = await page.evaluate(() => window.__e2e.ipc.filter((r) => r.cmd === "set_taskbar_progress").map((r) => ({ state: r.args?.state, progress: r.args?.progress, ok: r.ok })));
    const normal = taskbar.filter((t) => t.state === "normal");
    assert(normal.length > 0 && normal.every((t) => t.ok && t.progress >= 0 && t.progress <= 100), `任务栏进度：${JSON.stringify(taskbar)}`);
    assert(taskbar.at(-1)?.state === "none" && taskbar.at(-1)?.ok, `结束后任务栏进度应清除：${JSON.stringify(taskbar.slice(-3))}`);
    const recordCount = /处理记录\s*(\d+) 条/.exec(logText)?.[1];
    return `${stageNote}任务栏进度 ${normal.length} 次（${normal.map((t) => t.progress).join("→")}）后清除；${summary}；${Object.entries(probes).map(([f, p]) => `${f} ${p.width}×${p.height}@${p.fps.toFixed(0)} ${p.vcodec}/${p.acodec} ${p.duration.toFixed(2)}s`).join("；")}；处理记录 ${recordCount} 条`;
  });
  await saveShot(s, "batch-done");

  await scenario(s, "预览：舞台切到原片出图；生成预览（整条流水线）在 <video> 中播放，关闭后舞台补渲染", async () => {
    // 舞台“原片”：同一时刻的源画面
    await ui.choose(page, "原片", STAGE);
    const source = await ui.waitStageImage(page, "舞台原片画面", { kind: "source", timeout: 30000 });
    await ui.choose(page, "效果", STAGE);
    const effect = await ui.waitStageImage(page, "切回效果画面");
    await page.click({ selector: `${STAGE} button`, textIncludes: "生成预览" });
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
    // 窗口关闭后舞台仍显示效果帧（被窗口打断的那一轮会补渲染）
    const after = await ui.waitStageImage(page, "关闭预览窗口后舞台的效果帧");
    const stageError = await page.evaluate((sel) => document.querySelector(`${sel} [role=alert]`)?.textContent ?? null, STAGE);
    assert(!stageError, `关闭预览窗口后舞台显示错误：${stageError}`);
    return `舞台原片 ${source.width}×${source.height}、效果 ${effect.width}×${effect.height}，关闭窗口后 ${after.width}×${after.height}；${r}；${info}；调整范围后 ${r2}`;
  });

  await scenario(s, "预览：双击队列行播放源视频（allow_media_file + asset 协议）", async () => {
    await page.click({ selector: "li[data-row-index] p[data-path]", nth: 0 }, { count: 2 });
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
    if (!(await ui.ensureStep(page, "加水印"))) await ui.setSwitch(page, "加水印", true);
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
    // 关掉开关后这一行仍在，摘要显示“已停用”
    await page.waitFor("加水印显示已停用", () => window.__e2e.find({ selector: "aside button[aria-expanded]", textStarts: "加水印" })?.textContent.includes("已停用"));
    await ui.expandModule(page, "文本");
    await page.click({ selector: "aside button", label: "字体" });
    const fonts = await page.waitFor("字体列表", () => {
      const n = document.querySelectorAll("[data-slot=select-content][data-open] [data-slot=select-item]").length;
      return n > 2 ? n : null;
    });
    await page.escape();
    // Base UI 的 Select 关闭后弹层仍挂在页面上（data-closed），以 data-open 判断是否打开
    await page.waitFor("字体列表关闭", () => document.querySelectorAll("[data-slot=select-content][data-open]").length === 0);
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

  await scenario(s, "队列交互：媒体信息、查看命令、右键菜单与操作条的工具菜单切到工具箱（范围为选中）、拖动与排序", async () => {
    // 媒体信息对话框（行内按钮只在悬停该行时可点）
    await ui.hoverRow(page, 0);
    await page.click({ selector: "li[data-row-index] button", label: "媒体信息", nth: 0 });
    const infoText = await page.waitFor("媒体信息对话框", () => {
      const d = document.querySelector("[role=dialog]");
      return d && /h264/.test(d.innerText) && /aac/.test(d.innerText) ? d.innerText.replace(/\s+/g, " ").slice(0, 120) : null;
    });
    await ui.closeDialog(page);
    // 查看命令（配方“更多”菜单）：preview_command → 处理记录里出现 FFmpeg 命令行
    await ui.recipeMore(page, "查看命令");
    await page.waitFor("处理记录中的命令", () => [...document.querySelectorAll("main section")].some((sec) => sec.innerText.includes("的处理命令")));
    // 右键第二行 → 独立工具 → 视频倒放：切到工具箱并选中它，范围为选中的 1 个
    await page.rightClick({ selector: "li[data-row-index] p[data-path]", nth: 1 });
    await page.click({ selector: "[role=menuitem]", textIncludes: "独立工具" });
    await page.click({ selector: "[role=menu] [role=menuitem]", text: "视频倒放" });
    await ui.waitView(page, "toolbox", "工具箱");
    const reverse = await ui.waitWorkspace(page, "视频倒放");
    assert(reverse === "tool.reverse", `工作区：${reverse}`);
    const selectedTab = await page.evaluate(() => document.querySelector("[role=tablist][aria-label=工具] [role=tab][aria-selected=true]")?.getAttribute("aria-label"));
    assert(selectedTab === "打开 视频倒放", `工具列表的选中项：${selectedTab}`);
    const scopeSpec = { selector: "[data-slot=toggle-group-item]", textStarts: "队列中选中的", within: WORKSPACE };
    const scope = await page.evaluate((sp) => `${window.__e2e.state(window.__e2e.find(sp))} ${window.__e2e.find(sp)?.textContent}`, scopeSpec);
    assert(scope.startsWith("on 队列中选中的 1 个"), `处理范围：${scope}`);
    await ui.nav(page, "batch", "视频队列");
    // 队列“更多 → 全选”，选中后出现的操作条里“工具 → 生成 GIF”：范围为选中的 3 个
    await ui.queueMore(page, "全选");
    await page.waitFor("操作条显示已选 3 个", (sel) => document.querySelector(sel)?.innerText.includes("已选 3 个"), [QUEUE]);
    await ui.menu(page, { selector: `${QUEUE} button`, text: "工具" }, "生成 GIF");
    await ui.waitView(page, "toolbox", "工具箱");
    await ui.waitWorkspace(page, "生成 GIF");
    const scope2 = await page.evaluate((sel) => window.__e2e.find({ selector: "[data-slot=toggle-group-item][data-pressed]", within: sel })?.textContent, WORKSPACE);
    assert(scope2 === "队列中选中的 3 个", `处理范围：${scope2}`);
    await ui.nav(page, "batch", "视频队列");
    // 拖动把手排序（指针事件，系统拖放开启时 HTML5 拖放不可用）：第 1 行拖到第 3 行下方
    const names = async () => (await ui.rows(page)).map((r) => r.name);
    const before = await names();
    await page.drag({ selector: "li[data-row-index] span[aria-label=拖动调整顺序]", nth: 0 }, { selector: "li[data-row-index]", nth: 2 }, 12);
    const after = await names();
    assert(JSON.stringify(after) === JSON.stringify([before[1], before[2], before[0]]), `拖动排序：${before} → ${after}`);
    // “更多 → 排序 → 名称 Z→A”
    await ui.queueMore(page, "名称 Z→A", "排序");
    const sorted = await names();
    const expected = [...before].sort((a, b) => new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" }).compare(b, a));
    assert(JSON.stringify(sorted) === JSON.stringify(expected), `排序：${sorted}，应为 ${expected}`);
    return `媒体信息：${infoText}…；命令已写入处理记录；右键菜单与操作条菜单都切到工具箱并选中了工具；拖动排序 ${before.join(",")} → ${after.join(",")}；名称 Z→A：${sorted.join(",")}`;
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
    assert(rows[0].state === "完成" && rows[0].detail.includes("2 个输出"), `行状态：${rows[0].state}，细节：${rows[0].detail}`);
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
    await page.click({ selector: "aside header button", label: "导出配方" });
    await page.waitFor("配方文件写出", () => true, [], { timeout: 1000 });
    const deadline = Date.now() + 10000;
    while (!fs.existsSync(file) && Date.now() < deadline) await sleep(200);
    assert(fs.existsSync(file), "导出的配方文件不存在");
    const json = JSON.parse(fs.readFileSync(file, "utf8"));
    assert(json.modules?.["pipe.crop"]?.enabled && json.modules["pipe.crop"].mode === "percent", `导出内容：${JSON.stringify(json.modules?.["pipe.crop"])}`);
    assert(json.modules?.["pipe.text"]?.enabled && json.modules?.["pipe.bitrate"]?.enabled, "导出的配方缺少启用的文本 / 码率");

    // 重置 / 导入整份替换配方后，配方面板只列出启用的步骤（停用的“加水印”等从列表消失）
    const reset = async () => {
      await ui.resetRecipe(page);
      await page.waitFor("重置后只列出帧率", () => [...document.querySelectorAll("aside [data-slot=collapsible] > div > [role=switch]")].map((x) => x.getAttribute("aria-label")).join() === "帧率");
      assert((await ui.switchState(page, "帧率")) === "checked", "重置后应只启用帧率");
    };
    const restored = async (how) => {
      await page.waitFor(`${how}后裁剪开启`, () => window.__e2e.state(window.__e2e.find({ selector: "[role=switch]", label: "裁剪", within: "aside" })) === "checked");
      assert((await ui.switchState(page, "文本")) === "checked" && (await ui.switchState(page, "码率")) === "checked", `${how}后文本 / 码率未恢复`);
      const listed = await ui.listedSteps(page);
      const enabled = Object.entries(json.modules).filter(([, m]) => m.enabled).length;
      assert(listed.length === enabled, `${how}后应只列出启用的 ${enabled} 个步骤：${listed}`);
      for (const name of listed) assert((await ui.switchState(page, name)) === "checked", `${how}后列出了未启用的步骤 ${name}`);
      await ui.expandModule(page, "裁剪");
      const mode = await page.evaluate(() => window.__e2e.state(window.__e2e.find({ selector: "[data-slot=toggle-group-item]", text: "百分比", within: "aside" })));
      assert(mode === "on", `${how}后裁剪方式不是百分比`);
    };
    await reset();
    await page.queueDialog(file);
    await page.click({ selector: "aside header button", label: "导入配方" });
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
    await ui.choose(page, "队列中全部 2 个", WORKSPACE);
    const out = w("out", s.mode, "tool-remux");
    await ui.setOutputDir(page, out, WORKSPACE);
    const r = await ui.runTool(page);
    assert(r.done === 2 && r.failed === 0, `${r.summary}：${r.items.join(" | ")}`);
    const files = videoOutputs(out);
    assert(files.length === 2, `输出：${files}`);
    for (const f of files) {
      const p = await probe(path.join(out, f));
      assert(p.width === 640 && p.vcodec === "h264" && p.acodec, `${f}：${JSON.stringify(p)}`);
    }
    return `${r.summary}；${files.join("、")}`;
  });

  await scenario(s, "工具：进度条（重编码）运行 + 工具预览", async () => {
    await ui.openTool(page, "进度条");
    await ui.choose(page, "队列中全部 2 个", WORKSPACE);
    const out = w("out", s.mode, "tool-progress");
    await ui.setOutputDir(page, out, WORKSPACE);
    const r = await ui.runTool(page);
    assert(r.done === 2 && r.failed === 0, `${r.summary}：${r.items.join(" | ")}`);
    const files = videoOutputs(out);
    assert(files.length === 2, `输出：${files}`);
    const p = await probe(path.join(out, files[0]));
    assert(p.width === 640 && p.height === 360 && Math.abs(p.duration - 4) < 0.3, `${files[0]}：${JSON.stringify(p)}`);
    // 工具预览（preview: true）：在预览窗口播放
    await page.click({ selector: `${WORKSPACE} button`, text: "预览" });
    const v = await ui.checkVideo(page, "工具预览");
    await saveShot(s, "tool-preview");
    await ui.closeDialog(page); // 预览窗口
    return `${r.summary}；${files.join("、")}；预览 ${v}`;
  });

  await scenario(s, "工具：图转视频（文件夹输入）", async () => {
    await ui.nav(page, "batch", "视频队列");
    const rowsBefore = (await ui.rows(page)).length;
    await ui.openTool(page, "图转视频");
    // 拖一个图片文件到文件夹输入框：取其所在文件夹（工具箱页没有窗口级导入，不会进队列）
    await page.dropFiles([path.join(media.imageDir, "one.png")], { selector: `${WORKSPACE} input[aria-label=图片文件夹]` });
    await page.waitFor("拖放设置图片文件夹", (d) => document.querySelector("[data-tool-workspace] input[aria-label=图片文件夹]")?.value === d, [media.imageDir]);
    await page.click({ selector: `${WORKSPACE} button`, label: "清除图片文件夹" });
    await page.waitFor("清除图片文件夹", () => document.querySelector("[data-tool-workspace] input[aria-label=图片文件夹]")?.value === "");
    await page.queueDialog(media.imageDir);
    await page.click({ selector: `${WORKSPACE} button`, textIncludes: "图片文件夹" });
    await page.waitFor("图片文件夹已设置", (d) => document.querySelector("[data-tool-workspace] input[aria-label=图片文件夹]")?.value === d, [media.imageDir]);
    const out = w("out", s.mode, "tool-images");
    await ui.setOutputDir(page, out, WORKSPACE);
    const r = await ui.runTool(page);
    assert(r.done >= 1 && r.failed === 0, `${r.summary}：${r.items.join(" | ")}`);
    const files = videoOutputs(out);
    assert(files.length === 2, `输出：${files}`);
    const p = await probe(path.join(out, files[0]));
    assert(p.width > 0 && p.duration > 0.5, `${files[0]}：${JSON.stringify(p)}`);
    const start = await page.evaluate(() => window.__e2e.ipc.filter((x) => x.cmd === "start_tool").pop()?.args?.request);
    assert(start?.folders?.images === media.imageDir && start.videos.length === 0, `start_tool 参数：${JSON.stringify(start).slice(0, 300)}`);
    await ui.nav(page, "batch", "视频队列");
    assert((await ui.rows(page)).length === rowsBefore, "拖到工具文件夹输入框的文件不应导入队列");
    return `${r.summary}；${files.join("、")}（${p.width}×${p.height}，${p.duration.toFixed(1)}s）`;
  });

  await scenario(s, "设置 + 同名字幕：自动加载显示 CC，烧录的字幕改变画面", async () => {
    await ui.settingsSwitch(page, "自动加载同名字幕", true);
    await ui.settingsSwitch(page, "文件名加队列序号", true);
    await ui.settingsSwitch(page, "静音预览", true);
    // 输出文件名格式（set.name-template）：输入固定前缀，再用变量按钮把 {name} 插到光标处
    await page.click({ selector: "[role=tab]", text: "转换" });
    const templateInput = { selector: "main input", label: "输出文件名格式" };
    await page.fill(templateInput, "out_");
    await page.click({ selector: "main button", text: "文件名 {name}" });
    await page.waitFor("输出文件名格式 = out_{name}", (spec) => window.__e2e.find(spec)?.value === "out_{name}", [templateInput]);
    await page.waitFor("输出文件名格式的示例", () => document.querySelector("main").innerText.includes("示例：out_示例视频.mp4"));
    await saveShot(s, "settings-name-template");
    await ui.nav(page, "batch", "视频队列");
    // 只留帧率，避免文本模块也画在底部
    await ui.resetRecipe(page);
    await ui.clearQueue(page);
    await ui.dropOnQueue(page, [media.subbed]);
    const rows = await ui.waitRows(page, "带字幕的视频导入", (r) => r.length === 1 && r[0].badges.some((b) => b.text === "CC" && b.title?.startsWith("烧录字幕")));
    const out = w("out", s.mode, "subtitle");
    await ui.setOutputDir(page, out);
    await ui.startBatch(page);
    const summary = await ui.waitBatchFinished(page);
    const files = videoOutputs(out);
    // 文件名按格式 out_{name}，再加队列序号前缀（按队列长度补零，1 个任务时为 “1_”）
    assert(files.length === 1 && /^0*1_out_subbed\.mp4$/.test(files[0]), `输出（应为 序号_out_源文件名）：${files}`);
    const jobs = await page.evaluate(() => window.__e2e.ipc.filter((r) => r.cmd === "start_jobs").pop());
    const startedAt = jobs?.args?.request?.startedAt;
    assert(jobs?.args?.request?.settings?.nameTemplate === "out_{name}" && startedAt?.year >= 2026, `start_jobs 的格式与开始时间：${JSON.stringify(jobs?.args?.request?.settings?.nameTemplate)} ${JSON.stringify(startedAt)}`);
    // 改回默认格式（之后的场景按源文件名检查输出）
    await ui.nav(page, "settings", "设置");
    await page.click({ selector: "[role=tab]", text: "转换" });
    await page.click({ selector: "main button", text: "恢复默认" });
    await page.waitFor("输出文件名格式恢复默认", (spec) => window.__e2e.find(spec)?.value === "{name}", [templateInput]);
    await ui.nav(page, "batch", "视频队列");
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
    await ui.queueAdd(page, "添加文件夹");
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
    // 快捷键 Esc 停止（没有打开的对话框时）
    await page.escape();
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
    // 选中队列行，从操作条的“工具”菜单打开：切到工具箱并选中“进度条”
    await page.click({ selector: "li[data-row-index] p[data-path]", nth: 0 });
    await ui.menu(page, { selector: `${QUEUE} button`, text: "工具" }, "进度条");
    await ui.waitView(page, "toolbox", "工具箱");
    await ui.waitWorkspace(page, "进度条");
    await ui.choose(page, "队列中全部 1 个", WORKSPACE);
    const out = w("out", s.mode, "tool-cancel");
    await ui.setOutputDir(page, out, WORKSPACE);
    await page.click({ selector: `${WORKSPACE} button`, text: "开始" });
    await page.waitFor("工具开始处理", () => /处理中|d+%/.test(document.querySelector("[data-tool-workspace] ul li")?.innerText ?? ""), [], { timeout: 60000 });
    const mid = await page.evaluate(() => document.querySelector("[data-tool-workspace] ul li")?.innerText.replace(/s+/g, " "));
    // 切到批量处理页：工具不中断，队列行上同时显示工具进度；再回到工具箱（仍选中进度条）停止
    await ui.nav(page, "batch", "视频队列");
    const rowTool = await page.waitFor("队列行显示工具进度", () => document.querySelector("li[data-row-index]")?.innerText.match(/进度条s*S*/)?.[0]);
    await ui.nav(page, "toolbox", "工具箱");
    await ui.waitWorkspace(page, "进度条");
    await page.click({ selector: `${WORKSPACE} button`, text: "停止" });
    const summary = await page.waitFor("工具停止", () => [...document.querySelectorAll("[data-tool-workspace] span")].map((x) => x.textContent.trim()).find((t) => t.startsWith("上次：")), [], { timeout: 30000 });
    assert(/取消 1/.test(summary), `工具汇总：${summary}`);
    await sleep(1000);
    assert(listFiles(out).length === 0, `工具取消后输出目录残留：${listFiles(out)}`);
    // 预览：开始生成后马上关闭窗口
    await ui.nav(page, "batch", "视频队列");
    await page.click({ selector: `${STAGE} button`, textIncludes: "生成预览" });
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
  await scenario(s, "重启前：队列与设置已保存到隔离的配置目录；移动窗口", async () => {
    await ui.dropOnQueue(page, [media.extra]);
    await ui.waitRows(page, "extra 导入", (r) => r.some((x) => x.name === "extra.mp4"));
    // 像用户拖动一样移动、缩放窗口（物理像素），重启后应恢复
    const pid = s.appPid();
    moveNativeWindow(pid, 60, 40, 1560, 1000);
    await sleep(800);
    s.windowBefore = nativeWindow(pid);
    assert(s.windowBefore && s.windowBefore.x === 60 && s.windowBefore.width === 1560, `移动窗口：${JSON.stringify(s.windowBefore)}`);
    await sleep(1800); // 队列保存防抖 800ms、设置 300ms
    const queue = JSON.parse(fs.readFileSync(path.join(s.configDir, "queue.json"), "utf8"));
    const settings = JSON.parse(fs.readFileSync(path.join(s.configDir, "settings.json"), "utf8"));
    const queued = (Array.isArray(queue) ? queue : queue.tasks ?? []).map((t) => path.basename(t.path));
    assert(queued.includes("extra.mp4"), `queue.json：${JSON.stringify(queue).slice(0, 300)}`);
    assert(settings.numberPrefix === true && settings.mutePreview === true && settings.autoSubtitle === true, `settings.json：${JSON.stringify(settings).slice(0, 300)}`);
    assert(!("theme" in settings), `settings.json 不应再有 theme：${JSON.stringify(settings).slice(0, 300)}`);
    s.expectQueue = queued;
    return `queue.json：${queued.join("、")}；settings.json：numberPrefix / mutePreview / autoSubtitle = true，没有 theme`;
  });

  log("  重启应用（quit_app）…");
  const oldPids = [...s.appPids];
  await s.stop({ graceful: true });
  // 正常退出时（RunEvent::Exit）应删除本进程的预览目录
  const leftPreview = oldPids.map((pid) => path.join(os.tmpdir(), "cliprecipe-preview", String(pid))).filter((d) => fs.existsSync(d));
  s.appPids.clear();
  // 旧版本（0.1.0）的设置文件里有 theme（浅色）：重启后照常读取、其它设置不丢、首帧仍是深色
  const settingsFile = path.join(s.configDir, "settings.json");
  fs.writeFileSync(settingsFile, JSON.stringify({ ...JSON.parse(fs.readFileSync(settingsFile, "utf8")), theme: "light" }, null, 2));
  await s.start();
  await scenario(s, "正常退出：删除本进程的预览目录", async () => {
    assert(oldPids.length > 0, "没有记录到应用进程号");
    assert(leftPreview.length === 0, `退出后预览目录仍在：${leftPreview}`);
    return `进程 ${oldPids.join("、")} 的预览目录已删除`;
  });

  await scenario(s, "重启后：设置保留（旧设置文件里的浅色主题被忽略）、未完成的任务恢复、配方保留、首帧深色", async () => {
    const loaded = await s.page.invoke("load_settings");
    assert(loaded.ok && loaded.value.numberPrefix && loaded.value.mutePreview && loaded.value.autoSubtitle, `load_settings：${JSON.stringify(loaded).slice(0, 300)}`);
    const rows = await ui.waitRows(s.page, "队列恢复", (r) => (s.expectQueue ?? []).every((n) => r.some((x) => x.name === n)), 20000);
    assert(rows.every((r) => r.summary.includes("×")), "恢复的任务缺少媒体信息");
    const firstFrame = await checkFirstFrame(s.page);
    // 窗口位置与大小（tauri-plugin-window-state，保存在隔离的配置目录）
    const win = nativeWindow(s.appPid());
    const b = s.windowBefore;
    const near = (a, c) => Math.abs(a - c) <= 2;
    assert(win && b && near(win.x, b.x) && near(win.y, b.y) && near(win.width, b.width) && near(win.height, b.height), `窗口位置大小未恢复：${JSON.stringify(b)} → ${JSON.stringify(win)}`);
    const stateFile = path.join(s.configDir, "window-state.json");
    assert(fs.existsSync(stateFile), "window-state.json 不在隔离的配置目录中");
    saveNativeShot(s, "after-restart-dark");
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
    return `恢复 ${rows.length} 个任务（${rows.map((r) => `${r.name} ${r.state}`).join("、")}）；${firstFrame}；窗口 ${win.x},${win.y} ${win.width}×${win.height} 已恢复；设置页显示已保存的开关`;
  });

  await scenario(s, "单实例：再次启动程序时激活已运行的窗口（从最小化恢复），并导入命令行里的文件", async () => {
    const pid = s.appPid();
    minimizeNativeWindow(pid);
    await sleep(600);
    const minimized = nativeWindow(pid);
    assert(minimized?.minimized, `窗口没有最小化：${JSON.stringify(minimized)}`);
    const port = await freePort();
    const second = spawn(s.exe, [media.b], { env: s.env(port), stdio: "ignore", windowsHide: false });
    const exit = await new Promise((resolve) => {
      const timer = setTimeout(() => resolve("timeout"), 20000);
      second.on("exit", (code) => {
        clearTimeout(timer);
        resolve(code);
      });
    });
    if (exit === "timeout") {
      killTree(second.pid);
      throw new Error("第二个实例没有退出（单实例未生效）");
    }
    await ui.waitRows(s.page, "命令行里的文件导入已运行的实例", (r) => r.some((x) => x.name === "bravo_clip.mp4"), 20000);
    await sleep(400);
    const win = nativeWindow(pid);
    assert(win && !win.minimized && win.visible, `已运行的窗口没有恢复：${JSON.stringify(win)}`);
    const running = processesOf(s.exe);
    assert(running.length === 1, `应只有一个程序进程：${running}`);
    return `第二个实例退出（代码 ${exit}）；原窗口从最小化恢复（${win.width}×${win.height}）；bravo_clip.mp4 已导入`;
  });
}

/**
 * 应用内下载 FFmpeg 的真实界面流程（假服务器，不联网）：
 * 隔离会话（CLIPRECIPE_E2E_ISOLATE_FFMPEG=1）不查找程序目录、PATH、node_modules，空的配置目录里也没有下载过的副本，
 * 相当于一台没有 FFmpeg 的电脑；CLIPRECIPE_E2E_FFMPEG_MANIFEST 指向假服务器的清单。这两个环境变量只在
 * `--features e2e-hooks` 构建的调试版中存在（正式构建编译时就没有这些代码，见 ffmpeg/managed.rs 的 compile_error!）。
 */
async function ffmpegDownloadScenarios(exe, media) {
  const { ffmpeg, ffprobe, source } = ffmpegPaths();
  if (source === "PATH") {
    results.push({ mode: "ffmpeg", name: "应用内下载 FFmpeg", status: "skip", details: "需要 node_modules 里的 @seankit 包或 CLIPRECIPE_FFMPEG（用来打包假服务器的 tgz）", artifacts: [], secs: "-" });
    return;
  }
  log("  准备假的 FFmpeg 下载服务器（打包 tgz）…");
  const server = await startFakeFfmpegServer({ dir: w("fake-ffmpeg"), ffmpeg, ffprobe });
  const s = new Session("prod", exe, {
    label: "ffmpeg",
    env: { CLIPRECIPE_E2E_ISOLATE_FFMPEG: "1", CLIPRECIPE_E2E_FFMPEG_MANIFEST: server.manifestPath },
  });
  sessions.push(s);
  try {
    try {
      await s.start();
    } catch (e) {
      results.push({ mode: s.label, name: "启动应用", status: "fail", details: e.message, artifacts: await saveArtifacts(s, "start"), secs: "-" });
      return;
    }
    const { page } = s;
    const managedRoot = path.join(s.configDir, "ffmpeg");
    const BANNER = "[role=region][aria-label='未找到 FFmpeg']";
    const bannerShown = () => page.evaluate((sel) => !!document.querySelector(sel), BANNER);
    const rootEntries = () => (fs.existsSync(managedRoot) ? fs.readdirSync(managedRoot) : []);
    const total = server.sizes.ffmpeg + server.sizes.ffprobe;

    await scenario(s, "缺少 FFmpeg：批量处理页提示一键下载；导入被引导到下载入口；不会自动下载", async () => {
      const st = await page.invoke("ffmpeg_status");
      assert(st.ok && !st.value.available && st.value.source === "missing", `ffmpeg_status：${JSON.stringify(st).slice(0, 400)}`);
      assert(st.value.managed.supported && st.value.managed.downloadSize === total, `managed：${JSON.stringify(st.value.managed)}`);
      await page.waitFor("下载提示", (sel) => !!document.querySelector(sel), [BANNER]);
      const text = await page.evaluate((sel) => document.querySelector(sel).innerText, BANNER);
      assert(/下载 FFmpeg/.test(text) && text.includes("GPL"), `提示内容：${text}`);
      await auditUi(page, "FFmpeg 下载提示", BANNER);
      // 关掉提示后拖入视频：不导入，提示重新出现，处理记录说明原因
      await page.click({ selector: "button", label: "关闭 FFmpeg 提示" });
      await page.waitFor("提示关闭", (sel) => !document.querySelector(sel), [BANNER]);
      await ui.dropOnQueue(page, [media.a]);
      await page.waitFor("提示重新出现", (sel) => !!document.querySelector(sel), [BANNER]);
      const logText = await ui.logText(page);
      assert(logText.includes("未找到可用的 FFmpeg"), `处理记录：${logText.slice(0, 300)}`);
      assert((await ui.rows(page)).length === 0, "没有 FFmpeg 时不应导入");
      const imports = await page.evaluate(() => window.__e2e.ipc.filter((r) => r.cmd === "import_paths").length);
      assert(imports === 0, "不应调用 import_paths");
      // 设置页：未找到 + 下载按钮
      await ui.nav(page, "settings", "设置");
      await page.click({ selector: "[role=tab]", text: "关于" });
      await page.waitFor("设置页显示未找到", (v) => {
        const t = document.querySelector("main").innerText;
        return t.includes("未找到可用的 FFmpeg") && t.includes(`下载 FFmpeg ${v}`);
      }, [server.version]);
      await auditUi(page, "设置 · 关于（缺少 FFmpeg）", "main");
      await saveShot(s, "ffmpeg-missing-settings");
      await ui.nav(page, "batch", "视频队列");
      assert(server.hits.length === 0, `未点击下载时不应有请求：${server.hits}`);
      return `提示：${text.split("\n")[0]}；下载大小 ${(total / 1048576).toFixed(1)} MB；导入被拦截并说明原因`;
    });
    await saveShot(s, "ffmpeg-banner");

    await scenario(s, "下载 FFmpeg：进度与速度显示，取消后临时文件清理", async () => {
      await page.click({ selector: `${BANNER} button`, textIncludes: "下载 FFmpeg" });
      const progress = await page.waitFor("下载进度", () => {
        const el = document.querySelector("[data-testid=ffmpeg-download-progress]");
        return el && /正在下载 ffmpeg：.+ \/ .+/.test(el.innerText) ? el.innerText.replace(/\s+/g, " ") : null;
      }, [], { timeout: 30000, interval: 100 });
      await page.evaluate(() => window.__e2e.audit({ root: "[role=region]" }))
        .then((r) => assert(r.problems.length === 0, `下载进度区审计：${r.problems.join(" | ")}`));
      await saveShot(s, "ffmpeg-downloading");
      await page.click({ selector: `${BANNER} button`, text: "取消" });
      await page.waitFor("取消后回到下载按钮", (sel) => {
        const b = document.querySelector(sel);
        return b && !b.querySelector("[data-testid=ffmpeg-download-progress]") && [...b.querySelectorAll("button")].some((x) => x.textContent.includes("下载 FFmpeg"));
      }, [BANNER], { timeout: 60000 });
      const left = rootEntries();
      assert(left.length === 0, `取消后不应留下文件：${left}`);
      const logText = await ui.logText(page);
      assert(logText.includes("已取消下载 FFmpeg"), "处理记录没有“已取消”");
      return `取消前：${progress}；取消后下载目录为空`;
    });

    await scenario(s, "下载 FFmpeg：第一个地址失败换下一个，校验后立即使用（不重启）", async () => {
      server.hits.length = 0;
      const started = Date.now();
      await page.click({ selector: `${BANNER} button`, textIncludes: "下载 FFmpeg" });
      await page.waitFor("下载完成、提示消失", (sel) => !document.querySelector(sel), [BANNER], { timeout: 240000, interval: 300 });
      const secs = ((Date.now() - started) / 1000).toFixed(1);
      const st = await page.invoke("ffmpeg_status");
      assert(st.ok && st.value.available && st.value.source === "managed", `ffmpeg_status：${JSON.stringify(st).slice(0, 400)}`);
      assert(st.value.managed.installedVersion === server.version && !st.value.managed.updateAvailable, `managed：${JSON.stringify(st.value.managed)}`);
      const receipt = JSON.parse(fs.readFileSync(path.join(managedRoot, server.version, "receipt.json"), "utf8"));
      assert(receipt.tools.map((t) => t.url.replace(/^.*\/(\w+\/\w+\.tgz)$/, "$1")).join(",") === "npm/ffmpeg.tgz,npm/ffprobe.tgz", `安装记录：${JSON.stringify(receipt)}`);
      assert(server.hits.join(",") === "/mirror/ffmpeg.tgz,/npm/ffmpeg.tgz,/mirror/ffprobe.tgz,/npm/ffprobe.tgz", `请求：${server.hits}`);
      assert(rootEntries().join(",") === server.version, `下载目录：${rootEntries()}`);
      // 不重启：导入马上可用
      await ui.dropOnQueue(page, [media.a]);
      await ui.waitRows(page, "下载后导入", (r) => r.length === 1 && r[0].summary.includes("640×360"));
      await ui.nav(page, "settings", "设置");
      await page.click({ selector: "[role=tab]", text: "关于" });
      await page.waitFor("设置页显示应用内下载", (v) => {
        const t = document.querySelector("main").innerText;
        return t.includes("来源：应用内下载") && t.includes(`已下载 FFmpeg ${v}`);
      }, [server.version], { timeout: 30000 });
      // 换了 FFmpeg 后设置页重新探测硬件编码器（试编码结束前删除会因为文件占用而被拒绝）
      await page.waitFor("硬件编码器检测完成", () => document.querySelector("main").innerText.includes("开启 GPU 加速时使用"), [], { timeout: 120000 });
      await saveShot(s, "ffmpeg-installed-settings");
      return `${secs}s；${st.value.version}；${st.value.path}；请求顺序 ${server.hits.join(" → ")}`;
    });

    await scenario(s, "删除已下载的 FFmpeg（确认对话框），回到缺少 FFmpeg 的状态", async () => {
      await page.click({ selector: "main button", textIncludes: "删除已下载的 FFmpeg" });
      await page.click({ selector: "[role=alertdialog] button", text: "删除" });
      await page.waitFor("设置页显示未找到", () => document.querySelector("main").innerText.includes("未找到可用的 FFmpeg"), [], { timeout: 30000 });
      assert(!fs.existsSync(path.join(managedRoot, server.version)), "版本目录应已删除");
      await ui.nav(page, "batch", "视频队列");
      await page.waitFor("下载提示重新出现", (sel) => !!document.querySelector(sel), [BANNER]);
      return `已删除 ${path.join(managedRoot, server.version)}`;
    });
    await s.stop({ graceful: true });
  } finally {
    await server.close();
  }
}

// ------------------------------------------------------------------ 开发环境的调试面板（--mode dev --debug-panel）

/** 调试面板（src/debug/，只在开发环境存在）的界面操作。 */
const panel = {
  async open(page) {
    const open = await page.evaluate(() => !!document.querySelector("[data-slot=sheet-content]"));
    if (!open) await page.evaluate(() => document.querySelector('[aria-label="调试面板"]').click());
    await page.waitFor("调试面板打开", () => !!document.querySelector("[data-slot=sheet-content] [role=switch]"));
  },
  async close(page) {
    await page.evaluate(() => document.querySelector("[data-slot=sheet-content] [data-slot=sheet-close]")?.click());
    await page.waitFor("调试面板关闭", () => !document.querySelector("[role=dialog]"));
  },
  /** 打开 / 关闭一项状态模拟（按开关旁的标题）。 */
  async sim(page, label, on) {
    await panel.open(page);
    await page.evaluate(
      (label, on) => {
        const sw = [...document.querySelectorAll("[data-slot=sheet-content] [role=switch]")].find((s) =>
          s.closest("[data-slot=field]").textContent.includes(label),
        );
        if ((sw.getAttribute("aria-checked") === "true") !== on) sw.click();
      },
      label,
      on,
    );
    await page.waitFor(
      `模拟“${label}”${on ? "打开" : "关闭"}`,
      (label, on) =>
        [...document.querySelectorAll("[data-slot=sheet-content] [role=switch]")]
          .find((s) => s.closest("[data-slot=field]").textContent.includes(label))
          .getAttribute("aria-checked") === String(on),
      [label, on],
    );
  },
  /** 点面板里文字以 text 开头的按钮（平台选项，“全部恢复默认”）。 */
  async press(page, text) {
    await panel.open(page);
    const ok = await page.evaluate(
      (text) => {
        const b = [...document.querySelectorAll("[data-slot=sheet-content] button")].find((x) => x.textContent.trim().startsWith(text));
        b?.click();
        return !!b;
      },
      text,
    );
    assert(ok, `调试面板里没有按钮“${text}”`);
  },
};

const ipcCount = (page) => page.evaluate(() => window.__e2e.ipc.length);
const ipcSince = (page, n) => page.evaluate((n) => window.__e2e.ipc.slice(n).map((r) => ({ cmd: r.cmd, args: JSON.stringify(r.args ?? null) })), n);

/**
 * 调试面板在真实的桌面应用（tauri dev）里不改动、不污染真实数据：模拟的队列行不写进 queue.json、对模拟数据的操作不发给后端、
 * 重启后没有“文件缺失”；FFmpeg 模拟时下载 / 取消 / 重新检测不调用后端；“全部恢复默认”不改设置；模拟非 Windows 平台时标题栏恢复系统默认。
 */
async function debugPanelScenarios(s, media) {
  const queueFile = path.join(s.configDir, "queue.json");
  const settingsFile = path.join(s.configDir, "settings.json");
  const FAKE = /示例视频|debug-sim-/;

  await scenario(s, "调试面板：队列模拟不写进 queue.json、对模拟行的操作不发给后端，重启后没有文件缺失警告", async () => {
    let page = s.page;
    await ui.clearQueue(page);
    await ui.dropOnQueue(page, [media.a]);
    await ui.waitRows(page, "导入一个真实视频", (r) => r.length === 1);
    const n0 = await ipcCount(page);
    await panel.sim(page, "队列里各种状态的任务", true);
    await panel.close(page);
    await ui.waitRows(page, "注入 10 个模拟行", (r) => r.length === 11);
    await sleep(1500); // 队列保存的防抖是 800 ms
    let saved = fs.readFileSync(queueFile, "utf8");
    assert(saved.includes(path.basename(media.a)), `queue.json 应保存真实的视频：${saved.slice(0, 300)}`);
    assert(!FAKE.test(saved), `queue.json 含模拟行：${saved.slice(0, 300)}`);

    // 停止：模拟的处理中 / 等待行在前端取消
    await page.evaluate(() => [...document.querySelectorAll("main footer button")].find((b) => b.textContent.includes("停止")).click());
    await ui.waitRows(page, "模拟行全部取消", (r) => r.length === 11 && !r.some((x) => /处理中|等待/.test(x.state)));
    // 播放模拟行：命令在前端被拒绝，界面说明是模拟数据
    await page.evaluate(() =>
      [...document.querySelectorAll("li[data-row-index]")]
        .find((li) => li.innerText.includes("示例视频 03"))
        .dispatchEvent(new MouseEvent("dblclick", { bubbles: true })),
    );
    await page.waitFor("播放模拟行时说明是模拟数据", () => document.querySelector("[role=dialog]")?.innerText.includes("调试面板模拟的数据"));
    await page.escape();
    await page.waitFor("预览关闭", () => !document.querySelector("[role=dialog]"));
    const sent = (await ipcSince(page, n0)).filter(
      (r) => FAKE.test(r.args) || ["stop_jobs", "cancel_task", "start_jobs", "prioritize_task"].includes(r.cmd),
    );
    assert(sent.length === 0, `模拟数据被发给了后端：${JSON.stringify(sent).slice(0, 600)}`);

    // 模拟开着时退出、再启动：模拟按 localStorage 重新打开，真实视频从 queue.json 恢复，没有“已不存在”
    await s.stop({ graceful: true });
    saved = fs.readFileSync(queueFile, "utf8");
    assert(!FAKE.test(saved), `退出后 queue.json 含模拟行：${saved.slice(0, 300)}`);
    await s.start();
    page = s.page;
    await ui.waitRows(page, "重启后：真实视频恢复 + 模拟重新打开", (r) => r.length === 11 && r.some((x) => x.name === path.basename(media.a)));
    const text = await page.evaluate(() => document.body.innerText);
    assert(!/已不存在|无法恢复/.test(text), `重启后出现缺失警告：${text.match(/.*(已不存在|无法恢复).*/)?.[0]}`);
    await panel.sim(page, "队列里各种状态的任务", false);
    await panel.close(page);
    await ui.waitRows(page, "关闭模拟后只剩真实视频", (r) => r.length === 1 && r[0].name === path.basename(media.a));
    return `queue.json 只有 ${path.basename(media.a)}；停止 / 播放模拟行没有发出后端命令；重启后没有缺失警告`;
  });

  await scenario(s, "调试面板：FFmpeg 模拟时下载 / 取消 / 重新检测不调用后端", async () => {
    const page = s.page;
    const n0 = await ipcCount(page);
    await panel.sim(page, "缺少 FFmpeg", true);
    await panel.close(page);
    await page.waitFor("未找到 FFmpeg 提示", () => !!document.querySelector('[aria-label="未找到 FFmpeg"]'));
    await page.evaluate(() =>
      [...document.querySelectorAll('[aria-label="未找到 FFmpeg"] button')].find((b) => b.textContent.includes("下载 FFmpeg")).click(),
    );
    await page.waitFor("模拟的下载进度", () => !!document.querySelector("[data-testid=ffmpeg-download-progress]"));
    await page.evaluate(() =>
      [...document.querySelectorAll("[data-testid=ffmpeg-download-progress] button")].find((b) => b.textContent.includes("取消")).click(),
    );
    await page.waitFor("取消后下载进度消失", () => !document.querySelector("[data-testid=ffmpeg-download-progress]"));
    await ui.nav(page, "settings", "设置");
    await page.click({ selector: "[role=tab]", text: "关于" });
    await page.waitFor("运行环境显示未找到", () => document.querySelector("main").innerText.includes("未找到可用的 FFmpeg"));
    await page.click({ selector: "button", textIncludes: "重新检测 FFmpeg", within: "main" });
    await sleep(500);
    await ui.nav(page, "batch", "视频队列");
    const sent = (await ipcSince(page, n0)).filter((r) =>
      ["download_ffmpeg", "cancel_ffmpeg_download", "remove_managed_ffmpeg", "redetect_ffmpeg"].includes(r.cmd),
    );
    assert(sent.length === 0, `模拟时调用了后端：${JSON.stringify(sent)}`);
    await panel.sim(page, "缺少 FFmpeg", false);
    await panel.close(page);
    await page.waitFor("关闭模拟后提示消失", () => !document.querySelector('[aria-label="未找到 FFmpeg"]'));
    return "下载、取消、重新检测都只在前端模拟";
  });

  await scenario(s, "调试面板：“全部恢复默认”只恢复面板自己的选择，不改设置", async () => {
    const page = s.page;
    await sleep(800); // 设置保存的防抖是 300 ms
    const before = fs.readFileSync(settingsFile, "utf8");
    await panel.press(page, "macOS");
    await page.waitFor("模拟 macOS", () => document.documentElement.dataset.platform === "macos");
    await panel.sim(page, "处理记录里各级别日志", true);
    await panel.press(page, "全部恢复默认");
    await page.waitFor("平台恢复为真实的 Windows", () => document.documentElement.dataset.platform === "windows");
    await sleep(800);
    const state = await page.evaluate(() => ({
      stored: localStorage.getItem("cliprecipe.debug-panel"),
      switches: [...document.querySelectorAll("[data-slot=sheet-content] [role=switch]")].map((s) => s.getAttribute("aria-checked")),
    }));
    assert(fs.readFileSync(settingsFile, "utf8") === before, `恢复默认改了 settings.json：${before.slice(0, 300)} → ${fs.readFileSync(settingsFile, "utf8").slice(0, 300)}`);
    assert(state.stored === null && state.switches.every((v) => v === "false"), JSON.stringify(state));
    await panel.close(page);
    return "settings.json 未变，模拟全部关闭，面板的 localStorage 已清除";
  });

  await scenario(s, "调试面板：模拟 macOS / Linux 时标题栏恢复系统默认颜色，切回 Windows 重新着色", async () => {
    const page = s.page;
    const pid = s.appPid();
    // 看系统标题栏实际显示的像素（Windows 不能读回 DWM 的标题栏颜色属性）
    const color = async (name) => {
      await sleep(800);
      const file = path.join(REPORT, `dev-window-${name}.png`);
      const rgb = await titlebarColor(pid, file);
      (s.shots ??= []).push(file);
      return rgb;
    };
    const near = (a, b, tol = 3) => a.every((v, i) => Math.abs(v - b[i]) <= tol);
    const hex = (c) => `#${c.map((v) => v.toString(16).padStart(2, "0")).join("")}`;
    // --titlebar 换算成 sRGB
    const expected = await page.evaluate(() => {
      const probe = document.createElement("span");
      probe.style.color = "var(--titlebar)";
      document.body.append(probe);
      const css = getComputedStyle(probe).color;
      probe.remove();
      const ctx = document.createElement("canvas").getContext("2d");
      ctx.fillStyle = css;
      ctx.fillRect(0, 0, 1, 1);
      return [...ctx.getImageData(0, 0, 1, 1).data.slice(0, 3)];
    });
    const windows = await color("windows-colored");
    assert(near(windows, expected), `Windows 下标题栏应是主题的 --titlebar ${hex(expected)}，实际 ${hex(windows)}`);
    const seen = [];
    for (const name of ["macOS", "Linux"]) {
      await panel.press(page, name);
      const c = await color(`${name.toLowerCase()}-default`);
      seen.push(`${name} ${hex(c)}`);
      assert(!near(c, expected, 4), `模拟 ${name} 时标题栏应恢复系统默认颜色，实际仍是主题色 ${hex(c)}`);
    }
    await panel.press(page, "真实");
    const back = await color("windows-recolored");
    assert(near(back, expected), `切回 Windows 后标题栏应重新着色为 ${hex(expected)}，实际 ${hex(back)}`);
    await panel.close(page);
    return `Windows ${hex(windows)}（= --titlebar）→ ${seen.join("、")}（系统默认）→ 切回 ${hex(back)}`;
  });
}

async function saveShot(s, label) {
  if (!s.page) return;
  try {
    const file = path.join(REPORT, `${s.label}-shot-${label}.png`);
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
  // 单实例测试改用的应用标识（com.xiaofany.cliprecipe.e2e）可能让 Tauri / 插件建了空的系统数据目录
  for (const base of [process.env.APPDATA, process.env.LOCALAPPDATA]) {
    const dir = base && path.join(base, "com.xiaofany.cliprecipe.e2e");
    if (dir && fs.existsSync(dir)) await removeDir(dir);
  }
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
  // e2e-hooks：只用于本测试的调试构建，允许用环境变量换成假服务器的 FFmpeg 下载清单（见 e2e-ffmpeg-server.mjs）
  log("构建生产形态（tauri build --debug --no-bundle --features e2e-hooks：内嵌前端、正式 CSP）…");
  execSync("npm run tauri -- build --debug --no-bundle --features e2e-hooks", {
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
  // 生产形态的主会话验证单实例（e2e-hooks：改用专用的应用标识，不会激活用户正在运行的 ClipRecipe）
  const s = new Session(mode, exe, mode === "prod" ? { env: { CLIPRECIPE_E2E_SINGLE_INSTANCE: "1" } } : {});
  sessions.push(s);
  try {
    await s.start();
  } catch (e) {
    results.push({ mode, name: "启动应用", status: "fail", details: e.message, artifacts: await saveArtifacts(s, "start"), secs: "-" });
    log(`✘ [${mode}] 启动应用：${e.message}`);
    await s.stop({ graceful: false });
    return;
  }
  if (mode === "dev" && opt("--debug-panel")) {
    // 只跑调试面板的场景（面板只在开发环境存在）：npm run e2e:debug-panel
    await debugPanelScenarios(s, media);
    await s.stop({ graceful: true });
    return;
  }
  await bootScenarios(s, media);
  await batchScenarios(s, media);
  if (mode === "prod") await persistenceScenarios(s, media);
  await saveShot(s, "final");
  await s.stop({ graceful: true });
  if (mode === "prod" && !SKIP?.test("ffmpeg")) await ffmpegDownloadScenarios(exe, media);
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
  if (auditWarnings.length) {
    // 按规则汇总降级为警告的审计结果：次数与实测值范围（每条消息里的第一个比值 / 不透明度）
    console.log(`\n界面审计警告（暂时降级的规则，见 scripts/e2e-audit-page.js 的 SOFT_RULES）：共 ${auditWarnings.length} 条`);
    const byRule = new Map();
    for (const w of auditWarnings) {
      const rule = /^\[([\w-]+)\]/.exec(w.text)?.[1] ?? "?";
      const value = Number(/(\d+(?:\.\d+)?)(?: <|，|\/255)/.exec(w.text)?.[1]);
      const entry = byRule.get(rule) ?? { count: 0, values: [], sample: w };
      entry.count++;
      if (Number.isFinite(value)) entry.values.push(value);
      byRule.set(rule, entry);
    }
    for (const [rule, e] of byRule) {
      const range = e.values.length ? `实测 ${Math.min(...e.values)} ~ ${Math.max(...e.values)}` : "";
      console.log(`  ${rule}：${e.count} 条，${range}；例：（${e.sample.label}）${e.sample.text}`);
    }
    fs.writeFileSync(path.join(REPORT, "audit-warnings.json"), JSON.stringify(auditWarnings, null, 2));
  }
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
