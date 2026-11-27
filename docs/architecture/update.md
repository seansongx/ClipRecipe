# 自动更新与发布的实现说明

- **应用标识**：`com.xiaofany.cliprecipe`（`tauri.conf.json` 的 `identifier`，也决定系统数据目录名，例如
  `%APPDATA%\com.xiaofany.cliprecipe\`、`%LOCALAPPDATA%\com.xiaofany.cliprecipe\ffmpeg\`）。发布前曾用 `io.github.cliprecipe`，
  旧目录里的设置与已下载的 FFmpeg 不会被读取，也不做迁移（没有发布过使用旧标识的版本）。
- **插件**：`tauri-plugin-updater`（2.14）。只在 Rust 侧使用（`commands/system.rs`），没有授权给前端：capabilities 不变；
  网络请求都在 Rust 中，CSP 不变。重启用 Tauri 自带的 `AppHandle::restart`，不需要 process 插件。配置在 `tauri.conf.json`：
  `plugins.updater.endpoints` 依次为 xiaofany.com 的 Tauri 升级接口（`/api/update/cliprecipe/tauri/{{target}}/{{arch}}/{{current_version}}`）
  与 GitHub 的 `releases/latest/download/latest.json`（顺序有单元测试 `updater_tries_xiaofany_before_github`），
  `windows.installMode` 为 `passive`（只显示进度条，不需要用户操作），`pubkey` 为 updater 公钥。
- **两个更新地址的关系**：插件（2.14 的 `Updater::check`）按顺序请求：连不上、或返回非 2xx 时请求下一个；返回 204 时就此认定
  没有更新，不再请求 GitHub（xiaofany 的 204 也包括“未进入灰度”，不应绕过）；返回更新信息时用它的下载地址与签名。
  所以下载没有兜底：从 xiaofany 查到的更新只从 `dl.xiaofany.cn` 下载，失败时走下面的“下载失败”流程（重试 / 打开发布页）。
  不设插件的 `timeout`（它同时限制下载，会掐断慢速网络上的大文件下载）。不发 `X-Client-Id`，xiaofany 的灰度按全量处理。
- **公钥占位**：项目所有者生成密钥前，`pubkey` 是占位值 `system::UPDATER_PUBKEY_PLACEHOLDER`（`scripts/release.mjs` 中有同一个值）。
  插件只在校验签名时才解析公钥，所以占位值不影响启动、`cargo test`、`npm run build` 与 `tauri build`；程序检测到占位值时不做自动更新。
- **签名产物放在单独的配置里**：`bundle.createUpdaterArtifacts: true` 写在 `src-tauri/tauri.release.conf.json`，只有发布流水线
  以 `--config` 合并。打开它而没有 `TAURI_SIGNING_PRIVATE_KEY` 时 `tauri build` 会直接失败（“A public key has been found, but no
  private key”），放进主配置会让本地打包和端到端构建都依赖私钥。发布用的 tauri CLI 还会核对私钥与 `pubkey` 是否成对。
- **能否自动更新**（纯函数 `system::manual_update_reason(安装方式, 公钥是否已配置)`，有单元测试）：安装方式取自
  `tauri::utils::platform::bundle_type()`（打包工具写进程序）。NSIS / MSI / AppImage 可以；macOS 只发布没有开发者签名的 dmg、没有更新包，提示重新执行安装命令（`website/install.sh`）；deb / rpm 交给用户重新下载
  （插件虽能调用 dpkg / rpm，但需要管理员权限；xiaofany 的 `linux` 与 `latest.json` 的 `linux-x86_64` 都指向 AppImage）；未打包的程序（开发版、`cargo run`、
  端到端测试的 `--no-bundle` 构建）与未配置公钥的构建不行。
- **检查**（`check_update`）：能自动更新时用插件检查（xiaofany 的接口只在有新版本时返回该平台的文件；`latest.json` 由插件比较版本，
  按 `{os}-{arch}-{安装方式}` 再按 `{os}-{arch}` 取下载地址与签名），
  有新版本时把插件的 `Update` 存为待装更新并返回 `auto: true` 与更新说明；不能自动更新、或两个地址都失败时，改用原来的
  GitHub 公开 API（`system::check_update`，`ureq`）取最新发布，返回 `auto: false` 与 `manualReason`。`UpdateInfo` 新增
  `notes` / `auto` / `manualReason`；原有的版本解析、发布页地址限制、注入 fetch 的测试保留。
- **下载与安装**（`install_update`，立即返回）：处理或工具运行中时拒绝；在异步任务里 `Update::download`（进度事件 `update-progress`，
  `system::DownloadProgress` 节流：百分比每变 1 或每 1 MiB 一次）→ 插件校验 minisign 签名 → 事件 `update-installing` →
  `Update::install`。Windows 上插件启动 NSIS 安装程序后直接结束进程（之前通过 `on_before_exit` 保存窗口位置、清理预览目录），
  安装完成后安装程序重新打开 ClipRecipe；AppImage 替换文件后 `AppHandle::restart`。失败时事件 `update-failed`。
  `cancel_update` 中止下载任务（安装开始后不能取消）。状态流转 `system::UpdateFlow`（空闲 → 可安装 → 下载中 → 安装中；取消或下载失败回到
  可安装，安装失败回到可安装；下载中再次检查不替换待装更新）有单元测试。
- **界面**：`state/update.ts`（`run`：idle / downloading / installing / failed）、`components/UpdateDialog.tsx`（版本号、更新说明、
  “下载并安装” / “打开发布页” / “稍后”，下载进度与“取消下载”，失败时“重试” / “打开发布页”）。何时检查不变：`set.auto-update`
  启动时检查（失败静默），设置页“立即检查”。
- **发布**：见 [RELEASING.md](../RELEASING.md)。`.github/workflows/ci.yml`（前端检查；Windows、Linux 与 macOS 上 fmt / clippy / test）、
  `.github/workflows/release.yml`（`v*` tag → 检查版本 / CHANGELOG / 公钥 → 草稿 Release → `tauri-apps/tauri-action@v1` 逐个平台打包签名上传 →
  检查 `latest.json`）、`.github/workflows/publish-xiaofany.yml`（Release 发布后 → `scripts/xiaofany-manifest.mjs` 生成清单 →
  `scripts/publish-xiaofany.mjs` 上传到 xiaofany.com → 核对版本接口与升级接口）。`bundle.targets` 为 `nsis`、`appimage`、`deb`、`dmg`（每个平台只打本平台的格式；macOS 的 dmg 由 `release.yml` 的 `build-macos` 任务单独打包上传，见 RELEASING.md）；NSIS 语言为简体中文与英文、
  安装到当前用户。版本号的同步与检查：`scripts/release.mjs`（`npm run release:version` / `release:check`）。
