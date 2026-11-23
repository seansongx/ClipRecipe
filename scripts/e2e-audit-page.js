// 端到端测试：界面状态审计（在页面中执行，由 e2e-cdp.mjs 注入到每个文档）。
//
// window.__e2e.audit({ root }) 检查 root（默认整页）里可见的部分，返回 { problems, stats, pairs }：
// 1. 一组同类选项（标签页、分段选择 / 开关组、列表）里，选中项与未选中项在视觉上明显不同——背景对比 ≥ 1.2、
//    或有只属于选中项的指示条（::after / ::before）/ 边框 / 阴影；只是文字颜色深浅不同不算。
// 2. aria 状态与 Base UI 的状态属性一致（shadcn 的选中样式按状态属性写：标签页 data-active、开关 / 复选框 / 单选
//    data-checked 与 data-unchecked、切换按钮 data-pressed）；带状态的控件的 data-slot 没有被外层的提示改写成
//    tooltip-trigger（Mira 的样式与组合按 data-slot 选择元素，components/Hint.tsx 负责保留它）。
// 3. 开关、复选框各自的选中 / 未选中外观不同；它们与输入框的边界和所在背景的对比 ≥ 3:1（WCAG 1.4.11）。
// 4. 文字（含占位符）与实际背景（逐层叠加半透明背景与 opacity）的对比：正文 ≥ 4.5、大字 ≥ 3（WCAG 1.4.3）；
//    禁用的控件按 WCAG 豁免，只计数。
// 调用前应把鼠标移开、取消焦点，避免悬停 / 焦点样式干扰（测试脚本的 ui.audit 会做）。
// 返回 { problems, warnings, stats, pairs }：problems 让用例失败；warnings 是 SOFT_RULES 里暂时降级的规则，照常计算、只报告。
(() => {
  const e2e = window.__e2e;
  if (!e2e || e2e.audit) return;

  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 1;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  /** 任意 CSS 颜色（含 oklab、color-mix 的计算结果）→ [r, g, b, a(0..1)] */
  const rgba = (css) => {
    if (!css || css === "transparent") return [0, 0, 0, 0];
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillStyle = "rgba(0,0,0,0)";
    ctx.fillStyle = css;
    ctx.fillRect(0, 0, 1, 1);
    const d = ctx.getImageData(0, 0, 1, 1).data;
    return [d[0], d[1], d[2], d[3] / 255];
  };
  const over = (top, bottom) => {
    const a = top[3];
    return [0, 1, 2].map((i) => top[i] * a + bottom[i] * (1 - a)).concat(1);
  };
  const lum = (c) => {
    const f = (v) => {
      v /= 255;
      return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
    };
    return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]);
  };
  const ratio = (a, b) => {
    const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
    return (x + 0.05) / (y + 0.05);
  };
  const hex = (c) => "#" + c.slice(0, 3).map((v) => Math.round(v).toString(16).padStart(2, "0")).join("");

  const pageBg = () => {
    const b = rgba(getComputedStyle(document.body).backgroundColor);
    return b[3] > 0 ? over(b, [255, 255, 255, 1]) : [255, 255, 255, 1];
  };
  /** 元素背后实际看到的颜色：从最近的不透明祖先开始，逐层叠加半透明背景。 */
  const backgroundOf = (el) => {
    const layers = [];
    for (let n = el; n && n.nodeType === 1; n = n.parentElement) {
      const c = rgba(getComputedStyle(n).backgroundColor);
      if (c[3] > 0) layers.push(c);
      if (c[3] >= 1) break;
    }
    let base = layers.length && layers[layers.length - 1][3] >= 1 ? layers.pop() : pageBg();
    for (let i = layers.length - 1; i >= 0; i--) base = over(layers[i], base);
    return base;
  };
  const opacityOf = (el) => {
    let o = 1;
    for (let n = el; n && n.nodeType === 1; n = n.parentElement) o *= Number(getComputedStyle(n).opacity);
    return o;
  };
  const shown = (el) => {
    if (!el.isConnected) return false;
    const r = el.getBoundingClientRect();
    if (r.width < 2 || r.height < 2) return false;
    if (r.bottom < 0 || r.right < 0 || r.top > innerHeight || r.left > innerWidth) return false;
    const s = getComputedStyle(el);
    return s.visibility !== "hidden" && s.display !== "none" && opacityOf(el) > 0.05;
  };
  const disabled = (el) =>
    !!el.closest(":disabled, [aria-disabled='true'], [data-disabled]:not([data-disabled='false']), fieldset:disabled");
  const describe = (el) => {
    const t = (el.getAttribute("aria-label") || el.textContent || "").replace(/\s+/g, " ").trim().slice(0, 30);
    const role = el.getAttribute("role");
    return el.tagName.toLowerCase() + (role ? "[" + role + "]" : "") + (t ? "「" + t + "」" : "");
  };

  /** 真正看得见的阴影 / 焦点环（Tailwind 总会写出一串透明、零尺寸的阴影，这些不算）。 */
  const visibleShadow = (value) => {
    if (!value || value === "none") return "";
    const re = /((?:rgba?|oklab|oklch|lab|lch|color|hsla?)\([^)]*\)|#[0-9a-fA-F]{3,8})\s+(-?[\d.]+)px\s+(-?[\d.]+)px(?:\s+(-?[\d.]+)px)?(?:\s+(-?[\d.]+)px)?/g;
    const kept = [];
    for (let m = re.exec(value); m; m = re.exec(value)) {
      const sizes = [m[2], m[3], m[4], m[5]].map((v) => Math.abs(Number(v || 0)));
      if (rgba(m[1])[3] > 0.05 && sizes.some((v) => v > 0)) kept.push(m[0]);
    }
    return kept.join(", ");
  };

  /** 视觉特征：背景、文字、边框、阴影、粗细、伪元素指示条。 */
  const look = (el) => {
    const s = getComputedStyle(el);
    const pseudo = (p) => {
      const ps = getComputedStyle(el, p);
      const c = rgba(ps.backgroundColor);
      const visible = ps.content !== "none" && ps.display !== "none" && Number(ps.opacity) > 0.05 && c[3] > 0.05;
      return visible ? hex(c) + "@" + ps.width + "x" + ps.height : "";
    };
    const sides = ["Top", "Right", "Bottom", "Left"].map((k) =>
      Number.parseFloat(s["border" + k + "Width"]) > 0 && s["border" + k + "Style"] !== "none" ? rgba(s["border" + k + "Color"]) : null,
    );
    return {
      bg: backgroundOf(el),
      color: rgba(s.color),
      borders: sides,
      shadow: visibleShadow(s.boxShadow),
      weight: s.fontWeight,
      after: pseudo("::after"),
      before: pseudo("::before"),
    };
  };
  /** 选中项 a 与未选中项 b 是否明显不同；返回 null 或原因。 */
  const distinct = (a, b) => {
    if (ratio(a.bg, b.bg) >= 1.2) return null;
    if (a.after !== b.after && a.after) return null;
    if (a.before !== b.before && a.before) return null;
    if (a.shadow !== b.shadow && a.shadow && !b.shadow) return null;
    for (let i = 0; i < 4; i++) {
      const x = a.borders[i];
      const y = b.borders[i];
      if (x && x[3] > 0.5 && ratio(over(x, a.bg), a.bg) >= 3 && (!y || ratio(over(x, a.bg), over(y, b.bg)) >= 1.5)) return null;
    }
    return "背景对比只有 " + ratio(a.bg, b.bg).toFixed(2) + "，没有只属于选中项的指示条 / 边框" +
      (ratio(over(a.color, a.bg), over(b.color, b.bg)) > 1.05 ? "（只有文字颜色不同）" : "");
  };

  /**
   * Base UI 控件的状态（由状态属性得出）：标签页 active / inactive，开关、复选框、单选、菜单勾选项 checked / unchecked /
   * indeterminate，切换按钮（aria-pressed）on / off；不是 Base UI 控件（没有这些属性）时为 null。
   */
  const baseState = (el) => {
    if (!el) return null;
    const role = el.getAttribute("role");
    if (role === "tab") return el.hasAttribute("data-active") ? "active" : "inactive";
    if (el.hasAttribute("aria-pressed")) return el.hasAttribute("data-pressed") ? "on" : "off";
    if (el.hasAttribute("data-indeterminate")) return "indeterminate";
    if (el.hasAttribute("data-checked")) return "checked";
    if (el.hasAttribute("data-unchecked")) return "unchecked";
    return null;
  };
  e2e.state = baseState;

  /** 每种控件的 aria 状态属性，以及它为 true（或 mixed）/ false 时应有的 Base UI 状态。 */
  /**
   * 暂时降级为“警告”的对比度规则（照常计算、e2e 输出里逐条打印实测值，但不让用例失败）。
   * 原因：暂用 shadcn 默认主题（base-mira + neutral，项目所有者的决定），它的 --muted / --input 达不到这些门槛；
   * 这些门槛是为旧的自定义配色定的。换回自定义主题时把对应项从这里删掉（或清空）即可恢复为硬失败。门槛数字不要改。
   * 不在这里的规则（文字对比 ≥ 4.5 / 3、aria 与状态属性一致、data-slot 未被改写等）始终是硬失败。
   */
  const SOFT_RULES = {
    // 1. 同组选项里选中项与未选中项的底色对比 ≥ 1.2（或有只属于选中项的指示条 / 边框）：默认主题的选中底是 muted / background
    "selected-distinct": "暂用 shadcn 默认主题",
    // 3. 开关、复选框的选中与未选中外观不同（同一判定）
    "toggle-distinct": "暂用 shadcn 默认主题",
    // 3. 开关、复选框与背景的边界对比 ≥ 3（默认主题的 --input 约 1.26 / 1.42）
    "control-boundary": "暂用 shadcn 默认主题",
    // 3. 输入框边框与背景对比 ≥ 3（同上）
    "input-border": "暂用 shadcn 默认主题",
    // 键盘焦点环的实色描边与背景对比 ≥ 3（默认主题的 --ring 在浅色下约 2.4）；焦点环“存在且 :focus-visible”仍是硬失败
    "focus-ring-contrast": "暂用 shadcn 默认主题",
  };
  e2e.softRules = SOFT_RULES;

  const STATE_MAP = {
    tab: { attr: "aria-selected", on: ["active"], off: ["inactive"] },
    radio: { attr: "aria-checked", on: ["checked"], off: ["unchecked"] },
    switch: { attr: "aria-checked", on: ["checked"], off: ["unchecked"] },
    checkbox: { attr: "aria-checked", on: ["checked", "indeterminate"], off: ["unchecked"] },
    menuitemradio: { attr: "aria-checked", on: ["checked"], off: ["unchecked"] },
    menuitemcheckbox: { attr: "aria-checked", on: ["checked"], off: ["unchecked"] },
    pressed: { attr: "aria-pressed", on: ["on"], off: ["off"] },
  };
  const isOn = (el) =>
    ["aria-selected", "aria-checked", "aria-pressed"].some((a) => el.getAttribute(a) === "true");

  e2e.audit = (opts) => {
    const options = opts || {};
    const root = options.root ? document.querySelector(options.root) : document.body;
    const problems = [];
    const warnings = [];
    /** 按规则记录：SOFT_RULES 里的规则记为警告（注明原因），其余是问题。 */
    const report = (rule, message) => {
      if (rule && SOFT_RULES[rule]) warnings.push("[" + rule + "] " + message + "【降级为警告：" + SOFT_RULES[rule] + "】");
      else problems.push(message);
    };
    const stats = { groups: 0, controls: 0, texts: 0, disabledTexts: 0, boundaries: 0 };
    const pairs = {};
    if (!root) return { problems: ["找不到审计范围 " + options.root], warnings, stats, pairs };

    // 2. aria 状态与 Base UI 的状态属性一致；带状态的控件保留自己的 data-slot
    for (const el of root.querySelectorAll("[role=tab],[role=radio],[role=switch],[role=checkbox],[role=menuitemradio],[role=menuitemcheckbox],[aria-pressed]")) {
      if (!shown(el)) continue;
      const map = STATE_MAP[el.hasAttribute("aria-pressed") ? "pressed" : el.getAttribute("role")];
      const state = baseState(el);
      if (state === null) continue;
      const on = el.getAttribute(map.attr) === "true" || el.getAttribute(map.attr) === "mixed";
      const ok = (on ? map.on : map.off).includes(state);
      if (!ok) problems.push(describe(el) + "：" + map.attr + "=" + el.getAttribute(map.attr) + " 但 Base UI 状态是 " + state + "（状态属性不一致，选中样式会失效）");
      if (el.getAttribute("data-slot") === "tooltip-trigger") {
        problems.push(describe(el) + " 的 data-slot 被提示改写成 tooltip-trigger（依赖 data-slot 的样式会失效）");
      }
    }

    // 1. 组内选中项与未选中项明显不同
    const groups = root.querySelectorAll("[role=tablist],[role=radiogroup],[role=group],[role=listbox],[role=toolbar]");
    for (const g of groups) {
      if (!shown(g)) continue;
      const members = [...g.querySelectorAll("[role=tab],[role=radio],[role=option],[aria-pressed]")].filter(
        (m) => shown(m) && m.parentElement.closest("[role=tablist],[role=radiogroup],[role=group],[role=listbox],[role=toolbar]") === g,
      );
      const on = members.filter(isOn);
      const off = members.filter((m) => !isOn(m) && !disabled(m));
      if (!on.length || !off.length) continue;
      stats.groups++;
      const label = g.getAttribute("aria-label") || describe(g);
      const offLooks = off.map(look);
      for (const m of on) {
        const a = look(m);
        for (let i = 0; i < off.length; i++) {
          const why = distinct(a, offLooks[i]);
          if (why) {
            report("selected-distinct", "「" + label + "」中选中的 " + describe(m) + " 与未选中的 " + describe(off[i]) + " 看起来几乎一样：" + why);
            break;
          }
        }
      }
    }

    // 3. 开关 / 复选框：选中与未选中外观不同；边界对比 ≥ 3
    for (const role of ["switch", "checkbox"]) {
      const all = [...root.querySelectorAll("[role=" + role + "]")].filter((el) => shown(el) && !disabled(el));
      stats.controls += all.length;
      const on = all.filter((el) => el.getAttribute("aria-checked") === "true");
      const off = all.filter((el) => el.getAttribute("aria-checked") === "false");
      if (on.length && off.length) {
        const why = distinct(look(on[0]), look(off[0]));
        if (why) report("toggle-distinct", role + " 的选中与未选中外观几乎一样：" + why);
      }
      for (const el of all) {
        const s = getComputedStyle(el);
        const around = backgroundOf(el.parentElement || el);
        const fill = rgba(s.backgroundColor);
        const border = Number.parseFloat(s.borderTopWidth) > 0 ? rgba(s.borderTopColor) : [0, 0, 0, 0];
        // 边界取填充色与边框色中更明显的一个
        const filled = over(fill, around);
        const edge = border[3] > 0.05 ? over(border, filled) : filled;
        const r = Math.max(ratio(filled, around), ratio(edge, around));
        stats.boundaries++;
        if (r < 3) report("control-boundary", describe(el) + " 的边界与背景对比 " + r.toFixed(2) + " < 3（" + hex(edge) + " / " + hex(around) + "）");
      }
    }
    for (const el of root.querySelectorAll("input:not([type=checkbox]):not([type=radio]):not([type=range]), textarea, [data-slot=select-trigger]")) {
      if (!shown(el) || disabled(el)) continue;
      const box = el.closest("[data-slot=input-group]") || el;
      const s = getComputedStyle(box);
      if (!(Number.parseFloat(s.borderTopWidth) > 0)) continue;
      const around = backgroundOf(box.parentElement || box);
      const r = ratio(over(rgba(s.borderTopColor), around), around);
      stats.boundaries++;
      if (r < 3) report("input-border", describe(el) + " 输入框边框与背景对比 " + r.toFixed(2) + " < 3");
    }

    // 4. 文字对比
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    const seen = new Set();
    for (let t = walker.nextNode(); t; t = walker.nextNode()) {
      if (!t.nodeValue.trim()) continue;
      const el = t.parentElement;
      if (!el || seen.has(el)) continue;
      seen.add(el);
      if (el.closest("video, svg, [data-slot=kbd], script, style, .sr-only") || !shown(el)) continue;
      const s = getComputedStyle(el);
      if (s.clipPath && s.clipPath !== "none") continue;
      if (disabled(el) || opacityOf(el) < 0.75) {
        stats.disabledTexts++;
        continue;
      }
      const bg = backgroundOf(el);
      const fg = over(rgba(s.color), bg);
      const r = ratio(fg, bg);
      const size = Number.parseFloat(s.fontSize);
      const large = size >= 24 || (size >= 18.66 && Number(s.fontWeight) >= 700);
      const need = large ? 3 : 4.5;
      stats.texts++;
      const key = hex(fg) + " on " + hex(bg);
      pairs[key] = Math.min(pairs[key] ?? 99, Number(r.toFixed(2)));
      if (r < need) problems.push(describe(el) + " 文字对比 " + r.toFixed(2) + " < " + need + "（" + key + "）");
    }
    for (const el of root.querySelectorAll("input[placeholder], textarea[placeholder]")) {
      if (!shown(el) || el.value || disabled(el)) continue;
      const bg = backgroundOf(el);
      const fg = over(rgba(getComputedStyle(el, "::placeholder").color), bg);
      const r = ratio(fg, bg);
      stats.texts++;
      if (r < 4.5) problems.push(describe(el) + " 占位符对比 " + r.toFixed(2) + " < 4.5");
    }
    // 同一个问题在多个元素上重复出现时合并
    return { problems: [...new Set(problems)], warnings: [...new Set(warnings)], stats, pairs };
  };

  /** 键盘焦点环：元素有可见的描边或阴影环，且与背景对比 ≥ 3。 */
  e2e.focusRing = async (selector) => {
    const el = document.querySelector(selector);
    if (!el) return { ok: false, why: "找不到 " + selector };
    el.focus({ focusVisible: true });
    // 等过渡动画结束再读样式
    await new Promise((r) => setTimeout(r, 300));
    const s = getComputedStyle(el);
    const bg = backgroundOf(el.parentElement || el);
    // 描边宽度只要求实际画出来（> 0）：Chromium 把 1px 描边按设备像素向下取整，150% 缩放时读出来是 0.67px。
    // 对比度门槛（≥ 3）不变。
    const outline = s.outlineStyle !== "none" && Number.parseFloat(s.outlineWidth) > 0 ? rgba(s.outlineColor) : null;
    const ring = visibleShadow(s.boxShadow);
    const contrast = outline ? ratio(over(outline, bg), bg) : 0;
    const visible = el.matches(":focus-visible");
    el.blur();
    return { ok: visible && (contrast >= 3 || !!ring), focusVisible: visible, outline: outline ? hex(outline) : null, ring, contrast: Number(contrast.toFixed(2)) };
  };
})();
