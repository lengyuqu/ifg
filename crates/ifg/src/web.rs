//! Browser helpers for `--web` (design doc §5.4): print the URL, then hand
//! it to the platform opener. Failure to open is not an error — the URL is
//! already on screen.

use crate::context::RepoContext;
use crate::error::AppError;

/// `{base}/{owner}/{repo}` — the SPA route tree mirrors the API layout.
pub fn repo_url(base: &str, repo: &RepoContext) -> String {
    format!(
        "{}/{}/{}",
        base.trim_end_matches('/'),
        repo.owner,
        repo.name
    )
}

pub fn issue_url(base: &str, repo: &RepoContext, number: i64) -> String {
    format!("{}/issues/{number}", repo_url(base, repo))
}

pub fn pr_url(base: &str, repo: &RepoContext, number: i64) -> String {
    format!("{}/pulls/{number}", repo_url(base, repo))
}

/// The server has no per-tag page; the release listing is the landing page.
pub fn releases_url(base: &str, repo: &RepoContext) -> String {
    format!("{}/releases", repo_url(base, repo))
}

/// Print and open `url`; returns Ok even when no opener exists.
///
/// Only http(s) reaches the opener: the URL is built from user-supplied
/// owner/name segments, and a Windows `cmd /c start` cannot be made safe for
/// arbitrary text.
pub fn open(url: &str) -> Result<(), AppError> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(AppError::Usage(format!(
            "refusing to open a non-HTTP URL: {url}"
        )));
    }
    println!("{url}");
    let result = open_command(url);
    match result {
        Ok(_) => Ok(()),
        Err(error) => {
            eprintln!("ifg: could not open a browser ({error}); visit the URL above");
            Ok(())
        }
    }
}

#[cfg(target_os = "macos")]
fn open_command(url: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new("open").arg(url).spawn()
}

// No `cmd /c start`: cmd.exe re-parses its arguments, so a URL containing
// `&` or `|` would run a second command. rundll32 takes the URL as a plain
// argument and hands it to the shell handler without a command interpreter.
#[cfg(target_os = "windows")]
fn open_command(url: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler"])
        .arg(url)
        .spawn()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_command(url: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new("xdg-open").arg(url).spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> RepoContext {
        RepoContext {
            owner: "alice".into(),
            name: "demo".into(),
        }
    }

    #[test]
    fn non_http_urls_are_refused_before_the_opener() {
        assert!(open("javascript:alert(1)").is_err());
        assert!(open("file:///etc/passwd").is_err());
        assert!(open("not a url").is_err());
    }

    #[test]
    fn urls_follow_the_spa_routes() {
        assert_eq!(
            repo_url("http://h:8080", &repo()),
            "http://h:8080/alice/demo"
        );
        assert_eq!(
            repo_url("http://h:8080/", &repo()),
            "http://h:8080/alice/demo"
        );
        assert_eq!(
            issue_url("http://h:8080", &repo(), 7),
            "http://h:8080/alice/demo/issues/7"
        );
        assert_eq!(
            pr_url("http://h:8080", &repo(), 3),
            "http://h:8080/alice/demo/pulls/3"
        );
        assert_eq!(
            releases_url("http://h:8080", &repo()),
            "http://h:8080/alice/demo/releases"
        );
    }
}
