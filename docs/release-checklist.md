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
| 核心修复源码 | 已提交 | `2ee746c45c4220edb0404dfeb0943011f0e6664f`；验收补充源码 `70c85734a48bcb34f903b23b2b0081020cb1f292` |
| 本地核心门禁 | 通过 | 补充后 251 项测试；fmt、Clippy、man、依赖边界与 cargo-audit 通过 |
| 固定基线 E2E | 通过 | `70c8573` 的干净检出，11 项通过；镜像 `sha256:257a6167ab94a4735abfe53060d758bbe080b36f0f419aed0950d8ac208fa627` |
| macOS 原生 keyring | 通过 | `IFG_KEYRING_TEST=1 cargo test -p ifg --bin ifg native_ --locked -- --test-threads=1`，2 项通过；只操作独立测试条目。恢复测试构造写失败后的文件状态，并使用真实钥匙串读写，不锁定用户钥匙串 |
| CNB CI | 已恢复并通过 | [`70c8573` 的完整门禁](https://cnb.cool/zdzso/ifg/-/build/logs/cnb-r0i-1k70oi6eu)；节点设为 1 核，工具链、审计、检查全部实际执行并成功 |
| GitHub Linux / Windows CI | 待运行 | 发布前验证用途；需记录候选 SHA 对应的 run |
| fork 删除场景 | 通过 | 干净 `70c8573`，`--fork-mirror` 专项 1 项通过：删除源分支、再删除 fork 仓库后，仍从目标镜像恢复相同 head SHA |
| 兼容服务端来源 | 已核对 | 从提交 `1c1559f32bd777cfb5c5c6279027f2ea60dfd75c` 的 `git archive` 构建；镜像 `sha256:a1900f680da2ebe9cc3f831e614761476a5e0d774adf62ef5e69f5ca651c0f8d`；能力接口报告相同提交、`build_dirty=false`；未修改 IronForge 工作区或部署 |
| 本地 Unix 候选包 | 构建与哈希通过 | 4 个 tar.gz 和 man 页齐全；macOS arm64、Linux arm64/amd64 容器基本运行通过，Linux `/dev/full` 写入错误返回非零；本机无 Rosetta，macOS x86_64 运行留给 Intel GitHub runner |
| 五平台 GitHub 候选产物 | 待运行 | 手动触发仅构建的 release workflow，合并 SHA256SUMS 并验证安装；不发布 Release |

候选构建不能代表正式发布。负责人选择本轮仅完成自动验证：Windows GUI、SmartScreen、浏览器与编辑器交互明确列为后续实机待办，不标记为通过。
