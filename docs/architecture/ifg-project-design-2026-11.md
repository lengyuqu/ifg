# ifg 项目设计（独立仓库落地版）

> **状态**：草案（Proposed） ｜ **日期**：2026-11-03 ｜ **前置文档**：[客户端 CLI 设计草案](client-cli-design-2026-11.md)（下称「草案」）
> **可信度**：标 `确认` 的条目已对照 IronForge 源码（本地 `ironforge` 仓库 `ce4834f`）；其余为设计提议，待评审。

草案假设 CLI 放在 IronForge 主仓库的 Cargo workspace 里（`crates/rg-client`、`crates/rg-cli-user`）。现在 `ifg` 是一个**独立仓库**（`aicodeing/ifg`），本文说明由此带来的变化，并把草案细化到可以直接开工的程度：仓库结构、crate 边界、关键决策、客户端库与 CLI 的内部设计、API 契约管理、测试、本地检查与发布、里程碑。

草案中的命令设计（§4）、配置格式（§5）、输出约定（§6）、服务端缺口（§7）仍然有效，本文只写增量和修正，不重复。

---

## 1. 独立仓库带来的变化

| 草案中的假设 | 独立仓库下的情况 | 处理 |
|------|------|------|
| 从 `rg-mcp/src/client.rs` 抽出 `rg-client`，`rg-mcp` 改为依赖它（草案 P0） | 跨仓库共享 crate 需要 git 依赖或发布到 registry，`rg-mcp` 的客户端只有 57 行，三个 GET 方法 | **不做共享**。`ifg-client` 在本仓库从零写；`rg-mcp` 保持现状。等 `ifg-client` 稳定后再评估是否让 `rg-mcp` 以 git 依赖引入（见 D3） |
| 端到端测试复用 `rg-http/tests/common`，或把测试放进 `rg-http/tests/` | 本仓库无法引用 IronForge 的测试私有模块 | 改为**黑盒**：启动真实 `ironforge serve` 二进制，经公开 API 引导出用户和 PAT（见 §7.3） |
| crate 名沿用 `rg-` 前缀 | `rg-` 是 IronForge workspace 的命名空间 | 本仓库用 `ifg-` 前缀，草案 §10.2 的命名问题随之消失 |
| 与服务端同步发布 | 两个仓库各自发版，CLI 可能连到更老或更新的服务端 | 新增版本协商与 API 契约快照（§6） |
| 用 IronForge 的 Release 发布 CLI | 本仓库托管在 git233（Gitea），不是 IronForge 实例 | 发布走 git233 的 Release；IronForge 上的自举发布（用 `ifg release upload` 发 `ifg`）作为后续目标 |

---

## 2. 仓库结构

```
ifg/
├── Cargo.toml                 # workspace，统一依赖版本与 lint
├── rust-toolchain.toml        # 固定 stable 版本，避免不同机器间漂移
├── crates/
│   ├── ifg-client/            # 库：类型化 REST 客户端，无 CLI 依赖
│   │   ├── src/
│   │   │   ├── lib.rs         # Client、ClientBuilder、Error
│   │   │   ├── http.rs        # 请求发送、鉴权头、重试、错误体解析
│   │   │   ├── pagination.rs  # Page<T>、Paginator<T>
│   │   │   ├── api/           # 按资源分组：repos / issues / pulls / pipelines / releases / users
│   │   │   └── types/         # 手写 DTO，按资源分文件
│   │   └── tests/             # wiremock 驱动的请求/响应测试
│   └── ifg/                   # 二进制 `ifg`
│       ├── src/
│       │   ├── main.rs        # 入口：解析参数 → 构造上下文 → 分发 → 映射退出码
│       │   ├── cli.rs         # clap 定义（全局参数 + 子命令树）
│       │   ├── context.rs     # host / repo / token 的解析与优先级
│       │   ├── config.rs      # hosts.toml 读写、0600 权限检查
│       │   ├── git.rs         # 调用系统 git：读 remote、clone、fetch、credential 协议
│       │   ├── output.rs      # 表格 / JSON / 颜色 / 分页器
│       │   ├── exit.rs        # 错误 → 退出码
│       │   └── cmd/           # auth / api / repo / issue / pr / ci / release
│       └── tests/             # assert_cmd 驱动的 CLI 测试（打 mock server）
├── tests/e2e/                 # 对本地 Docker 中 ironforge 的端到端测试（独立 crate，默认不跑）
├── fixtures/
│   ├── openapi/               # 从 IronForge 拉取的 openapi.json 快照
│   └── responses/             # 真实响应样本，供 DTO 反序列化测试
├── docs/
│   ├── architecture/          # 设计文档（本文、草案）
│   └── adr/                   # 单条决策记录（后续按需补）
├── scripts/                   # check.sh（提交前检查）、e2e.sh、sync-openapi.sh
└── LICENSE                    # MIT，与 IronForge 一致
```

**crate 依赖方向**：`ifg` → `ifg-client`，反向禁止；`ifg-client` 不依赖 clap、终端、配置文件，保证它也能被 `rg-mcp` 或别的工具直接用。

---

## 3. 关键决策

草案 §10 的待决问题，以及独立仓库新冒出的问题，逐条给出建议。

| # | 决策 | 建议 | 理由 |
|---|------|------|------|
| D0 | 实现语言：Rust 还是 Go | Rust | 与 IronForge 同一语言、同一套工具（cargo、clippy、cargo-audit），维护者无需切换；保留日后与 `rg-mcp` 共享 `ifg-client` 的可能（D3）；`serde` 适合按需声明部分字段的 DTO（D4）。Go 的优势（交叉编译简单、编译快、`gh`/`tea` 均为 Go）真实但有限：交叉编译用 `cargo zigbuild` 可解决（§8.2），而 `tea` 面向 Gitea API，代码无法直接复用 |
| D1 | 二进制名 | `ifg` | 短、好打。Homebrew 和 crates.io 的查重放到 M4 发布前做；若撞名，备选 `ironforge-cli`，命令内部不受影响 |
| D2 | crate 名 | `ifg-client`（库）、`ifg`（二进制） | 与仓库同名，不占 IronForge 的 `rg-` 前缀 |
| D3 | 是否与 `rg-mcp` 共享客户端 | 暂不共享 | 共享的收益（省掉 57 行）小于跨仓库版本耦合的代价。`ifg-client` 1.0 之后再提议 `rg-mcp` 通过 git tag 依赖 |
| D4 | DTO 来源 | 手写，加 OpenAPI 漂移检查 | 草案已说明生成不可行（OpenAPI 注解覆盖不全）。手写 DTO 只声明 CLI 用到的字段，未知字段一律忽略（不加 `deny_unknown_fields`），服务端加字段不会弄坏 CLI |
| D5 | `--json` 的数据来源 | 透传服务端原始 JSON（`serde_json::Value`），不经 DTO 重新序列化 | 与草案 §6「字段名即 API 字段名」一致，且不会因 DTO 只声明部分字段而丢数据 |
| D6 | token 存储 | M1 用 0600 明文文件；M3 加可选的 `keyring` 后端（`token_store = "keyring"`），默认仍为文件 | keychain 在 Linux 无桌面环境下不可用，不能作为默认 |
| D7 | TLS 实现 | `reqwest` 关闭默认特性，启用 `rustls-tls` + `rustls-native-certs` | 静态链接、交叉编译简单；读系统证书库，支持自签 CA 的内网实例 |
| D8 | 异步运行时 | `tokio`，只开 `rt` + `macros` + `time` + `fs` | 不用 `full`，减小体积 |
| D9 | 是否兼容 `tea` | 不兼容 | IronForge 目前没有 Gitea 兼容层（沿用草案判断） |
| D10 | 服务端最低版本 | `ifg` 声明 `MIN_SERVER_VERSION`，低于时警告但不阻断 | `/health` 已返回 `version`（`确认`：`rg-http/src/lib.rs::health`）。只警告是因为多数接口跨版本不变，硬阻断会误伤 |

---

## 4. `ifg-client` 详细设计

### 4.1 公开接口

```rust
let client = Client::builder("https://forge.example.com")
    .token(Secret::new("ifp_…"))      // 可选；匿名访问公开仓库时不传
    .user_agent(concat!("ifg/", env!("CARGO_PKG_VERSION")))
    .timeout(Duration::from_secs(30))
    .build()?;

let me   = client.users().me().await?;
let repo = client.repos().get("alice", "demo").await?;
let open = client.issues("alice", "demo")
    .list(IssueFilter { state: Some(State::Open), ..Default::default() })
    .take(50)                         // Paginator，自动翻页
    .collect().await?;
let raw  = client.raw(Method::GET, "users/me", None).await?;   // 给 `ifg api` 用
```

- `Secret` 包一层，`Debug` / `Display` 只输出 `ifp_xxxx…`，防止 token 被日志带出。
- 资源访问器（`repos()`、`issues(owner, repo)` 等）只是借用 `&Client` 的轻量结构体，不持有状态。
- 每个资源方法有两个变体：返回 DTO 的（给人读输出用）和返回 `serde_json::Value` 的 `*_raw`（给 `--json` 用，见 D5）。为避免方法数量翻倍，内部统一走 `send::<T>()`，`T = Value` 即为原始形态。

### 4.2 错误模型（修正草案 §7.4）

服务端错误体实际是嵌套结构（`确认`：`rg-http/src/error.rs::ErrorResponse`）：

```json
{ "error": { "code": "NOT_FOUND", "message": "…", "request_id": null } }
```

`code` 取值：`NOT_FOUND` / `BAD_REQUEST` / `UNAUTHORIZED` / `FORBIDDEN` / `CONFLICT` / `GONE` / `INTERNAL_ERROR` / `RATE_LIMITED`。

```rust
pub enum Error {
    Api { status: u16, code: Option<String>, message: String, request_id: Option<String> },
    Transport(reqwest::Error),   // 连接、TLS、超时
    Decode { body_snippet: String, source: serde_json::Error },
    Config(String),              // URL 非法等
}
```

解析顺序：先按 `ErrorResponse` 解析；失败（比如反向代理返回的 HTML 502）则 `code = None`，`message` 取 body 前 200 字节。CLI 按 `code` 优先、`status` 兜底来决定提示文案和退出码。

### 4.3 分页

- 请求固定带 `per_page=100`（服务端上限，`确认`：`rg-http/src/pagination.rs`），减少往返。
- `Paginator<T>` 是一个 `Stream`，读 `pagination.has_next` 决定是否继续；`.take(n)` 到量即停，不多拉一页。
- 防御：若服务端返回 `has_next = true` 但 `data` 为空，停止并告警，避免死循环。

### 4.4 重试与限流

- 只对**幂等方法**（GET / HEAD）重试；POST / PATCH / DELETE 一律不自动重试，避免重复建 Issue、重复合并。
- 可重试情况：连接错误、`429`、`502` / `503` / `504`。最多 3 次，指数退避（0.5s、1s、2s）加抖动；响应带 `Retry-After` 时以它为准（服务端维护模式的 `503` 会带，`确认`：`rg-http/src/middleware.rs`；限流的 `429` 未见该头，按退避处理）。
- 登录相关接口有单独的严格限流（草案 §4.1 提到的 10 次/60 秒），CLI 不调用密码登录，不受影响。

### 4.5 上传与下载

- Release 资产上传用流式 body（`reqwest::Body::wrap_stream`），文件名放 `x-asset-filename` 头（草案 §2 列出的服务端约定），不把整个文件读进内存。
- 下载边写边算 SHA-256，结束后与 `x-checksum-sha256` 比对；不一致删除临时文件。先写 `<name>.part`，校验通过后再 rename，避免留下半截文件。

---

## 5. `ifg` CLI 内部设计

### 5.1 执行流程

```
main
 ├─ cli::parse()                    clap 解析；用法错误由 clap 以退出码 2 结束
 ├─ Context::resolve(&args)         决定 host、token、owner/repo（惰性：不需要 repo 的命令不读 git）
 ├─ cmd::<group>::run(ctx, args)    调用 ifg-client，返回 Output
 ├─ output::render(Output, mode)    表格 / JSON
 └─ exit::code(&err)                错误 → 退出码 + stderr 提示
```

### 5.2 上下文解析（细化草案 §5）

host 的优先级不变：`--host` > `IRONFORGE_URL` + `IRONFORGE_PAT` > git remote 推断 > `default_host`。补充三条规则：

1. **`-R owner/name` 不含 host 信息**。它只决定仓库，host 仍按上面的顺序决定；若 `-R` 写成完整 URL（`https://host/git/owner/name`），则同时决定 host。
2. **环境变量只给了 `IRONFORGE_PAT` 没给 URL** 时，token 绑定到解析出的 host 上使用；只给 URL 没给 PAT 时，从配置文件取该 host 的 token。
3. **配置目录**：`$XDG_CONFIG_HOME/ironforge`，未设置时用 `$HOME/.config/ironforge`。不要用 `dirs::config_dir()`，它在 macOS 上返回 `~/Library/Application Support`，与草案约定不符。

### 5.3 调用 git

一律调用系统 `git` 子进程，不引入 gix / libgit2：

| 用途 | 命令 |
|------|------|
| 读 remote | `git config --get-regexp '^remote\..*\.url$'` |
| 当前分支（`pr create` 默认 head） | `git symbolic-ref --short HEAD` |
| 是否已推送 | `git rev-parse @{upstream}`，失败则提示先 push |
| clone / fetch | 直接透传，stdout/stderr 继承给用户 |

`auth git-credential` 只在请求的 `host` 与已配置 host 完全一致时才返回凭据，其他 host 一律静默不答，避免把 PAT 交给别的站点。

### 5.4 输出

- 表格用 `comfy-table`，TTY 下按终端宽度截断标题列；非 TTY 输出制表符分隔、无颜色，便于 `cut` / `awk`。
- 颜色遵守 `NO_COLOR` 和 `--no-color`；分页器遵守 `PAGER`，`--json` 时不启用分页器。
- 时间：人读输出用相对时间（「3 小时前」），`--json` 保留服务端原值。

### 5.5 退出码（修正草案 §6）

| 码 | 含义 | 判定 |
|----|------|------|
| 0 | 成功 | |
| 1 | 一般错误 | 其余所有错误 |
| 2 | 用法错误 | clap 默认行为 |
| 4 | 需要认证 | `UNAUTHORIZED` / `FORBIDDEN`，或本地没有 token |
| 8 | 资源不存在 | `NOT_FOUND` |

草案说「数值对齐 `gh`」不完全准确：`gh` 的 `2` 表示用户取消、没有 `8`。这里只有 `4` 与 `gh` 一致，`8` 是 `ifg` 自定义，文档里应如实说明。

### 5.6 日志与调试

`-v` 输出请求方法、URL、状态码、耗时；`-vv` 加上请求/响应头。`Authorization` 头一律打码为 `Bearer ifp_xxxx…`。实现上用 `tracing` + `tracing-subscriber`，默认级别 `warn`，日志写 stderr，不污染 `--json` 的 stdout。

---

## 6. API 契约与版本兼容

客户端与服务端分仓库发布后，最大的风险是服务端改了字段而 CLI 不知道。措施：

1. **契约快照**：`fixtures/openapi/openapi-<ironforge 版本>.json` 存一份服务端 `GET /api-docs/openapi.json`（`确认`：路由存在）。脚本 `scripts/sync-openapi.sh <url>` 拉取新快照；`cargo test` 中的契约测试遍历 `ifg-client` 用到的每个路径和字段，检查它们仍存在于最新快照里。OpenAPI 覆盖不全的接口登记在 `fixtures/openapi/uncovered.txt`，由端到端测试兜底。
2. **响应样本**：`fixtures/responses/` 存真实响应，DTO 测试直接反序列化这些文件。样本从 e2e 测试中录制，避免手写样本与现实不符。
3. **运行时探测**：`ifg auth status` 与 `ifg version` 调 `GET /health` 显示服务端版本；低于 `MIN_SERVER_VERSION` 时提示升级（D10）。注意 OpenAPI 文档中的 `info.version` 是硬编码的 `0.1.0`（`确认`：`rg-http/src/openapi.rs`），与实际版本 `0.2.0` 不一致，不能用它判断版本，应以 `/health` 为准（这一点值得在 IronForge 侧顺手修掉）。
4. **固定基线**：`Cargo.toml` 的 `[workspace.metadata.ifg]` 里写 `ironforge_rev = "<commit>"`，即本地 Docker 镜像所用的 IronForge 提交；契约快照取自同一提交。升级基线是一次显式的提交：重建镜像 → 同步快照 → 跑 e2e。

---

## 7. 测试

| 层 | 工具 | 覆盖 | 何时跑 |
|----|------|------|--------|
| DTO | `serde_json` + `fixtures/responses` | 每个 DTO 能解析真实样本 | 每次 |
| 客户端 | `wiremock` | 鉴权头、分页拼接、错误体解析（含非 JSON 错误体）、重试只发生在幂等方法上 | 每次 |
| CLI | `assert_cmd` + `wiremock` + `insta` 快照 | 参数解析、上下文优先级、表格与 JSON 输出、退出码 | 每次 |
| remote 推断 | 表驱动单测 | `/git/` 前缀、scp 写法、自定义端口、`.git` 后缀、未匹配 host | 每次 |
| 契约 | 快照比对（§6.1） | 用到的路径/字段仍存在 | 每次 |
| 端到端 | 本地 Docker 中的 `ironforge` | 草案 §8 的全链路 | 手动：改动 API 调用或升级基线时 |

### 7.1 端到端测试环境：本地 Docker

开发期的对接测试统一打本地 Docker 中的 IronForge，测试本身不负责启动服务端。

**启动服务端**（在 IronForge 仓库根目录，`确认`：`deploy/docker-compose.yml`）：

```bash
cp deploy/.env.example deploy/.env   # 填 IRONFORGE_JWT_SECRET（openssl rand -hex 32）
docker compose -f deploy/docker-compose.yml up -d --build
```

HTTP 映射到 `localhost:8080`，SSH 映射到 `localhost:2222`，数据在命名卷 `ironforge-data` 里，重启容器后仍保留。

**运行测试**：`scripts/e2e.sh`，本质是 `cargo test -p ifg-e2e -- --ignored`。

| 环境变量 | 默认值 | 作用 |
|----------|--------|------|
| `IFG_E2E_URL` | `http://localhost:8080` | 被测实例 |
| `IFG_E2E_PAT` | 无 | 提供则直接使用，跳过注册；用于关闭了注册的实例 |
| `IFG_E2E_SSH` | `ssh://git@localhost:2222` | `repo clone --ssh` 用例 |

**流程**

1. 轮询 `GET /health`（最多 30 秒）；连不上就失败，并在报错里提示上面的 `docker compose` 命令。同时打印服务端 `version`，便于对照基线。
2. 没有 `IFG_E2E_PAT` 时，经公开 API 拿到 PAT（路由均 `确认` 存在于 `rg-http/src/lib.rs`）：
   `POST /api/v1/users/register` → `POST /api/v1/users/login`（得 JWT）→ `POST /api/v1/users/tokens`（用 JWT，scope 选 `repo` 与 `user`）。
3. 用 `IRONFORGE_URL` / `IRONFORGE_PAT` 驱动 `ifg` 跑全链路，不写用户的配置文件（`XDG_CONFIG_HOME` 指向临时目录）；另有一个用例专测 `auth login --with-token` 写配置。
4. 涉及 commit 的步骤显式设置 `GIT_AUTHOR_*` / `GIT_COMMITTER_*`（草案 §8）。

**数据隔离**：因为数据卷会保留，每次运行的用户名和仓库名都带上运行 ID（如 `e2e-<unix 时间戳>`），互不冲突；测试结束时用 `DELETE /repos/{owner}/{name}`（`确认`：路由存在）删除本次建的仓库。测试用户会逐渐累积，需要清空时执行 `docker compose -f deploy/docker-compose.yml down -v`。

**与自建实例的分工**：本地 Docker 用于跑 e2e；你正在部署的自建 IronForge 用于手工验证（真实域名、TLS、反向代理下的行为），不对它跑会写数据的自动化测试。

需要验证的点：注册是否默认开放、第一个用户是否需要邮件激活。若不开放，先用 `IFG_E2E_PAT` 绕过；长期退路是让 `ironforge` 运维 CLI 增加一个 `create-user` 子命令（目前没有，`确认`：`rg-cli/src/main.rs` 的子命令列表），登记为上游需求。

---

## 8. 本地检查与发布

### 8.1 本地检查（暂不做 CI）

当前阶段不搭 CI；等仓库上传到 GitHub 后再接 GitHub Actions。在此之前，`scripts/check.sh` 在提交前手动跑：

1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`（不含 e2e）
4. `cargo audit`（安全公告；与 IronForge 一致，忽略项写在 `.cargo/audit.toml` 并注明理由与移除条件；未安装时跳过并提示）
5. 依赖边界检查：`cargo tree -p ifg-client` 中不得出现 `clap`、`comfy-table` 等 CLI 依赖

脚本只调用 cargo 和 shell，不绑定任何 CI 平台，日后接 GitHub Actions 时 workflow 直接调用它即可。e2e 依赖本地 Docker，不放进 `check.sh`。

### 8.2 发布

- 目标平台：`aarch64-apple-darwin`、`x86_64-apple-darwin`、`x86_64-unknown-linux-musl`、`aarch64-unknown-linux-musl`。Linux 用 musl 静态链接，配合 D7 的 rustls 即可不依赖系统 OpenSSL。
- 没有 CI 时在本机构建：macOS 两个目标原生编译；Linux 目标用 `cargo zigbuild` 交叉编译，不需要 Docker 或 Linux 机器。打包成 `scripts/release.sh`，上 GitHub 后改由 Actions 调用。
- 产物：`ifg-<version>-<target>.tar.gz` + `SHA256SUMS`，上传到仓库所在平台的 Release（目前是 git233，迁到 GitHub 后改为 GitHub Release）。
- 版本号：`ifg` 独立 semver，1.0 之前允许命令行为变动；`--json` 输出的字段即 API 字段，不另作承诺。
- Homebrew tap 与 crates.io 发布放在 M4，发布前完成 D1 查重。

---

## 9. 里程碑（替换草案 §9）

| 里程碑 | 内容 | 验收 |
|--------|------|------|
| M0 脚手架 | workspace、两个 crate 空壳、toolchain、`scripts/check.sh` | `cargo test` 与检查脚本通过 |
| M1 客户端与骨架 | `ifg-client` 的 HTTP 层、错误、分页、重试；`ifg auth login/status/logout/token`、`ifg api`、配置文件、上下文解析、`--json` | 对本地 Docker 实例 `ifg api users/me` 成功；wiremock 测试覆盖 §4.2–4.4 |
| M2 核心协作 | `repo`、`issue`、`pr`（不含 checkout）；契约快照与漂移检查；`scripts/e2e.sh` 与引导流程 | e2e：建仓 → Issue → PR → 合并 |
| M3 流水线与 Release | `ci`（轮询日志）、`release`（流式上传、校验下载）、`pr checkout`、`auth git-credential`、keyring 后端（可选） | e2e 全链路；在自建 IronForge 实例上手工验证 |
| M4 发布 | `scripts/release.sh` 交叉编译、Release 产物、名称查重、Homebrew tap | 从 Release 下载的二进制在干净机器上可运行 |
| 后续 | 上传 GitHub，接 GitHub Actions（调用 `check.sh` / `release.sh`） | 不在当前范围 |

### 9.1 需要 IronForge 上游配合的事项

按对 CLI 的影响排序，均应在 IronForge 仓库单独立项：

1. ~~WebSocket `/ws/job/{job_id}` 接受 PAT~~ —— **已完成**（ironforge `df0ba3f`：Authorization 头与 PAT 解析；客户端 `ci logs --follow` 走 WS，握手失败自动退回轮询）。
2. ~~`GET releases/tags/{tag}`~~ —— **已完成**（ironforge `a8b8103`，客户端 `releases::get_by_tag` 带翻页兜底）。
3. ~~维护 `refs/pull/<n>/head`~~ —— **已完成**（ironforge `a8b8103`：PR 创建时写入、push 时随 head 同步；客户端 checkout 在分支缺失时回退）。
4. ~~OpenAPI `info.version` 改为读 `CARGO_PKG_VERSION`~~ —— **已完成**（ironforge `a8b8103`，快照生成时验证为 `0.2.0`）。
5. 视 §7.1 验证结果，增加 `ironforge create-user` 运维命令。**未做**
6. `GET /users/{username}`（公开资料）—— **已完成**（ironforge `4cb2b06`，`--assignee` 名字解析用）。
7. label PATCH 静默不生效（SeaORM ActiveModel 全 Unchanged）—— **已修复**（ironforge `4cb2b06`）。

---

## 10. 已定事项（2026-11-03）

1. **许可证**：MIT，与 IronForge 一致，见根目录 `LICENSE`。
2. **CI**：当前不做；上传 GitHub 后再接 GitHub Actions（§8.1）。
3. **对接测试环境**：本地 Docker 中的 IronForge（§7.1）；自建实例用于手工验证。

## 11. 已决事项（追加）

- **v0.7 决策（2026-11-04）**：
   - **CLI 健康监控（ironforge `22f43c8`）**：`/metrics`（需令牌）上新增
  `pat_auth_total{outcome=ok|invalid_token|scope_denied|inactive_owner}` 与
  `ws_connections_total{channel}` / `ws_active_connections{channel}`。
  排障口径：`invalid_token` 突增 → 用户令牌过期或被删；`scope_denied` → 令牌缺 scope；
  `inactive_owner` → 账号被停用；`ws_active_connections` 长期不降 → 客户端未正常关闭；
  `ws_connections_total` 增速远高于活跃数 → 反复重连（看服务端重启或代理超时）。
- 契约测试上游化：ifg 依赖的 7 个接口形状搬到 IronForge 仓库
     （`crates/rg-http/tests/cli_contract_tests.rs`，ironforge `dba4366`）：
     users/{username}、releases/tags、orgs members 的 username、label PATCH 落库、
     openapi 版本、PAT 用于 HTTP 与 WS、PR 详情与行内评论字段。
     分工：上游测试防「形状变了」，ifg 的 `fixtures/openapi` 快照防「接口消失了」。

   - 文档面：`--help` 走 clap 长描述（`long_about` + `after_help` 示例），`-h` 保持一行摘要；
     man 页由 `clap_mangen` 从同一份定义生成（`crates/ifg/examples/gen_man.rs`），`man/*.1` 入库，
     `scripts/gen-man.sh --check` 进 check.sh/CI，归档内附 `man/`，brew formula `man1.install`。
   - 理由：命令面已覆盖 8 组，README 不可能逐条讲；man 页与 `help` 同源，不会漂移。

   - `ifg org` 命令组：list / view / create / members / add-member / remove-member；成员一律按**用户名**操作（add-member 经 `GET /users/{username}` 解析 id），服务端成员列表补 username/display_name（ironforge `1de3e55`），未解析到则回退 `#id` 展示。
   - 团队（teams）暂不做：权限模型复杂、使用频率低，需要时走 `ifg api`。

- **v0.6 决策（2026-11-04）**：
   - `ci logs --follow` 首选 WebSocket（`?since=<已打印行数>` 只补增量），`tokio-tungstenite` 只进 CLI crate，不进 `ifg-client`（保持客户端无 WS 依赖）；首次握手失败 → 退回原轮询；已建立流后中途断开 → 带 `since=<已打印行数>` **自动续传**（250ms→2s 退避，最多 10 次），不会重复打印。
   - WS 鉴权四个来源：cookie → 子协议 → **Authorization 头** → `?token=`；JWT 校验失败时用 `pat_actor` 解析 PAT。e2e 断言"缺 job 返回 404 而非 401"以证明 PAT 生效。

- **v0.4 决策（2026-11-04）**：
   - `--template` 做子集（`{{.path}}`、`\n` 转义、`{{range}}…{{end}}` 可嵌套）：列表仍逐项渲染，`range` 用于嵌套数组（如 issue 的 labels）；不引模板引擎依赖。
   - `search` 直接映射 `/search`（results/total/page 信封）；`notifications` 覆盖 list/read/delete/count。
   - `issue|pr create --editor`：文件首行标题、其余正文，`#` 开头行丢弃；`$VISUAL` 优先于 `$EDITOR`。
   - `pr diff` 复用输出层能力（TTY 上色 + `$PAGER`）。
   - 上游 `ironforge create-user`（`79bf88d`）：直接写库 + `--admin`；`create_user` 为 SSO 置 `auth_provider=oauth2` 而密码登录只认 `local`，新增 `user_ops::set_auth_provider` 在建号后置为 `local`（实测可登录）。

- **fork PR checkout（2026-11-03 M3 后续）**：服务端 PR 详情响应补
   `head_repo_owner` / `head_repo_name`（ironforge 7c9f07e），客户端据此从
   fork 仓库 fetch；重复 checkout 改为快进更新而非报错。e2e 双用户覆盖。
1. **二进制名查重（D1，2026-11-03 M4 实测）**：`ifg` 在 crates.io（404）、
   Homebrew formula 与 cask（404）、Repology 各发行版包名（404）均未占用，
   本机 PATH 无冲突。名称 `ifg` 确认可用，无需备选。
2. 本地 Docker 实例的注册默认开放（M2 e2e 已实测，见 2026-11-03 提交记录）。
