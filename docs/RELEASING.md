# 发布流程

ClipRecipe 通过 GitHub Actions 打包并发布到 [GitHub Releases](https://github.com/seansongx/ClipRecipe/releases)：
推送 `v*` tag 后，[`.github/workflows/release.yml`](../.github/workflows/release.yml) 在 Windows 与 Linux 上打包，
把安装包、更新包签名和 `latest.json` 上传到一个**草稿** Release，由项目所有者检查后手动发布。
程序内的自动更新读取的就是最新正式 Release 里的 `latest.json`。

## 发布哪些文件

| 平台 | 文件 | 自动更新 |
|---|---|---|
| Windows 10 / 11 x64 | `ClipRecipe_<版本>_x64-setup.exe`（NSIS 安装包） | 支持 |
| Linux x64 | `ClipRecipe_<版本>_amd64.AppImage` | 支持 |
| Linux x64 | `ClipRecipe_<版本>_amd64.deb` | 不支持：程序提示后打开发布页，由用户重新下载安装 |
| 自动更新用 | `*.sig`（更新包签名）、`latest.json`（版本、说明、下载地址与签名） | — |

- **Windows 只发布 NSIS 安装包**（不发布 MSI）：NSIS 默认安装到当前用户目录、不需要管理员权限，自动更新时可以静默替换；
  安装界面支持简体中文；体积较小。两种格式同时发布会让用户困惑，`latest.json` 里也只能指向其中一种。
- **不提供 macOS 版本**：没有条件测试，且未签名、未公证的应用会被 Gatekeeper 拦截。
- **没有 Windows 代码签名**：安装包未经 Authenticode 签名，首次下载或运行时 Windows SmartScreen 可能提示
  “Windows 已保护你的电脑”，用户需点“更多信息”→“仍要运行”。这与下面的“更新包签名”无关——后者只用于程序内自动更新时
  校验下载的文件确实来自本项目。

## 首次配置（只做一次）

自动更新要求每个更新包都用项目的 updater 私钥签名（[Tauri updater](https://v2.tauri.app/plugin/updater/) 自带的 minisign 签名），
程序内置对应的公钥用来校验。仓库里的公钥目前是占位值 `REPLACE_WITH_UPDATER_PUBLIC_KEY (see docs/RELEASING.md)`：
在换成真正的公钥之前，构建出的程序不会自动更新（只提示并打开发布页），发布流水线也会在检查这一步失败。

1. **生成密钥对**（在自己的电脑上，仓库根目录）：

   ```powershell
   npm run tauri signer generate -- -w "$HOME\.tauri\cliprecipe.key"
   ```

   按提示设置口令（不要留空）。生成两个文件：私钥 `cliprecipe.key` 与公钥 `cliprecipe.key.pub`。
   **私钥和口令一旦丢失，已安装的程序就再也无法接收自动更新**（只能让用户手动下载新版本），请备份到密码管理器等安全的地方。
   私钥绝不能提交到仓库。

2. **把公钥填进配置**：打开 `cliprecipe.key.pub`，把整行内容（一串 base64）填到
   [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json) 的 `plugins.updater.pubkey`，替换占位值，提交。

3. **设置 GitHub Secrets**：仓库 Settings → Secrets and variables → Actions → New repository secret：

   | 名称 | 内容 |
   |---|---|
   | `TAURI_SIGNING_PRIVATE_KEY` | 私钥文件 `cliprecipe.key` 的全部内容 |
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 第 1 步设置的口令 |

   也可以用 GitHub CLI：

   ```powershell
   Get-Content "$HOME\.tauri\cliprecipe.key" -Raw | gh secret set TAURI_SIGNING_PRIVATE_KEY
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
   ```

4. **确认 Actions 可以写 Release**：发布流水线在 workflow 里声明了 `contents: write`，一般不需要改仓库设置；
   如果组织策略限制了 `GITHUB_TOKEN` 的权限，请在 Settings → Actions → General → Workflow permissions 中允许。

## 每次发布

以发布 `0.2.0` 为例：

1. **改版本号**：版本号记录在 `package.json`、`package-lock.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`、
   `src-tauri/tauri.conf.json` 五处，用脚本一次改好：

   ```powershell
   npm run release:version -- 0.2.0
   ```

   `npm run release:check` 可以随时检查几处是否一致（CI 也会检查）。

2. **更新 CHANGELOG**：在 [CHANGELOG.md](../CHANGELOG.md) 顶部写 `## [0.2.0] - 2026-12-01`（发布日期）一节，
   按“新增 / 变更 / 修复 / 移除”分组，并在文末加上链接 `[0.2.0]: https://github.com/seansongx/ClipRecipe/releases/tag/v0.2.0`。
   这一节的内容就是 Release 的说明（后面会自动附上固定的下载说明）。标题里还写着“未发布”时发布流水线会失败。

3. **跑验收命令并提交**（见 [CONTRIBUTING.md](../CONTRIBUTING.md#提交前)），合并到 `main`，等 CI 通过。

4. **打 tag 并推送**（tag 必须是 `v` + 版本号，打在要发布的那个提交上）：

   ```powershell
   git switch main
   git pull
   npm run release:check -- v0.2.0
   git tag v0.2.0
   git push origin v0.2.0
   ```

   `release:check` 带上 tag 时与流水线的检查相同：tag 与版本号一致、CHANGELOG 有该版本的小节并写了日期、已填真正的公钥。

5. **等发布流水线完成**（Actions → Release，约 20～40 分钟）。它依次：
   - `prepare`：做上面的检查，确认设置了签名私钥，创建草稿 Release（标题 `ClipRecipe 0.2.0`，说明取自 CHANGELOG）；
   - `build`：先 Windows、后 Linux（逐个进行，避免两边同时改写 `latest.json`）用
     [`tauri-apps/tauri-action`](https://github.com/tauri-apps/tauri-action) 打包、签名并上传；
   - `verify`：下载草稿里的 `latest.json`，确认版本号正确，Windows 指向 NSIS 安装包、Linux 指向 AppImage，且都有签名。

6. **检查草稿 Release**（Releases 页面里标为 Draft）：文件齐全（见上表），说明无误；最好下载 Windows 安装包和 AppImage
   各装一次试试。确认后点 **Publish release**。发布之后，已安装的程序（Windows 安装包版、AppImage 版）检查更新时就能发现新版本。

### 出错时

- 检查没通过（版本号、CHANGELOG、公钥、Secret）：按日志提示修好后删除远程 tag 再重新打：
  `git push origin :refs/tags/v0.2.0`，`git tag -d v0.2.0`，修复并提交后从第 4 步重来。草稿 Release 如已创建，可以保留（会被沿用）
  或在网页上删除。
- 打包失败：在 Actions 中重新运行失败的任务即可（草稿 Release 会被沿用，已上传的同名文件会被覆盖）。
- 已经发布的 Release 不能再用同一个 tag 重新发布（流水线会拒绝），请改版本号发布新版本。

## 本地打包

`npm run tauri build` 按 `tauri.conf.json` 打包（Windows 上生成 NSIS 安装包，Linux 上生成 AppImage 与 deb），
不需要签名私钥，但生成的程序不带更新包签名。生成更新包签名需要打开 `bundle.createUpdaterArtifacts`，它放在
[`src-tauri/tauri.release.conf.json`](../src-tauri/tauri.release.conf.json) 里，只有发布流水线通过 `--config` 使用：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content "$HOME\.tauri\cliprecipe.key" -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "<口令>"
npm run tauri build -- --config src-tauri/tauri.release.conf.json
```

没有设置私钥时这条命令会失败（Tauri 要求签名），这正是把它从主配置里拆出去的原因：日常开发、`cargo test` 与端到端测试都不需要私钥。
