# Windows 验证清单

在 Windows 上逐项执行，确认已发布/待发布产物的行为。每项给出**命令**、**预期**和**判定方法**；任何一条不符即为缺陷，按末尾模板回报。

> 本轮选择仅完成自动验证。具体 CI、原生凭据与安装包结果见 [Unreleased 验收记录](release-checklist.md)。V3 浏览器、V6 终端外观、V7 GUI 编辑器与 SmartScreen 提示仍需实机交互验收；自动测试通过不能勾选这些项目。

> 覆盖范围：构建与门禁、凭据后端（Credential Manager）、`--web` 的打开方式与命令注入防护、路径段编码、配置并发写、终端行为、编辑器、发布产物。
> 不在范围内：真实服务端 e2e（`scripts/e2e.sh` 只在 macOS runner 上跑），如需覆盖请在 Windows 上自建实例后执行 `cargo test -p ifg-e2e -- --ignored`。

## 0. 前置

| 项 | 要求 |
|---|---|
| 系统 | Windows 10/11 x64 |
| Shell | PowerShell 5.1 或 7+（下文均可） |
| Rust | `rustup default stable-x86_64-pc-windows-msvc`（需 VS Build Tools + Windows SDK） |
| Git | Git for Windows（仅 V8 需要 bash） |

```powershell
git clone https://git233.phynews.com:18443/aicodeing/ifg.git
cd ifg
cargo build --locked
```

后续命令都在仓库根目录执行，二进制为 `.\target\debug\ifg.exe`（可用 `$ifg = '.\target\debug\ifg.exe'` 简化）。

凭据和配置检查使用专用测试账号、测试 host 和临时配置目录。测试 host 不得与日常已登录的 host 相同：`XDG_CONFIG_HOME` 不隔离系统凭据管理器中按 host 保存的条目。

```powershell
$previousXdg = $env:XDG_CONFIG_HOME
$testRoot = Join-Path $env:TEMP ('ifg-acceptance-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$env:XDG_CONFIG_HOME = $testRoot
```

结束时先对本轮测试 host 执行 `auth logout` 清除测试凭据，再恢复 `$env:XDG_CONFIG_HOME = $previousXdg`。不要删除或修改其他 host 的凭据。

## V1 构建与门禁

```powershell
$ifg = '.\target\debug\ifg.exe'
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1
```

**预期**：`fmt` / `clippy` / man 页一致性 / `cargo test` 全过，最后无异常退出。未装 `cargo-audit` 时会打印跳过提示，属正常。

```powershell
& $ifg version
& $ifg --help
```

**预期**：打印客户端版本；`--help` 列出全部命令组。

## V2 凭据后端（Credential Manager）

这是本次改动最需要实机确认的部分。

### V2.1 钥匙串写入不落盘

```powershell
$cfg = Join-Path $env:XDG_CONFIG_HOME 'ironforge\hosts.toml'
New-Item -ItemType Directory -Force (Split-Path $cfg) | Out-Null
'token_store = "keyring"' | Set-Content -Path $cfg -Encoding utf8
'ifp_paste_your_pat_here' | & $ifg auth login --host https://your-forge.example.com --with-token
```

**预期**：`Logged in to ... as <user>` + `Token stored in the system keychain`。

```powershell
Select-String -Path $cfg -Pattern '^\s*token\s*=' -Quiet   # 期望 False
cmdkey /list | Select-String -Pattern 'ifg'                # 期望有命中
```

**判定**：配置文件里**没有** `token =` 行，凭据管理器里能看到 `ifg` 条目 → 通过。若文件里出现明文 token，说明 keychain 写入失败被静默降级，检查是否有 `ifg: warning: keychain unavailable` 输出。

### V2.2 读取与删除

```powershell
& $ifg auth token          # 期望打印完整 PAT
& $ifg auth logout --host https://your-forge.example.com
cmdkey /list | Select-String -Pattern 'ifg'   # 期望无命中
```

### V2.3 logout 不泄露其他 host 的 token（回归重点）

两个 host 都用 keyring 登录后，登出其中一个：

```powershell
'ifp_one' | & $ifg auth login --host http://127.0.0.1:8935 --with-token
'ifp_two' | & $ifg auth login --host http://127.0.0.1:8936 --with-token
& $ifg auth logout --host http://127.0.0.1:8935
Get-Content $cfg
```

**预期**：`[hosts."127.0.0.1:8936"]` 下**没有** `token =` 行；只剩 `url`/`user`。

**判定**：出现任何 `token = "ifp_...` 即失败——这是修复前的行为（keychain token 被写回明文）。

### V2.4 降级路径

把 `token_store` 删掉（或改成 `"file"`）后重新登录：

```powershell
(Get-Content $cfg) -notmatch 'token_store' | Set-Content $cfg -Encoding utf8
'ifp_file_mode' | & $ifg auth login --host http://127.0.0.1:8937 --with-token
Select-String -Path $cfg -Pattern 'ifp_file_mode'   # 期望有命中
```

**预期**：`file` 模式下 token 正常写入文件。

## V3 `--web` 与命令注入防护（回归重点）

```powershell
$env:IRONFORGE_URL = 'http://127.0.0.1:8080'
$env:IRONFORGE_PAT  = 'ifp_dummy'
& $ifg repo view --web -R alice/demo
```

**预期**：打印 URL 并打开默认浏览器；地址为 `http://127.0.0.1:8080/alice/demo`。

注入防护（关键）：

```powershell
& $ifg repo view --web -R 'alice/demo&calc'
```

**预期**：浏览器打开 `.../alice/demo&calc`（404 页或 SPA 兜底），**不弹出计算器**。

**判定**：弹出 `calc` / 任何新进程 → 回归到 `cmd /c start`，必须修复。当前实现走 `rundll32 url.dll,FileProtocolHandler`，不经 `cmd.exe`。

非 HTTP URL 的拒绝在 CLI 层触发不到（所有调用方都用配置里的 http(s) host 拼 URL），由单测 `web::tests::non_http_urls_are_refused_before_the_opener` 覆盖，此处不重复验证。

## V4 路径段编码

```powershell
& $ifg repo view -R '../users'; $LASTEXITCODE
# 预期输出：ifg: invalid owner '..': / \ ? # and dot segments are not allowed
# 预期退出码：2

& $ifg api '../health'; $LASTEXITCODE
# 预期输出：ifg: path must stay under /api/v1/; parent segments are not allowed: ../health
# 预期退出码：2

& $ifg label list -R 'x/y?per_page=1'; $LASTEXITCODE
# 预期退出码：2（invalid repository name）
```

**判定**：任何一条返回 0 或发出请求（可在服务端日志确认）即失败。

## V5 配置并发写

两个 `ifg auth login` 同时写同一份配置，验证不丢 host：

```powershell
$env:XDG_CONFIG_HOME = Join-Path $testRoot 'concurrent'
$cfg2 = Join-Path $env:XDG_CONFIG_HOME 'ironforge\hosts.toml'
$exe = (Resolve-Path $ifg).Path

Set-Content "$testRoot\t1.txt" -Value 'ifp_one' -NoNewline
Set-Content "$testRoot\t2.txt" -Value 'ifp_two' -NoNewline

$p1 = Start-Process -FilePath $exe -ArgumentList 'auth','login','--host','http://127.0.0.1:8935','--with-token' `
      -RedirectStandardInput "$testRoot\t1.txt" -RedirectStandardOutput "$testRoot\o1.txt" -PassThru
$p2 = Start-Process -FilePath $exe -ArgumentList 'auth','login','--host','http://127.0.0.1:8936','--with-token' `
      -RedirectStandardInput "$testRoot\t2.txt" -RedirectStandardOutput "$testRoot\o2.txt" -PassThru
Wait-Process -Id $p1.Id, $p2.Id
Get-Content $cfg2
```

**预期**：`hosts."127.0.0.1:8935"` 与 `hosts."127.0.0.1:8936"` **同时存在**；目录下有 `hosts.toml` 与 `.hosts.toml.lock`，无 `.hosts-*.toml` 残留。

**判定**：只剩一个 host → 锁失效。若本机只有一个实例，跳过此项并在记录表中注明"由单测 `config::tests::concurrent_updates_keep_every_host` 覆盖"。

## V6 终端行为

| 场景 | 命令 | 预期 |
|---|---|---|
| Windows Terminal / VS Code | `& $ifg issue list -R alice/demo` | 状态列有颜色（open 绿、closed 红） |
| 传统 cmd.exe | 同上 | 无 ANSI 颜色（等价 `--no-color`） |
| 强制关闭颜色 | `& $ifg --no-color issue list -R alice/demo` | 无颜色 |
| 分页器 | 默认直出；`$env:PAGER = 'more'` 后跑长列表 | 走 `more` |

**判定**：cmd.exe 下出现 `←[32m` 之类乱码 → 失败（应为纯文本）。

## V7 编辑器

```powershell
$env:EDITOR = 'notepad'
& $ifg issue create --editor -R alice/demo
```

**预期**：打开记事本，首行作为标题、其余为正文；保存关闭后创建 issue。

带参数的编辑器（路径含空格）：

```powershell
$env:EDITOR = '"C:\Program Files\Notepad++\notepad++.exe" -multiInst'
& $ifg issue create --editor -R alice/demo
```

**判定**：能正常启动且不把引号当作文件名的一部分。

## V8 发布产物

在 Git Bash 中：

```bash
bash scripts/release.sh --windows
```

PowerShell 校验：

```powershell
Get-FileHash .\dist\ifg-*-x86_64-pc-windows-msvc.zip -Algorithm SHA256
Get-Content .\dist\SHA256SUMS | Select-String 'windows'
```

**预期**：两个 SHA-256 一致。

```powershell
Expand-Archive .\dist\ifg-*-x86_64-pc-windows-msvc.zip -DestinationPath "$env:TEMP\ifg-pkg" -Force
& "$env:TEMP\ifg-pkg\ifg.exe" version
Test-Path "$env:TEMP\ifg-pkg\man\ifg.1"
```

**预期**：解压包含 `ifg.exe` 与 `man/`，能直接运行。

## V9 已知限制（确认行为符合文档，不算缺陷）

| 项 | 命令 | 预期 |
|---|---|---|
| 协作会话文件 | `& $ifg -R alice/demo collab --session-file "$env:TEMP\s.json" session start --name A --client codex` | 报 `secure session file storage is not yet supported on this platform` |
| 分页器默认 | `$env:PAGER` 未设置时跑长列表 | 直出，不调用 `less` |
| 未签名 exe | 首次运行 | 可能出现 SmartScreen / 杀软告警 |

## 结果记录

| 项 | 结果 | 备注 |
|---|---|---|
| V1 构建与门禁 | ☐ 通过 ☐ 失败 | |
| V2.1 keyring 不落盘 | ☐ 通过 ☐ 失败 | |
| V2.2 读取与删除 | ☐ 通过 ☐ 失败 | |
| V2.3 logout 不泄露 | ☐ 通过 ☐ 失败 | 回归重点 |
| V2.4 降级写文件 | ☐ 通过 ☐ 失败 | |
| V3 `--web` 注入防护 | ☐ 通过 ☐ 失败 | 回归重点 |
| V4 路径段编码 | ☐ 通过 ☐ 失败 | |
| V5 并发写 | ☐ 通过 ☐ 失败 ☐ 跳过 | |
| V6 终端行为 | ☐ 通过 ☐ 失败 | |
| V7 编辑器 | ☐ 通过 ☐ 失败 | |
| V8 发布产物 | ☐ 通过 ☐ 失败 | |
| V9 已知限制 | ☐ 符合 ☐ 不符 | |

## 失败回报模板

```text
项：V2.3
命令：<原样粘贴>
实际输出：<粘贴 stdout/stderr>
期望：hosts.toml 中 127.0.0.1:8936 下无 token 行
环境：Windows 11 23H2 / PowerShell 7.4 / rustc 1.99.1
补充：.\target\debug\ifg.exe doctor --json 输出（token 已脱敏）
       Get-Content $env:APPDATA\ironforge\hosts.toml（token 已脱敏）
```
