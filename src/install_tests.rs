use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::{install_hook, is_executable, is_managed_hook, is_symlink, HookInstallStrategy};

#[test]
fn managed_single_arch_hook_symlinks_to_runner() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hooks_dir = temp.path().join("hooks");
    let ci_dir = temp.path().join("ci");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks dir");
    std::fs::create_dir_all(&ci_dir).expect("create ci dir");
    let hook = hooks_dir.join("pre-push");
    let runner = ci_dir.join("run.x64");
    std::fs::write(&runner, "runner").expect("write runner");
    let mut permissions = std::fs::metadata(&runner)
        .expect("runner metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&runner, permissions).expect("chmod runner");

    install_hook(
        &hook,
        "pre-push",
        false,
        false,
        &HookInstallStrategy::DirectSymlink(PathBuf::from("../ci/run.x64")),
    )
    .expect("install hook");

    assert!(is_symlink(&hook));
    assert_eq!(
        std::fs::read_link(&hook).expect("read hook symlink"),
        PathBuf::from("../ci/run.x64")
    );

    assert!(is_managed_hook(&hook));
    assert!(is_executable(&hook));
}

#[test]
fn managed_multi_arch_hook_uses_universal_script() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hooks_dir = temp.path().join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks dir");
    let hook = hooks_dir.join("pre-push");

    install_hook(
        &hook,
        "pre-push",
        false,
        false,
        &HookInstallStrategy::UniversalScript,
    )
    .expect("install hook");

    assert!(!is_symlink(&hook));
    let content = std::fs::read_to_string(&hook).expect("read hook");
    assert!(content.contains("ci_hook=$(basename \"$0\")"));
    assert!(content.contains("run.$ci_arch"));
    assert!(content.contains("ci_runner=\"$ci_dir/run\""));
    assert!(is_managed_hook(&hook));
    assert!(is_executable(&hook));
}
