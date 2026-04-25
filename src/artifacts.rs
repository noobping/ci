use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use glob::glob;
use serde::{Deserialize, Serialize};

use crate::cli::CleanArgs;
use crate::config::{ArtifactConfig, ArtifactMode};
use crate::error::{CiError, Result};
use crate::output::Output;
use crate::repo::RepoInfo;
use crate::runner::AppContext;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactEntry {
    pub name: String,
    pub stored_at: PathBuf,
    pub original_path: PathBuf,
    pub mode: ArtifactMode,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowRunRecord {
    pub workflow: String,
    pub provider: String,
    pub kind: String,
    pub path: PathBuf,
    pub status: i32,
    pub artifacts: Vec<ArtifactEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunManifest {
    pub run_id: String,
    pub event: String,
    pub branch: Option<String>,
    pub created_at_unix: u64,
    pub workflows: Vec<WorkflowRunRecord>,
}

pub struct ArtifactSession {
    repo_root: PathBuf,
    artifact_store: PathBuf,
    runs_dir: PathBuf,
    manifest: RunManifest,
    pending: BTreeMap<String, Vec<ArtifactEntry>>,
    output: Output,
}

impl ArtifactSession {
    pub fn new(
        repo: &RepoInfo,
        event: &str,
        branch: Option<&str>,
        run_id: &str,
        output: Output,
    ) -> Result<Self> {
        repo.ensure_state_dirs()?;
        Ok(Self {
            repo_root: repo.root.clone(),
            artifact_store: repo.artifact_store.clone(),
            runs_dir: repo.runs_dir.clone(),
            pending: BTreeMap::new(),
            output,
            manifest: RunManifest {
                run_id: run_id.to_string(),
                event: event.to_string(),
                branch: branch.map(ToOwned::to_owned),
                created_at_unix: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                workflows: Vec::new(),
            },
        })
    }

    pub fn capture_declared(
        &mut self,
        workflow: &str,
        artifacts: &ArtifactConfig,
        dry_run: bool,
    ) -> Result<Vec<ArtifactEntry>> {
        let mode = artifacts.mode.unwrap_or(ArtifactMode::Keep);
        if artifacts.paths.is_empty() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        for pattern in &artifacts.paths {
            let joined = self.repo_root.join(pattern);
            let pattern = joined.to_str().ok_or_else(|| {
                CiError::Message(format!("invalid artifact pattern {}", joined.display()))
            })?;
            for path in glob(pattern)? {
                let path = path?;
                if !path.exists() {
                    continue;
                }
                let relative = path.strip_prefix(&self.repo_root).unwrap_or(&path);
                let target = self
                    .artifact_store
                    .join(workflow)
                    .join(&self.manifest.run_id)
                    .join(relative);
                if dry_run {
                    println!(
                        "would store artifact {} at {}",
                        path.display(),
                        target.display()
                    );
                } else {
                    store_path(&path, &target, mode)?;
                }
                entries.push(ArtifactEntry {
                    name: relative
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("artifact")
                        .to_string(),
                    stored_at: target,
                    original_path: path,
                    mode,
                });
            }
        }

        Ok(entries)
    }

    pub fn upload_named_artifact(
        &mut self,
        workflow: &str,
        name: &str,
        paths: &[String],
        dry_run: bool,
    ) -> Result<Vec<ArtifactEntry>> {
        let config = ArtifactConfig {
            paths: paths.to_vec(),
            mode: Some(ArtifactMode::Keep),
            destination: None,
        };
        let mut entries = self.capture_declared(workflow, &config, dry_run)?;
        for entry in &mut entries {
            entry.name = name.to_string();
        }
        self.pending
            .entry(workflow.to_string())
            .or_default()
            .extend(entries.clone());
        Ok(entries)
    }

    pub fn download_named_artifact(&self, name: &str, dest: &Path, dry_run: bool) -> Result<usize> {
        let mut restored = 0;
        for artifacts in self.pending.values() {
            for artifact in artifacts {
                if artifact.name != name {
                    continue;
                }
                let target = dest.join(
                    artifact
                        .stored_at
                        .file_name()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from(name)),
                );
                if dry_run {
                    println!(
                        "would restore artifact {} to {}",
                        artifact.stored_at.display(),
                        target.display()
                    );
                } else if artifact.stored_at.exists() {
                    copy_recursively(&artifact.stored_at, &target)?;
                }
                restored += 1;
            }
        }

        let manifests = load_manifests(&self.runs_dir)?;
        for manifest in manifests {
            for workflow in manifest.workflows {
                for artifact in workflow.artifacts {
                    if artifact.name != name {
                        continue;
                    }
                    let relative = artifact
                        .stored_at
                        .file_name()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from(name));
                    let target = dest.join(relative);
                    if dry_run {
                        println!(
                            "would restore artifact {} to {}",
                            artifact.stored_at.display(),
                            target.display()
                        );
                    } else {
                        copy_recursively(&artifact.stored_at, &target)?;
                    }
                    restored += 1;
                }
            }
        }
        Ok(restored)
    }

    pub fn record_workflow(
        &mut self,
        workflow: &str,
        provider: &str,
        kind: &str,
        path: &Path,
        status: i32,
        artifacts: Vec<ArtifactEntry>,
    ) {
        self.output.verbose(format!(
            "recording run manifest entry for {workflow} with {} artifact(s)",
            artifacts.len()
        ));
        self.manifest.workflows.push(WorkflowRunRecord {
            workflow: workflow.to_string(),
            provider: provider.to_string(),
            kind: kind.to_string(),
            path: path.to_path_buf(),
            status,
            artifacts,
        });
    }

    pub fn take_pending_artifacts(&mut self, workflow: &str) -> Vec<ArtifactEntry> {
        self.pending.remove(workflow).unwrap_or_default()
    }

    pub fn finish(self) -> Result<()> {
        if self.manifest.workflows.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(&self.runs_dir)?;
        let path = self.runs_dir.join(format!("{}.json", self.manifest.run_id));
        fs::write(path, serde_json::to_vec_pretty(&self.manifest)?)?;
        Ok(())
    }
}

pub fn cmd_clean(ctx: &AppContext, args: &CleanArgs) -> Result<i32> {
    let manifests = load_manifests(&ctx.repo.runs_dir)?;
    let mut matched = false;
    let export_root = args
        .dest
        .clone()
        .unwrap_or_else(|| ctx.repo.artifact_store.join("export"));

    for manifest in manifests {
        if let Some(run_id) = &args.run_id {
            if &manifest.run_id != run_id {
                continue;
            }
        }

        let mut touched_manifest = false;
        for workflow in &manifest.workflows {
            if let Some(requested) = &args.workflow {
                if &workflow.workflow != requested {
                    continue;
                }
            }
            matched = true;
            touched_manifest = true;
            for artifact in &workflow.artifacts {
                match args.mode {
                    ArtifactMode::Keep => {
                        println!("keeping {}", artifact.stored_at.display());
                    }
                    ArtifactMode::Move => {
                        let target = export_root
                            .join(&workflow.workflow)
                            .join(&manifest.run_id)
                            .join(
                                artifact
                                    .stored_at
                                    .file_name()
                                    .map(PathBuf::from)
                                    .unwrap_or_else(|| PathBuf::from(&artifact.name)),
                            );
                        if args.dry_run {
                            println!(
                                "would move {} to {}",
                                artifact.stored_at.display(),
                                target.display()
                            );
                        } else if artifact.stored_at.exists() {
                            move_path(&artifact.stored_at, &target)?;
                        }
                    }
                }
            }
        }

        if touched_manifest && matches!(args.mode, ArtifactMode::Move) {
            let manifest_path = ctx.repo.runs_dir.join(format!("{}.json", manifest.run_id));
            if args.dry_run {
                println!("would remove manifest {}", manifest_path.display());
            } else if manifest_path.exists() {
                fs::remove_file(manifest_path)?;
            }
        }
    }

    if !matched {
        ctx.output.info("No matching artifacts found.");
    }

    Ok(0)
}

pub fn load_manifests(dir: &Path) -> Result<Vec<RunManifest>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut manifests: Vec<RunManifest> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        manifests.push(serde_json::from_slice(&fs::read(path)?)?);
    }
    manifests.sort_by_key(|manifest| std::cmp::Reverse(manifest.created_at_unix));
    Ok(manifests)
}

fn store_path(source: &Path, target: &Path, mode: ArtifactMode) -> Result<()> {
    match mode {
        ArtifactMode::Keep => copy_recursively(source, target),
        ArtifactMode::Move => move_path(source, target),
    }
}

fn move_path(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_recursively(source, target)?;
            if source.is_dir() {
                fs::remove_dir_all(source)?;
            } else if source.exists() {
                fs::remove_file(source)?;
            }
            Ok(())
        }
    }
}

fn copy_recursively(source: &Path, target: &Path) -> Result<()> {
    if source.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let from = entry.path();
            let to = target.join(entry.file_name());
            copy_recursively(&from, &to)?;
        }
        Ok(())
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, target)?;
        Ok(())
    }
}
