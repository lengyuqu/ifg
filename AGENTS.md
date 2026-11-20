# 仓库协作约定

- `github`（`lengyuqu/ifg`）是主要上游。日常开发、代码同步和 CI 优先使用 GitHub（`.github/workflows/ci.yml` 在 push 到 `main` 和 pull request 时触发）。
- `cnb`（`zdzso/ifg`）不再是主要上游，仅作备用；是否继续同步由用户决定，不要自动推送。
- git233 的 `zdzso/ifg` 是 CNB 仓库的镜像，通过镜像机制同步，无需手动推送或配置 git233 远端。
- 推送时先确认目标分支和提交，使用非强制推送；保留现有未提交改动。
