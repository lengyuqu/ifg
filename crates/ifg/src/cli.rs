//! Command-line definition (design doc §5): global flags plus the `auth`
//! and `api` subcommands of M1.

use clap::{ArgAction, Parser, Subcommand};

/// Work with IronForge repositories, issues, pull requests, CI and releases
/// from the terminal.
///
/// `ifg` talks to the IronForge REST API with a personal access token. The
/// token is validated by `auth login`, stored in
/// `$XDG_CONFIG_HOME/ironforge/hosts.toml` (or the system keychain when
/// `token_store = "keyring"`), and reused by every command.
///
/// Machine-readable output: `--json` prints the server's own field names,
/// `-q/--template` renders a small template per result
/// (`{{.number}} {{.title}}`, `{{range .labels}}{{.}} {{end}}`). Colors,
/// relative timestamps and the pager only switch on when stdout is a
/// terminal.
///
/// Long-tail API surface (wiki, webhooks, packages, secrets, …) is reachable
/// through `ifg api <path>`.
#[derive(Debug, Parser)]
#[command(
    name = "ifg",
    version,
    arg_required_else_help = true,
    after_help = "EXAMPLES:
  ifg auth login --host https://forge.example.com   store a token for a host
  ifg issue list -R alice/demo --state open        list open issues
  ifg pr checkout 12                               fetch a PR into pr-12
  ifg ci logs 7 --follow                           stream a job log
  ifg pr list -q '{{.number}} {{.title}}'           script-friendly output
  ifg api users/me                                 raw API passthrough

Run `ifg help <command>` for the full description of one command,
or `man ifg` / `man ifg-<command>` when the man pages are installed."
)]
pub struct Cli {
    /// Host name from the config, or a base URL like https://forge.example.com
    ///
    /// Resolution order: this flag, `IRONFORGE_URL`, the repository's git
    /// remote (when run inside a checkout), then `default_host` from the
    /// config file.
    #[arg(long, global = true, value_name = "NAME|URL")]
    pub host: Option<String>,

    /// Target repository as OWNER/NAME, or a full remote URL
    ///
    /// Inside a git checkout `ifg` infers OWNER/NAME from `origin`, so the
    /// flag is only needed outside a repository or to target another one.
    #[arg(short = 'R', long, global = true, value_name = "OWNER/NAME")]
    pub repo: Option<String>,

    /// Output JSON (server field names) instead of a table
    ///
    /// Field names are the server's own, so `jq` expressions from the API
    /// docs work unchanged. Mutually exclusive with `--template`.
    #[arg(long, global = true, conflicts_with = "template")]
    pub json: bool,
    /// Render each result through a small template (`{{.number}} {{.title}}`,
    /// nested paths allowed; lists render once per row)
    ///
    /// Supported subset: `{{.field}}`, `{{.nested.field}}`, `{{.array.0}}`,
    /// `{{range .list}}…{{end}}` (nestable; objects iterate their values) and
    /// the escapes `\n` / `\t`. Missing fields render empty. Lists render the
    /// template once per item, so `{{range}}` is only needed for nested
    /// arrays such as an issue's labels.
    #[arg(short = 'q', long, global = true, value_name = "TEMPLATE")]
    pub template: Option<String>,

    /// Disable colored output (also honors NO_COLOR)
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Verbosity: -v logs each request, -vv also logs headers
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count, global = true, default_value_t = 0)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage credentials for IronForge hosts
    ///
    /// Tokens are personal access tokens created in the web UI
    /// (Settings → Tokens). `auth login` reads one from a no-echo prompt on a
    /// terminal, or from stdin with `--with-token`/a pipe. `token_store =
    /// "keyring"` in the config keeps the token out of the config file.
    /// `auth git-credential` implements the git credential helper protocol so
    /// `git clone`/`push` reuse the same token.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Raw pass-through to the IronForge REST API (paths relative to /api/v1/)
    ///
    /// Long-tail endpoints (wiki, webhooks, packages, secrets, runners) work
    /// here until they get first-class commands. `-f key=value` sends a JSON
    /// field, `--input file` sends a raw body, and `--paginate` merges the
    /// `data` arrays of every page into one array (GET only).
    Api {
        #[command(flatten)]
        args: ApiArgs,
    },
    /// Work with repositories
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    /// Work with issues
    Issue {
        #[command(subcommand)]
        command: IssueCommand,
    },
    /// Work with pull requests (`checkout` fetches the head branch)
    ///
    /// `ifg pr create` defaults `--head` to the current branch and refuses to
    /// open a PR whose commits are not pushed yet. `checkout` fetches the PR
    /// head — from the fork when the PR came from one — into a local branch
    /// named `pr-<number>`; running it again fast-forwards that branch, and
    /// it falls back to the server's `refs/pull/<n>/head` mirror when the
    /// head branch has been deleted.
    Pr {
        #[command(subcommand)]
        command: PrCommand,
    },
    /// Work with CI pipelines
    ///
    /// `ci logs --follow` streams the job log over a WebSocket and resumes
    /// from the last printed line when the connection drops; servers without
    /// WebSocket support fall back to polling. `ci approve` clears an
    /// environment gate; `ci retry` re-runs a finished pipeline.
    Ci {
        #[command(subcommand)]
        command: CiCommand,
    },
    /// Coordinate agent sessions, tasks, messages and handoffs
    Collab {
        /// Private file holding this agent's session credentials
        #[arg(long, global = true)]
        session_file: Option<std::path::PathBuf>,
        #[command(subcommand)]
        command: CollabCommand,
    },
    /// Work with releases and their assets
    ///
    /// `release download` verifies every asset against the server-provided
    /// SHA-256 and deletes it on mismatch. `release create` uploads files
    /// with a streaming request, so large artifacts do not sit in memory.
    Release {
        #[command(subcommand)]
        command: ReleaseCommand,
    },
    /// Work with repository labels
    Label {
        #[command(subcommand)]
        command: LabelCommand,
    },
    /// Read and manage repository Wiki pages
    Wiki {
        #[command(subcommand)]
        command: WikiCommand,
    },
    /// Work with repository milestones
    Milestone {
        #[command(subcommand)]
        command: MilestoneCommand,
    },
    /// Show the current branch's pull request and recent pipeline runs
    ///
    /// A one-screen answer to "where am I": the repository, the branch, the
    /// open PR whose head is this branch, and the last few pipeline runs for
    /// it. Must be run inside a git checkout.
    Status,
    /// Search repositories, issues and wiki pages
    ///
    /// `--type` narrows to repo/issue/wiki (default: all). The server
    /// understands qualifiers such as `repo:owner/name` and `state:open` in
    /// the query text.
    Search {
        /// Query text (the server understands repo:/state: qualifiers)
        query: String,
        /// all / repo / issue / wiki
        #[arg(long = "type", default_value = "all")]
        search_type: String,
        /// Stop after N hits (default 30)
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Work with organizations
    ///
    /// Members are addressed by username: `org add-member` resolves the name
    /// through `GET /users/{username}` and posts the numeric id, and
    /// `org members` shows the username the server resolves for each row
    /// (falling back to `#id` when the account is gone).
    Org {
        #[command(subcommand)]
        command: OrgCommand,
    },
    /// List and manage your notifications
    Notifications {
        #[command(subcommand)]
        command: Option<NotificationsCommand>,
    },
    /// Check the whole setup: host, token, scopes, server version, streaming
    ///
    /// Answers "why does this command fail" without a debugging session, and
    /// exits 1 when something is actually broken (warnings do not fail).
    Doctor,
    /// Print the client version and (when reachable) the server version
    Version,
    /// Print a shell completion script (bash / zsh / fish / powershell)
    Completions {
        /// Target shell
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Subcommand)]
pub enum WikiCommand {
    /// List Wiki pages
    List,
    /// Read a page
    View { title: String },
    /// Create a page from text or a UTF-8 file (use - for stdin)
    Create {
        title: String,
        #[arg(long, conflicts_with = "file", required_unless_present = "file")]
        body: Option<String>,
        #[arg(long, conflicts_with = "body", required_unless_present = "body")]
        file: Option<String>,
        #[arg(long)]
        message: Option<String>,
    },
    /// Replace a page's content
    Edit {
        title: String,
        #[arg(long, conflicts_with = "file", required_unless_present = "file")]
        body: Option<String>,
        #[arg(long, conflicts_with = "body", required_unless_present = "body")]
        file: Option<String>,
        #[arg(long)]
        message: Option<String>,
    },
    /// List previous revisions
    History { title: String },
    /// Delete a page after confirmation
    Delete {
        title: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum CollabCommand {
    /// Register, list, heartbeat or end agent sessions
    Session {
        #[command(subcommand)]
        command: CollabSessionCommand,
    },
    /// Create, list, view and update collaboration tasks
    Task {
        #[command(subcommand)]
        command: CollabTaskCommand,
    },
    /// Send, list and acknowledge task messages
    Message {
        #[command(subcommand)]
        command: CollabMessageCommand,
    },
    /// Create, list, accept or cancel task handoffs
    Handoff {
        #[command(subcommand)]
        command: CollabHandoffCommand,
    },
    /// Ask, inspect, answer or cancel collaboration questions
    Question {
        #[command(subcommand)]
        command: CollabQuestionCommand,
    },
    /// List the session's or user's inbox
    Inbox {
        /// Read the account inbox instead of a session inbox
        #[arg(long)]
        user: bool,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        limit: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CollabSessionCommand {
    /// Register and save the one-time secret to a new private session file
    Start {
        #[arg(long)]
        name: String,
        #[arg(long)]
        client: String,
    },
    /// List repository sessions
    List {
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Refresh this session's presence
    Heartbeat,
    /// End this session
    End,
}

#[derive(Debug, Subcommand)]
pub enum CollabTaskCommand {
    /// List tasks with cursor pagination
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Show a task and its current version
    View { id: String },
    /// Create a task under the current session
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        owner_session: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        path: Vec<String>,
        #[arg(long)]
        issue: Option<i64>,
        #[arg(long)]
        pr: Option<i64>,
    },
    /// Update a task using its expected version
    Update {
        id: String,
        #[arg(long)]
        expected_version: i64,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, conflicts_with = "clear_description")]
        description: Option<String>,
        #[arg(long, conflicts_with = "description")]
        clear_description: bool,
        #[arg(long, conflicts_with = "clear_owner_session")]
        owner_session: Option<String>,
        #[arg(long, conflicts_with = "owner_session")]
        clear_owner_session: bool,
        #[arg(long, conflicts_with = "clear_branch")]
        branch: Option<String>,
        #[arg(long, conflicts_with = "branch")]
        clear_branch: bool,
        #[arg(long, conflicts_with = "clear_paths")]
        path: Vec<String>,
        #[arg(long, conflicts_with = "path")]
        clear_paths: bool,
        #[arg(long, conflicts_with = "clear_issue")]
        issue: Option<i64>,
        #[arg(long, conflicts_with = "issue")]
        clear_issue: bool,
        #[arg(long, conflicts_with = "clear_pr")]
        pr: Option<i64>,
        #[arg(long, conflicts_with = "pr")]
        clear_pr: bool,
    },
    /// Claim an unowned or abandoned task as this session
    Claim {
        id: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        expected_version: i64,
        #[arg(long)]
        expected_owner: Option<String>,
        #[arg(long)]
        reason: String,
    },
    /// Reassign a task as the account user
    Reassign {
        id: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        expected_version: i64,
        #[arg(long)]
        expected_owner: Option<String>,
        #[arg(long)]
        to: String,
        #[arg(long)]
        reason: String,
    },
    /// List task ownership changes
    Takeovers {
        id: String,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        limit: Option<u64>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CollabQuestionCommand {
    /// Ask a session, user or repository writers a question
    Ask {
        task: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: String,
        /// repo_writers, user or session
        #[arg(long, default_value = "repo_writers", value_parser = ["repo_writers", "user", "session"])]
        recipient: String,
        #[arg(long)]
        recipient_id: Option<String>,
        #[arg(long)]
        option: Vec<String>,
        #[arg(long)]
        task_version: Option<i64>,
        #[arg(long)]
        base_sha: Option<String>,
        #[arg(long)]
        head_sha: Option<String>,
    },
    /// List questions on a task
    List {
        task: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        limit: Option<u64>,
    },
    /// View a question, its context and freshness
    View { id: String },
    /// Answer with the exact version and context hash that were read
    Answer {
        id: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        expected_version: i64,
        #[arg(long)]
        context_hash: String,
        #[arg(long, default_value = "")]
        body: String,
        #[arg(long)]
        option_id: Option<String>,
    },
    /// Cancel a question using its current version
    Cancel {
        id: String,
        #[arg(long)]
        expected_version: i64,
    },
}

#[derive(Debug, Subcommand)]
pub enum CollabMessageCommand {
    /// List task messages after a sequence number
    List {
        task: String,
        #[arg(long, default_value_t = 0)]
        after_seq: i64,
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Send a message with a caller-supplied idempotency key
    Send {
        task: String,
        #[arg(long)]
        key: String,
        #[arg(long, default_value = "note")]
        kind: String,
        #[arg(long)]
        body: String,
        #[arg(long)]
        reply_to: Option<String>,
        #[arg(long)]
        target_session: Option<String>,
        #[arg(long)]
        commit: Option<String>,
    },
    /// Mark a task's messages read through a sequence number
    Read {
        task: String,
        #[arg(long)]
        seq: i64,
    },
}

#[derive(Debug, Subcommand)]
pub enum CollabHandoffCommand {
    /// List handoffs for a task
    List {
        task: String,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Create a handoff with a caller-supplied idempotency key
    Create {
        task: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        summary: String,
        #[arg(long, default_value = "")]
        completed: String,
        #[arg(long, default_value = "")]
        next_steps: String,
        #[arg(long, default_value = "")]
        blockers: String,
        #[arg(long)]
        commit: Option<String>,
        #[arg(long)]
        evidence: Vec<String>,
    },
    /// Accept a handoff addressed to this session
    Accept {
        id: String,
        #[arg(long)]
        expected_version: i64,
    },
    /// Cancel a handoff created by this session
    Cancel {
        id: String,
        #[arg(long)]
        expected_version: i64,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Log in with a personal access token (validates it against /users/me)
    Login {
        /// Read the token from stdin instead of an interactive prompt
        #[arg(long)]
        with_token: bool,
    },
    /// List configured hosts, token prefixes and usable scopes
    Status,
    /// Remove local credentials (the token stays valid server-side)
    Logout,
    /// Print the token of the resolved host (for scripts)
    Token,
    /// git credential helper: answer `get` for configured hosts only
    /// (protocol and host must both match). Configure it for your host only:
    /// git config --global credential.https://HOST.helper "!ifg auth git-credential"
    GitCredential {
        /// The helper action git asks for: get / store / erase
        action: String,
    },
}

#[derive(Debug, clap::Args)]
pub struct ApiArgs {
    /// Path relative to /api/v1/, e.g. "users/me"
    pub path: String,

    /// HTTP method (default GET)
    #[arg(short = 'X', long, default_value = "GET")]
    pub method: String,

    /// Add a field. With GET these become query parameters; with other
    /// methods they form a JSON body of strings
    #[arg(short = 'f', long = "field", value_name = "KEY=VALUE")]
    pub fields: Vec<String>,

    /// Read the JSON request body from a file ("-" for stdin)
    #[arg(long, value_name = "FILE")]
    pub input: Option<String>,

    /// Follow pagination and merge all `data` arrays into one JSON array
    #[arg(long)]
    pub paginate: bool,

    /// Filter a GET response with the system jq executable
    #[arg(long, value_name = "EXPRESSION")]
    pub jq: Option<String>,

    /// Output jq string results without JSON quoting
    #[arg(long, requires = "jq")]
    pub jq_raw: bool,
}

#[derive(Debug, Subcommand)]
pub enum RepoCommand {
    /// List or manage direct repository collaborators
    Collaborators {
        #[command(subcommand)]
        command: RepoCollaboratorsCommand,
    },
    /// List an owner's repositories (default: the logged-in user)
    List {
        /// Owner (user or organization name)
        owner: Option<String>,
        /// Stop after N entries (default 30)
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Show one repository
    View {
        /// OWNER/NAME; defaults to the current repository context
        #[arg(value_name = "OWNER/NAME")]
        target: Option<String>,
        /// Open the repository page in the browser
        #[arg(long)]
        web: bool,
    },
    /// Create a repository under your account (or an organization)
    Create {
        name: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        private: bool,
        #[arg(long, value_name = "ORG")]
        org: Option<String>,
        /// Initialize with template files (README, LICENSE, .gitignore)
        #[arg(long)]
        init: bool,
        #[arg(long, value_name = "BRANCH")]
        default_branch: Option<String>,
    },
    /// Clone a repository (HTTPS by default)
    Clone {
        /// OWNER/NAME
        #[arg(value_name = "OWNER/NAME")]
        target: String,
        /// Target directory (default: the repository name)
        dir: Option<String>,
        /// Clone over SSH (ssh://git@HOST:PORT/OWNER/NAME; port from
        /// IRONFORGE_SSH_PORT, default 2222)
        #[arg(long)]
        ssh: bool,
    },
    /// Delete a repository (asks for confirmation unless --yes)
    Delete {
        /// OWNER/NAME; defaults to the current repository context
        #[arg(value_name = "OWNER/NAME")]
        target: Option<String>,
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum RepoCollaboratorsCommand {
    /// List direct collaborators (user IDs and collaborator record IDs)
    List,
    /// Add a collaborator by username
    Add {
        username: String,
        #[arg(long, default_value = "read", value_parser = ["read", "write", "admin"])]
        permission: String,
    },
    /// Change a collaborator's permission by username
    Set {
        username: String,
        #[arg(long, value_parser = ["read", "write", "admin"])]
        permission: String,
    },
    /// Remove a collaborator by username
    Remove {
        username: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum IssueCommand {
    /// List issues
    List {
        /// open / closed / all
        #[arg(long, default_value = "open")]
        state: String,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        /// Stop after N entries (default 30)
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Show one issue
    View {
        number: i64,
        /// Include the comment thread
        #[arg(long)]
        comments: bool,
        /// Open the issue page in the browser
        #[arg(long)]
        web: bool,
    },
    /// Create an issue
    Create {
        #[arg(short = 't', long)]
        title: Option<String>,
        #[arg(short = 'b', long)]
        body: Option<String>,
        #[arg(long, value_name = "FILE")]
        body_file: Option<String>,
        #[arg(long)]
        label: Vec<String>,
        /// Compose the title and body in $EDITOR (first line = title)
        #[arg(long)]
        editor: bool,
    },
    /// Comment on an issue
    Comment {
        number: i64,
        #[arg(short = 'b', long)]
        body: String,
    },
    /// Close an issue
    Close { number: i64 },
    /// Reopen an issue
    Reopen { number: i64 },
    /// Edit an issue's title, body, labels, assignee or milestone
    Edit {
        number: i64,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
        /// Replace the label set (repeatable)
        #[arg(long)]
        label: Vec<String>,
        /// Assign a user by name
        #[arg(long)]
        assignee: Option<String>,
        /// Attach a milestone by title
        #[arg(long)]
        milestone: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum PrReviewersCommand {
    /// List requested reviewers
    List { number: i64 },
    /// Request a reviewer by username
    Add { number: i64, username: String },
    /// Remove a requested reviewer
    Remove { number: i64, username: String },
}

#[derive(Debug, Subcommand)]
pub enum PrCommand {
    /// List pull requests
    List {
        /// open / merged / closed / all
        #[arg(long, default_value = "open")]
        state: String,
        /// Stop after N entries (default 30)
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Show one pull request
    View {
        number: i64,
        /// Open the pull request page in the browser
        #[arg(long)]
        web: bool,
        /// Include inline review comments
        #[arg(long)]
        comments: bool,
    },
    /// Print the diff of a pull request
    Diff { number: i64 },
    /// Create a pull request (head defaults to the current branch)
    Create {
        #[arg(long)]
        base: Option<String>,
        /// Branch, or OWNER:branch for fork PRs; defaults to the current branch
        #[arg(long)]
        head: Option<String>,
        #[arg(short = 't', long)]
        title: Option<String>,
        #[arg(short = 'b', long)]
        body: Option<String>,
        #[arg(long)]
        draft: bool,
        /// Compose the title and body in $EDITOR (first line = title)
        #[arg(long)]
        editor: bool,
    },
    /// Update the PR title, body or draft state
    Edit {
        number: i64,
        #[arg(short = 't', long)]
        title: Option<String>,
        #[arg(short = 'b', long)]
        body: Option<String>,
        #[arg(long, conflicts_with = "ready")]
        draft: bool,
        #[arg(long, conflicts_with = "draft")]
        ready: bool,
    },
    /// Close an open PR
    Close { number: i64 },
    /// Reopen a closed PR
    Reopen { number: i64 },
    /// Add a general review comment to a PR
    Comment {
        number: i64,
        #[arg(short = 'b', long)]
        body: String,
    },
    /// List PR reviews and their general comments
    Reviews { number: i64 },
    /// Manage requested reviewers
    Reviewers {
        #[command(subcommand)]
        command: PrReviewersCommand,
    },
    /// Merge a pull request
    Merge {
        number: i64,
        #[arg(long, group = "strategy")]
        merge: bool,
        #[arg(long, group = "strategy")]
        squash: bool,
        #[arg(long, group = "strategy")]
        rebase: bool,
        /// Enable auto-merge instead of merging now
        #[arg(long)]
        auto: bool,
    },
    /// Review a pull request
    Review {
        number: i64,
        #[arg(long, group = "verdict")]
        approve: bool,
        #[arg(long, group = "verdict")]
        request_changes: bool,
        #[arg(short = 'b', long)]
        body: Option<String>,
    },
    /// Fetch the PR head branch and check it out as pr-<number>
    Checkout { number: i64 },
}

#[derive(Debug, Subcommand)]
pub enum CiCommand {
    /// Trigger a pipeline on a branch, tag ref or commit
    Run {
        /// Git ref to resolve on the server; defaults to server's main branch
        #[arg(long = "ref")]
        ref_name: Option<String>,
        /// Wait for a terminal pipeline result
        #[arg(long)]
        wait: bool,
        /// Maximum wait in seconds (used with --wait)
        #[arg(long, default_value_t = 600, requires = "wait")]
        timeout: u64,
    },
    /// Wait for a pipeline and return nonzero on failure, block or timeout
    Wait {
        id: i64,
        /// Maximum wait in seconds
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// List pipeline runs
    Runs {
        /// Filter by ref (branch or tag), applied locally
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
        /// Filter by status, applied locally
        #[arg(long)]
        status: Option<String>,
        /// Stop after N entries (default 30)
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Show a pipeline with its stages and jobs
    View { id: i64 },
    /// Print a job log; --follow polls until the job finishes
    Logs {
        id: i64,
        /// Job id or name (default: the pipeline's only job)
        #[arg(long, value_name = "ID|NAME")]
        job: Option<String>,
        /// Keep polling every 2s and print increments (§7.1-A)
        #[arg(long)]
        follow: bool,
    },
    /// Cancel a running pipeline
    Cancel { id: i64 },
    /// Retry a pipeline
    Retry { id: i64 },
    /// Approve an environment-gated job
    Approve {
        id: i64,
        /// Job id or name
        job: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ReleaseCommand {
    /// List releases
    List {
        /// Stop after N entries (default 30)
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Show one release (by tag) with its assets
    View {
        tag: String,
        /// Open the releases page in the browser
        #[arg(long)]
        web: bool,
    },
    /// Create a release and optionally upload files
    Create {
        tag: String,
        /// Release title (default: the tag)
        #[arg(short = 't', long)]
        title: Option<String>,
        /// Release notes text
        #[arg(short = 'n', long)]
        notes: Option<String>,
        /// Read release notes from a file
        #[arg(long, value_name = "FILE")]
        notes_file: Option<String>,
        #[arg(long)]
        draft: bool,
        #[arg(long)]
        prerelease: bool,
        /// Files to upload as assets after creation
        files: Vec<String>,
    },
    /// Upload files as assets of a release
    Upload { tag: String, files: Vec<String> },
    /// Edit a release's title, notes or flags
    Edit {
        tag: String,
        #[arg(short = 't', long)]
        title: Option<String>,
        #[arg(short = 'n', long)]
        notes: Option<String>,
        #[arg(long, value_name = "FILE")]
        notes_file: Option<String>,
        /// --draft / --draft=false
        #[arg(long, num_args = 0..=1, default_missing_value = "true")]
        draft: Option<bool>,
        /// --prerelease / --prerelease=false
        #[arg(long, num_args = 0..=1, default_missing_value = "true")]
        prerelease: Option<bool>,
    },
    /// Delete a release (asks for confirmation unless --yes)
    Delete {
        tag: String,
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Download release assets (SHA-256 verified) into the current directory
    Download {
        tag: String,
        /// Asset filename filter; `*` wildcards allowed (default: all)
        #[arg(short = 'p', long, value_name = "PATTERN")]
        pattern: Option<String>,
        /// Overwrite files that already exist in the current directory
        #[arg(long)]
        clobber: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum LabelCommand {
    /// List labels
    List,
    /// Create a label
    Create {
        name: String,
        /// Hex color without the leading '#'
        #[arg(short = 'c', long)]
        color: Option<String>,
        #[arg(short = 'd', long)]
        description: Option<String>,
    },
    /// Edit a label identified by its current name
    Edit {
        name: String,
        #[arg(long)]
        new_name: Option<String>,
        #[arg(short = 'c', long)]
        color: Option<String>,
        #[arg(short = 'd', long)]
        description: Option<String>,
    },
    /// Delete a label (asks for confirmation unless --yes)
    Delete {
        name: String,
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum MilestoneCommand {
    /// List milestones
    List,
    /// Create a milestone
    Create {
        title: String,
        #[arg(short = 'd', long)]
        description: Option<String>,
        /// Due date, YYYY-MM-DD
        #[arg(long)]
        due: Option<String>,
    },
    /// Edit a milestone identified by its current title
    Edit {
        title: String,
        #[arg(long)]
        new_title: Option<String>,
        #[arg(short = 'd', long)]
        description: Option<String>,
        /// open / closed
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        due: Option<String>,
    },
    /// Delete a milestone (asks for confirmation unless --yes)
    Delete {
        title: String,
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum NotificationsCommand {
    /// List notifications (default when no subcommand is given)
    List {
        /// Only unread notifications
        #[arg(long)]
        unread: bool,
        /// Stop after N entries (default 30)
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Mark one notification (or all of them) as read
    Read {
        /// Notification id; omit with --all
        id: Option<i64>,
        #[arg(long)]
        all: bool,
    },
    /// Delete one notification
    Delete { id: i64 },
    /// Print the unread count
    Count,
}

#[derive(Debug, Subcommand)]
pub enum OrgCommand {
    /// List the organizations you belong to
    List,
    /// Show one organization
    View { name: String },
    /// Create an organization
    Create {
        name: String,
        /// Display name
        #[arg(long)]
        display: Option<String>,
        #[arg(short = 'd', long)]
        description: Option<String>,
        /// Create it as a private organization
        #[arg(long)]
        private: bool,
    },
    /// List an organization's members
    Members { name: String },
    /// Add a member by username
    AddMember {
        name: String,
        username: String,
        /// owner / admin / member (default: member)
        #[arg(long)]
        role: Option<String>,
    },
    /// Remove a member by username
    RemoveMember { name: String, username: String },
}
