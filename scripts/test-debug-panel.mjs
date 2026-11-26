#!/usr/bin/env node
// 调试面板（src/debug/）的状态层测试：模拟数据不写到磁盘、不发给后端；“全部恢复默认”不改设置。
//
// 用法：npm run test:debug-panel
//
// 做法：用 Vite 的开发服务器（不监听端口）按开发模式（import.meta.env.DEV 为 true，与 `npm run dev` 相同）加载真实的 TS 模块，
// 直接操作 zustand 状态并断言。没有 Tauri（isTauri 为 false），命令调用一律被拒绝，所以这里只验证前端状态层的行为；
// 桌面应用里的完整流程（真实写 queue.json、重启、标题栏）见 `npm run e2e:debug-panel`。

import assert from "node:assert/strict";
import path from "node:path";
import { createServer } from "vite";

const REPO = path.resolve(import.meta.dirname, "..");
const server = await createServer({
  root: REPO,
  configFile: path.join(REPO, "vite.config.ts"),
  mode: "development",
  logLevel: "error",
  server: { middlewareMode: true, hmr: false, watch: null },
  appType: "custom",
});

const load = (file) => server.ssrLoadModule(file);
let failed = 0;
async function test(name, fn) {
  try {
    await fn();
    console.log(`PASS  ${name}`);
  } catch (e) {
    failed++;
    console.log(`FAIL  ${name}\n      ${String(e.stack ?? e).split("\n").join("\n      ")}`);
  }
}

try {
  const { useQueue, unfinishedTasks } = await load("/src/state/queue.ts");
  const { useLog } = await load("/src/state/log.ts");
  const { useSettings } = await load("/src/state/settings.ts");
  const { useFfmpeg } = await load("/src/state/ffmpeg.ts");
  const { isSimulated, findSimulated } = await load("/src/lib/simulated.ts");
  const { useDebug } = await load("/src/debug/store.ts");

  const realItem = {
    id: "real-1",
    path: "D:\\真实\\a.mp4",
    name: "a.mp4",
    sourceRoot: null,
    info: null,
    probeError: null,
    state: "idle",
    percent: 0,
    speed: 0,
    output: null,
    outputs: [],
    round: 0,
    rounds: 1,
    error: null,
    subtitle: null,
    cover: null,
  };
  const reset = () => {
    useDebug.getState().resetAll();
    useQueue.setState({ items: [realItem], selected: new Set(), running: false, runIds: [], lastRun: null });
    useLog.setState({ entries: [] });
  };

  await test("队列模拟：保存队列（queue.json 的内容）排除模拟行，只剩真实的任务", () => {
    reset();
    useDebug.getState().setSimulation("queueStates", true);
    const items = useQueue.getState().items;
    assert.equal(items.length, 11, "应注入 10 个模拟行");
    const fake = items.filter((i) => i.id !== realItem.id);
    assert.ok(fake.every((i) => isSimulated(i.id) && isSimulated(i.path)), "模拟行的 ID 与路径都已登记");
    const saved = unfinishedTasks(items);
    assert.deepEqual(
      saved.map((t) => t.path),
      [realItem.path],
    );
  });

  await test("队列模拟：命令参数里出现模拟的 ID / 路径时被识别（lib/backend.ts 据此拒绝调用）", () => {
    const items = useQueue.getState().items;
    const fake = items.find((i) => i.id !== realItem.id);
    assert.equal(findSimulated({ tasks: [{ id: fake.id }] }), fake.id);
    assert.equal(findSimulated({ path: fake.path }), fake.path);
    assert.equal(findSimulated({ tasks: [realItem] }), null);
  });

  await test("队列模拟：停止 / 取消在前端完成，不调用后端（没有“停止失败”记录）", async () => {
    const one = useQueue.getState().items.find((i) => i.id !== realItem.id && i.state === "waiting");
    await useQueue.getState().cancel(one.id);
    assert.equal(useQueue.getState().items.find((i) => i.id === one.id).state, "cancelled");
    await useQueue.getState().stop();
    const state = useQueue.getState();
    assert.equal(state.running, false);
    assert.ok(!state.items.some((i) => i.state === "running" || i.state === "waiting"));
    assert.ok(state.lastRun && state.lastRun.cancelled >= 6, JSON.stringify(state.lastRun));
    assert.deepEqual(
      useLog.getState().entries.filter((e) => e.level === "error").map((e) => e.message),
      [],
    );
  });

  await test("队列模拟：关闭后模拟行移除、登记清除，真实任务保留", () => {
    const ids = useQueue.getState().items.map((i) => i.id).filter((id) => id !== realItem.id);
    useDebug.getState().setSimulation("queueStates", false);
    assert.deepEqual(
      useQueue.getState().items.map((i) => i.id),
      [realItem.id],
    );
    assert.ok(ids.every((id) => !isSimulated(id)));
  });

  await test("FFmpeg 模拟：点“下载”只进入下载中模拟，“取消”结束它；删除 / 重新检测不调用后端", async () => {
    reset();
    const realStart = useFfmpeg.getState().startDownload;
    useDebug.getState().setSimulation("ffmpegMissing", true);
    assert.notEqual(useFfmpeg.getState().startDownload, realStart, "模拟期间下载操作被替换");
    await useFfmpeg.getState().startDownload();
    assert.ok(useDebug.getState().simulations.includes("ffmpegDownloading"));
    assert.equal(useFfmpeg.getState().download.kind, "running");
    await useFfmpeg.getState().cancelDownload();
    assert.ok(!useDebug.getState().simulations.includes("ffmpegDownloading"));
    assert.equal(useFfmpeg.getState().download.kind, "idle");
    await useFfmpeg.getState().remove();
    await useFfmpeg.getState().redetect();
    assert.deepEqual(
      useLog.getState().entries.filter((e) => e.level === "error").map((e) => e.message),
      [],
    );
    useDebug.getState().setSimulation("ffmpegMissing", false);
    assert.equal(useFfmpeg.getState().startDownload, realStart, "关闭全部 FFmpeg 模拟后换回真实操作");
    assert.equal(useFfmpeg.getState().status, null);
  });

  await test("全部恢复默认：关掉全部模拟与平台模拟，不改设置", () => {
    reset();
    const settings = useSettings.getState().settings;
    useDebug.getState().setPlatform("macos");
    useDebug.getState().setSimulation("queueStates", true);
    useDebug.getState().setSimulation("logLevels", true);
    useDebug.getState().resetAll();
    assert.equal(useSettings.getState().settings, settings);
    assert.deepEqual(useDebug.getState().simulations, []);
    assert.equal(useDebug.getState().platform, null);
    assert.deepEqual(
      useQueue.getState().items.map((i) => i.id),
      [realItem.id],
    );
    assert.equal(useLog.getState().entries.length, 0);
  });
} finally {
  await server.close();
}

console.log(failed ? `\n${failed} 个失败` : "\n全部通过");
process.exit(failed ? 1 : 0);
