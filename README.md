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
ifg issue list -R yourname/demo
ifg pr checkout 12                                     # 同仓或 fork PR 都可用，重复执行会快进更新；分支已删时走 refs/pull
ifg pr view 12 --comments                              # 含行内评审评论
ifg pr create -t "标题"                                 # head 默认当前分支，未推送会拦截
ifg ci logs 7 --follow                                 # WebSocket 流式日志（断线自动续传；无 WS 时退回轮询）
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
ifg release create v1.0.0 -n "说明" dist/pkg.tar.gz     # 建 Release 并流式上传
ifg release edit v1.0.0 -n "修订说明" --prerelease       # 改发布说明/标记
ifg release download v1.0.0 -p 'pkg*'                   # SHA-256 校验后落盘
git config --global credential.https://your-forge.example.com.helper "!ifg auth git-credential"   # git clone/push 免密
```

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
- 钥匙串（可选）：`token_store = "keyring"` 时 PAT 存系统钥匙串（macOS `security` / Linux `secret-tool` / Windows 凭据管理器），不落文件
- `--json` 输出服务端原始字段；`-q/--template` 走轻量子集（`{{.field}}`、`{{.a.b}}`、`{{range .list}}…{{end}}`，列表自动逐项渲染）
- 管道输出自动关闭颜色与相对时间；`pr diff` 在终端里上色并走分页器
- Windows 终端：仅在 Windows Terminal / VS Code / mintty（`WT_SESSION`、`TERM_PROGRAM`、`TERM`）下启用颜色，传统 `cmd.exe` 等价 `--no-color`；分页器默认直出，可设 `PAGER` 覆盖

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

Windows 已知限制：颜色仅在 Windows Terminal / VS Code / mintty 下启用；默认不启分页器（可用 `PAGER` 覆盖）；
`--editor` 需要单程序编辑器（如 `notepad`），暂不支持 `code --wait`；未签名 exe 可能触发杀软告警。

## 开发

```bash
scripts/check.sh    # fmt、clippy、test、audit、ifg-client 依赖边界（CI 同款）
```

Windows（PowerShell，无需 Git Bash）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1
```

```bash
bash scripts/e2e-docker.sh  # 用固定的本地镜像启动隔离实例，运行 5 项真实端到端测试
scripts/e2e.sh      # 端到端：需要可达的 IronForge 实例（默认 localhost:8080）
                    # CI 里跑在自托管 runner 上，配置见 docs/ci-e2e-runner.md
scripts/release.sh  # 发布产物：四架构 tar.gz + Windows zip + SHA256SUMS（Windows 部分需在 Windows 上运行 `--windows`）
scripts/bump-tap.sh <version> <formula>  # 发版后同步 Homebrew formula（读 SHA256SUMS）
scripts/gen-man.sh                       # 重新生成 man/（check.sh 会校验是否最新）
```

工具链版本固定在 `rust-toolchain.toml`（若设置了 `RUSTUP_TOOLCHAIN` 环境变量，它会覆盖该文件）。
