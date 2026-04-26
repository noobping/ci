use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

pub struct TestRepo {
    _temp: TempDir,
    root: PathBuf,
}

impl TestRepo {
    pub fn new() -> Self {
        let temp = TempDir::new().expect("create temp repo");
        let root = temp.path().to_path_buf();
        run_ok(Command::new("git").arg("init").arg(&root));
        run_ok(Command::new("git").arg("-C").arg(&root).args([
            "config",
            "user.email",
            "ci@example.test",
        ]));
        run_ok(
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["config", "user.name", "ci tests"]),
        );
        run_ok(Command::new("git").arg("-C").arg(&root).args([
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ]));
        Self { _temp: temp, root }
    }

    pub fn new_bare() -> Self {
        let temp = TempDir::new().expect("create temp repo");
        let root = temp.path().to_path_buf();
        run_ok(Command::new("git").arg("init").arg("--bare").arg(&root));
        Self { _temp: temp, root }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn write(&self, path: &str, content: &str) {
        let path = self.root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(path, content).expect("write test file");
    }

    pub fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root.join(path)).expect("read test file")
    }

    pub fn exists(&self, path: &str) -> bool {
        self.root.join(path).exists()
    }

    pub fn ci(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ci"));
        command.arg("--repo").arg(&self.root);
        command
    }
}

fn run_ok(command: &mut Command) {
    let output = command.output().expect("run setup command");
    assert!(
        output.status.success(),
        "setup command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
