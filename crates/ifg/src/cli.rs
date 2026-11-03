//! Command-line definition (design doc §5): global flags plus the `auth`
//! and `api` subcommands of M1.

use clap::{ArgAction, Parser, Subcommand};

/// Work with IronForge repositories, issues, pull requests, CI and releases
/// from the terminal.
#[derive(Debug, Parser)]
#[command(name = "ifg", version, arg_required_else_help = true)]
pub struct Cli {
    /// Host name from the config, or a base URL like https://forge.example.com
    #[arg(long, global = true, value_name = "NAME|URL")]
    pub host: Option<String>,

    /// Target repository as OWNER/NAME, or a full remote URL
    #[arg(short = 'R', long, global = true, value_name = "OWNER/NAME")]
    pub repo: Option<String>,

    /// Output JSON (server field names) instead of a table
    #[arg(long, global = true)]
    pub json: bool,

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
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Raw pass-through to the IronForge REST API (paths relative to /api/v1/)
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
    Pr {
        #[command(subcommand)]
        command: PrCommand,
    },
    /// Work with CI pipelines
    Ci {
        #[command(subcommand)]
        command: CiCommand,
    },
    /// Work with releases and their assets
    Release {
        #[command(subcommand)]
        command: ReleaseCommand,
    },
    /// Print the client version and (when reachable) the server version
    Version,
    /// Print a shell completion script (bash / zsh / fish / powershell)
    Completions {
        /// Target shell
        shell: clap_complete::Shell,
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
}

#[derive(Debug, Subcommand)]
pub enum RepoCommand {
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
    /// Clone a repository over HTTPS
    Clone {
        /// OWNER/NAME
        #[arg(value_name = "OWNER/NAME")]
        target: String,
        /// Target directory (default: the repository name)
        dir: Option<String>,
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
    /// Edit an issue's title or body
    Edit {
        number: i64,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
    },
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
