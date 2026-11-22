# 发布验收清单

发布以 GitHub [`lengyuqu/ifg`](https://github.com/lengyuqu/ifg) 为准。记录工作区版本、候选提交 SHA、服务端构建提交和镜像 ID，以及对应的 GitHub Actions 日志。按实际结果标记通过、失败、跳过或未运行。

## 1. 开发基线

- [ ] 从 GitHub `main` 选定候选 SHA，确认本地验收使用该提交的干净检出。记录该 SHA 的 GitHub Actions `ci` 运行，确认 Linux `check`、`windows` 和 `e2e` 的实际结果。
- [ ] 用 `bash scripts/e2e-docker.sh` 验证固定服务端基线。GitHub `e2e` 未运行时，在干净检出本地执行。记录构建提交、镜像 ID 和测试结果，确认契约快照对应基线。镜像元数据缺失时记录来源及限制。
- [ ] 按本轮变更在隔离容器中验证固定基线尚未提供的新版接口。从可核对的提交构建或选用镜像，用 `IFG_E2E_IMAGE` 显式指定镜像，并记录新增命令的验收结果。涉及 fork 恢复时执行 [fork 镜像专项](ci-e2e-runner.md#3-较新服务端的-fork-镜像专项)。
- [ ] 按 [JSON 兼容契约](json-compatibility.md) 检查机器输出回归测试与服务端版本边界；1.x 不得无迁移方案引入列出的破坏性变更。

## 2. 候选打包与验收

- [ ] 在 GitHub 手动运行 [release workflow](../.github/workflows/release.yml)，例如 `gh workflow run release.yml -R lengyuqu/ifg --ref main`。确认 `headSha` 与候选 SHA 一致，`unix`、`windows`、`checksums` 成功，`publish` 为 `skipped`。
- [ ] 下载 `candidate-<完整 SHA>`，核对 macOS 和 Linux 各两种架构的 tar.gz、Windows x86_64 ZIP 及合并的 `SHA256SUMS`。确认文件名与工作区版本一致，逐个校验哈希。
- [ ] 解包并运行 `ifg --version`，检查 man 页和基本命令。记录平台、架构、测试环境和结果。Windows 实机交互按 [Windows 验证清单](windows-verification.md) 逐项记录。

## 3. 正式发布与交付

- [ ] 确认版本与拟发布的 `v*` tag 一致，tag 指向已验收的候选 SHA。若版本调整产生新提交，对新 SHA 重跑 CI 与候选验收。
- [ ] 正式发布时向 GitHub 推送 tag，核对触发的 `release` 运行及 `publish` 结果。检查 Release 的五个包、`SHA256SUMS`、发布说明与已知限制。
- [ ] 从 GitHub Release 下载产物，重新校验哈希并验证安装。更新 Homebrew tap 后检查下载、校验和安装。

## 审计策略

GitHub Actions 的 Linux `check` 在 PR 和 `main` push 上执行 `cargo-audit`，审计失败会使检查失败。合并前核对 PR 的检查结果，合并后核对 `main` 上候选 SHA 的检查结果。发布前若出现新的安全公告，重新执行审计。

## 候选验收（2026-11-22）

候选取自 GitHub `main` 的 `b22df177bdf933208003bb087599d278ad93cce9`，工作区版本为 `0.9.0`。[该提交的 CI](https://github.com/lengyuqu/ifg/actions/runs/44882595879) 中 Linux `check`、Windows 检查和固定基线 Docker E2E 全部通过。

### 五平台候选产物

[手动打包运行](https://github.com/lengyuqu/ifg/actions/runs/44933381875) 的 `headSha` 与候选一致，`unix`、`windows` 和 `checksums` 成功，`publish` 已跳过。本轮未创建 Git tag 或正式 Release。

[候选制品](https://github.com/lengyuqu/ifg/actions/runs/44933381875/artifacts/14719411833) 已下载至 `dist/github-b22df17/packages/`。四个 tar.gz 和一个 Windows ZIP 均匹配合并后的 `SHA256SUMS`，归档内包含对应架构的二进制及 man 页。

| 目标平台 | 运行验证 |
|---|---|
| macOS arm64 | 本机解包后验证 `ifg --version` 和 Bash 补全。 |
| macOS x86_64 | GitHub Intel macOS runner 原生解包后验证 `ifg --version` 和 man 页。 |
| Linux arm64、x86_64 | 分别在对应架构的 Alpine 3.21 容器验证 `ifg --version`、Bash 补全及 man 页。 |
| Windows x86_64 | GitHub Windows runner 原生解包后验证 `ifg.exe --version` 和 man 页，本地复核 ZIP 完整性。浏览器、终端外观、GUI 编辑器及 SmartScreen 仍待实机交互验收。 |

### 新版 IronForge 兼容性

从 IronForge 提交 `6542845284ba2db347e6ad319706c832939c25b3` 的 `git archive` 构建 Linux arm64 镜像 `sha256:6846efa2b3ff42f0e89116340cffb40682c2ca04d1f3090d0a3373680e926da1`。隔离实例健康，服务端版本为 `0.2.0`，能力接口回报相同构建提交及 `build_dirty=false`。客户端在候选 SHA 的干净检出上构建，测试使用临时容器、临时数据和测试账号。

| 场景 | 结果 |
|---|---|
| 常规 E2E 与 fork 镜像专项 | `scripts/e2e-docker.sh` 的 11 项通过。`--fork-mirror` 的 1 项通过，删除测试 fork 的源分支和源仓库后，仍可从目标仓库镜像恢复同一 head SHA。 |
| `commit create` | dry-run 后分支未移动，实际写入产生新提交。旧 SHA 被拒为 `REF_ADVANCED`，分支仍停在新提交。 |
| `commit apply-patch`、`cherry-pick`、`revert` | Git 中可读到补丁后的文件内容，撤销后的 tree 与原始 tree 相同。 |
| `pr readiness`、`pr update-branch` | 无冲突 PR 可合并，dry-run 后 head 未移动，实际更新后可从 head 读到 base 的新增文件。 |
| `branch delete` | dry-run 保留测试分支，确认删除后远端引用消失。 |
| `context index`、`symbols`、`map` | 对指定提交建立索引后，找到预期函数和源文件。 |
| Agent 账号与令牌 | 创建、查看、列表通过。限定仓库和分支的令牌可创建、列出和撤销。密钥文件权限为 `0600`，创建响应不含密钥。 |
| 草稿命名空间 | 创建和列表通过，续期后到期时间延长，关闭后进入 `expiring`。 |
| 通知偏好 | 全局静音与单仓库显式开启同时保存，重新读取后均生效。 |

从 GitHub 下载的 macOS arm64 候选包解包后，用其中的 `ifg` 在同一镜像的临时实例上重复执行新增命令验收，8 组全部通过。本地证据位于 `dist/github-b22df17/`，包含镜像构建、两次 E2E、新增命令及候选包验证日志。

## Unreleased 验收（2026-11-10）

本轮只准备候选版本，工作区版本仍为 `0.9.0`；不创建发布 tag，不发布 GitHub Release，不更新 Homebrew tap。手动运行 `release.yml` 只构建和校验产物，`publish` job 仅在 `v*` tag 的 push 事件执行。

### 变更摘要

- 修复文件回退 PAT 被旧 keyring 条目覆盖，以及 URL 协议/部署路径变化时自动复用配置凭据的问题。
- 默认仓库列表按实际 PAT 识别账号；统一分支 ref 匹配与 fork PR 目标库镜像回退。
- 修改接口拒绝非空的无效 JSON 响应；stdout 写入失败返回非零，BrokenPipe 保持正常结束。
- 统一 JSON/模板支持规则、保留搜索扩展字段，并修复 doctor 的按主机凭据助手提示。
- 增加原生 keyring 恢复验证和显式选择的 fork 镜像真实服务兼容性场景。
- Windows 原生验收发现并修复协作模块条件编译与编辑器临时文件共享冲突；测试改用跨平台临时目录，并兼容 jq 的 CRLF 输出。
- 自托管 CI 使用独立 `CARGO_HOME` 和缓存前缀，防止缓存清理删除 runner 用户已有工具或注册表凭据。

### 证据与边界

| 项目 | 状态 | 证据或后续动作 |
|---|---|---|
| 候选源码 | 已提交并同步 | `1f31c8b08efe765d0fe8af00667bb547987e0933`；包含核心修复 `2ee746c`、验收补充 `70c8573`、Windows 修复 `dba2032` / `e69837a` 和 CI / 测试修正；候选已同步至 CNB、git233、GitHub 的 `main`，后续归档提交仅含文档 |
| 本地核心门禁 | 通过 | 候选 `1f31c8b`，macOS 251 项测试；fmt、Clippy、man、依赖边界与 cargo-audit（263 个依赖、1430 条公告）通过 |
| 固定基线 E2E | 通过 | `1f31c8b` 的干净检出，11 项通过；镜像 `sha256:257a6167ab94a4735abfe53060d758bbe080b36f0f419aed0950d8ac208fa627`，服务端提交 `7d27c2d519674194ab016bcfb8f13f235fe4ed81` |
| macOS 原生 keyring | 通过 | 候选 `1f31c8b`：`IFG_KEYRING_TEST=1 cargo test -p ifg --bin ifg native_ --locked -- --test-threads=1`，2 项通过；只操作独立测试条目并检查清理。恢复测试构造写失败后的文件状态，并使用真实钥匙串读写，不锁定用户钥匙串 |
| CNB CI | 已恢复并通过 | [`e69837a` 完整门禁](https://cnb.cool/zdzso/ifg/-/build/logs/cnb-kk6-1k70qn12p)：1 核节点，Linux 249 项测试及工具链、审计、检查全部成功；此前 [`70c8573`](https://cnb.cool/zdzso/ifg/-/build/logs/cnb-r0i-1k70oi6eu) 亦通过 |
| 后续 CNB 同步 | 明确跳过重跑 | 验收时查询本月剩余 256 核秒，不足一次任务预留。`e69837a..1f31c8b` 只修改 CI、两个测试及 runner 文档，运行时代码和依赖完全相同；按 [CNB 跳过机制](https://docs.cnb.cool/en/build/skip-pipeline.html) 以 `git push -o ci.skip` 同步，最终文档提交使用 `[ci skip]`。跳过不算 CI 通过，候选精确 SHA 由 GitHub 验证 |
| GitHub Linux / Windows CI | 通过 | [候选 `1f31c8b` 的完整运行](https://github.com/lengyuqu/ifg/actions/runs/42680895047)：Linux 249 项、Windows 244 项、隔离固定基线 E2E 11 项全部通过；Windows 原生凭据读写、删除及文件回退恢复测试实际执行成功 |
| fork 删除场景 | 通过 | 干净 `1f31c8b`，`--fork-mirror` 专项 1 项通过：删除源分支、再删除 fork 仓库后，仍从目标镜像恢复 head `bbb676e4b84f7967075eff7e101b33d1cdced86a` |
| 兼容服务端来源 | 已核对 | 从提交 `1c1559f32bd777cfb5c5c6279027f2ea60dfd75c` 的 `git archive` 构建；镜像 `sha256:a1900f680da2ebe9cc3f831e614761476a5e0d774adf62ef5e69f5ca651c0f8d`；能力接口报告相同提交、`build_dirty=false`；未修改 IronForge 工作区或部署 |
| 本地 Unix 候选包 | 历史补充通过 | `70c8573` 的 4 个 tar.gz 和 man 页齐全；macOS arm64、Linux arm64/amd64 容器基本运行通过，Linux `/dev/full` 写入错误返回非零；本机无 Rosetta，macOS x86_64 运行留给 Intel GitHub runner。最终候选以如下 GitHub 产物为准 |
| 五平台 GitHub 候选产物 | 通过 | [手动候选构建](https://github.com/lengyuqu/ifg/actions/runs/42681756618)，源码 `1f31c8b`；Unix、Windows、checksums 均成功，`publish` 明确跳过。4 个 tar.gz、1 个 zip、统一 SHA256SUMS 已下载复核，与实际执行的安装包字节一致 |
| 候选安装与运行 | 通过 | macOS x86_64 在 Intel runner、Windows x86_64 在 Windows runner 解压后运行 `--version` 并检查 man 页；macOS arm64 在本机、Linux arm64/amd64 在干净 Alpine 3.21 容器执行 `--version` 与 Bash completions；两个 Linux 包的 `/dev/full` 写入均返回错误及退出码 1。容器执行不代表物理 Linux 桌面验收 |

五个包的文件名、SHA-256 与执行方式见 [产物验收明细](release-evidence-2026-11-10.json)。GitHub 合并制品名为 `candidate-1f31c8b08efe765d0fe8af00667bb547987e0933`；本机副本位于被 Git 忽略的 `dist/github-1f31c8b/`，包含官方 `SHA256SUMS` 和 `VALIDATION.json`。GitHub 构建缓存保存成功后，工作流完整结束为成功。

候选构建不能代表正式发布。负责人选择本轮仅完成自动验证：Windows GUI、SmartScreen、浏览器与编辑器交互明确列为后续实机待办，不标记为通过。

已有平台限制继续适用：Windows 安全协作会话文件存储尚不支持，见 [Windows 验证清单 V9](windows-verification.md)；本轮没有将其记作已实现或已验收功能。
