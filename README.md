# ifg

IronForge 的命令行客户端：在终端里通过 REST API 操作仓库、Issue、PR、CI 和 Release，定位类似 GitHub 的 `gh`、Gitea 的 `tea`。

> 状态：v0.9.0 已发布：`auth` / `api` / `repo` / `issue` / `pr` / `ci` / `release` 全套命令，含流式上传下载、SHA-256 校验、git credential helper、终端感知输出（颜色 / 分页器 / 相对时间）与命令补全。

与 IronForge 自带的 `ironforge` 二进制不同，`ifg` 不连数据库、不做运维，只用个人访问令牌（PAT）调用公开 API，可以装在任何机器上。

## 安装

发布产物：macOS（arm64/x86_64）与 Linux（musl 静态链接，arm64/x86_64）四架构 tar.gz、Windows（x86_64）zip，另附 `SHA256SUMS`，见
[Releases](https://github.com/lengyuqu/ifg/releases)。

Homebrew：

```bash
brew tap lengyuqu/ifg https://github.com/lengyuqu/homebrew-ifg.git
brew install lengyuqu/ifg/ifg
```

SSH 克隆：`ifg repo clone OWNER/NAME --ssh`（`ssh://git@HOST:2222/...`，端口可用 `IRONFORGE_SSH_PORT` 覆盖）。

手动安装：

```bash
# 下载对应架构的 tar.gz 与同一 Release 的 SHA256SUMS
archive='ifg-0.9.0-<target>.tar.gz'  # 替换为实际文件名
if awk -v f="$archive" '$2 == f { print; n++ } END { if (n != 1) exit 1 }' SHA256SUMS > "$archive.sha256" &&
   shasum -a 256 -c "$archive.sha256"; then
  tar xzf "$archive" && sudo mv ifg /usr/local/bin/
else
  echo '校验失败或找不到唯一的校验记录，停止安装' >&2
fi
```

Windows（PowerShell）：

```powershell
$archive = 'ifg-0.9.0-x86_64-pc-windows-msvc.zip'
$entries = @(Get-Content .\SHA256SUMS | Where-Object { $_ -match ('^[0-9a-fA-F]{64}\s+\*?' + [regex]::Escape($archive) + '$') })
if ($entries.Count -ne 1) { throw '找不到唯一的校验记录，停止安装' }
$expected = ($entries[0] -split '\s+')[0]
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expected) { throw 'SHA-256 不匹配，停止安装' }
Expand-Archive -LiteralPath $archive -DestinationPath C:\tools\ifg
# 把 C:\tools\ifg 加入 PATH；升级前先退出正在运行的 ifg.exe
```

从源码（musl 交叉编译需 `zig` + `cargo-zigbuild`）：

```bash
scripts/release.sh   # 构建四架构产物到 dist/
cargo install --git https://github.com/lengyuqu/ifg.git ifg
```

## 快速上手

```bash
ifg auth login --host https://your-forge.example.com   # 粘贴 Web 端创建的 PAT（不回显）
ifg version                                            # 客户端 + 服务端版本
ifg repo create demo --init
ifg repo collaborators list -R yourname/demo            # 直接协作者及用户/记录 ID
ifg repo collaborators add alice --permission read -R yourname/demo
ifg repo collaborators set alice --permission write -R yourname/demo
ifg repo collaborators remove alice -R yourname/demo   # 交互确认；脚本可加 --yes
ifg issue list -R yourname/demo
ifg pr checkout 12                                     # 同仓或 fork PR 都可用，重复执行会快进更新；分支已删时走 refs/pull
ifg pr view 12 --comments                              # 含行内评审评论
ifg pr create -t "标题"                                 # head 默认当前分支，未推送会拦截
ifg pr edit 12 --title '新标题' --ready               # 修改标题并标记为可评审
ifg pr close 12 && ifg pr reopen 12                    # 关闭、重新打开
ifg pr comment 12 -b '请检查测试结果'                   # 提交普通评审评论
ifg pr reviews 12                                      # 查看评审及普通评论
ifg pr reviewers add 12 alice                          # 请求评审；也可 list / remove
ifg ci logs 7 --follow                                 # WebSocket 流式日志（断线自动续传；无 WS 时退回轮询）
ifg ci run --ref refs/heads/main --wait --timeout 600  # 触发并等待；也可传提交 SHA
ifg ci wait 7 --timeout 600                            # 等待已有流水线
ifg label create bug -c ff0000 -d "坏了"                # 标签
ifg milestone create v1 --due 2026-12-31               # 里程碑
ifg issue edit 3 --label bug --milestone v1 --assignee alice
ifg doctor                                             # 一条命令自检：主机/令牌/权限/版本/流式
ifg status                                             # 当前分支的 PR + 最近流水线
ifg search "登录 超时" --type issue                      # 全局搜索（repo / issue / wiki）
ifg notifications                                      # 通知列表；notifications count 只看未读
ifg org list                                           # 组织；org members / add-member / remove-member
ifg org create acme -d "Acme 团队"                      # 建组织，成员用 --role 指定
ifg issue create --editor                              # 用 $EDITOR 写标题与正文
ifg pr list -q '{{.number}} {{.title}}'                 # 轻量模板输出（列表逐行渲染）
ifg issue view 3 -q '{{range .labels}}{{.}} {{end}}'    # range 遍历嵌套数组
ifg api users/me --jq '.username' --jq-raw             # 使用系统 jq 处理 GET 响应
ifg wiki create '使用说明' --file README.md -R yourname/demo
ifg wiki view '使用说明' -R yourname/demo
ifg wiki edit '使用说明' --file - -R yourname/demo      # 从 stdin 读取并替换页面内容
ifg wiki edit '使用说明' --file page.md --expected-version '12:34' -R yourname/demo  # 使用之前读取的版本
ifg repo edit -R yourname/demo --description "新描述" --visibility private --default-branch main
ifg wiki history '使用说明' -R yourname/demo
ifg wiki delete '使用说明' -R yourname/demo             # 交互确认；脚本可加 --yes
ifg release create v1.0.0 -n "说明" dist/pkg.tar.gz     # 建 Release 并流式上传
ifg release edit v1.0.0 -n "修订说明" --prerelease       # 改发布说明/标记
ifg release download v1.0.0 -p 'pkg*'                   # SHA-256 校验后落盘
git config --global credential.https://your-forge.example.com.helper "!ifg auth git-credential"   # git clone/push 免密
```

Wiki 编辑要求服务端支持版本保护。未指定 `--expected-version` 时，ifg 在提交前读取最新版本，仅保护这次读取之后的并发修改；若正文基于更早的读取，请从 `ifg wiki view --json` 保存版本并显式传入。`repo edit` 同样支持 `--expected-version`。旧版服务端不提供版本时，ifg 会拒绝无保护的更新。

`pr comment` 提交的是带正文的普通评审（`action=comment`），不是文件行内评论；可通过 `pr reviews` 查看。行内评论仍由 `pr view --comments` 查看。

`--editor` 支持带参数的 `VISUAL` 或 `EDITOR`，例如 `EDITOR='code --wait'`；含空格的程序路径需加引号，例如 `EDITOR='"/path with spaces/editor" --wait'`。命令直接启动程序而不经过 shell，不会展开变量、管道或重定向；可执行文件路径本身含空格时也可单独指定完整路径。

`ifg api <path> --jq '<表达式>'` 使用 PATH 中的 `jq` 处理 GET 的 JSON 响应，`--jq-raw` 输出未加引号的字符串；缺少 jq、响应不是 JSON 或表达式出错时返回非零。`--paginate --jq` 将各页合并为数组后再执行表达式。该选项当前仅属于 `api` 子命令且仅支持 GET；其他子命令可继续使用 `--json | jq ...`。

`ifg api` 输出服务端原始字节，**不做**终端控制字符清洗（它的用途是管道给 jq 或重定向到文件）。其余子命令会剥离 `ESC` 等控制序列，避免仓库里的正文劫持终端；直接查看不受信任的原始响应时请走 `--jq` 或 `| cat -v`。

Wiki 的 `edit` 替换整个页面，并使用服务端版本进行并发保护；正文基于较早读取时应显式传入当时的 `--expected-version`。固定基线已修复单条历史版本接口的跨仓库授权问题；`history` 列出当前授权仓库中该页面的历史。旧服务端可能仍受影响，应先更新部署。

`ci run` 的 ref 由 IronForge 服务端解析，省略时使用服务端默认的 `refs/heads/main`。`ci wait` 或 `ci run --wait` 只有流水线成功才返回 0；失败、取消、人工门禁、未知状态和超时都返回非零。人工门禁可先用 `ci view` 查看作业并按服务端流程处理。触发成功后即使等待超时也不会取消流水线，流水线编号会输出到 stderr。

仓库协作者权限为 `read`、`write` 或 `admin`；列表仅包含直接协作者，用户 ID 和协作者记录 ID 是不同的值。权限调整后会重新读取确认是否生效；旧版服务端可能存在更新未持久化的问题，CLI 会报错而不声称成功。

凭据助手按主机配置，只对 IronForge 生效，不会替换其他站点已有的 helper（如 osxkeychain）。

Windows 上 helper 必须写成 `.exe`（Git 不接受无扩展名的裸命令）：

```powershell
git config --global credential.https://your-forge.example.com.helper '!ifg.exe auth git-credential'
```

手册（man 页随归档一起发布，Homebrew 安装时自动放进 man1）：

```bash
man ifg            # 总览：全局开关、退出码、示例
man ifg-pr         # 单命令：ifg pr ..., ifg org ..., ifg ci ...
ifg help pr        # 同一份长描述，不装 man 也能看
```

命令补全：

```bash
ifg completions zsh  > ~/.zfunc/_ifg      # 或 bash / fish / powershell
ifg completions bash > /etc/bash_completion.d/ifg
```

## 配置与凭据

- 配置：`$XDG_CONFIG_HOME/ironforge/hosts.toml`；Windows 默认 `%APPDATA%\ironforge\hosts.toml`（POSIX 的 0600 告警不适用，权限由用户目录 ACL 控制）
- 钥匙串（可选）：`token_store = "keyring"` 时 PAT 存系统钥匙串（macOS 钥匙串 / Linux `secret-tool` / Windows 凭据管理器），不落文件；macOS 与 Windows 直接调用系统凭据 API，token 不会出现在进程参数里
- `--json` 输出服务端原始字段；`-q/--template` 走轻量子集（`{{.field}}`、`{{.a.b}}`、`{{range .list}}…{{end}}`，列表自动逐项渲染）
- 管道输出自动关闭颜色与相对时间；`pr diff` 在终端里上色并走分页器
- Windows 终端：仅在 Windows Terminal / VS Code / mintty（`WT_SESSION`、`TERM_PROGRAM`、`TERM`）下启用颜色，传统 `cmd.exe` 等价 `--no-color`；分页器默认直出，可设 `PAGER` 覆盖

## 多 Agent 协作

固定基线的本地镜像 ID 和 OpenAPI 快照包含协作接口；不同部署应先确认其服务端具备这些接口。用户 PAT 用于身份和仓库权限；每个 Agent 还需要独立的会话凭据。会话名称和消息正文均是自报内容，不是安全身份或可信指令。

```bash
# 每个 Agent 使用各自的新文件；文件不会覆盖已有文件
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" session start --name 'Agent A' --client codex
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-b.json" session start --name 'Agent B' --client codex
ifg -R owner/repo collab session list
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" task create --title '实现功能' --owner-session '<A 的会话 ID>'
ifg -R owner/repo collab task list
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" message send '<任务 ID>' --key message-1 --body '已完成初步实现' --target-session '<B 的会话 ID>'
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" handoff create '<任务 ID>' --key handoff-1 --to '<B 的会话 ID>' --summary '请继续验证'
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-b.json" inbox
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-b.json" handoff accept '<交接 ID>' --expected-version '<交接版本>'
# Agent 发起问题；不带 --session-file 时以用户身份操作
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-b.json" question ask '<任务 ID>' --key q-1 --title '选择方案' --body '请选择' --option 方案一 --option 方案二
ifg -R owner/repo collab inbox --user
ifg -R owner/repo collab question view '<问题 ID>'
ifg -R owner/repo collab question answer '<问题 ID>' --key a-1 --expected-version '<版本>' --context-hash '<已查看的 context_hash>' --option-id o1
ifg -R owner/repo collab task reassign '<任务 ID>' --key move-1 --expected-version '<任务版本>' --expected-owner '<原会话 ID>' --to '<新会话 ID>' --reason '用户重新分配'
```

会话文件含仅展示一次的密钥，绑定服务端 URL 和仓库，Unix 创建权限为 `0600`；不要提交或分享，保留该文件可在新进程中恢复会话。当前安全文件存储仅支持 Unix；Windows 上需要会话凭据的操作尚不可用。任务更新和交接接受/取消需传入最新 `--expected-version`；消息与交接的 `--key` 由调用方生成并在重试同一请求时复用，不同请求不能复用。任务、会话和交接列表返回 `next_cursor`；消息列表使用 `next_after_seq` 和 `--after-seq` 翻页。协作命令默认输出 JSON，可用 `--template` 格式化。

`question view` 返回上下文及 `context_freshness`；回答时应先检查上下文，再明确传入读取到的版本和哈希，服务端会拒绝过期上下文。`task claim` 以会话身份接管空闲或已离线任务，活跃的原负责人可能导致拒绝；`task reassign` 以用户身份操作且不得携带会话文件。两者都要求调用方提供看到的任务版本、原负责人和幂等键；无负责人时省略 `--expected-owner`。问答与接管列表使用 `next_cursor` 翻页。

## 文档

- [ifg 项目设计](docs/architecture/ifg-project-design-2026-11.md)：结构、关键决策、内部设计、测试、发布与里程碑
- [客户端 CLI 设计草案](docs/architecture/client-cli-design-2026-11.md)：命令设计、配置格式、输出约定、服务端缺口
- [发布验收清单](docs/release-checklist.md)：CNB 开发验证、git233 同步、GitHub 发布及产物验收

## 许可证

[MIT](LICENSE)，与 IronForge 一致。

## 平台支持

| 平台 | 状态 |
|---|---|
| macOS（arm64 / x86_64） | 已发布，CI + 本地 e2e 覆盖 |
| Linux（musl，arm64 / x86_64） | 已发布 |
| Windows（x86_64-msvc） | 已支持：构建 / 测试 / zip 产物，CI `windows-latest` 覆盖（e2e 仍只在 macOS） |

Windows 已知限制：颜色仅在 Windows Terminal / VS Code / mintty 下启用；默认不启分页器（可用 `PAGER` 覆盖）；未签名 exe 可能触发杀软告警。带参数编辑器配置支持解析 Windows 路径，但尚需在 Windows 上实测。

## 开发

```bash
scripts/check.sh    # fmt、clippy、test、audit、ifg-client 依赖边界（CI 同款）
```

Windows（PowerShell，无需 Git Bash）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1
```

```bash
bash scripts/e2e-docker.sh  # 用固定的本地镜像启动隔离实例，运行真实端到端测试
scripts/e2e.sh      # 端到端：需要可达的 IronForge 实例（默认 localhost:8080）
                    # CI 里跑在自托管 runner 上，配置见 docs/ci-e2e-runner.md
scripts/release.sh  # 发布产物：四架构 tar.gz + Windows zip + SHA256SUMS（Windows 部分需在 Windows 上运行 `--windows`）
scripts/bump-tap.sh <version> <formula>  # 发版后同步 Homebrew formula（读 SHA256SUMS）
scripts/gen-man.sh                       # 重新生成 man/（check.sh 会校验是否最新）
```

工具链版本固定在 `rust-toolchain.toml`（若设置了 `RUSTUP_TOOLCHAIN` 环境变量，它会覆盖该文件）。
