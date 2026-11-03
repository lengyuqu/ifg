# ifg

IronForge 的命令行客户端：在终端里通过 REST API 操作仓库、Issue、PR、CI 和 Release，定位类似 GitHub 的 `gh`、Gitea 的 `tea`。

> 状态：v0.5.0 已发布：`auth` / `api` / `repo` / `issue` / `pr` / `ci` / `release` 全套命令，含流式上传下载、SHA-256 校验、git credential helper、终端感知输出（颜色 / 分页器 / 相对时间）与命令补全。

与 IronForge 自带的 `ironforge` 二进制不同，`ifg` 不连数据库、不做运维，只用个人访问令牌（PAT）调用公开 API，可以装在任何机器上。

## 安装

发布产物：macOS（arm64/x86_64）与 Linux（musl 静态链接，arm64/x86_64）四架构 tar.gz + `SHA256SUMS`，见
[Releases](https://github.com/lengyuqu/ifg/releases)。

Homebrew：

```bash
brew tap lengyuqu/ifg https://github.com/lengyuqu/homebrew-ifg.git
brew install lengyuqu/ifg/ifg
```

SSH 克隆：`ifg repo clone OWNER/NAME --ssh`（`ssh://git@HOST:2222/...`，端口可用 `IRONFORGE_SSH_PORT` 覆盖）。

手动安装：

```bash
# 下载对应架构的 tar.gz 与 SHA256SUMS 后先校验
shasum -a 256 -c SHA256SUMS 2>/dev/null || shasum -a 256 ifg-*.tar.gz
tar xzf ifg-0.5.0-<target>.tar.gz && sudo mv ifg /usr/local/bin/
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
ifg ci logs 7 --follow                                 # 轮询增量打印日志
ifg label create bug -c ff0000 -d "坏了"                # 标签
ifg milestone create v1 --due 2026-12-31               # 里程碑
ifg issue edit 3 --label bug --milestone v1 --assignee alice
ifg status                                             # 当前分支的 PR + 最近流水线
ifg search "登录 超时" --type issue                      # 全局搜索（repo / issue / wiki）
ifg notifications                                      # 通知列表；notifications count 只看未读
ifg issue create --editor                              # 用 $EDITOR 写标题与正文
ifg pr list -q '{{.number}} {{.title}}'                 # 轻量模板输出（列表逐行渲染）
ifg issue view 3 -q '{{range .labels}}{{.}} {{end}}'    # range 遍历嵌套数组
ifg release create v1.0.0 -n "说明" dist/pkg.tar.gz     # 建 Release 并流式上传
ifg release edit v1.0.0 -n "修订说明" --prerelease       # 改发布说明/标记
ifg release download v1.0.0 -p 'pkg*'                   # SHA-256 校验后落盘
git config --global credential.https://your-forge.example.com.helper "!ifg auth git-credential"   # git clone/push 免密
```

凭据助手按主机配置，只对 IronForge 生效，不会替换其他站点已有的 helper（如 osxkeychain）。

命令补全：

```bash
ifg completions zsh  > ~/.zfunc/_ifg      # 或 bash / fish / powershell
ifg completions bash > /etc/bash_completion.d/ifg
```

## 配置与凭据

- 配置：`$XDG_CONFIG_HOME/ironforge/hosts.toml`（0600；权限过宽会拒绝加载令牌）
- 钥匙串（可选）：`token_store = "keyring"` 时 PAT 存系统钥匙串（macOS `security` / Linux `secret-tool`），不落文件
- `--json` 输出服务端原始字段；`-q/--template` 走轻量子集（`{{.field}}`、`{{.a.b}}`、`{{range .list}}…{{end}}`，列表自动逐项渲染）
- 管道输出自动关闭颜色与相对时间；`pr diff` 在终端里上色并走分页器

## 文档

- [ifg 项目设计](docs/architecture/ifg-project-design-2026-11.md)：结构、关键决策、内部设计、测试、发布与里程碑
- [客户端 CLI 设计草案](docs/architecture/client-cli-design-2026-11.md)：命令设计、配置格式、输出约定、服务端缺口

## 许可证

[MIT](LICENSE)，与 IronForge 一致。

## 开发

```bash
scripts/check.sh    # fmt、clippy、test、audit、ifg-client 依赖边界（CI 同款）
scripts/e2e.sh      # 端到端：需要可达的 IronForge 实例（默认 localhost:8080）
                    # CI 里跑在自托管 runner 上，配置见 docs/ci-e2e-runner.md
scripts/release.sh  # 发布产物：四架构交叉编译 + SHA256SUMS
scripts/bump-tap.sh <version> <formula>  # 发版后同步 Homebrew formula（读 SHA256SUMS）
```

工具链版本固定在 `rust-toolchain.toml`（若设置了 `RUSTUP_TOOLCHAIN` 环境变量，它会覆盖该文件）。
