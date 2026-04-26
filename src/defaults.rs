use std::fs;
use std::path::Path;

use crate::config::{ContainerType, EventFilter, WorkflowOverride};
use crate::workflow::{
    NativeStep, NativeWorkflow, Workflow, WorkflowKind, WorkflowProvider, WorkflowSource,
};

const DEFAULT_RUST_WORKFLOW: &str = r#"name: build
on:
  - manual
  - pre-push
tech: rust
container:
  components: [cargo-fmt, cargo-clippy]
steps:
  - name: format
    run: cargo fmt --check
  - name: lint
    run: cargo clippy --all-targets -- -D warnings
  - name: test
    run: cargo test --all
  - name: build
    run: cargo build --release
"#;

const DEFAULT_SHELL_WORKFLOW: &str = r#"name: build
on:
  - manual
steps:
  - name: build
    run: echo "Add your build command to .ci/build.yml"
"#;

#[derive(Clone, Debug)]
pub(crate) struct DefaultBuildStack {
    pub container_type: ContainerType,
    pub build_command: String,
    pub host_tool: String,
}

pub(crate) fn init_build_workflow_content(
    repo_root: &Path,
    requested_stack: Option<ContainerType>,
) -> String {
    match default_build_stack(repo_root, requested_stack) {
        Some(stack) if stack.container_type == ContainerType::Rust => {
            DEFAULT_RUST_WORKFLOW.to_string()
        }
        Some(stack) => default_build_workflow_content(&stack),
        None => DEFAULT_SHELL_WORKFLOW.to_string(),
    }
}

pub(crate) fn default_build_stack(
    repo_root: &Path,
    requested_stack: Option<ContainerType>,
) -> Option<DefaultBuildStack> {
    match requested_stack.unwrap_or(ContainerType::Auto) {
        ContainerType::Auto => detect_default_build_stack(repo_root),
        ContainerType::General => None,
        stack => default_build_stack_for_type(repo_root, stack),
    }
}

pub(crate) fn detect_default_build_stack(repo_root: &Path) -> Option<DefaultBuildStack> {
    if repo_root.join("Cargo.toml").exists() {
        return default_build_stack_for_type(repo_root, ContainerType::Rust);
    }
    if repo_root.join("package.json").exists() {
        return default_build_stack_for_type(repo_root, ContainerType::Node);
    }
    if repo_root.join("go.mod").exists() {
        return default_build_stack_for_type(repo_root, ContainerType::Go);
    }
    if repo_root.join("pom.xml").exists() {
        return default_build_stack_for_type(repo_root, ContainerType::Maven);
    }
    if gradle_project_exists(repo_root) {
        return default_build_stack_for_type(repo_root, ContainerType::Gradle);
    }
    if dotnet_project_exists(repo_root) {
        return default_build_stack_for_type(repo_root, ContainerType::Dotnet);
    }
    if repo_root.join("pyproject.toml").exists() || repo_root.join("setup.py").exists() {
        return default_build_stack_for_type(repo_root, ContainerType::Python);
    }
    None
}

pub(crate) fn generated_default_workflows(
    repo_root: &Path,
    ci_dir: &Path,
    requested_stack: Option<ContainerType>,
    command_available: &dyn Fn(&str) -> bool,
) -> Vec<Workflow> {
    default_build_stack(repo_root, requested_stack)
        .map(|stack| generated_build_workflow(ci_dir, &stack, command_available(&stack.host_tool)))
        .into_iter()
        .collect()
}

fn default_build_stack_for_type(
    repo_root: &Path,
    stack: ContainerType,
) -> Option<DefaultBuildStack> {
    let (build_command, host_tool) = match stack {
        ContainerType::Rust => ("cargo build".to_string(), "cargo".to_string()),
        ContainerType::Node => (
            "npm install && npm run build --if-present".to_string(),
            "npm".to_string(),
        ),
        ContainerType::Go => ("go build ./...".to_string(), "go".to_string()),
        ContainerType::Python => (
            "python3 -m pip install --upgrade build && python3 -m build".to_string(),
            "python3".to_string(),
        ),
        ContainerType::Maven => ("mvn package".to_string(), "mvn".to_string()),
        ContainerType::Gradle if repo_root.join("gradlew").exists() => (
            "chmod +x ./gradlew && ./gradlew build".to_string(),
            "java".to_string(),
        ),
        ContainerType::Gradle => ("gradle build".to_string(), "gradle".to_string()),
        ContainerType::Dotnet => ("dotnet build".to_string(), "dotnet".to_string()),
        ContainerType::Auto | ContainerType::General => return None,
    };

    Some(DefaultBuildStack {
        container_type: stack,
        build_command,
        host_tool,
    })
}

fn default_build_workflow_content(stack: &DefaultBuildStack) -> String {
    format!(
        "name: build\n\
         on:\n\
           - manual\n\
           - pre-push\n\
         tech: {}\n\
         steps:\n\
           - name: build\n\
             run: {}\n",
        stack.container_type.as_name(),
        stack.build_command
    )
}

fn gradle_project_exists(repo_root: &Path) -> bool {
    [
        "gradlew",
        "build.gradle",
        "build.gradle.kts",
        "settings.gradle",
        "settings.gradle.kts",
    ]
    .iter()
    .any(|name| repo_root.join(name).exists())
}

fn dotnet_project_exists(repo_root: &Path) -> bool {
    fs::read_dir(repo_root)
        .map(|entries| {
            entries.filter_map(|entry| entry.ok()).any(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|extension| {
                        matches!(extension.to_ascii_lowercase().as_str(), "sln" | "csproj")
                    })
            })
        })
        .unwrap_or(false)
}

fn generated_build_workflow(
    ci_dir: &Path,
    stack: &DefaultBuildStack,
    host_tool_available: bool,
) -> Workflow {
    let mut metadata = WorkflowOverride {
        on: EventFilter::Many(vec!["manual".to_string(), "pre-push".to_string()]),
        tech_stack: Some(stack.container_type),
        ..WorkflowOverride::default()
    };
    if !host_tool_available {
        metadata.container.kind = Some(stack.container_type);
    }

    Workflow {
        name: "build".to_string(),
        path: ci_dir.join("build.yml"),
        kind: WorkflowKind::NativeYaml,
        provider: WorkflowProvider::Native,
        needs: Vec::new(),
        source: WorkflowSource::NativeYaml(NativeWorkflow {
            metadata,
            steps: vec![NativeStep {
                name: Some("build".to_string()),
                run: Some(stack.build_command.clone()),
                uses: None,
                container: None,
                container_config: None,
                readonly: None,
                with: Default::default(),
                extra: Default::default(),
                shell: None,
                env: Default::default(),
                if_condition: None,
                working_directory: None,
                continue_on_error: false,
                timeout_minutes: None,
            }],
        }),
    }
}

#[cfg(test)]
#[path = "defaults_tests.rs"]
mod tests;
