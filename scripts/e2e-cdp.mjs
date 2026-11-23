// 端到端测试：最小的 Chrome DevTools Protocol 客户端（无第三方依赖）与页面操作封装。
// WebView2 以 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<端口> 启动后，
// 通过 http://127.0.0.1:<端口>/json/list 找到页面，再用 WebSocket 连接。
// 只连 127.0.0.1，用 node:http（不读 HTTP_PROXY），绕开系统代理。

import fs from "node:fs";
import http from "node:http";
import path from "node:path";

/** GET http://127.0.0.1:port/path，解析 JSON。 */
export function getJson(port, path) {
  return new Promise((resolve, reject) => {
    const req = http.get({ host: "127.0.0.1", port, path, timeout: 2000, agent: false }, (res) => {
      let body = "";
      res.setEncoding("utf8");
      res.on("data", (chunk) => (body += chunk));
      res.on("end", () => {
        try {
          resolve(JSON.parse(body));
        } catch (e) {
          reject(e);
        }
      });
    });
    req.on("error", reject);
    req.on("timeout", () => req.destroy(new Error("timeout")));
  });
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** 等待 DevTools 端口上出现页面（url 满足 accept）。 */
export async function waitForPage(port, accept, timeoutMs, isAlive = () => true) {
  const deadline = Date.now() + timeoutMs;
  let last = "";
  while (Date.now() < deadline) {
    if (!isAlive()) throw new Error("应用进程已退出或编译失败（见应用日志）");
    try {
      const list = await getJson(port, "/json/list");
      const page = list.find((t) => t.type === "page" && accept(t.url));
      if (page) return page;
      last = list.map((t) => `${t.type}:${t.url}`).join(", ");
    } catch (e) {
      last = String(e);
    }
    await sleep(500);
  }
  throw new Error(`等待 WebView2 调试页面超时（最后：${last}）`);
}

export class Cdp {
  /** @param {string} wsUrl */
  static connect(wsUrl) {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(wsUrl);
      ws.addEventListener("open", () => resolve(new Cdp(ws)), { once: true });
      ws.addEventListener("error", (e) => reject(new Error(`CDP 连接失败：${e.message ?? e.type}`)), { once: true });
    });
  }

  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    this.handlers = new Map();
    this.closed = false;
    ws.addEventListener("message", (e) => this.#onMessage(e.data));
    ws.addEventListener("close", () => {
      this.closed = true;
      for (const { reject } of this.pending.values()) reject(new Error("CDP 连接已关闭"));
      this.pending.clear();
    });
  }

  #onMessage(data) {
    const msg = JSON.parse(typeof data === "string" ? data : Buffer.from(data).toString("utf8"));
    if (msg.id !== undefined) {
      const p = this.pending.get(msg.id);
      if (!p) return;
      this.pending.delete(msg.id);
      clearTimeout(p.timer);
      if (msg.error) p.reject(new Error(`${p.method}: ${msg.error.message}`));
      else p.resolve(msg.result);
      return;
    }
    for (const fn of this.handlers.get(msg.method) ?? []) fn(msg.params);
  }

  send(method, params = {}, timeoutMs = 60000) {
    if (this.closed) return Promise.reject(new Error("CDP 连接已关闭"));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} 超时`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, method, timer });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  on(method, fn) {
    if (!this.handlers.has(method)) this.handlers.set(method, []);
    this.handlers.get(method).push(fn);
  }

  close() {
    try {
      this.ws.close();
    } catch {
      // 忽略
    }
  }
}

/**
 * 注入到每个文档的测试探针（Page.addScriptToEvaluateOnNewDocument）：
 * - 记录 CSP 违规（securitypolicyviolation）、未捕获异常、未处理的 Promise 拒绝；
 * - 包一层 window.fetch，记录每次 IPC（http://ipc.localhost/<命令>）的命令、参数与是否成功；
 * - 系统文件对话框（plugin:dialog|open / save）无法自动操作：测试预先排队的返回值代替对话框结果，
 *   没有排队时返回 null（等同用户取消）并记为“意外的对话框”，绝不弹出真正的对话框。
 * Tauri 把 __TAURI_INTERNALS__.invoke / ipc 定义为只读，所以只能在 fetch 这一层截取。
 * - find()：按选择器 / 文本 / aria-label 找可见元素，供测试点击。
 */
export const PROBE_SCRIPT = String.raw`
(() => {
  if (window.__e2e) return;
  const e2e = (window.__e2e = { csp: [], errors: [], ipc: [], dialogQueue: [], unexpectedDialogs: [] });
  document.addEventListener("securitypolicyviolation", (e) => {
    e2e.csp.push({ directive: e.violatedDirective, blocked: e.blockedURI, source: e.sourceFile, line: e.lineNumber, sample: e.sample });
  });
  window.addEventListener("error", (e) => {
    e2e.errors.push({ kind: "error", message: String(e.message || (e.target && e.target.src) || e), source: e.filename, line: e.lineno });
  }, true);
  window.addEventListener("unhandledrejection", (e) => {
    e2e.errors.push({ kind: "unhandledrejection", message: String(e.reason && (e.reason.stack || e.reason.message) || e.reason) });
  });
  // 桌面体验：按键 / 右键 / 滚轮 / 鼠标侧键事件最终是否被取消了默认行为（window 冒泡阶段最后执行，看到的是最终结果）
  e2e.events = [];
  for (const type of ["keydown", "contextmenu", "wheel", "mouseup"]) {
    window.addEventListener(type, (e) => {
      e2e.events.push({ type, key: e.key, code: e.code, ctrl: e.ctrlKey, shift: e.shiftKey, alt: e.altKey, button: e.button, prevented: e.defaultPrevented, target: e.target && e.target.tagName });
    }, { passive: true });
  }
  // 首帧的主题与底色，以及之后 data-theme 的每次变化（检查启动时没有先浅后深的闪烁）
  e2e.firstFrame = null;
  e2e.themeHistory = [];
  requestAnimationFrame(() => {
    const root = document.documentElement;
    e2e.firstFrame = {
      theme: root ? root.dataset.theme || null : null,
      background: root ? getComputedStyle(root).backgroundColor : null,
      bodyBackground: document.body ? getComputedStyle(document.body).backgroundColor : null,
      boot: window.__CLIPRECIPE_BOOT__ || null,
    };
  });
  new MutationObserver(() => {
    e2e.themeHistory.push({ t: Math.round(performance.now()), theme: document.documentElement.dataset.theme || null, frame: !!e2e.firstFrame });
  }).observe(document, { attributes: true, subtree: true, attributeFilter: ["data-theme"] });
  const IPC = /^https?:\/\/ipc\.localhost\//;
  const realFetch = window.fetch;
  window.fetch = async function (input, init) {
    const url = typeof input === "string" ? input : input && input.url;
    if (!url || !IPC.test(url)) return realFetch.apply(this, arguments);
    const cmd = decodeURIComponent(url.replace(IPC, ""));
    let args = null;
    try {
      args = init && typeof init.body === "string" ? JSON.parse(init.body) : null;
    } catch {}
    const rec = { cmd, args, t: Date.now(), ok: null, error: null };
    e2e.ipc.push(rec);
    if (cmd === "plugin:dialog|open" || cmd === "plugin:dialog|save" || cmd === "plugin:dialog|message" || cmd === "plugin:dialog|ask" || cmd === "plugin:dialog|confirm") {
      let value = null;
      if (e2e.dialogQueue.length > 0) value = e2e.dialogQueue.shift();
      else e2e.unexpectedDialogs.push({ cmd, args });
      rec.ok = true;
      rec.stubbed = value;
      return new Response(JSON.stringify(value), { headers: { "Content-Type": "application/json", "Tauri-Response": "ok" } });
    }
    const res = await realFetch.apply(this, arguments);
    rec.ok = res.headers.get("Tauri-Response") === "ok";
    if (!rec.ok) {
      try { rec.error = await res.clone().text(); } catch {}
    }
    return res;
  };

  const visible = (el) => {
    if (!el || !el.isConnected) return false;
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) return false;
    const s = getComputedStyle(el);
    return s.visibility !== "hidden" && s.display !== "none";
  };
  const text = (el) => (el.textContent || "").replace(/\s+/g, " ").trim();
  /** spec: { selector, within, text, textIncludes, textStarts, label, title, nth, includeHidden, enabledOnly } */
  e2e.findAll = (spec) => {
    let roots = [document];
    if (spec.within) roots = [...document.querySelectorAll(spec.within)].filter(visible);
    let els = [];
    for (const root of roots) els.push(...root.querySelectorAll(spec.selector || "*"));
    if (spec.label !== undefined) els = els.filter((e) => e.getAttribute("aria-label") === spec.label);
    if (spec.title !== undefined) els = els.filter((e) => e.getAttribute("title") === spec.title);
    if (spec.text !== undefined) els = els.filter((e) => text(e) === spec.text);
    if (spec.textIncludes !== undefined) els = els.filter((e) => text(e).includes(spec.textIncludes));
    if (spec.textStarts !== undefined) els = els.filter((e) => text(e).startsWith(spec.textStarts));
    if (!spec.includeHidden) els = els.filter(visible);
    if (spec.enabledOnly) els = els.filter((e) => !e.disabled && e.getAttribute("aria-disabled") !== "true" && !e.hasAttribute("data-disabled"));
    return els;
  };
  e2e.find = (spec) => {
    const els = e2e.findAll(spec);
    const nth = spec.nth ?? 0;
    return (nth < 0 ? els[els.length + nth] : els[nth]) || null;
  };
  e2e.text = text;
  e2e.visible = visible;
})();
`;

/** 界面状态审计（scripts/e2e-audit-page.js，在探针之后注入，定义 window.__e2e.audit / focusRing）。 */
export const AUDIT_SCRIPT = fs.readFileSync(path.join(import.meta.dirname, "e2e-audit-page.js"), "utf8");

/** 页面操作：求值、等待、真实鼠标 / 键盘输入（经 CDP Input 域，Radix 的 pointerdown / mousedown 也能触发）。 */
export class Page {
  constructor(cdp) {
    this.cdp = cdp;
    this.console = [];
  }

  async init() {
    const { cdp } = this;
    cdp.on("Runtime.consoleAPICalled", (p) => {
      const text = p.args.map((a) => (a.value !== undefined ? String(a.value) : a.description ?? a.type)).join(" ");
      this.console.push({ t: Date.now(), kind: `console.${p.type}`, text });
    });
    cdp.on("Runtime.exceptionThrown", (p) => {
      const d = p.exceptionDetails;
      this.console.push({ t: Date.now(), kind: "exception", text: d.exception?.description ?? d.text });
    });
    cdp.on("Log.entryAdded", (p) => {
      const e = p.entry;
      this.console.push({ t: Date.now(), kind: `log.${e.source}.${e.level}`, text: `${e.text}${e.url ? ` (${e.url})` : ""}` });
    });
    await cdp.send("Runtime.enable");
    await cdp.send("Log.enable");
    await cdp.send("Page.enable");
    await cdp.send("Page.addScriptToEvaluateOnNewDocument", { source: PROBE_SCRIPT });
    await cdp.send("Page.addScriptToEvaluateOnNewDocument", { source: AUDIT_SCRIPT });
  }

  /** 重新加载页面，使探针从文档开始就生效。 */
  async reload() {
    const loaded = new Promise((resolve) => {
      const done = () => resolve();
      this.cdp.on("Page.loadEventFired", done);
    });
    await this.cdp.send("Page.reload", { ignoreCache: true });
    await Promise.race([loaded, sleep(20000)]);
    await this.evaluate(PROBE_SCRIPT);
    await this.evaluate(AUDIT_SCRIPT);
  }

  /** 在页面中执行 fn(...args)（fn 会被序列化，不能引用外部变量）。 */
  async evaluate(fn, ...args) {
    const expression =
      typeof fn === "string" ? fn : `(${fn.toString()})(...${JSON.stringify(args)})`;
    const res = await this.cdp.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
      userGesture: true,
    });
    if (res.exceptionDetails) {
      const d = res.exceptionDetails;
      throw new Error(`页面脚本出错：${d.exception?.description ?? d.text}`);
    }
    return res.result.value;
  }

  /** 调用后端命令（与前端 invoke 走同一条 IPC）。 */
  invoke(cmd, args = {}) {
    return this.evaluate(
      (c, a) =>
        window.__TAURI_INTERNALS__.invoke(c, a).then(
          (value) => ({ ok: true, value }),
          (error) => ({ ok: false, error: String(error?.message ?? error) }),
        ),
      cmd,
      args,
    );
  }

  /** 轮询直到 fn(...args) 为真值，返回该值。 */
  async waitFor(what, fn, args = [], { timeout = 15000, interval = 200 } = {}) {
    const deadline = Date.now() + timeout;
    let last;
    let lastError;
    while (Date.now() < deadline) {
      try {
        last = await this.evaluate(fn, ...args);
        if (last) return last;
      } catch (e) {
        lastError = e;
      }
      await sleep(interval);
    }
    throw new Error(`等待超时：${what}${lastError ? `（${lastError.message}）` : ""}`);
  }

  /** 找元素并返回其中心点（视口坐标）；找不到时返回 null。 */
  async locate(spec) {
    return this.evaluate((s) => {
      const el = window.__e2e.find(s);
      if (!el) return null;
      el.scrollIntoView({ block: "center", inline: "center" });
      const r = el.getBoundingClientRect();
      const x = r.left + r.width / 2;
      const y = r.top + r.height / 2;
      // 可点击：中心点最上层的元素就是它（或它的子元素）——对话框关闭动画的遮罩还在时不算
      const top = document.elementFromPoint(x, y);
      return {
        x,
        y,
        disabled: !!(el.disabled || el.getAttribute("aria-disabled") === "true" || el.hasAttribute("data-disabled")),
        covered: !(top && (el === top || el.contains(top))),
        text: window.__e2e.text(el).slice(0, 80),
      };
    }, spec);
  }

  async waitLocate(spec, timeout = 15000, { enabled = true } = {}) {
    const deadline = Date.now() + timeout;
    let pos = null;
    while (Date.now() < deadline) {
      pos = await this.locate(spec);
      if (pos && (!enabled || (!pos.disabled && !pos.covered))) return pos;
      await sleep(150);
    }
    const why = !pos ? "找不到" : pos.disabled ? "元素不可用" : "元素被遮挡";
    throw new Error(`${why}：${JSON.stringify(spec)}`);
  }

  async mouse(type, x, y, clickCount = 1) {
    await this.cdp.send("Input.dispatchMouseEvent", { type, x, y, button: "left", buttons: type === "mousePressed" ? 1 : 0, clickCount, pointerType: "mouse" });
  }

  async clickAt(x, y, count = 1) {
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y, button: "none", pointerType: "mouse" });
    for (let i = 1; i <= count; i++) {
      await this.mouse("mousePressed", x, y, i);
      await this.mouse("mouseReleased", x, y, i);
    }
  }

  /** 点击元素（真实鼠标事件）。 */
  async click(spec, { timeout = 15000, count = 1 } = {}) {
    const pos = await this.waitLocate(spec, timeout);
    await this.clickAt(pos.x, pos.y, count);
    await sleep(120);
    return pos;
  }

  /** 按住 from 元素中心拖到 to 元素（dy 为相对 to 中心的纵向偏移），分步移动。 */
  async drag(fromSpec, toSpec, dy = 0) {
    const from = await this.waitLocate(fromSpec);
    const to = await this.waitLocate(toSpec, 15000, { enabled: false });
    const ty = to.y + dy;
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: from.x, y: from.y, button: "none", pointerType: "mouse" });
    await this.mouse("mousePressed", from.x, from.y);
    for (let i = 1; i <= 8; i++) {
      const x = from.x + ((to.x - from.x) * i) / 8;
      const y = from.y + ((ty - from.y) * i) / 8;
      await this.cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y, button: "left", buttons: 1, pointerType: "mouse" });
      await sleep(30);
    }
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: to.x, y: ty, button: "left", buttons: 0, clickCount: 1, pointerType: "mouse" });
    await sleep(150);
  }

  /** 右键点击元素（contextmenu）。 */
  async rightClick(spec) {
    const { x, y } = await this.waitLocate(spec);
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y, button: "none", pointerType: "mouse" });
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "right", buttons: 2, clickCount: 1, pointerType: "mouse" });
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "right", buttons: 0, clickCount: 1, pointerType: "mouse" });
    await sleep(150);
  }

  async key(key, code = key, keyCode = 0, modifiers = 0) {
    const base = { key, code, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode, modifiers };
    await this.cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
    await this.cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
  }

  escape() {
    return this.key("Escape", "Escape", 27);
  }

  /** 鼠标滚轮（modifiers：1 Alt、2 Ctrl、4 Meta、8 Shift）。 */
  async wheel(x, y, deltaY, modifiers = 0) {
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mouseWheel", x, y, deltaX: 0, deltaY, modifiers, pointerType: "mouse" });
  }

  /** 在坐标处右键（不经过元素查找，用于空白区域）。 */
  async rightClickAt(x, y) {
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y, button: "none", pointerType: "mouse" });
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "right", buttons: 2, clickCount: 1, pointerType: "mouse" });
    await this.cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "right", buttons: 0, clickCount: 1, pointerType: "mouse" });
    await sleep(150);
  }

  enter() {
    return this.key("Enter", "Enter", 13);
  }

  /** 点击输入框、全选、输入文字并回车。 */
  async fill(spec, value) {
    await this.click(spec);
    await this.key("a", "KeyA", 65, 2); // Ctrl+A
    await this.cdp.send("Input.insertText", { text: String(value) });
    await this.enter();
    await sleep(150);
  }

  async screenshot() {
    const { data } = await this.cdp.send("Page.captureScreenshot", { format: "png" }, 20000);
    return Buffer.from(data, "base64");
  }

  /** 预先排队系统文件对话框的返回值（路径、路径数组或 null）。 */
  queueDialog(value) {
    return this.evaluate((v) => window.__e2e.dialogQueue.push(v), value);
  }

  /** 模拟 Tauri 的系统拖放事件（tauri://drag-enter / over / drop），坐标为目标元素中心。 */
  async dropFiles(paths, spec) {
    const pos = await this.waitLocate(spec, 10000, { enabled: false });
    return this.evaluate(
      async (p, x, y) => {
        const ratio = window.devicePixelRatio || 1;
        const position = { x: x * ratio, y: y * ratio };
        const emit = (event, payload) => window.__TAURI_INTERNALS__.invoke("plugin:event|emit", { event, payload });
        await emit("tauri://drag-enter", { paths: p, position });
        await emit("tauri://drag-over", { position });
        await emit("tauri://drag-drop", { paths: p, position });
        return true;
      },
      paths,
      pos.x,
      pos.y,
    );
  }
}
