import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { installDesktopGuards } from "./lib/desktop";
import { applyPlatformAttribute } from "./lib/platform";
import "./index.css";

// 去掉右键菜单、浏览器快捷键、缩放、拖动导航等网页行为（见 lib/desktop.ts）
installDesktopGuards();
// 界面字体等按平台的样式以 <html data-platform> 为准（lib/platform.ts），首次渲染前写好
applyPlatformAttribute();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
