use assert_cmd::Command;

fn ifg() -> Command {
    Command::cargo_bin("ifg").unwrap()
}

#[test]
fn version_prints_crate_version() {
    let out = ifg().arg("--version").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(stdout.trim(), concat!("ifg ", env!("CARGO_PKG_VERSION")));
}

#[test]
fn no_arguments_shows_help_with_usage_exit_code() {
    let out = ifg().assert().code(2);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("Usage: ifg"), "stderr was: {stderr}");
}
