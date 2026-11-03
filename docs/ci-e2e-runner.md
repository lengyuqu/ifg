# e2e 的自托管 runner（可选）

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

```bash
# 一次性的本地实例（示例路径随意）
ironforge serve --repo-root ~/ci/ifg-e2e-server/repos \
  --db-url "sqlite://$HOME/ci/ifg-e2e-server/ironforge.db?mode=rwc"

# 建号（注册关闭时也能用），再登录换 JWT、换 PAT
ironforge create-user ci-e2e --email ci-e2e@ifg.local --password '<强密码>' --admin \
  --db-url "sqlite://$HOME/ci/ifg-e2e-server/ironforge.db?mode=rwc"
JWT=$(curl -s -X POST http://localhost:8080/api/v1/users/login \
  -H 'content-type: application/json' \
  -d '{"login":"ci-e2e","password":"<强密码>"}' | jq -r .token)
PAT=$(curl -s -X POST http://localhost:8080/api/v1/users/tokens \
  -H "authorization: Bearer $JWT" -H 'content-type: application/json' \
  -d '{"name":"ci-e2e","scopes":"repo,user,admin"}' | jq -r .token)
```

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
