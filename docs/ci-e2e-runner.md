# E2E 自托管 runner（可选）

GitHub 的 `e2e` job 使用 `bash scripts/e2e-docker.sh` 启动**隔离的临时实例**，不读取部署实例的 URL 或 PAT。脚本使用 `Cargo.toml` 中固定的本地镜像 ID，结束时清理临时容器和数据；日志记录镜像、候选提交、工作区状态和服务健康信息。

## 1. Runner 前置条件

需要 macOS/Linux 自托管 runner、Git、Rust、Docker、`curl` 和 `openssl`。启动 runner 的用户必须能够访问 Docker 引擎，并预先拥有固定镜像：

```bash
baseline=$(sed -n 's/^ironforge_image_id = "\([^"]*\)"/\1/p' Cargo.toml)
docker image inspect "$baseline" --format '{{.Id}}'
bash scripts/e2e-docker.sh
```

若镜像不存在，先从可核对来源恢复该精确镜像；不要悄悄改为 `latest`。镜像 ID 和能力接口报告的构建提交记录在 `Cargo.toml`；能力接口自报信息不是供应链签名。更新基线时须同步镜像来源、`Cargo.toml`、OpenAPI 快照并重新验证。

## 2. 注册与开关

通过 GitHub 仓库 Settings → Actions → Runners 注册机器，添加 `ifg-e2e` 标签。检查在线状态：

```bash
gh api /repos/OWNER/REPO/actions/runners
gh variable set IFG_E2E_ENABLED --body true
```

当前工作流不需要 `IFG_E2E_URL` / `IFG_E2E_PAT` Secrets；已有 Secrets 不会被删除，但此 job 不再引用它们。运行日志中的 11 项是固定基线场景，不包含下面的新服务端兼容性专项。

## 3. 较新服务端的 fork 镜像专项

固定基线不发布 fork PR 镜像。专项必须显式选择具备该能力的镜像：

```bash
IFG_E2E_IMAGE=<已核对来源的兼容镜像> bash scripts/e2e-docker.sh --fork-mirror
```

该命令只运行 fork 专项，并额外验证：源分支删除、fork 仓库删除后，从 PR 目标仓库镜像恢复相同 head SHA。来源与本轮结果见 [发布验收记录](release-checklist.md)。正常基线测试通过不代表该专项已执行。

## 4. 安全边界

`e2e` job 只接受所有者仓库 `main` 的 push，并要求 `IFG_E2E_ENABLED=true`；fork PR 不运行自托管任务。保留工作流中的事件、分支与仓库所有者检查。

CI 不执行 `docker compose up`，不替换已有部署，不复用个人 PAT。所有新建账号、仓库和测试 Release 都位于隔离容器的临时数据目录内。

仍可手动连接专用测试实例：`IFG_E2E_URL` / `IFG_E2E_PAT` 配合 `scripts/e2e.sh`。这是与自动 CI 不同的操作，须自行确认实例和测试账号；禁止指向生产数据。

## 5. 关闭

```bash
gh variable set IFG_E2E_ENABLED --body false
```

GitHub 仅用于发布准备和发布 CI；日常开发门禁优先在 CNB 运行。
