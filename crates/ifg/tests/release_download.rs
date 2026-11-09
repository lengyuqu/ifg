//! `ifg release download`: the temporary file must be exclusive (a planted
//! `<name>.part` symlink must not redirect the write), the checksum must be
//! enforced, and `--clobber` must decide whether an existing file is replaced.

use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn command(server: &MockServer, home: &TempDir, work: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .current_dir(work.path())
        .args(["-R", "a/r", "release"]);
    cmd
}

/// Mount the three endpoints a download walks: tag → assets → download.
async fn mount_release(server: &MockServer, payload: &[u8], checksum: Option<&str>) {
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/releases/tags/v1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7, "repo_id": 1, "tag_name": "v1", "target_commitish": "main",
            "title": "v1", "body": null, "is_draft": false, "is_prerelease": false,
            "author_id": 1
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/releases/7/assets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id": 9, "release_id": 7, "filename": "pkg.tar.gz",
            "size": payload.len(), "content_type": "application/gzip",
            "download_count": 0, "sha256": null
        }])))
        .mount(server)
        .await;
    let mut response = ResponseTemplate::new(200).set_body_bytes(payload.to_vec());
    if let Some(sum) = checksum {
        response = response.insert_header("x-checksum-sha256", sum);
    }
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/releases/assets/9/download"))
        .respond_with(response)
        .mount(server)
        .await;
}

#[tokio::test]
async fn downloads_with_checksum_and_leaves_no_temporary_file() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let payload = b"hello asset";
    mount_release(&server, payload, Some(&sha256_hex(payload))).await;

    command(&server, &home, &work)
        .args(["download", "v1"])
        .assert()
        .success();

    assert_eq!(
        std::fs::read(work.path().join("pkg.tar.gz")).unwrap(),
        payload
    );
    let leftovers: Vec<String> = std::fs::read_dir(work.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "pkg.tar.gz")
        .collect();
    assert!(leftovers.is_empty(), "leftover files: {leftovers:?}");
}

/// A fixed `<name>.part` used to be opened with `File::create`, which follows
/// symlinks: a planted link turned the download into a write to `keep.txt`.
#[cfg(unix)]
#[tokio::test]
async fn planted_part_symlink_cannot_redirect_the_write() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let payload = b"downloaded payload";
    mount_release(&server, payload, Some(&sha256_hex(payload))).await;

    let victim = work.path().join("keep.txt");
    std::fs::write(&victim, "original").unwrap();
    std::os::unix::fs::symlink(&victim, work.path().join("pkg.tar.gz.part")).unwrap();

    command(&server, &home, &work)
        .args(["download", "v1"])
        .assert()
        .success();

    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "original",
        "the symlink target must not be overwritten"
    );
    assert_eq!(
        std::fs::read(work.path().join("pkg.tar.gz")).unwrap(),
        payload
    );
}

#[tokio::test]
async fn checksum_mismatch_leaves_no_output_file() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    mount_release(&server, b"payload", Some("deadbeef")).await;

    command(&server, &home, &work)
        .args(["download", "v1"])
        .assert()
        .failure();

    assert!(!work.path().join("pkg.tar.gz").exists());
    let leftovers: Vec<String> = std::fs::read_dir(work.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(leftovers.is_empty(), "leftover files: {leftovers:?}");
}

#[tokio::test]
async fn existing_file_needs_clobber() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let payload = b"new payload";
    mount_release(&server, payload, Some(&sha256_hex(payload))).await;
    std::fs::write(work.path().join("pkg.tar.gz"), "old").unwrap();

    let refused = command(&server, &home, &work)
        .args(["download", "v1"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&refused.get_output().stderr).into_owned();
    assert!(stderr.contains("--clobber"), "stderr: {stderr}");

    assert_eq!(
        std::fs::read_to_string(work.path().join("pkg.tar.gz")).unwrap(),
        "old"
    );

    command(&server, &home, &work)
        .args(["download", "v1", "--clobber"])
        .assert()
        .success();

    assert_eq!(
        std::fs::read(work.path().join("pkg.tar.gz")).unwrap(),
        payload
    );
}
