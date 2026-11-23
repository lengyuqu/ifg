# 仓库协作约定

- `github`（`lengyuqu/ifg`）是唯一远端。日常开发、代码同步和 CI 均使用 GitHub（`.github/workflows/ci.yml` 在 push 到 `main` 和 pull request 时触发）。
- CNB（`zdzso/ifg`）远端已于 2026-11-23 按用户要求移除，不再同步；git233 镜像随 CNB 一并失效，不要重新添加这两个远端。
- 推送时先确认目标分支和提交，使用非强制推送；保留现有未提交改动。
