# JSON 兼容契约（1.x）

本契约从 1.0 生效；当前版本的实现和测试提前遵循它。适用于明确支持 `--json` 的命令，以及默认输出 JSON 的协作命令。README 列出不支持结构化输出的命令。它约束 ifg 在相同参数和相同服务端响应下的转换，不冻结 IronForge 服务端 API，也不保证不同版本服务端返回相同字段或数据。

## 输出和版本规则

- 成功的结构化输出是 stdout 上一个完整 JSON 值；进度、警告和诊断写 stderr。`doctor` 的检查失败时可以输出 JSON 检查结果并返回非零；其他失败不保证 stdout 含 JSON。stdout 写入失败返回非零，消费者提前关闭管道的 BrokenPipe 视为正常结束。
- JSON 是解析后的值，不保证原始字节、空白、对象键顺序或转义格式。客户端和脚本应解析 JSON，忽略不认识的字段；数组顺序反映服务端结果及命令的筛选和截断，不保证跨请求数据不变。
- 从服务端取得的原始资源对象保留字段及 JSON 值，包括未知字段；不通过展示用 DTO 重新序列化。服务端增删字段、改变类型或权限导致的变化由服务端契约决定，ifg 不承诺补齐缺失字段或固定枚举值。下文列出的客户端组合字段覆盖同名服务端字段。
- 1.x 中，ifg 不主动删除或重命名下文承诺的客户端字段、改变其类型或含义、改变已支持命令的顶层 JSON 形状，或丢弃应保留的服务端字段；添加字段及新的枚举值允许。破坏这些保证须留待下一主版本，或引入明确的版本化输出并提供迁移说明。安全修复若必须打破兼容，需在发布说明中明确影响和迁移方法。

## 命令转换

| 命令 | 1.x 输出契约 |
| --- | --- |
| 一般资源列表 | JSON 数组，按服务端顺序合并请求到的页面，再按命令过滤和限制；分页元数据不输出，空列表为 `[]`。具体默认值及筛选含义由命令帮助定义。 |
| `issue view --comments`、`pr view --comments` | 对应资源对象的 `comments` 被评论接口的数组覆盖，包含空数组；评论请求失败时命令失败。 |
| `release view` | Release 原始对象的 `assets` 被附件接口的原始数组覆盖。 |
| `release create` | 输出服务端创建响应；指定附件时，`assets` 被本次成功上传的原始附件对象数组覆盖。上传失败返回非零，不输出不完整的成功结果，已创建资源不会自动回滚。 |
| `status` | 对象含 `repository`、`branch` 字符串，`pull`（对象或 `null`）、`runs` 数组、`scanned_runs` 非负整数；嵌套资源为服务端原始对象。 |
| `notifications count` | 对象含非负整数 `unread_count`。 |
| `search` | 以首个响应对象为基础，`results` 替换为跨页合并并按 `--limit` 截断的数组；如果首个响应有 `total`，使用最后取得页面的 `total`（非数字时为 0）；其余首屏字段保留，因此 `page` 等元数据仍描述首屏。 |
| `doctor` | 检查结果数组，每项含字符串 `check`、`status`、`detail` 和字符串或 `null` 的 `hint`；检查集合可随环境或版本增加。 |
| `api --json` | 非分页响应解析为 JSON；空响应为 `null`，非 JSON 报错。`api --paginate --json` 合并分页数据为数组。`api` 不带 `--json`、`--template`、`--jq` 或 `--paginate` 时原样输出响应字节，不添加换行。 |
| `schema` | 命令树描述对象，见下节。本地生成，不访问网络，不读取配置或令牌。 |

## `ifg schema` 文档

`ifg schema [COMMAND...]` 从 clap 定义在运行时生成一个 JSON 对象，供 agent 发现命令。不带参数时列出全部命令；给出命令路径（如 `ifg schema pr merge`）时，`commands` 只含该子树。未知路径返回用法错误（退出码 2）。

顶层字段：

- `schema_version`：整数，当前为 `1`。
- `ifg_version`：字符串，生成本文档的 ifg 版本。
- `commands`：数组，每项为一个命令（子树根或全部顶层命令）。
- `global_args`：根命令的全局参数（如 `--host`、`--json`），各命令不重复列出。
- `exit_codes`：对象，键为退出码字符串（`"0"`、`"1"`、`"2"`、`"4"`、`"8"`），值为含义说明。

命令对象字段：

- `path`：字符串数组，从顶层到该命令的名字，例如 `["pr", "merge"]`。
- `about`：字符串或 `null`，简短描述。
- `args`：参数数组。每项含 `name`、`long`（如 `"--merge"`，无则 `null`）、`short`（如 `"-R"`，无则 `null`）、`positional`、`required`、`takes_value`、`multiple`（布尔）、`possible_values`（字符串数组或 `null`）、`help`（字符串或 `null`）。
- `subcommands`：子命令对象数组；叶子命令为 `[]`。
- `json_supported`：叶子命令为布尔值，表示是否接受 `--json` / `--template`（`view` 命令加 `--web` 时不接受，该限制未在此表示）；非叶子命令为 `null`。
- `effect`：叶子命令为 `"read"`、`"write"` 或 `"destructive"`；非叶子命令为 `null`。`api` 固定为 `write`，因为其 HTTP 方法在运行时决定。

兼容规则：1.x 内可以新增字段和新的枚举值（例如新的 `effect` 取值），但不删除或改变已列出字段的类型与含义。使用方应忽略不认识的字段；遇到不认识的 `effect` 值按 `write` 处理，不把它当作只读。`json_supported` 与 `effect` 的依据分别是 `cmd::STRUCTURED_OUTPUT` 表和 `cmd/schema.rs` 中的副作用列表，新增命令时须同步更新，未列出的叶子命令默认为 `read`。

其他客户端构造的结果按当前命令语义使用；不把它们误认为单个服务端响应。模板输出不是 JSON 契约的一部分。

## 验证

`bash scripts/check.sh` 包含格式、Clippy、手册和工作区测试。重点回归位于 `crates/ifg/tests/`：`cli.rs`（PR 评论聚合）、`issue_view_comments.rs`、`release_create.rs`、`release_view_json.rs`、`search_raw.rs`、`status.rs`、`notifications.rs`、`api_raw.rs`、`output_errors.rs` 和 `structured_commands.rs`。这些本地模拟测试验证 ifg 的转换；OpenAPI 快照测试仅检查登记的路径和部分字段，不能证明所有服务端版本兼容。发布验收还须按发布清单检查指定服务端和真实环境。
