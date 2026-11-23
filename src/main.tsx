import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { installDesktopGuards } from "./lib/desktop";
import "./index.css";

// 去掉右键菜单、浏览器快捷键、缩放、拖动导航等网页行为（见 lib/desktop.ts）
installDesktopGuards();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
