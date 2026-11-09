# e2e 的自托管 runner（可选）

本地优先使用 `bash scripts/e2e-docker.sh`：它读取 `Cargo.toml` 中固定的本地镜像 ID，启动隔离实例并在结束时清理容器和临时数据。需要 Docker、`curl`、`openssl`、Git 和 Rust；设置 `IFG_E2E_IMAGE` 可测试另一张已存在的本地镜像。脚本打印镜像 ID、客户端提交、工作区状态和服务健康信息，不复用 `IFG_E2E_PAT`。该本地测试尚未接入 CNB 自动流水线。

当前基线服务版本为 0.2.0，镜像 ID 和服务端能力接口自报的构建提交记录在 `Cargo.toml`；OpenAPI 快照从该镜像启动的隔离实例获取。镜像没有 OCI 源码提交标签，自报构建信息不是供应链签名。更新基线时记录可验证的镜像来源、更新 `Cargo.toml`，从新实例用 `scripts/sync-openapi.sh` 更新快照，并运行契约检查和隔离 e2e。

`scripts/e2e.sh` 驱动真实的 `ifg` 二进制去连一台**在跑的 IronForge 实例**：建仓、开 PR、合并、走 CI、发布 release。
公共 runner 到不了内网实例，所以这段放在自托管 runner 上，并用开关控制。

## 1. 准备一台能访问 IronForge 的机器

runner 需要：`git`、Rust 工具链（`rustup`）、以及到 IronForge 的网络连通性。
本机示例（macOS arm64）：

```bash
mkdir -p ~/ci/actions-runner-ifg && cd ~/ci/actions-runner-ifg
gh release download --repo actions/runner \
  --pattern 'actions-runner-osx-arm64-*.tar.gz' --dir . --clobber
tar xzf actions-runner-osx-arm64-*.tar.gz

# 用仓库级 registration token 注册，打上 ifg-e2e 标签
TOKEN=$(gh api -X POST /repos/OWNER/REPO/actions/runners/registration-token --jq .token)
./config.sh --unattended --url https://github.com/OWNER/REPO \
  --token "$TOKEN" --labels ifg-e2e --name mac-ifg-e2e --replace
./run.sh            # 或注册成 launchd/systemd 服务
```

`gh api /repos/OWNER/REPO/actions/runners` 里 `status: online` 即可。

## 2. 准备 e2e 用的实例与凭据

实例跑 Docker 容器（与用户实际部署同一镜像，顺带持续验证 Docker 路径）：

```bash
cd ../ironforge                     # 仓库根
export IRONFORGE_JWT_SECRET=$(openssl rand -hex 32)
docker compose up -d --build        # 首次约 11 分钟；之后改代码只需重建

# 建号（注册关闭时也能用）；容器内执行，DB 在 /data
docker compose exec ironforge ironforge create-user ci-e2e \
  --email ci-e2e@ifg.local --password '<强密码>' --admin \
  --db-url "sqlite:///data/ironforge.db?mode=rwc"
```

再换一个 PAT 给 CI：

```bash
JWT=$(curl -s -X POST http://localhost:8080/api/v1/users/login \
  -H 'content-type: application/json' \
  -d '{"login":"ci-e2e","password":"<强密码>"}' | jq -r .token)
PAT=$(curl -s -X POST http://localhost:8080/api/v1/users/tokens \
  -H "authorization: Bearer $JWT" -H 'content-type: application/json' \
  -d '{"name":"ci-e2e","scopes":"repo,user,admin"}' | jq -r .token)
```

**更新服务端**：`cd ../ironforge && docker compose up -d --build` 后旧容器被替换、
数据在 `ironforge-data` 卷里不受影响。不再需要「手动 cp 二进制」那一步。

## 3. 打开开关

```bash
gh variable set IFG_E2E_ENABLED --body true
gh secret set IFG_E2E_URL --body 'http://127.0.0.1:8080'
gh secret set IFG_E2E_PAT --body "$PAT"
```

之后每次推到 main，`.github/workflows/ci.yml` 的 `e2e` job 会跑
`cargo test -p ifg-e2e -- --ignored`（三个用例：完整链路 / fork checkout / 元数据与 status）。

## 安全边界

`.github/workflows/ci.yml` 里 `e2e` job 的条件是**推到 main 且仓库属于所有者**：

```yaml
if: >-
  ${{ vars.IFG_E2E_ENABLED == 'true'
      && github.event_name == 'push'
      && github.ref == 'refs/heads/main'
      && github.repository_owner == 'OWNER' }}
```

自托管 runner **绝不能**执行 fork PR 的代码——那等于让陌生人在你的机器上跑命令。
这个条件保证 `pull_request` 事件永远不会命中 e2e job；改 condition 时请保持这一点。

## 关闭

```bash
gh variable set IFG_E2E_ENABLED --body false
# 需要彻底摘掉 runner：./config.sh remove --token <removal-token>
```
