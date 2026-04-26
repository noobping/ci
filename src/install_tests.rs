use super::{install_hook, install_hook_dispatcher, is_executable, is_managed_hook, is_symlink};

#[test]
fn managed_hook_symlinks_to_shared_dispatcher() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hooks_dir = temp.path().join("hooks");
    let ci_dir = temp.path().join("ci");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks dir");
    std::fs::create_dir_all(&ci_dir).expect("create ci dir");
    let hook = hooks_dir.join("pre-push");
    let dispatcher = ci_dir.join("hook");

    install_hook_dispatcher(&dispatcher).expect("install dispatcher");
    install_hook(&hook, "pre-push", false, false).expect("install hook");

    assert!(is_symlink(&hook));
    assert_eq!(
        std::fs::read_link(&hook).expect("read hook symlink"),
        std::path::PathBuf::from("../ci/hook")
    );

    let content = std::fs::read_to_string(&dispatcher).expect("read dispatcher");
    assert!(content.contains("ci_hook=$(basename \"$0\")"));
    assert!(content.contains("run.$ci_arch"));
    assert!(content.contains("ci_runner=\"$ci_dir/run\""));
    assert!(is_managed_hook(&hook));
    assert!(is_executable(&hook));
}
