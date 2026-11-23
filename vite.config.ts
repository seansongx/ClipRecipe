import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

// Tauri 需要固定端口；Rust 侧的改动由 tauri-cli 负责监听。
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    // 与 tsconfig.json 的 paths 一致（shadcn/ui 组件用 @/ 导入）
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  build: {
    rolldownOptions: {
      output: {
        // 第三方库单独成块（应用代码更新时它们的缓存仍有效），并让每块都在 500 kB 以内
        codeSplitting: {
          groups: [
            { name: "react", test: /node_modules[\\/](react|react-dom|scheduler)[\\/]/, priority: 3 },
            { name: "radix", test: /node_modules[\\/](@radix-ui|radix-ui|@floating-ui)[\\/]/, priority: 2 },
            { name: "vendor", test: /node_modules[\\/]/, priority: 1 },
          ],
        },
      },
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
