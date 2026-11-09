# Windows 平台交接文档

> 目标读者：在 **Windows 机器**上有 shell + Rust 工具链的开发者（或 agent）。
> 读完这页就能开工，不需要再问上下文。
>
> 基线：`ifg 0.9.0`（main `0592d52`），120 个测试，三层质量网。macOS / Linux 已发布并在用；
> **Windows 从未编译过、也从未实机运行过**。

---

## 0. 一句话目标

让 `ifg` 在 Windows 上 **能编译、能跑通主要工作流、能装出来**，并把 CI 补上 Windows job，
使「平台支持」从"未验证"变成"CI 覆盖"。

### 验收定义（Definition of Done）

- [x] `cargo build --release -p ifg` 在 Windows 成功，产物 `target/release/ifg.exe`
- [x] `cargo test -p ifg -p ifg-client` 全绿（本机 115 passed；e2e 5 个 `--ignored` 另跑，5/5 通过）
- [x] `ifg doctor` 对可达的 IronForge 输出 `all good.`（本地 Docker IronForge 0.2.0，仅 `git` 一项 warn）
- [x] 手测通过：`auth login` → `repo list` → `issue create`（notepad）→ `pr list` → `pr diff`（Windows Terminal 有颜色）→ `completions powershell`；`ci logs --follow` 已用本地 Docker runner 验证（等待 pending → 日志到达 → stream closed），runner 在任务结束时一次性上传日志，逐行实时取决于服务端 runner 实现
- [ ] CI 新增 `windows` job 且绿（workflow 已加，待推送后首次运行）
- [x] `scripts/release.sh --windows` 产出 `ifg-0.9.0-x86_64-pc-windows-msvc.zip` 并进入 `SHA256SUMS`（本地已验证；release workflow 待 rc tag dry-run）

> 完成情况（2026-11-04）：W1–W7 已落地，提交见 `feat/windows-support`。
> 与本文初稿的差异：配置目录在 Windows 回退 `%APPDATA%\ironforge`（PowerShell 通常
> 没有 `HOME`）；颜色检测额外接受 `TERM`（mintty/Git Bash）；Windows keyring 的凭据管理器
> target 由 `keyring` crate 生成为 `<host_key>.ifg`（User 为 host_key），功能等价；
> 发布脚本新增 `--windows`，release workflow 用独立 `windows-latest` job 构建后合并
> `SHA256SUMS`。未签名 exe 可能被 Defender/杀软拦截，签名不在本次范围。
>
> 本地补充验证：Linux 容器门禁（fmt/clippy/test/man/依赖边界）通过；
> `cargo audit`（261 个依赖）无告警；`scripts/check.ps1` 已提供并通过。
> 本地 Docker 服务端为 `84ececc`（晚于 workspace pin 的 `ce4834f`）：重试 pipeline 的
> `triggered_by` 返回整数，`ifg ci runs` 会解码失败；升级 `ironforge_rev` 时需同步放宽
> 客户端字段类型。

---

## 1. 现状快照（不用重新调研）

| 维度 | 现状 |
|---|---|
| 命令面 | 17 个一级命令组，38 个类型化 API 操作，其余走 `ifg api` |
| 代码 | 11.7k 行 Rust；`unsafe_code = "forbid"`（workspace lints，**不能用 unsafe**） |
| 测试 | 120 个（wiremock 单测 + wiremock CLI 测试 + `ifg-client` 契约/模板/配置） |
| TLS | `reqwest` 用 **rustls-tls-native-roots**；`tokio-tungstenite` 用 **rustls-tls-webpki-roots**。**没有 OpenSSL 依赖** → 不需要 vcpkg |
| 颜色/TTY | `std::io::stdout().is_terminal()`，纯 std，可移植 |
| 浏览器打开 | `crates/ifg/src/web.rs` 的 Windows 分支用 `rundll32 url.dll,FileProtocolHandler`：不经 `cmd.exe`，URL 里的 `&`/`\|` 不会被当成第二个命令 |
| 分页器 | `crates/ifg/src/output.rs:93` 默认 `less -R`；spawn 失败会**自动退回直出**（不阻塞功能） |
| 凭据后端 | `crates/ifg/src/keyring.rs`：Windows 与 macOS 走 `keyring` crate（直接调用系统凭据 API，token 不进 argv）、Linux 走 `secret-tool`（token 走 stdin） |
| CI | `.github/workflows/ci.yml`：`check`（ubuntu，跑 `scripts/check.sh`）+ `e2e`（自托管 macOS runner）。**无 windows job** |
| 发布 | `scripts/release.sh` 只构建 apple×2 + musl×2，**无 Windows 目标** |

---

## 2. 任务清单（按顺序做，每条都给了验收命令）

### W0 环境准备

```powershell
# 1) Visual Studio Build Tools（C++ 生成工具）+ Windows SDK —— ring 需要 C 编译器
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
# 2) Rust（MSVC 工具链，不要装 GNU 版）
winget install Rustlang.Rustup
rustup default stable-x86_64-pc-windows-msvc
# 3) Git for Windows（提供 bash，用于 scripts/*.sh 与 git 凭据测试）
winget install Git.Git
```

校验：`cargo -V && rustc -Vv | findstr host` 应显示 `host: x86_64-pc-windows-msvc`。

> 注意：不要用 `x86_64-pc-windows-gnu`。`ring` 在 GNU 目标上需要额外 mingw 配置，官方路径是 MSVC。

### W1 编译打通（有已知阻塞，先修这个）

**阻塞**：`crates/ifg/src/config.rs` 在 **89 行**和 **136 行**调用了只在 unix 下定义的
`has_group_or_other_permissions` / `tighten_permissions`（定义在 166–181 行，带 `#[cfg(unix)]`），
非 unix 平台没有对应实现 → 编译失败。

建议改法（二选一，推荐前者，最小改动）：

```rust
// crates/ifg/src/config.rs 末尾，紧挨着两个 unix 版本
#[cfg(not(unix))]
fn has_group_or_other_permissions(_path: &std::path::Path) -> bool {
    // Windows 的 ACL 不是 POSIX 位；文件权限由用户目录 ACL 控制。
    false
}

#[cfg(not(unix))]
fn tighten_permissions(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}
```

语义说明（写进注释，评审会问）：Windows 上 `%APPDATA%` 默认只有当前用户可读，
不存在 POSIX 的 group/other 位，所以"放宽权限"这个告警不适用；不要静默改成 `true`。

已核对、无需改动的地方（别重复劳动）：
- `crates/ifg/src/cmd/mod.rs` 的 editor 组合测试整块 `#[cfg(unix)]` ✓
- `crates/ifg/src/web.rs` 已有 windows 分支 ✓
- `completions` 用 `clap_complete::Shell`，powershell 已支持 ✓
- `auth git-credential` 的 stdin 解析用 `str::lines()`，CRLF 天然兼容 ✓
- `ifg auth login --with-token` 读 stdin 后 `trim()`，CRLF 无影响 ✓

验收：

```powershell
cargo check -p ifg --target x86_64-pc-windows-msvc
cargo build --release -p ifg
.\target\release\ifg.exe --version   # 应输出 ifg 0.9.0
cargo test -p ifg -p ifg-client      # 120 个测试
```

### W2 keyring 后端（当前 Windows 完全缺失）

现状：`keyring.rs` 用 `std::process::Command::new("sh").args(["-c", "command -v …"])` 探测后端，
Windows 上没有 `sh`（除非装了 Git Bash，但不能假定）→ 永远退化到配置文件。功能不阻塞，但
`token_store = "keyring"` 在 Windows 上是**静默失效**，这是要修的体验问题。

推荐实现（安全、不用 unsafe）：

```toml
# crates/ifg/Cargo.toml
[target.'cfg(windows)'.dependencies]
keyring = { version = "3", default-features = false, features = ["windows-native"] }
```

```rust
// crates/ifg/src/keyring.rs
enum Backend { MacOsSecurity, SecretTool, WindowsCredentialManager }

fn backend() -> Option<Backend> {
    // 先按平台判定，再探测 CLI；Windows 不走 `sh`。
    #[cfg(windows)]
    { return Some(Backend::WindowsCredentialManager); }
    // …原有 macOS / Linux 分支保持不变
}
```

实现 `store/get/delete` 三个函数对应的分支即可（service 名沿用 `ifg`，account 用 `host_key`）。
**如果 `keyring` crate 在 MSVC 下引入意外依赖**，允许降级方案：Windows 上保持"仅文件存储"，
但在 `auth login` 输出一条明确警告（"keyring 存储在此平台不可用，令牌写入配置文件"），
并在 README 的平台表里标注。**不要用 `windows-sys` + `CredRead` 手写**——workspace 禁止 unsafe。

验收：

```powershell
ifg auth login --host http://localhost:8080 --with-token   # 配置里写 token_store = "keyring"
ifg auth token                                              # 能取回
# 打开"凭据管理器 → Windows 凭据"，应看到 service=ifg 的条目
```

### W3 终端体验：颜色 + 分页器

- 颜色：`output.rs` 直接写 ANSI 转义。Windows 10+ 的 Windows Terminal 默认支持；
  传统 `cmd.exe` 需要先开启 VT 处理（`SetConsoleMode` 含 `ENABLE_VIRTUAL_TERMINAL_PROCESSING`）。
  **不要为此引 unsafe**；可行做法：`#[cfg(windows)]` 时若 `WT_SESSION` / `TERM_PROGRAM`
  环境变量不存在则关闭颜色（等价于 `--no-color`），并在 `docs` 里说明。
- 分页器：默认 `less -R` 在 Windows 通常不存在（spawn 失败会退回直出，不报错）。
  建议 `#[cfg(windows)]` 默认改成 `more`，或直接不启用分页。

验收：`ifg pr diff 1` 在 Windows Terminal 有颜色；`ifg issue list | Out-File x.txt` 无 ANSI 转义残留。

### W4 git 凭据助手（`ifg auth git-credential`）

**换行已经没问题**：`crates/ifg/src/cmd/auth.rs:361` 用 `request.lines()`，Rust 的
`str::lines()` 会剥掉行尾 `\r`，git 传 `\r\n` 也能正确解析（已核对源码，不要重复修）。

真正需要改的是 **helper 语法**：Windows 上
`git config --global credential.helper 'ifg auth git-credential'` 会失败，必须写成
`!ifg.exe auth git-credential`（或带完整路径 `!'C:/tools/ifg.exe' auth git-credential`）。
README 的钥匙串/凭据段落（`README.md:87` 附近）要按平台给两种写法。

验收（PowerShell）：

```powershell
git config --global --add credential.helper '!ifg.exe auth git-credential'
git clone http://localhost:8080/git/OWNER/REPO.git   # 不输密码即可（凭据来自 ifg 配置）
```

### W5 编辑器与临时文件（`ifg issue create --editor`）

`crates/ifg/src/cmd/mod.rs` 的 `compose_in_editor` 用 `$VISUAL`/`$EDITOR` + `std::env::temp_dir()`。
Windows 注意：

- 历史情况：`EDITOR=notepad` 可用；交接时 `EDITOR="code --wait"` 会被当成完整程序名而失败。后续已添加参数解析，Windows 实测仍待补充。
- 临时文件路径含空格/中文时 `Command::new(editor).arg(path)` 是安全的（不经 shell）✓

验收：`$env:EDITOR="notepad"; ifg issue create --editor` 能打开记事本并成功建 issue。

### W6 发布产物

`scripts/release.sh` 只构建 apple/musl。Windows 需要：

- 目标 `x86_64-pc-windows-msvc`，产物 `ifg.exe`，打包成 `ifg-<ver>-x86_64-pc-windows-msvc.zip`
- 追加到 `SHA256SUMS`（格式不变：`<sha256>  <文件名>`）
- `scripts/bump-tap.sh` 的四架构字典**不受影响**（多出的行会被忽略），但 PR 里要说明这一点

注意：**Windows 构建必须在 Windows runner 上做**（`ring` 交叉编译需要 Windows SDK）。
推荐在 release workflow 里加一个 `windows-latest` job 产出 zip，再统一上传。

### W7 CI

在 `.github/workflows/ci.yml` 增加（`check` job 之后）：

```yaml
  windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      # scripts/check.sh 是 bash；Git Bash 在 windows-latest 上自带
      - name: checks (fmt, clippy, test)
        shell: bash
        run: |
          cargo fmt --all --check
          cargo clippy --workspace --all-targets -- -D warnings
          cargo test -p ifg -p ifg-client
```

**不要**在 Windows job 里跑 `cargo audit`（慢）和 `scripts/e2e.sh`（需要活服务端 + 自托管 runner）。
`ifg-client` 的依赖边界检查在 `check.sh` 里，可用 `bash scripts/check.sh` 之外的两行替代（如上）。

---

## 3. 本地自检

```powershell
# 快速（fmt + clippy，约 40s）
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings

# 完整（等同 CI 的非 e2e 部分）
cargo test -p ifg -p ifg-client
```

`scripts/check.sh` 需要 bash（Git Bash 可用：`& 'C:\Program Files\Git\bin\bash.exe' scripts/check.sh`）。
`scripts/check.ps1` 已提供：纯 PowerShell 等价门禁（fmt/clippy/man/test/audit/依赖边界），无需 Git Bash。

---

## 4. 交付方式（请严格遵守，仓库是多人/多 agent 并行）

1. 分支：`feat/windows-support`（或按任务拆分 `fix/windows-build`、`feat/windows-keyring`）
2. 只提交自己改的文件：**不要 `git add -A`**（本仓库历史上已经因此把别人未提交的工作卷进提交两次）
3. 提交前：`git status` 复核 → `cargo fmt` 只影响自己文件 → `scripts/check.sh --quick`（或等价命令）
4. 提交信息用 `git commit -F 文件`（消息里含 `${…}`、引号、反引号时，shell 会吃掉）
5. PR 描述里贴：`cargo test` 摘要 + `ifg doctor` 输出 + 手测截图（终端）
6. 需要同步更新的文档：
   - `README.md`：发布产物段落（第 11 行，现写「macOS 与 Linux 四架构」）补 Windows 产物；
     钥匙串段落（第 87 行）补 Windows 的凭据助手写法
   - `docs/architecture/ifg-project-design-2026-11.md`「已决事项」追加一条 Windows 决策
   - `docs/compatibility.md`（若已创建）里 Windows 的特殊行为（颜色默认关闭等）

---

## 5. 已知风险与坑

| 风险 | 说明 | 对策 |
|---|---|---|
| `ring` 构建失败 | 缺 VS Build Tools / Windows SDK | 按 W0 安装；确认 `host: x86_64-pc-windows-msvc` |
| rustls 根证书 | `rustls-tls-native-roots` 在 Windows 读系统证书库，企业代理下可能缺根 | 若报证书错误，先确认系统证书；必要时允许 `IRONFORGE_INSECURE=1`？**不要**默认关闭校验 |
| Defender/杀软 | 未签名的 `ifg.exe` 可能被拦 | 文档说明；后续可考虑签名（不在本次范围） |
| 路径大小写 | Windows 不区分大小写，`OWNER/REPO` 解析需保持原样透传 | 不要自己 `to_lowercase()` |
| 文件锁 | 升级时 `ifg.exe` 正在运行会替换失败 | 安装文档提示先退出 |
| CRLF | 配置/输出对比测试可能因换行不同失败 | 测试断言用 `contains`，不要断言尾随换行 |

---

## 6. 关键上下文（需要时再读）

- 设计文档：`docs/architecture/ifg-project-design-2026-11.md`（§5 命令面、§7 CI/e2e、§9 上游依赖、§11 已决事项）
- 依赖边界规则：`crates/ifg-client` 不得依赖 CLI 侧 crate（CI 会查，见 `scripts/check.sh`）
- 错误处理约定：`AppError` 分类（Usage/Other/…），退出码约定见 `crates/ifg/src/main.rs`
- 验收利器：`ifg doctor`（八项自检，`--json` 可脚本化）
- 上游服务端：`lengyuqu/ironforge`（Docker 部署见其 README「Docker 部署」段）

---

## 7. 明确不要做的事

- 不要重写 CLI 架构、不要引入 GUI/TUI 框架
- 不要改 `--json` 字段名、`-q/--template` 语义、退出码含义（这些是稳定面）
- 不要为了"Windows 能用"而引入需要 unsafe 的依赖（workspace 禁止 unsafe）
- 不要顺手 `cargo fmt --all` 到无关文件（仓库已 fmt-clean，但你只需要保证自己的文件）

---

有卡住的地方，把 `cargo check` 的报错原文 + `rustc -Vv` 贴给上游即可；本文档里的行号基于
`main@0592d52`，如果漂移了按函数名搜索（`has_group_or_other_permissions`、`backend()`、`page()`）。
