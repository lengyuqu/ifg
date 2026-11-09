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

### 证据与边界

| 项目 | 状态 | 证据或后续动作 |
|---|---|---|
| 核心修复源码 | 已提交 | `2ee746c45c4220edb0404dfeb0943011f0e6664f`；后续验收补充提交另行记录 |
| 本地核心门禁 | 通过 | 250 项测试；fmt、Clippy、man、依赖边界通过；cargo-audit 网络重试后通过 |
| 固定基线 E2E | 通过 | 11 项；镜像 `sha256:257a6167ab94a4735abfe53060d758bbe080b36f0f419aed0950d8ac208fa627`；提交后的干净检出复核待补 |
| macOS 原生 keyring | 通过 | `IFG_KEYRING_TEST=1 cargo test -p ifg --bin ifg native_ --locked -- --test-threads=1`，2 项通过；只操作独立测试条目。恢复测试构造写失败后的文件状态，并使用真实钥匙串读写，不锁定用户钥匙串 |
| CNB CI | 核时不足，恢复中 | 默认 8 核预冻结超出剩余额度；本轮将节点设为 1 核，保留全部审计与测试门禁，待记录实跑结果 |
| GitHub Linux / Windows CI | 待运行 | 发布前验证用途；需记录候选 SHA 对应的 run |
| fork 删除场景 | 待运行 | 固定服务端提交 `1c1559f` 的隔离镜像；不修改 IronForge 工作区或当前部署 |
| 五平台候选产物 | 待运行 | 手动触发仅构建的 release workflow，合并 SHA256SUMS 并验证安装；Windows GUI/编辑器交互仍需实机检查 |

候选构建不能代表正式发布。没有实际执行的 Windows GUI、SmartScreen、浏览器与编辑器操作，不标记为通过。
