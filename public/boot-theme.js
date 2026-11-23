// 首帧之前设置主题（在 <head> 中同步执行，早于样式与应用脚本），避免启动时先画浅色再变深色。
// 主题偏好来自后端在页面脚本之前注入的 window.__CLIPRECIPE_BOOT__（读取 settings.json，见 src-tauri/src/desktop.rs），
// 浏览器预览时退回上次的主题（localStorage）或系统主题。“跟随系统”时 WebView 的 prefers-color-scheme 就是系统主题。
(function () {
  var pref = (window.__CLIPRECIPE_BOOT__ && window.__CLIPRECIPE_BOOT__.theme) || null;
  var theme = pref === "light" || pref === "dark" ? pref : null;
  if (!theme && !pref) {
    try {
      var saved = localStorage.getItem("theme");
      if (saved === "light" || saved === "dark") theme = saved;
    } catch (e) {
      // 读不到就跟随系统
    }
  }
  if (!theme) theme = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  document.documentElement.dataset.theme = theme;
})();
