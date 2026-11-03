# ifg

IronForge 的命令行客户端：在终端里通过 REST API 操作仓库、Issue、PR、CI 和 Release，定位类似 GitHub 的 `gh`、Gitea 的 `tea`。

> 状态：设计阶段，尚无可用代码。

与 IronForge 自带的 `ironforge` 二进制不同，`ifg` 不连数据库、不做运维，只用个人访问令牌（PAT）调用公开 API，可以装在任何机器上。

## 文档

- [ifg 项目设计](docs/architecture/ifg-project-design-2026-11.md)：独立仓库的结构、关键决策、内部设计、测试、发布与里程碑
- [客户端 CLI 设计草案](docs/architecture/client-cli-design-2026-11.md)：命令设计、配置格式、输出约定、服务端缺口

## 许可证

[MIT](LICENSE)，与 IronForge 一致。

## 开发

```bash
scripts/check.sh   # 提交前检查：fmt、clippy、test、audit、ifg-client 依赖边界
```

工具链版本固定在 `rust-toolchain.toml`（若设置了 `RUSTUP_TOOLCHAIN` 环境变量，它会覆盖该文件）。
