**English** | [简体中文](README.md)

# ifg

The command-line client for IronForge: work with repositories, issues, PRs, CI and releases from the terminal over the REST API, the way GitHub's `gh` and Gitea's `tea` do for those services.

> Status: v0.9.1 is released: the full `auth` / `api` / `repo` / `issue` / `pr` / `ci` / `release` command set, with streaming upload/download, SHA-256 verification, the git credential helper, terminal-aware output (color / pager / relative time) and shell completions.

Unlike IronForge's own `ironforge` binary, `ifg` never touches the database and does no operations work; it calls the public API with a personal access token (PAT) only, and can be installed on any machine.

## Installation

Release artifacts: tar.gz for four targets — macOS (arm64/x86_64) and Linux (musl static, arm64/x86_64) — plus a Windows (x86_64) zip and a `SHA256SUMS` file. See
[Releases](https://github.com/lengyuqu/ifg/releases).

Homebrew:

```bash
brew tap lengyuqu/ifg https://github.com/lengyuqu/homebrew-ifg.git
brew install lengyuqu/ifg/ifg
```

Scoop (Windows):

```powershell
scoop bucket add ifg https://github.com/lengyuqu/scoop-ifg.git
scoop install ifg
```

Package repos: `lengyuqu/homebrew-ifg` (tap) and `lengyuqu/scoop-ifg` (bucket). The manifest source and release sync scripts live in this repo at `scoop-ifg/bucket/ifg.json`, `scripts/bump-scoop.sh` and `scripts/bump-tap.sh`.

SSH clone: `ifg repo clone OWNER/NAME --ssh` (`ssh://git@HOST:2222/...`; override the port with `IRONFORGE_SSH_PORT`).

Manual installation:

```bash
# Download the tar.gz for your architecture and the SHA256SUMS from the same Release
archive='ifg-0.9.1-<target>.tar.gz'  # replace with the actual file name
if awk -v f="$archive" '$2 == f { print; n++ } END { if (n != 1) exit 1 }' SHA256SUMS > "$archive.sha256" &&
   shasum -a 256 -c "$archive.sha256"; then
  tar xzf "$archive" && sudo mv ifg /usr/local/bin/
else
  echo 'checksum verification failed or no unique entry found; not installing' >&2
fi
```

Windows (PowerShell):

```powershell
$archive = 'ifg-0.9.1-x86_64-pc-windows-msvc.zip'
$entries = @(Get-Content .\SHA256SUMS | Where-Object { $_ -match ('^[0-9a-fA-F]{64}\s+\*?' + [regex]::Escape($archive) + '$') })
if ($entries.Count -ne 1) { throw 'no unique checksum entry found; not installing' }
$expected = ($entries[0] -split '\s+')[0]
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expected) { throw 'SHA-256 mismatch; not installing' }
Expand-Archive -LiteralPath $archive -DestinationPath C:\tools\ifg
# Add C:\tools\ifg to PATH; exit any running ifg.exe before upgrading
```

From source (musl cross-compilation needs `zig` + `cargo-zigbuild`):

```bash
scripts/release.sh   # build artifacts for four targets into dist/
cargo install --git https://github.com/lengyuqu/ifg.git ifg
```

## Quick start

```bash
ifg auth login --host https://your-forge.example.com   # paste the PAT created in the web UI (no echo)
ifg version                                            # client + server version
ifg repo create demo --init
ifg repo collaborators list -R yourname/demo            # direct collaborators with user/record IDs
ifg repo collaborators add alice --permission read -R yourname/demo
ifg repo collaborators set alice --permission write -R yourname/demo
ifg repo collaborators remove alice -R yourname/demo   # interactive confirmation; scripts can pass --yes
ifg issue list -R yourname/demo
ifg pr checkout 12                                     # works for same-repo and fork PRs; reruns fast-forward; falls back to refs/pull when the branch is gone
ifg pr view 12 --comments                              # includes inline review comments
ifg pr create -t "Title"                               # head defaults to the current branch; refuses unpushed commits
ifg pr edit 12 --title 'New title' --ready             # retitle and mark ready for review
ifg pr close 12 && ifg pr reopen 12                    # close, reopen
ifg pr comment 12 -b 'Please check the test results'   # post a general review comment
ifg pr reviews 12                                      # list reviews and their general comments
ifg pr reviewers add 12 alice                          # request a review; also list / remove
ifg pr checks 12                                       # combined status and individual checks of the head commit; still exits 0 while pending or failing
ifg pr merge 12 --squash --expected-head <SHA>         # merge only while the head still is the reviewed full SHA; returns 409 REF_ADVANCED when it moved, never retries
ifg pr readiness 12                                    # merge preflight: conflicting files, protection rules, missing checks; --json ready is true/false/null and the exit code is always 0
ifg pr update-branch 12 --expected-head <SHA>          # merge the base branch into the PR head; a moved head returns 409 REF_ADVANCED, conflicts return 409 MERGE_CONFLICTS
ifg commit create --branch main --expected-sha <SHA> -m 'Batch changes' --files ops.json   # no checkout needed: commit many file changes at once (create/update/delete/move)
git diff | ifg commit apply-patch --branch fix --expected-sha <SHA> --patch-file -   # apply a unified diff as one commit
ifg commit cherry-pick <SHA> --branch release --expected-sha <TIP> --dry-run          # preview only; same for revert
ifg branch delete feature/old --expected-sha <SHA> --yes                              # delete a branch (the server refuses the default branch, protected branches and branches with open PRs)
ifg ci logs 7 --follow                                 # streaming WebSocket log (resumes after a drop; falls back to polling without WS)
ifg ci run --ref refs/heads/main --wait --timeout 600  # trigger and wait; a commit SHA also works
ifg ci wait 7 --timeout 600                            # wait for an existing pipeline
ifg events list -R yourname/demo --since 40            # repository events: one read of events after --since; next cursor goes to stderr
ifg label create bug -c ff0000 -d "Broken"             # labels
ifg milestone create v1 --due 2026-12-31               # milestones
ifg issue edit 3 --label bug --milestone v1 --assignee alice
ifg doctor                                             # one-command self-check: host/token/scopes/version/streaming
ifg capabilities [-R yourname/demo] [--has wiki]         # server contract version and capability list; --has only returns an exit code (0 present / 1 absent)
ifg schema pr merge                                    # JSON description of the command tree (arguments, --json support, read/write effects); no login needed
ifg mcp config --client claude-code                    # print an MCP client config (without the token; export IRONFORGE_PAT yourself)
ifg mcp doctor                                         # self-check whether the MCP server can be used here (server declaration, binary, IRONFORGE_PAT); never runs it
ifg status                                             # the current branch's PR + recent pipeline runs
ifg search "login timeout" --type issue                # global search (repo / issue / wiki)
ifg context repo                                       # AI context: repository summary (--json prints the server object unchanged)
ifg context tree src --ref main                        # one page of the file tree; continue with --cursor
ifg context search "retry" --limit 10                  # code search (default branch only)
ifg context map src --budget 2000                        # repository code map: files and declarations ranked by importance (budget 64–32000 tokens, default 4000)
ifg context symbols Greeter                              # symbol definition lookup (exact matches first, prefix candidates tagged [name_match])
ifg context symbols --path src/lib.rs                    # list every declaration in a file
ifg context index                                        # rebuild the code-search and symbol index of the default branch (needs write access); --commit picks a commit or branch
ifg context pr 12                                      # aggregated PR context object (server default budget)
ifg context pr 12 --budget 2000                        # request a server budget (tokens, 500–50000); omit it for the server default
ifg context issues --state closed --limit 10           # AI context: issue summary list (--state open|closed)
ifg context prs --state merged                         # AI context: PR summary list (--state open|closed|merged)
ifg notifications                                      # notification list; notifications count shows unread only
ifg notifications prefs list                           # notification preferences: defaults plus your explicit overrides
ifg notifications prefs set --event push --channel email --off   # turn off globally; add -R OWNER/NAME to scope it to one repository (--on turns it back on)
ifg agent create bot --description "CI helper"           # create an agent account under your name (<you>-agent-bot)
ifg agent token create <you>-agent-bot --name ci --allow-repo me/demo --allow-ref 'agent/*' --token-file ./bot.token   # the token is written only to a new file (0600), never printed
ifg draft create agent/42/ --ttl 2h                    # reserve a draft branch directory: pushes trigger no webhooks or mail; cleaned up after expiry
ifg org list                                           # organizations; org members / add-member / remove-member
ifg org create acme -d "Acme team"                      # create an organization; members use --role
ifg issue create --editor                              # write the title and body in $EDITOR
ifg pr list -q '{{.number}} {{.title}}'                 # lightweight template output (lists render per row)
ifg issue view 3 -q '{{range .labels}}{{.}} {{end}}'    # range over nested arrays
ifg api users/me --jq '.username' --jq-raw             # process a GET response with the system jq
ifg wiki create 'Usage guide' --file README.md -R yourname/demo
ifg wiki view 'Usage guide' -R yourname/demo
ifg wiki edit 'Usage guide' --file - -R yourname/demo      # read from stdin and replace the page content
ifg wiki edit 'Usage guide' --file page.md --expected-version '12:34' -R yourname/demo  # use the version read earlier
ifg repo edit -R yourname/demo --description "New description" --visibility private --default-branch main
ifg wiki history 'Usage guide' -R yourname/demo
ifg wiki delete 'Usage guide' -R yourname/demo             # interactive confirmation; scripts can pass --yes
ifg release create v1.0.0 -n "Notes" dist/pkg.tar.gz     # create a release and stream-upload assets
ifg release edit v1.0.0 -n "Revised notes" --prerelease       # edit notes/mark prerelease
ifg release download v1.0.0 -p 'pkg*'                   # SHA-256 verified before writing to disk
git config --global credential.https://your-forge.example.com.helper "!ifg auth git-credential"   # passwordless git clone/push
```

Wiki editing requires server-side version protection. When `--expected-version` is not given, `ifg` reads the latest version before committing and only protects against concurrent changes after that read; if the body is based on an earlier read, save the version from `ifg wiki view --json` and pass it explicitly. `repo edit` supports `--expected-version` too. On older servers that do not provide a version, `ifg` refuses the unprotected update.

`pr comment` posts a general review with a body (`action=comment`), not an inline file comment; view it with `pr reviews`. Inline comments are still shown by `pr view --comments`.

`--editor` accepts a `VISUAL` or `EDITOR` value with arguments, e.g. `EDITOR='code --wait'`; a program path containing spaces must be quoted, e.g. `EDITOR='"/path with spaces/editor" --wait'`. The command starts the program directly without a shell, so variables, pipes and redirections are not expanded; when the executable path itself contains spaces, you can also pass the full path on its own.

`ifg api <path> --jq '<expression>'` processes a JSON GET response with `jq` from PATH; `--jq-raw` prints string results without JSON quoting. A missing `jq`, a non-JSON response or an expression error returns nonzero. `--paginate --jq` first merges the pages into one array, then runs the expression. The option currently belongs to the `api` subcommand and supports GET only; other subcommands can keep using `--json | jq ...`.

`ifg api` prints the server's raw bytes by default, appending no newline, and does **not** sanitize terminal control characters (its purpose is piping into jq or redirecting to a file). With an explicit `--json` or `--template` the response is parsed as JSON, a non-JSON response returns nonzero, and a valid empty response becomes `null`; templates render arrays item by item. Other subcommands strip control sequences such as `ESC` so a repository's body text cannot hijack the terminal; when viewing untrusted raw responses directly, use `--jq` or `| cat -v`.

`ifg status --json` returns an aggregate object: `repository` is `OWNER/NAME`, `branch` is the current branch, `pull` is the matching PR's raw object (`null` when there is none), `runs` is an array of the raw objects of up to the last 5 pipeline runs for the branch, and `scanned_runs` is how many runs this check examined (at most the last 500). `--template` can read the same object, e.g. `ifg status -q '{{.branch}}: {{range .runs}}{{.id}} {{end}}'`. Both short branch names and the `refs/heads/` form match; a same-named `refs/tags/` does not count.

`ifg release create --json` and `--template` both support attaching assets; when assets are given, the result object's `assets` array contains the raw fields of the assets uploaded in this run. Progress goes to stderr; stdout prints the result once, after everything succeeded. If an upload fails, the command returns nonzero, stderr keeps the created Release's number, and the Release and already-uploaded assets are not deleted automatically.

CI cancel, retry and approve, PR reviews, organization membership operations, notification operations, repository settings changes and the auth status list all support `--json` / `--template`. `notifications count --json` returns `{"unread_count": N}`. Machine output's stdout contains only the result; no other messages are mixed in. A failed write to the output returns nonzero; a BrokenPipe caused by a consumer exiting early is still treated as a normal finish.

The following commands do not support structured output; passing `--json` / `--template` returns a usage error before anything runs: `auth login/logout/token/git-credential`, `repo clone/delete`, `pr checkout/diff`, `ci logs`, `release upload/download/delete`, `label delete`, `milestone delete`, `version` and `completions`, as well as commands using `--web`.

`pr checkout` prefers fetching the current source branch; when the branch or fork has been deleted, it fetches `refs/pull/<number>/head` from the PR's **target repository**. On older servers that do not publish the fork-PR mirror, the command fails with an explicit error: keep the source branch or upgrade the server; the pinned baseline only provides the mirror for same-repo PRs.

Wiki `edit` replaces the whole page and uses the server version for concurrency protection; when the body is based on an earlier read, pass the `--expected-version` from that read explicitly. The pinned baseline has fixed a cross-repository authorization issue in the single-history-version endpoint; `history` lists this page's history within the currently authorized repository. Older servers may still be affected and should be updated first.

`ci run`'s ref is resolved by the IronForge server; when omitted, the server default `refs/heads/main` is used. `ci runs --branch feature` matches both the short name and `refs/heads/feature`, and does not count a same-named tag; `--branch refs/tags/feature` matches only that tag. `ci wait` or `ci run --wait` returns 0 only when the pipeline succeeds; failures, cancellations, manual gates, unknown states and timeouts all return nonzero. For a manual gate, inspect the job with `ci view` first and follow the server's process. A successful trigger is not cancelled even if the wait times out; the pipeline number is printed to stderr.

Repository collaborator permissions are `read`, `write` or `admin`; the list contains direct collaborators only, and the user ID and collaborator record ID are different values. After a permission change, ifg re-reads to confirm it took effect; with older servers where the update might not persist, the CLI reports an error instead of claiming success.

The credential helper is configured per host, applies only to IronForge, and never replaces an existing helper for other sites (such as osxkeychain).

On Windows the helper must be written with `.exe` (Git does not accept a bare command without an extension):

```powershell
git config --global credential.https://your-forge.example.com.helper '!ifg.exe auth git-credential'
```

Man pages (shipped inside the archives; Homebrew installs them into man1):

```bash
man ifg            # overview: global options, exit codes, examples
man ifg-pr         # a single command: ifg pr ..., ifg org ..., ifg ci ...
ifg help pr        # the same long description without installing man pages
```

Shell completions:

```bash
ifg completions zsh  > ~/.zfunc/_ifg      # or bash / fish / powershell
ifg completions bash > /etc/bash_completion.d/ifg
```

## Configuration and credentials

- The config directory is chosen by priority: `IRONFORGE_CONFIG_DIR` (when non-empty it is the config directory, and `hosts.toml` sits directly inside it) > `$XDG_CONFIG_HOME/ironforge` > the platform default (`$HOME/.config/ironforge`; `%APPDATA%\ironforge` on Windows). An empty value counts as unset. The config file is `hosts.toml` (the POSIX 0600 warning does not apply to Windows, where permissions are governed by the user-directory ACL). Writes are "temp file + atomic replace" under an exclusive lock on `.hosts.toml.lock` in the same directory, so concurrent `ifg auth` runs cannot overwrite each other
- Several agents on one machine can each use their own config directory; each directory's token lives only in its own `hosts.toml` (0600), and requests carry only the token of the directory in use:

  ```bash
  IRONFORGE_CONFIG_DIR="$HOME/.ifg-agent-a" ifg auth login --host https://forge.example.com --with-token
  IRONFORGE_CONFIG_DIR="$HOME/.ifg-agent-b" ifg auth login --host https://forge.example.com --with-token
  IRONFORGE_CONFIG_DIR="$HOME/.ifg-agent-a" ifg -R owner/repo issue list
  ```

- When `IRONFORGE_CONFIG_DIR` is set, ifg never reads or writes the system keychain, even if `hosts.toml` says `token_store = "keyring"`: keychain entries are keyed by host only, so two config directories would share one token. The token is then stored in that directory's `hosts.toml`, and `auth login` says so
- Environment-only credentials: with both `IRONFORGE_URL` and `IRONFORGE_PAT` set, commands do not read `hosts.toml`, do not probe the keychain, and create no config directory or files. Suited to containers without a writable HOME or keychain. `--host` must then be a URL, not a configured name; `auth login` and `auth logout` still read and write the config file

  ```bash
  IRONFORGE_URL=https://forge.example.com IRONFORGE_PAT="$TOKEN" ifg api users/me
  ```

- The `source` field of `ifg auth status` (the SOURCE column in the table) gives the credential source: `env`, `config-dir` or `default`; `config_dir` gives the directory whose config file was read. Tokens are shown only in redacted form. The `config-dir` check item of `ifg doctor` shows the directory and whether it is writable; a non-writable directory is only a warning, and in env-only mode it does not affect credentials either
- Keychain (optional): with `token_store = "keyring"` the PAT is saved to the system keychain first (macOS Keychain / Linux `secret-tool` / Windows Credential Manager); when the write is unavailable, ifg warns and falls back to the config file. A file PAT explicitly saved in the host entry wins over a stale keychain entry until one successful login writes the PAT to the keychain and removes the file copy; a PAT read from the keychain is never written back to the file during config updates. On macOS and Windows the system credential APIs are called directly, so the token never appears in process arguments
- Configured credentials are reused automatically only for the same scheme, host, effective port and deployment base path; overriding to HTTP or another deployment path does not carry the configured PAT automatically. An explicit `IRONFORGE_PAT` is still bound to the selected target. When it overrides the config, `repo list` reads the username of that token; without the `user` scope, name `repo list OWNER` explicitly
- For the 1.x compatibility rules of `--json`, the server field boundary and aggregated output, see [JSON compatibility contract](docs/json-compatibility.md). Output is re-serialized, not raw bytes
- `-q/--template` uses a lightweight subset (`{{.field}}`, `{{.a.b}}`, `{{range .list}}…{{end}}`; lists render per item automatically)
- Piped output turns off color and relative time automatically; `pr diff` is colored and goes through the pager on a terminal
- Windows terminals: color is enabled only under Windows Terminal / VS Code / mintty (`WT_SESSION`, `TERM_PROGRAM`, `TERM`); classic `cmd.exe` behaves like `--no-color`. The pager prints directly by default and can be overridden with `PAGER`

## Non-interactive use (scripts and AI agents)

Pass `--no-input`, or set `IFG_NO_INPUT=1` (values `1` or `true`; the command-line flag wins over the environment variable), and ifg will never wait for human input. Everywhere a person would be asked a question, it exits at once with a usage error (exit code 2) and names the flag to use instead:

- Destructive commands (`repo delete`, `repo collaborators remove`, `wiki delete`, `release delete`, `label delete`, `milestone delete`, `branch delete`; the last one with `--dry-run` deletes nothing and needs no `--yes`) refuse to run without `--yes` and delete nothing.
- `auth login` no longer reads the no-echo prompt or implicit stdin; use `auth login --with-token` and pipe the PAT in.
- `issue create --editor` and `pr create --editor` do not start `$EDITOR`; use `-t`/`-b` (and `--body-file` for `issue create`) instead.
- The pager is not started; output is printed directly.
- Git subprocesses started by ifg get `GIT_TERMINAL_PROMPT=0` and an empty `GIT_ASKPASS`; if the user has not set `GIT_SSH` or `GIT_SSH_COMMAND`, `ssh -o BatchMode=yes` is used. Missing credentials make `git` fail instead of hanging. Note that `GIT_SSH_COMMAND` then overrides `core.sshCommand` from the git config; to customize ssh options, set the `GIT_SSH_COMMAND` environment variable instead.
- Explicit `--file -`, `--input -` and `auth login --with-token` still read stdin. Unlock prompts raised by the OS keychain itself are not governed by this switch.

**`--no-input` is not `--yes`.** It never confirms on the caller's behalf: destructive commands still require an explicit `--yes`, e.g. `ifg --no-input repo delete OWNER/NAME --yes`.

## Idempotent writes (safe to retry after a timeout)

After a write times out, the caller cannot know whether the server executed it. Add `--idempotency-key <KEY>` to the command (or set `IRONFORGE_IDEMPOTENCY_KEY`; the flag wins) and ifg sends it as the `Idempotency-Key` request header. Replaying the same request with the same key makes the server return the first result instead of creating a duplicate.

- The key is chosen by the caller; ifg never generates one. It is 1 to 255 characters from `A-Z`, `a-z`, `0-9`, `.`, `_`, `:` and `-`; a malformed key fails with exit code 2 before any request is sent.
- Only commands that make exactly one write request per invocation on a route the server supports accept it, e.g. `issue create/comment/close/reopen/edit`, `pr create/edit/close/reopen/comment/review/merge/update-branch/reviewers add|remove`, `commit create/apply-patch/cherry-pick/revert`, `branch delete`, the `label`, `milestone` and `wiki` create/edit/delete commands, `release edit/delete` and `repo create`. `accepts_idempotency_key` in `ifg schema` lists all of them.
- Any other command (all reads, `release create`, `release upload`, `repo edit`, `ci`, `collab`, …) rejects the option with exit code 2 and sends nothing. `release create` makes one request per asset, and reusing one key would be refused by the server with `IDEMPOTENCY_KEY_REUSED`.
- A keyed write is retried automatically with the same key on dropped connections, HTTP 5xx and 429 (with the same backoff and retry count, honoring `Retry-After`). Unkeyed writes are never retried automatically.
- When the server refuses, the error object's `reason` is printed unchanged (e.g. `IDEMPOTENCY_NOT_SUPPORTED`, or `IDEMPOTENCY_KEY_REUSED` as a 409).

```sh
ifg --idempotency-key "release-notes-v1" -R alice/demo issue comment 12 -b "shipped"
IRONFORGE_IDEMPOTENCY_KEY="retry-42" ifg -R alice/demo pr merge 7 --squash
```

## Server-side commits and merge preflight

`commit` and `pr update-branch` let the server write commits directly; no local checkout is needed. They require a newer IronForge (the pinned baseline `7d27c2d` does not include these endpoints; the verified version is `65428452`); older servers return 404.

- Every write command must carry `--expected-sha` (`--expected-head` for `pr update-branch`): the full 40-character SHA the caller read; abbreviations are refused. If the branch has moved, the server returns 409 `REF_ADVANCED` and writes nothing; ifg never retries automatically. Re-read and retry with the new SHA. The server never creates a branch for you.
- `--dry-run` makes the server run every check and return a preview without writing; the preview object has a different shape from the real result, and `not_evaluated` lists the side effects that were not evaluated (webhooks, CI, notifications).
- `commit create --files` takes a JSON array (`-` reads stdin): `{"operation":"create","path":…,"content":…}`, `{"operation":"update","path":…,"content":…,"sha":<blob you read>}`, `{"operation":"delete","path":…,"sha":…}`, `{"operation":"move","from":…,"to":…,"sha":…}`. Either every change lands in one commit or the branch does not move.
- `commit apply-patch` takes a unified diff in `git diff` shape (at most 1 MiB and 500 paths); if the patch contains secrets the server returns 422.
- Commits written through an agent token are attributed to that agent (`<username>@agents.invalid`).
- `pr readiness` and the merge use the same rules, so the preflight verdict matches what `pr merge` would do right now. It answers only "can this merge now"; merge queue position, CODEOWNERS requests and the effect of auto-merge are out of scope (`not_evaluated`).

## Agent accounts and draft branches

These commands require a newer IronForge (verified `65428452`; the pinned baseline `7d27c2d` does not include these endpoints, and older servers return 404).

- `ifg agent …` manages agent accounts under your name (`list`, `create`, `view`, `edit`, `token list|create|revoke`). You must use your own login: an agent's own token cannot manage agents, and another person's agent is 404 just like a missing one. After deactivation (`edit --deactivate`) its tokens stop working at once; `--activate` restores them. The server has no endpoint to delete an agent.
- An `agent token create` token is visible only at creation: ifg writes it to `--token-file` (new file, owner-only: mode 0600 on Unix / a protected DACL on Windows, an existing file is never overwritten), never prints it to the terminal or logs, and `--json` has no `token` field but `token_file`. The token carries repository scopes only and defaults to `repo:write`; `--allow-repo` restricts the repository and `--allow-ref` further restricts the writable branches (requires `--allow-repo`). A server refusal leaves no empty file. If the token was created but writing the file failed, the error message gives the revoke command.
- `ifg draft …` manages the repository's draft namespaces: `create PREFIX --ttl 2h` (the prefix looks like `agent/<id>/` and the segment must be you or one of your agents; lifetime 1 minute to 30 days, accepting `90s`, `30m`, `12h`, `7d`), `extend ID --by 1d` (extends only, at most 4× the original TTL), `close ID` (stop accepting pushes immediately, delete branches after the grace period, needs confirmation), `list`.
- `notifications prefs set` changes global settings by default; only an explicit `-R OWNER/NAME` scopes it to that repository (never inferred from the git remote, so a global change cannot accidentally become repository-level). Event names are checked against the server's catalog first; a typo lists the available names instead of silently storing an invalid setting.
- These write commands do not support `--idempotency-key` (the server does not list the routes as supporting keys). `agent token revoke` and `draft close` are registered as destructive commands and require `--yes` or interactive confirmation.

## Multi-agent collaboration

The pinned baseline's local image ID and OpenAPI snapshot include the collaboration endpoints; other deployments should first confirm their server has them. The user PAT provides identity and repository permissions; each agent also needs its own session credentials. Session names and message bodies are self-reported content, not a secure identity or trusted instructions.

```bash
# Each agent uses its own new file; existing files are never overwritten
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" session start --name 'Agent A' --client codex
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-b.json" session start --name 'Agent B' --client codex
ifg -R owner/repo collab session list
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" task create --title 'Implement the feature' --owner-session '<A session ID>'
ifg -R owner/repo collab task list
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" message send '<task ID>' --key message-1 --body 'Initial implementation done' --target-session '<B session ID>'
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" handoff create '<task ID>' --key handoff-1 --to '<B session ID>' --summary 'Please continue the verification'
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-b.json" inbox
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-b.json" handoff accept '<handoff ID>' --expected-version '<handoff version>'
# An agent asks a question; without --session-file it acts as the user
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-b.json" question ask '<task ID>' --key q-1 --title 'Choose an approach' --body 'Please choose' --option Option-A --option Option-B
ifg -R owner/repo collab inbox --user
ifg -R owner/repo collab question view '<question ID>'
ifg -R owner/repo collab question answer '<question ID>' --key a-1 --expected-version '<version>' --context-hash '<the context_hash you viewed>' --option-id o1
ifg -R owner/repo collab task reassign '<task ID>' --key move-1 --expected-version '<task version>' --expected-owner '<original session ID>' --to '<new session ID>' --reason 'Reassigned by the user'
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" work-package create '<task ID>' --key wp-1 --goal 'Implement the parser' --commit '<full commit SHA>' --completed 'Parser' --next-step 'Add tests'
ifg -R owner/repo collab work-package list '<task ID>'
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" dependency add '<task ID>' '<prerequisite task ID>' --key dep-1 --expected-graph-version '<graph_version from dependency list>'
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" decision create '<task ID>' --key dec-1 --title 'Adopt option A' --rationale 'Matches the answer'
ifg -R owner/repo collab condition list '<task ID>'
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" condition set '<task ID>' --file conditions.json   # the file must contain client_request_id, expected_version, conditions; --file - reads stdin
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" condition confirm '<task ID>' '<condition ID>' --key confirm-1 --note 'Manually confirmed'
ifg -R owner/repo collab task status '<task ID>'
ifg -R owner/repo collab budget view '<task ID>'
ifg -R owner/repo collab --session-file "$HOME/.ifg-agent-a.json" usage report '<task ID>' --key usage-1 --unit tokens --source ci --amount 1200 --period-start 2026-11-01T00:00:00Z --period-end 2026-11-02T00:00:00Z
ifg -R owner/repo collab usage list '<task ID>' --unit tokens
ifg -R owner/repo collab rules --ref main --path src
ifg -R owner/repo collab overview --sections awaiting_me,in_progress --limit 10
```

A session file contains a secret shown only once, bound to the server URL and repository, created with mode `0600` on Unix and a protected owner-only DACL on Windows (the `0600` equivalent); do not commit or share it, and keep it to resume the session in a new process. Loading re-checks that the file is owner-only and refuses a loosened or swapped file. Task updates and handoff accept/cancel require the latest `--expected-version`; a message or handoff `--key` is generated by the caller and reused when retrying the same request, never across different requests. Task, session and handoff lists return `next_cursor`; message lists page with `next_after_seq` and `--after-seq`. Collaboration commands print JSON by default and can be formatted with `--template`.

`question view` returns the context plus `context_freshness`; before answering, check the context and then pass the version and hash you read explicitly — the server rejects stale context. `task claim` takes over an idle or offline task as the session; an active original owner may cause a refusal. `task reassign` acts as the user and must not carry a session file. Both require the task version you saw, the original owner and an idempotency key; omit `--expected-owner` when there is none. Q&A and takeover lists page with `next_cursor`.

`condition set` sends the JSON object given to `--file` unchanged, so the file must contain `client_request_id`, `expected_version` (the version last read, 0 meaning none defined yet) and `conditions`; a non-object or invalid JSON returns a usage error and sends nothing. The `--key` of `condition confirm` and `usage report` is an idempotency key. Budgets and usage are informational only; the server enforces no limits from them.

## Documentation

- [ifg project design](docs/architecture/ifg-project-design-2026-11.md): structure, key decisions, internal design, testing, release and milestones
- [Client CLI design draft](docs/architecture/client-cli-design-2026-11.md): command design, config format, output conventions, server gaps
- [Release acceptance checklist](docs/release-checklist.md): GitHub CI, server compatibility, candidate packaging and release acceptance
- [Windows verification checklist](docs/windows-verification.md): itemized hands-on verification (credential backend, `--web`, path encoding, concurrent writes, release artifacts)

## License

[MIT](LICENSE), the same as IronForge.

## Platform support

| Platform | Status |
|---|---|
| macOS (arm64 / x86_64) | Released; covered by CI + local e2e |
| Linux (musl, arm64 / x86_64) | Released |
| Windows (x86_64-msvc) | Supported: build / test / zip artifact, covered by CI `windows-latest` (e2e still macOS-only) |

Platform implementations and past release status do not mean every new candidate commit has been accepted on every platform; for this Unreleased round's specific commit, CI, native credentials and artifact evidence, see the [release acceptance record](docs/release-checklist.md).

Known Windows limitations: color is enabled only under Windows Terminal / VS Code / mintty; the pager is off by default (override with `PAGER`); an unsigned exe may trigger antivirus warnings. Editor configuration with arguments parses Windows paths, but has yet to be tested on Windows.

## Development

```bash
scripts/check.sh    # fmt, clippy, test, audit, ifg-client dependency boundary (the same as CI)
```

Windows (PowerShell, no Git Bash needed):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1
```

```bash
bash scripts/e2e-docker.sh  # start an isolated instance from the pinned local image and run the real end-to-end tests
# Use a newer image that explicitly supports the fork-PR mirror to verify source-branch and fork-deletion recovery separately:
IFG_E2E_IMAGE=<local compatible image> bash scripts/e2e-docker.sh --fork-mirror
scripts/e2e.sh      # end-to-end: needs a reachable IronForge instance (default localhost:8080)
                    # CI runs it on a self-hosted runner; see docs/ci-e2e-runner.md
scripts/release.sh  # release artifacts: four-target tar.gz + Windows zip + SHA256SUMS (run with `--windows` on Windows for the Windows part)
scripts/bump-tap.sh <version> <formula>  # sync the Homebrew formula after a release (reads SHA256SUMS)
scripts/bump-scoop.sh <version> <manifest>  # sync the Scoop manifest after a release (reads SHA256SUMS)
scripts/gen-man.sh                       # regenerate man/ (check.sh verifies it is up to date)
```

The toolchain version is pinned in `rust-toolchain.toml` (a set `RUSTUP_TOOLCHAIN` environment variable overrides it).
