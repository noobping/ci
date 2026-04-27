use super::git_args_for_verbosity;

#[test]
fn verbose_git_args_adds_command_verbose_when_supported() {
    assert_eq!(
        git_args_for_verbosity(&["clone", "--depth", "1", "repo"], true, false),
        vec!["clone", "--verbose", "--depth", "1", "repo"]
    );
    assert_eq!(
        git_args_for_verbosity(&["push", "origin", "HEAD"], true, false),
        vec!["push", "--verbose", "origin", "HEAD"]
    );
}

#[test]
fn verbose_git_args_leaves_unsupported_or_quiet_commands_alone() {
    assert_eq!(
        git_args_for_verbosity(&["rev-parse", "--abbrev-ref", "HEAD"], true, false),
        vec!["rev-parse", "--abbrev-ref", "HEAD"]
    );
    assert_eq!(
        git_args_for_verbosity(&["fetch", "--quiet", "origin"], true, false),
        vec!["fetch", "--quiet", "origin"]
    );
    assert_eq!(
        git_args_for_verbosity(&["status", "--verbose"], true, false),
        vec!["status", "--verbose"]
    );
}

#[test]
fn quiet_git_args_adds_command_quiet_when_supported() {
    assert_eq!(
        git_args_for_verbosity(&["add", "dist"], false, true),
        vec!["add", "--quiet", "dist"]
    );
    assert_eq!(
        git_args_for_verbosity(&["clone", "--depth", "1", "repo"], false, true),
        vec!["clone", "--quiet", "--depth", "1", "repo"]
    );
    assert_eq!(
        git_args_for_verbosity(&["fetch", "--all", "--prune"], false, true),
        vec!["fetch", "--quiet", "--all", "--prune"]
    );
    assert_eq!(
        git_args_for_verbosity(&["submodule", "update", "--init"], false, true),
        vec!["submodule", "--quiet", "update", "--init"]
    );
}

#[test]
fn quiet_git_args_falls_back_to_silent_for_unknown_commands() {
    assert_eq!(
        git_args_for_verbosity(&["custom-tool", "run"], false, true),
        vec!["custom-tool", "--silent", "run"]
    );
}

#[test]
fn quiet_git_args_leaves_known_unsupported_or_verbose_commands_alone() {
    assert_eq!(
        git_args_for_verbosity(&["rev-parse", "--abbrev-ref", "HEAD"], false, true),
        vec!["rev-parse", "--abbrev-ref", "HEAD"]
    );
    assert_eq!(
        git_args_for_verbosity(&["fetch", "--verbose", "origin"], false, true),
        vec!["fetch", "--verbose", "origin"]
    );
    assert_eq!(
        git_args_for_verbosity(&["fetch", "--silent", "origin"], false, true),
        vec!["fetch", "--silent", "origin"]
    );
    assert_eq!(
        git_args_for_verbosity(&["status", "--short"], false, true),
        vec!["status", "--short"]
    );
}
