use super::{install_hook, is_executable, is_managed_hook};

#[test]
fn managed_hook_dispatches_to_arch_runner_with_legacy_fallback() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hook = temp.path().join("pre-push");

    install_hook(&hook, "pre-push", false, false).expect("install hook");

    let content = std::fs::read_to_string(&hook).expect("read hook");
    assert!(content.contains("run.$ci_arch"));
    assert!(content.contains("ci_runner=\"$ci_dir/run\""));
    assert!(is_managed_hook(&hook));
    assert!(is_executable(&hook));
}
