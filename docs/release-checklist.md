# 发布验收清单

发布前为每次发布记录版本、候选提交 SHA、服务端镜像 ID 和对应日志链接。只有实际完成并核对的项目才能标记为通过；失败、跳过与未运行必须分别记录。

## 1. 开发基线

- [ ] 确认工作区干净、版本与 tag 一致，CNB `main` 包含候选 SHA；记录 CNB push 和 PR 的 CI 链接，确认工具链、格式、Clippy、man、测试、依赖边界及 cargo-audit 均通过。
- [ ] 在候选 SHA 的干净检出执行 `bash scripts/e2e-docker.sh`，记录镜像 ID、服务端版本及 5 项端到端结果；确认契约快照对应同一镜像。镜像无源码来源标签时记录该限制。
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
