# ifg

IronForge 的命令行客户端：在终端里通过 REST API 操作仓库、Issue、PR、CI 和 Release，定位类似 GitHub 的 `gh`、Gitea 的 `tea`。

> 状态：v0.1.0 已发布（M1–M4 交付）：`auth` / `api` / `repo` / `issue` / `pr` / `ci` / `release` 全套命令，含流式上传下载、SHA-256 校验与 git credential helper。

与 IronForge 自带的 `ironforge` 二进制不同，`ifg` 不连数据库、不做运维，只用个人访问令牌（PAT）调用公开 API，可以装在任何机器上。

## 安装

发布产物在 [git233 Release](https://git233.phynews.com:18443/aicodeing/ifg/releases/tag/v0.1.0)：macOS（arm64/x86_64）与 Linux（musl 静态链接，arm64/x86_64）四架构 + `SHA256SUMS`。

Homebrew（产物在 internal 可见性的内网仓库，下载需凭据——在 `~/.netrc` 加一行
`machine git233.phynews.com login <user> password <gitea-token>`）：

```bash
brew tap aicodeing/ifg https://git233.phynews.com:18443/aicodeing/homebrew-ifg.git
brew trust aicodeing/ifg
brew install aicodeing/ifg/ifg
```

手动安装：

```bash
# 下载后校验（见 Release 页 SHA256SUMS）
shasum -a 256 -c <(grep "$(uname -m)" SHA256SUMS)   # 按架构选择对应行
tar xzf ifg-0.1.0-<target>.tar.gz && sudo mv ifg /usr/local/bin/
```

从源码（musl 交叉编译需 `zig` + `cargo-zigbuild`）：

```bash
scripts/release.sh   # 构建四架构产物到 dist/
cargo install --git https://git233.phynews.com:18443/aicodeing/ifg.git ifg
```

## 快速上手

```bash
ifg auth login --host https://your-forge.example.com   # 粘贴 Web 端创建的 PAT
ifg repo create demo --init
ifg issue list -R yourname/demo
ifg pr create -t "标题"          # head 默认当前分支，未推送会拦截
git config --global credential.helper "!ifg auth git-credential"   # git clone/push 免密
```

## 文档

- [ifg 项目设计](docs/architecture/ifg-project-design-2026-11.md)：独立仓库的结构、关键决策、内部设计、测试、发布与里程碑
- [客户端 CLI 设计草案](docs/architecture/client-cli-design-2026-11.md)：命令设计、配置格式、输出约定、服务端缺口

## 许可证

[MIT](LICENSE)，与 IronForge 一致。

## 开发

```bash
scripts/check.sh    # 提交前检查：fmt、clippy、test、audit、ifg-client 依赖边界
scripts/e2e.sh      # 端到端：需要本地 IronForge（默认 localhost:8080）
scripts/release.sh  # 发布产物：四架构交叉编译 + SHA256SUMS
```

工具链版本固定在 `rust-toolchain.toml`（若设置了 `RUSTUP_TOOLCHAIN` 环境变量，它会覆盖该文件）。
