# 仓库协作约定

- `github`（`lengyuqu/ifg`）是主仓库。日常开发、代码同步和 CI 均使用 GitHub（`.github/workflows/ci.yml` 在 push 到 `main` 和 pull request 时触发）。
- `cnb`（`lengyuqu/ifg`，https://cnb.cool/lengyuqu/ifg）是备份仓库，不作为日常主推送目标。旧的 CNB `zdzso/ifg` 与 git233 镜像仍失效，不要重新添加。
- 推送时先确认目标分支和提交，使用非强制推送；保留现有未提交改动。
