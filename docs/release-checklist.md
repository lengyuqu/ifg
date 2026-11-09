# 发布验收清单

发布前为每次发布记录版本、候选提交 SHA、服务端镜像 ID 和对应日志链接。只有实际完成并核对的项目才能标记为通过；失败、跳过与未运行必须分别记录。

## 1. 开发基线

- [ ] 确认工作区干净、版本与 tag 一致，CNB `main` 包含候选 SHA；记录 CNB push 和 PR 的 CI 链接，确认工具链、格式、Clippy、man、测试、依赖边界及 cargo-audit 均通过。
- [ ] 在候选 SHA 的干净检出执行 `bash scripts/e2e-docker.sh`，记录镜像 ID、服务端版本及所有端到端结果；确认契约快照对应同一镜像。镜像无源码来源标签时记录该限制。
- [ ] 确认 CNB 已完成开发验证后，以非强制方式将候选提交同步到 git233，并核对目标分支 SHA。

## 2. 发布仓库与构建

- [ ] 准备发布时，将已验证的候选提交非强制推送至 GitHub 的目标分支；核对远端 SHA 与候选 SHA 一致。
- [ ] 确认 GitHub Actions 的 Linux 和 Windows CI 均针对该 SHA 成功；未运行 Windows 或真实服务 E2E 不得标为已验证。
- [ ] 核对发布 tag 指向候选 SHA。推送 `v*` tag 会触发 [release workflow](../.github/workflows/release.yml) 并发布 Release；只有实际准备发布时执行。
- [ ] 确认发布流水线成功，核对四个 macOS/Linux tar.gz、Windows zip 和合并的 SHA256SUMS 均已上传且版本、文件名正确。

## 3. 产物验证与交付

- [ ] 从 Release 下载产物和 SHA256SUMS，分别验证每个目标平台的哈希；校验失败即停止安装。
- [ ] 在干净的各目标平台上安装并运行 `ifg --version` 和基本命令；记录平台、架构、版本及结果。未具备的平台标记为未验证。
- [ ] 更新 Homebrew tap 后检查下载、校验和安装；检查发布说明与已知限制。

## 审计策略

CNB PR 与 main push 均执行 `cargo-audit`，失败即为该流水线失败；PR 缓存未命中可能增加安装时间。合并前确认 PR 的具体提交检查通过，合并后还需确认 main push 的检查通过。CI 的安全审计不替代新公告出现后的再次检查。

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
