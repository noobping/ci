use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::actions::ActionService;
use crate::config::{Architecture, ContainerRuntime};
use crate::error::{CiError, Result};
use crate::git::{command_exists, flatpak_host_command_exists, sanitize_component};
use crate::workflow::ResolvedWorkflow;

pub(crate) struct ContainerShellSpec<'a> {
    pub image: &'a str,
    pub repo_root: &'a Path,
    pub shell: &'a str,
    pub script: &'a str,
    pub env: &'a BTreeMap<String, String>,
    pub workdir: &'a Path,
    pub platform: Option<&'a str>,
    pub options: Option<&'a str>,
    pub extra_volumes: &'a [String],
    pub cache_mounts: &'a [(PathBuf, String)],
    pub container_workdir: Option<&'a str>,
    pub readonly: bool,
}

pub(crate) struct ContainerCommandExistsSpec<'a> {
    pub image: &'a str,
    pub repo_root: &'a Path,
    pub env: &'a BTreeMap<String, String>,
    pub platform: Option<&'a str>,
}

pub(crate) struct ContainerBackend {
    runtime: ContainerRuntimeName,
    command: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContainerRuntimeName {
    Podman,
    Docker,
}

impl ContainerRuntimeName {
    fn as_str(self) -> &'static str {
        match self {
            Self::Podman => "podman",
            Self::Docker => "docker",
        }
    }
}

impl ContainerBackend {
    pub(crate) fn detect(runtime: ContainerRuntime) -> Result<Self> {
        match runtime {
            ContainerRuntime::Podman => Self::detect_named(ContainerRuntimeName::Podman),
            ContainerRuntime::Docker => Self::detect_named(ContainerRuntimeName::Docker),
            ContainerRuntime::Auto => Self::detect_auto(),
        }
    }

    pub(crate) fn preferred_runtime_label() -> Option<String> {
        Self::detect(ContainerRuntime::Auto)
            .ok()
            .map(|backend| backend.render_runtime())
    }

    fn detect_auto() -> Result<Self> {
        Self::detect_available(ContainerRuntimeName::Podman)
            .or_else(|| Self::detect_available(ContainerRuntimeName::Docker))
            .ok_or_else(|| {
                CiError::Message(
                    "container runtime `podman` or `docker` is not available".to_string(),
                )
            })
    }

    fn detect_named(runtime: ContainerRuntimeName) -> Result<Self> {
        Self::detect_available(runtime).ok_or_else(|| {
            CiError::Message(format!(
                "container runtime `{}` is not available",
                runtime.as_str()
            ))
        })
    }

    fn detect_available(runtime: ContainerRuntimeName) -> Option<Self> {
        let runtime_name = runtime.as_str();
        if command_exists(runtime_name) {
            return Some(Self {
                runtime,
                command: vec![runtime_name.to_string()],
            });
        }
        if flatpak_host_command_exists(runtime_name) {
            return Some(Self {
                runtime,
                command: vec![
                    "flatpak-spawn".to_string(),
                    "--host".to_string(),
                    runtime_name.to_string(),
                ],
            });
        }
        None
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.command[0]);
        command.args(&self.command[1..]);
        command
    }

    pub(crate) fn command_tokens(&self) -> &[String] {
        &self.command
    }

    pub(crate) fn runtime_name(&self) -> &'static str {
        self.runtime.as_str()
    }

    fn render_runtime(&self) -> String {
        self.command.join(" ")
    }

    pub(crate) fn build(
        &self,
        file: &Path,
        context: &Path,
        tag: &str,
        platform: Option<&str>,
    ) -> Result<i32> {
        let mut command = self.command();
        if self.runtime == ContainerRuntimeName::Docker && platform.is_some() {
            command.arg("buildx").arg("build").arg("--load");
        } else {
            command.arg("build");
        }
        if let Some(platform) = platform {
            command.arg("--platform").arg(platform);
        }
        command
            .arg("-f")
            .arg(file)
            .arg("-t")
            .arg(tag)
            .arg(context)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        Ok(command.status()?.code().unwrap_or(1))
    }

    pub(crate) fn run_shell(&self, spec: &ContainerShellSpec<'_>) -> Result<i32> {
        let mount = self.bind_mount(spec.repo_root, "/work", spec.readonly);
        let container_workdir = spec
            .container_workdir
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                if let Ok(relative) = spec.workdir.strip_prefix(spec.repo_root) {
                    if relative.as_os_str().is_empty() {
                        "/work".to_string()
                    } else {
                        format!("/work/{}", relative.display())
                    }
                } else {
                    "/work".to_string()
                }
            });

        let mut command = self.command();
        command
            .arg("run")
            .arg("--rm")
            .arg("--network")
            .arg("host")
            .arg("-v")
            .arg(mount)
            .arg("-w")
            .arg(container_workdir);
        if let Some(platform) = spec.platform {
            command.arg("--platform").arg(platform);
        }
        if let Some(options) = spec.options {
            for part in options.split_whitespace() {
                command.arg(part);
            }
        }
        for volume in spec.extra_volumes {
            command.arg("-v").arg(volume);
        }
        for (source, target) in spec.cache_mounts {
            command
                .arg("-v")
                .arg(self.bind_mount(source, target, false));
        }
        for (key, value) in spec.env {
            command.arg("-e").arg(format!("{key}={value}"));
        }
        command
            .arg(spec.image)
            .arg(spec.shell)
            .arg("-c")
            .arg(spec.script)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        Ok(command.status()?.code().unwrap_or(1))
    }

    pub(crate) fn command_exists(
        &self,
        spec: &ContainerCommandExistsSpec<'_>,
        name: &str,
    ) -> Result<bool> {
        let mount = self.bind_mount(spec.repo_root, "/work", false);
        let mut command = self.command();
        command
            .arg("run")
            .arg("--rm")
            .arg("--network")
            .arg("host")
            .arg("-v")
            .arg(mount)
            .arg("-w")
            .arg("/work");
        if let Some(platform) = spec.platform {
            command.arg("--platform").arg(platform);
        }
        for (key, value) in spec.env {
            command.arg("-e").arg(format!("{key}={value}"));
        }
        command
            .arg(spec.image)
            .arg("/bin/sh")
            .arg("-c")
            .arg(format!(
                "command -v {} >/dev/null 2>&1",
                sh_single_quote(name)
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Ok(command.status()?.success())
    }

    pub(crate) fn run_action_container(
        &self,
        image: &str,
        action_dir: &Path,
        env: &BTreeMap<String, String>,
        entrypoint: Option<&str>,
        args: &[String],
        platform: Option<&str>,
    ) -> Result<i32> {
        let mount = self.bind_mount(action_dir, "/action", false);
        let mut command = self.command();
        command
            .arg("run")
            .arg("--rm")
            .arg("--network")
            .arg("host")
            .arg("-v")
            .arg(mount)
            .arg("-w")
            .arg("/action");
        if let Some(platform) = platform {
            command.arg("--platform").arg(platform);
        }
        if let Some(entrypoint) = entrypoint {
            command.arg("--entrypoint").arg(entrypoint);
        }
        for (key, value) in env {
            command.arg("-e").arg(format!("{key}={value}"));
        }
        command.arg(image);
        for arg in args {
            command.arg(arg);
        }
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        Ok(command.status()?.code().unwrap_or(1))
    }

    pub(crate) fn start_service(
        &self,
        name: &str,
        service: &ActionService,
        platform: Option<&str>,
    ) -> Result<()> {
        let mut command = self.command();
        command
            .arg("run")
            .arg("-d")
            .arg("--rm")
            .arg("--name")
            .arg(name)
            .arg("--network")
            .arg("host");
        if let Some(platform) = platform {
            command.arg("--platform").arg(platform);
        }
        if let Some(options) = service.options.as_deref() {
            for part in options.split_whitespace() {
                command.arg(part);
            }
        }
        for (key, value) in &service.env {
            command.arg("-e").arg(format!("{key}={value}"));
        }
        command.arg(&service.image);
        let status = command.status()?.code().unwrap_or(1);
        if status == 0 {
            Ok(())
        } else {
            Err(CiError::Message(format!(
                "failed to start service {} from {}",
                name, service.image
            )))
        }
    }

    pub(crate) fn stop_container(&self, name: &str) -> Result<()> {
        let status = self
            .command()
            .arg("rm")
            .arg("-f")
            .arg(name)
            .status()?
            .code()
            .unwrap_or(1);
        if status == 0 {
            Ok(())
        } else {
            Err(CiError::Message(format!("failed to stop container {name}")))
        }
    }

    fn bind_mount(&self, source: &Path, target: &str, readonly: bool) -> String {
        let mut mount = format!("{}:{target}", source.display());
        if self.runtime == ContainerRuntimeName::Podman {
            mount.push_str(":z");
        }
        if readonly {
            if self.runtime == ContainerRuntimeName::Podman {
                mount.push_str(",ro");
            } else {
                mount.push_str(":ro");
            }
        }
        mount
    }
}

pub(crate) fn container_platform(resolved: &ResolvedWorkflow, arch: &Architecture) -> String {
    resolved
        .container
        .platform
        .clone()
        .unwrap_or_else(|| arch.platform())
}

pub(crate) fn generated_native_container_image_name(workflow_name: &str, platform: &str) -> String {
    format!(
        "localhost/ci-{}-{}:latest",
        sanitize_component(workflow_name),
        sanitize_component(platform)
    )
}

pub(crate) fn normalized_rust_components(components: &[String]) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    for component in components {
        let value = component.trim();
        if value.is_empty() {
            return Err(CiError::Usage(
                "container component names must not be empty".to_string(),
            ));
        }
        if value.contains('\0') || value.contains('\n') || value.contains('\r') {
            return Err(CiError::Usage(format!(
                "container component `{component}` contains unsupported control characters"
            )));
        }

        let component = match value {
            "cargo-fmt" => "rustfmt",
            "cargo-clippy" => "clippy",
            other => other,
        };
        if !normalized.iter().any(|item| item == component) {
            normalized.push(component.to_string());
        }
    }
    Ok(normalized)
}

pub(crate) fn validate_container_packages(packages: &[String]) -> Result<()> {
    for package in packages {
        if package.trim().is_empty() {
            return Err(CiError::Usage(
                "container package names must not be empty".to_string(),
            ));
        }
        if package.contains('\0') || package.contains('\n') || package.contains('\r') {
            return Err(CiError::Usage(format!(
                "container package `{package}` contains unsupported control characters"
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_container_image_ref(image: &str) -> Result<()> {
    if image.trim().is_empty() {
        return Err(CiError::Usage(
            "container image must not be empty".to_string(),
        ));
    }
    if image.contains('\0') || image.chars().any(char::is_whitespace) {
        return Err(CiError::Usage(
            "container image contains unsupported whitespace or control characters".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn generated_native_containerfile(
    base_image: &str,
    packages: &[String],
    components: &[String],
) -> String {
    let mut content = format!("FROM {base_image}\n");

    if !components.is_empty() {
        let components = components
            .iter()
            .map(|component| sh_single_quote(component))
            .collect::<Vec<_>>()
            .join(" ");
        content.push_str(&format!("RUN rustup component add {components}\n"));
    }

    if packages.is_empty() {
        return content;
    }

    let packages = packages
        .iter()
        .map(|package| sh_single_quote(package))
        .collect::<Vec<_>>()
        .join(" ");
    content.push_str(&format!(
        "RUN set -eux; \\\n\
             if command -v apt-get >/dev/null 2>&1; then \\\n\
                 apt-get update; \\\n\
                 DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends {packages}; \\\n\
                 rm -rf /var/lib/apt/lists/*; \\\n\
             elif command -v dnf >/dev/null 2>&1; then \\\n\
                 dnf install -y {packages}; \\\n\
                 dnf clean all; \\\n\
             elif command -v apk >/dev/null 2>&1; then \\\n\
                 apk add --no-cache {packages}; \\\n\
             elif command -v zypper >/dev/null 2>&1; then \\\n\
                 zypper --non-interactive install {packages}; \\\n\
                 zypper clean --all; \\\n\
             else \\\n\
                 echo 'no supported package manager found in container image' >&2; \\\n\
                 exit 1; \\\n\
             fi\n"
    ));
    content
}

pub(crate) fn sh_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
