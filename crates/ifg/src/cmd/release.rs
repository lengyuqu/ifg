//! `ifg release …` (design doc §4.6): list/view/create/upload/download.
//! Downloads stream to a `.part` file, hash SHA-256 while writing, and only
//! rename on a matching checksum.

use futures_util::StreamExt;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::bootstrap;
use crate::cli::{Cli, ReleaseCommand};
use crate::context::{resolve_repo, RepoContext};
use crate::error::AppError;
use crate::output;

const DEFAULT_LIMIT: u64 = 30;

pub async fn run(command: &ReleaseCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    match command {
        ReleaseCommand::List { limit } => list(&ctx, cli, &repo, *limit).await,
        ReleaseCommand::View { tag, web } => {
            if *web {
                return crate::web::open(&crate::web::releases_url(&ctx.host.url, &repo));
            }
            view(&ctx, cli, &repo, tag).await
        }
        ReleaseCommand::Create {
            tag,
            title,
            notes,
            notes_file,
            draft,
            prerelease,
            files,
        } => {
            create(
                &ctx,
                cli,
                &repo,
                tag,
                title.clone(),
                notes.clone(),
                notes_file.clone(),
                *draft,
                *prerelease,
                files,
            )
            .await
        }
        ReleaseCommand::Edit {
            tag,
            title,
            notes,
            notes_file,
            draft,
            prerelease,
        } => {
            edit(
                &ctx,
                cli,
                &repo,
                tag,
                title.clone(),
                notes.clone(),
                notes_file.clone(),
                *draft,
                *prerelease,
            )
            .await
        }
        ReleaseCommand::Delete { tag, yes } => delete(&ctx, &repo, tag, *yes).await,
        ReleaseCommand::Upload { tag, files } => upload(&ctx, &repo, tag, files).await,
        ReleaseCommand::Download {
            tag,
            pattern,
            clobber,
        } => download(&ctx, &repo, tag, pattern, *clobber).await,
    }
}

async fn list(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    limit: Option<u64>,
) -> Result<(), AppError> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if cli.json || cli.template.is_some() {
        let items: Vec<Value> = ctx
            .client
            .releases(&repo.owner, &repo.name)
            .list_raw()
            .limit(limit)
            .collect()
            .await?;
        match &cli.template {
            Some(template) => output::print_template(&items, template),
            None => output::print_json(&Value::Array(items)),
        }
        return Ok(());
    }
    let releases = ctx
        .client
        .releases(&repo.owner, &repo.name)
        .list()
        .limit(limit)
        .collect()
        .await?;
    let rows: Vec<Vec<String>> = releases
        .iter()
        .map(|release| {
            vec![
                release.tag_name.clone(),
                release.title.clone(),
                yes_no(release.is_draft),
                yes_no(release.is_prerelease),
                short_date(release.created_at.as_deref()),
            ]
        })
        .collect();
    if rows.is_empty() {
        println!("No releases in {repo}.");
        return Ok(());
    }
    output::print_table(&["TAG", "TITLE", "DRAFT", "PRE", "CREATED"], &rows);
    Ok(())
}

async fn view(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    tag: &str,
) -> Result<(), AppError> {
    let releases = ctx.client.releases(&repo.owner, &repo.name);
    let release = releases
        .find_by_tag(tag)
        .await?
        .ok_or_else(|| not_found(tag))?;
    let assets = releases.assets(release.id).await?;

    if cli.json || cli.template.is_some() {
        let value = releases.get_raw(release.id).await?;
        let mut value = value;
        value["assets"] = serde_json::json!(assets);
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
        return Ok(());
    }
    let rows = vec![
        vec!["tag".into(), release.tag_name.clone()],
        vec!["title".into(), release.title.clone()],
        vec!["target".into(), release.target_commitish.clone()],
        vec!["draft".into(), release.is_draft.to_string()],
        vec!["prerelease".into(), release.is_prerelease.to_string()],
        vec!["created".into(), short_date(release.created_at.as_deref())],
        vec!["notes".into(), release.body.clone().unwrap_or_default()],
    ];
    output::print_table(&["FIELD", "VALUE"], &rows);

    if !assets.is_empty() {
        println!();
        let rows: Vec<Vec<String>> = assets
            .iter()
            .map(|asset| {
                vec![
                    asset.filename.clone(),
                    asset.size.to_string(),
                    asset.content_type.clone(),
                    asset
                        .sha256
                        .clone()
                        .map(|mut s| {
                            s.truncate(12);
                            s
                        })
                        .unwrap_or_else(|| "—".into()),
                ]
            })
            .collect();
        output::print_table(&["ASSET", "SIZE", "TYPE", "SHA-256"], &rows);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn create(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    tag: &str,
    title: Option<String>,
    notes: Option<String>,
    notes_file: Option<String>,
    draft: bool,
    prerelease: bool,
    files: &[String],
) -> Result<(), AppError> {
    let body = match (notes, notes_file) {
        (Some(_), Some(_)) => {
            return Err(AppError::Usage(
                "-n and --notes-file are mutually exclusive".into(),
            ))
        }
        (Some(notes), None) => Some(notes),
        (None, Some(file)) => {
            Some(std::fs::read_to_string(&file).map_err(|e| read_error(&file, &e))?)
        }
        (None, None) => None,
    };
    let options = ifg_client::CreateReleaseOptions {
        tag_name: tag.to_string(),
        title: title.unwrap_or_else(|| tag.to_string()),
        body,
        target_commitish: None,
        is_draft: Some(draft),
        is_prerelease: Some(prerelease),
    };
    let releases = ctx.client.releases(&repo.owner, &repo.name);
    if cli.json && files.is_empty() {
        let value = releases.create_raw(&options).await?;
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
        return Ok(());
    }
    let release = releases.create(&options).await?;
    println!("Created release {} ({})", release.tag_name, release.id);

    for file in files {
        upload_file(ctx, repo, release.id, file).await?;
    }
    Ok(())
}

async fn upload(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    tag: &str,
    files: &[String],
) -> Result<(), AppError> {
    if files.is_empty() {
        return Err(AppError::Usage(
            "no files given; usage: ifg release upload <tag> <files…>".into(),
        ));
    }
    let releases = ctx.client.releases(&repo.owner, &repo.name);
    let release = releases
        .find_by_tag(tag)
        .await?
        .ok_or_else(|| not_found(tag))?;
    for file in files {
        upload_file(ctx, repo, release.id, file).await?;
    }
    Ok(())
}

async fn upload_file(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    release_id: i64,
    file: &str,
) -> Result<(), AppError> {
    let path = std::path::Path::new(file);
    if !path.is_file() {
        return Err(AppError::Usage(format!("{file} is not a file")));
    }
    let content_type = guess_content_type(path);
    let asset = ctx
        .client
        .releases(&repo.owner, &repo.name)
        .upload_file(release_id, path, &content_type)
        .await?;
    println!(
        "Uploaded {} ({} bytes, {})",
        asset.filename, asset.size, content_type
    );
    Ok(())
}

async fn download(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    tag: &str,
    pattern: &Option<String>,
    clobber: bool,
) -> Result<(), AppError> {
    let releases = ctx.client.releases(&repo.owner, &repo.name);
    let release = releases
        .find_by_tag(tag)
        .await?
        .ok_or_else(|| not_found(tag))?;
    let assets = releases.assets(release.id).await?;
    let selected: Vec<&ifg_client::Asset> = assets
        .iter()
        .filter(|asset| {
            pattern
                .as_deref()
                .is_none_or(|p| glob_match(p, &asset.filename))
        })
        .collect();
    if selected.is_empty() {
        return Err(AppError::Other(format!(
            "no assets matching {} in release {tag}",
            pattern.clone().unwrap_or_else(|| "*".into())
        )));
    }
    // Validate every target before downloading anything, so a bad name or
    // an existing file fails fast instead of half-way through.
    let mut targets = Vec::with_capacity(selected.len());
    for asset in selected {
        let target = local_filename(&asset.filename)?;
        if !clobber && std::path::Path::new(&target).exists() {
            return Err(AppError::Other(format!(
                "{target} already exists; pass --clobber to overwrite"
            )));
        }
        targets.push((asset, target));
    }
    for (asset, target) in targets {
        download_asset(ctx, repo, asset, &target).await?;
    }
    Ok(())
}

/// Stream the asset to `<filename>.part`, hash while writing, verify against
/// `x-checksum-sha256` (or the recorded asset digest), then rename (§4.5).
async fn download_asset(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    asset: &ifg_client::Asset,
    target: &str,
) -> Result<(), AppError> {
    use tokio::io::AsyncWriteExt;

    let response = ctx
        .client
        .releases(&repo.owner, &repo.name)
        .download(asset.id)
        .await?;
    let expected = response
        .headers()
        .get("x-checksum-sha256")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .or_else(|| asset.sha256.clone());

    let part_path = std::path::PathBuf::from(format!("{target}.part"));
    let mut file = tokio::fs::File::create(&part_path)
        .await
        .map_err(|e| AppError::Io(format!("could not create {}: {e}", part_path.display())))?;
    let mut hasher = Sha256::new();
    let mut stream = response.bytes_stream();
    let mut size: u64 = 0;
    // Every failure below must remove the partial file: leaving it behind
    // means a retry sees a stale `.part` and a `--clobber` run silently
    // clobbers something the user never saw completed.
    let mut write_error: Option<AppError> = None;
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(chunk) => {
                hasher.update(&chunk);
                size += chunk.len() as u64;
                if let Err(e) = file.write_all(&chunk).await {
                    write_error = Some(AppError::Io(format!("write failed: {e}")));
                    break;
                }
            }
            Err(e) => {
                write_error = Some(AppError::Other(format!("download interrupted: {e}")));
                break;
            }
        }
    }
    drop(file);
    if let Some(error) = write_error {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(error);
    }

    let actual = format!("{:x}", hasher.finalize());
    match expected {
        Some(expected) if expected.eq_ignore_ascii_case(&actual) => {}
        Some(expected) => {
            let _ = tokio::fs::remove_file(&part_path).await;
            return Err(AppError::Other(format!(
                "SHA-256 mismatch for {}: expected {expected}, got {actual}; the partial file was deleted",
                asset.filename
            )));
        }
        None => {
            eprintln!(
                "ifg: warning: no checksum recorded for {}; skipping verification",
                asset.filename
            );
        }
    }

    let final_path = std::path::PathBuf::from(target);
    if let Err(e) = tokio::fs::rename(&part_path, &final_path).await {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(AppError::Io(format!(
            "could not finalize {}: {e}",
            final_path.display()
        )));
    }
    println!("Downloaded {target} ({size} bytes)");
    Ok(())
}

/// The local name for a server-supplied asset filename. The server stores
/// filenames verbatim, so a name like `../../.zshrc` or `/etc/x` must never
/// reach the filesystem as a path: only a plain file name in the current
/// directory is accepted.
fn local_filename(server_name: &str) -> Result<String, AppError> {
    let reject = || {
        AppError::Other(format!(
            "refusing to download asset with unsafe filename {server_name:?}"
        ))
    };
    if server_name.contains(['/', '\\', '\0']) {
        return Err(reject());
    }
    let mut components = std::path::Path::new(server_name).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(name)), None) => {
            name.to_str().map(str::to_string).ok_or_else(reject)
        }
        _ => Err(reject()),
    }
}

/// A notes file the user pointed at could not be read; exit code 1.
fn read_error(path: &str, error: &std::io::Error) -> AppError {
    AppError::Io(format!("could not read {path}: {error}"))
}

fn not_found(tag: &str) -> AppError {
    AppError::NotFound(format!(
        "release '{tag}' not found (releases are matched by tag, newest first)"
    ))
}

fn yes_no(flag: bool) -> String {
    if flag {
        "yes".into()
    } else {
        "no".into()
    }
}

fn short_date(iso: Option<&str>) -> String {
    crate::output::human_time(iso)
}

/// Tiny glob: `*` matches any run of characters; everything else literal.
fn glob_match(pattern: &str, text: &str) -> bool {
    fn inner(pattern: &[char], text: &[char]) -> bool {
        match (pattern.first(), text.first()) {
            (None, None) => true,
            (None, Some(_)) => false,
            (Some(_), None) => false,
            (Some('*'), _) => {
                for skip in 0..=text.len() {
                    if inner(&pattern[1..], &text[skip..]) {
                        return true;
                    }
                }
                false
            }
            (Some(p), Some(t)) => p == t && inner(&pattern[1..], &text[1..]),
        }
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    inner(&pattern, &text)
}

fn guess_content_type(path: &std::path::Path) -> String {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
    {
        Some(ext) => match ext.as_str() {
            "gz" => "application/gzip".into(),
            "zip" => "application/zip".into(),
            "tar" => "application/x-tar".into(),
            "json" => "application/json".into(),
            "txt" | "md" => "text/plain".into(),
            "sha256" | "sha256sums" => "text/plain".into(),
            "deb" => "application/vnd.debian.binary-package".into(),
            "rpm" => "application/x-rpm".into(),
            _ => "application/octet-stream".into(),
        },
        None => "application/octet-stream".into(),
    }
}

/// `release edit <tag>`: patch the fields that were given.
#[allow(clippy::too_many_arguments)]
async fn edit(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    tag: &str,
    title: Option<String>,
    notes: Option<String>,
    notes_file: Option<String>,
    draft: Option<bool>,
    prerelease: Option<bool>,
) -> Result<(), AppError> {
    let body = match (notes, notes_file) {
        (Some(_), Some(_)) => {
            return Err(AppError::Usage(
                "-n and --notes-file are mutually exclusive".into(),
            ))
        }
        (Some(notes), None) => Some(notes),
        (None, Some(file)) => {
            Some(std::fs::read_to_string(&file).map_err(|e| read_error(&file, &e))?)
        }
        (None, None) => None,
    };
    if title.is_none() && body.is_none() && draft.is_none() && prerelease.is_none() {
        return Err(AppError::Usage(
            "nothing to edit: pass --title, -n/--notes-file, --draft or --prerelease".into(),
        ));
    }
    let releases = ctx.client.releases(&repo.owner, &repo.name);
    let release = releases
        .get_by_tag(tag)
        .await?
        .ok_or_else(|| not_found(tag))?;
    let options = ifg_client::UpdateReleaseOptions {
        title,
        body,
        is_draft: draft,
        is_prerelease: prerelease,
    };
    if cli.json || cli.template.is_some() {
        let value = releases.update_raw(release.id, &options).await?;
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
        return Ok(());
    }
    let updated = releases.update(release.id, &options).await?;
    println!("Updated release {} ({}).", updated.tag_name, updated.title);
    Ok(())
}

/// `release delete <tag>`.
async fn delete(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    tag: &str,
    yes: bool,
) -> Result<(), AppError> {
    let releases = ctx.client.releases(&repo.owner, &repo.name);
    let release = releases
        .get_by_tag(tag)
        .await?
        .ok_or_else(|| not_found(tag))?;
    if !yes
        && !super::confirm(&format!(
            "Delete release {} ({} assets) from {repo}?",
            release.tag_name, release.tag_name
        ))?
    {
        println!("Aborted.");
        return Ok(());
    }
    releases.delete(release.id).await?;
    println!("Deleted release {}.", release.tag_name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_wildcards_and_literals() {
        assert!(glob_match("ifg-*", "ifg-0.1.0.tar.gz"));
        assert!(glob_match("*.tar.gz", "ifg-0.1.0.tar.gz"));
        assert!(glob_match("SHA256SUMS", "SHA256SUMS"));
        assert!(!glob_match("ifg-*", "other.txt"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn plain_asset_names_are_kept() {
        assert_eq!(
            local_filename("ifg-0.1.0.tar.gz").unwrap(),
            "ifg-0.1.0.tar.gz"
        );
        assert_eq!(local_filename("SHA256SUMS").unwrap(), "SHA256SUMS");
        assert_eq!(local_filename(".hidden").unwrap(), ".hidden");
    }

    #[test]
    fn traversal_and_absolute_asset_names_are_rejected() {
        for name in [
            "../evil",
            "../../.zshrc",
            "/etc/passwd",
            "dir/file",
            "..\\evil",
            "..",
            ".",
            "",
            "a\0b",
        ] {
            assert!(local_filename(name).is_err(), "{name:?} must be rejected");
        }
    }

    #[test]
    fn content_types_are_guessed() {
        assert_eq!(
            guess_content_type(std::path::Path::new("a.tar.gz")),
            "application/gzip"
        );
        assert_eq!(
            guess_content_type(std::path::Path::new("a.json")),
            "application/json"
        );
        assert_eq!(
            guess_content_type(std::path::Path::new("a.bin")),
            "application/octet-stream"
        );
    }
}
