use serde_json::json;

pub(crate) fn all_schema() -> serde_json::Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": {
            "config": config_schema(),
            "workflow": workflow_schema()
        }
    })
}

pub(crate) fn config_schema() -> serde_json::Value {
    let defaults = defaults_schema();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "ci config",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "defaults": defaults,
            "policy": defaults,
            "locked": defaults,
            "hooks": {
                "type": "object",
                "additionalProperties": workflow_override_schema()
            },
            "workflows": {
                "type": "object",
                "additionalProperties": workflow_override_schema()
            },
            "actions": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "node_image": { "type": "string" },
                    "node-image": { "type": "string" }
                }
            },
            "other_workflows": { "type": "boolean" },
            "other-workflows": { "type": "boolean" },
            "shell": { "type": "string" },
            "quiet": { "type": "boolean" },
            "fail_fast": { "type": "boolean" },
            "fail-fast": { "type": "boolean" },
            "tech": tech_schema(),
            "type": tech_schema(),
            "tech-stack": tech_schema(),
            "tech_stack": tech_schema(),
            "arch": arch_schema(),
            "container": container_schema(),
            "container_runtime": runtime_schema(),
            "container-runtime": runtime_schema(),
            "git_mode": git_mode_schema(),
            "git-mode": git_mode_schema(),
            "git_command": git_command_schema(),
            "git-command": git_command_schema(),
            "git_image": { "type": "string" },
            "git-image": { "type": "string" },
            "install_mode": { "enum": ["link", "copy"] },
            "install-mode": { "enum": ["link", "copy"] },
            "recursive_checkout": { "type": "boolean" },
            "recursive-checkout": { "type": "boolean" },
            "artifact_store": { "type": "string" },
            "artifact-store": { "type": "string" },
            "actions_cache": { "type": "string" },
            "actions-cache": { "type": "string" },
            "branches": branches_schema()
        }
    })
}

pub(crate) fn workflow_schema() -> serde_json::Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "ci native workflow",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": { "type": "string" },
            "defaults": workflow_override_schema(),
            "on": event_schema(),
            "needs": dependency_schema(),
            "requires": dependency_schema(),
            "depends": dependency_schema(),
            "dependencies": dependency_schema(),
            "tech": tech_schema(),
            "type": tech_schema(),
            "tech-stack": tech_schema(),
            "tech_stack": tech_schema(),
            "arch": arch_schema(),
            "branches": branches_schema(),
            "artifacts": artifacts_schema(),
            "execution": execution_schema(),
            "container": container_schema(),
            "env": string_map_schema(),
            "steps": {
                "type": "array",
                "items": native_step_schema()
            }
        }
    })
}

fn defaults_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "shell": { "type": "string" },
            "quiet": { "type": "boolean" },
            "fail_fast": { "type": "boolean" },
            "fail-fast": { "type": "boolean" },
            "tech": tech_schema(),
            "type": tech_schema(),
            "tech-stack": tech_schema(),
            "tech_stack": tech_schema(),
            "arch": arch_schema(),
            "container": container_schema(),
            "container_runtime": runtime_schema(),
            "container-runtime": runtime_schema(),
            "git_mode": git_mode_schema(),
            "git-mode": git_mode_schema(),
            "git_command": git_command_schema(),
            "git-command": git_command_schema(),
            "git_image": { "type": "string" },
            "git-image": { "type": "string" },
            "install_mode": { "enum": ["link", "copy"] },
            "install-mode": { "enum": ["link", "copy"] },
            "recursive_checkout": { "type": "boolean" },
            "recursive-checkout": { "type": "boolean" },
            "artifact_store": { "type": "string" },
            "artifact-store": { "type": "string" },
            "actions_cache": { "type": "string" },
            "actions-cache": { "type": "string" },
            "branches": branches_schema()
        }
    })
}

fn workflow_override_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "on": event_schema(),
            "tech": tech_schema(),
            "type": tech_schema(),
            "tech-stack": tech_schema(),
            "tech_stack": tech_schema(),
            "arch": arch_schema(),
            "branches": branches_schema(),
            "artifacts": artifacts_schema(),
            "execution": execution_schema(),
            "container": container_schema(),
            "env": string_map_schema()
        }
    })
}

fn native_step_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": true,
        "properties": {
            "name": { "type": "string" },
            "run": { "type": "string" },
            "use": { "type": "string" },
            "uses": { "type": "string" },
            "container": step_container_schema(),
            "readonly": { "type": "boolean" },
            "read-only": { "type": "boolean" },
            "read_only": { "type": "boolean" },
            "shell": { "type": "string" },
            "env": string_map_schema(),
            "with": { "type": "object" },
            "if": { "type": "string" },
            "working-directory": { "type": "string" },
            "continue-on-error": { "type": "boolean" },
            "timeout-minutes": { "type": "integer", "minimum": 1 }
        }
    })
}

fn step_container_schema() -> serde_json::Value {
    json!({
        "oneOf": [
            { "type": "boolean" },
            { "type": "string" },
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "image": { "type": "string" },
                    "file": { "type": "string" },
                    "containerfile": { "type": "string" },
                    "container-file": { "type": "string" },
                    "container_file": { "type": "string" },
                    "dockerfile": { "type": "string" },
                    "docker-file": { "type": "string" },
                    "docker_file": { "type": "string" },
                    "platform": { "type": "string" },
                    "workdir": { "type": "string" },
                    "working-directory": { "type": "string" },
                    "working_directory": { "type": "string" },
                    "readonly": { "type": "boolean" },
                    "read-only": { "type": "boolean" },
                    "read_only": { "type": "boolean" },
                    "env": string_map_schema(),
                    "volumes": string_array_schema(),
                    "packages": string_array_schema(),
                    "components": string_array_schema()
                }
            }
        ]
    })
}

fn container_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "type": tech_schema(),
            "image": { "type": "string" },
            "platform": { "type": "string" },
            "workdir": { "type": "string" },
            "working-directory": { "type": "string" },
            "working_directory": { "type": "string" },
            "readonly": { "type": "boolean" },
            "read-only": { "type": "boolean" },
            "read_only": { "type": "boolean" },
            "arch": arch_schema(),
            "packages": string_array_schema(),
            "components": string_array_schema(),
            "env": string_map_schema(),
            "volumes": string_array_schema()
        }
    })
}

fn branches_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "allow": string_array_schema(),
            "only": string_array_schema()
        }
    })
}

fn artifacts_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "paths": string_array_schema(),
            "mode": { "enum": ["keep", "move"] },
            "destination": { "type": "string" }
        }
    })
}

fn execution_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "workspace": { "type": "string" },
            "shell": { "type": "string" }
        }
    })
}

fn event_schema() -> serde_json::Value {
    json!({
        "oneOf": [
            { "type": "string" },
            string_array_schema()
        ]
    })
}

fn arch_schema() -> serde_json::Value {
    json!({
        "oneOf": [
            { "type": "string" },
            string_array_schema()
        ]
    })
}

fn dependency_schema() -> serde_json::Value {
    json!({
        "oneOf": [
            { "type": "string" },
            string_array_schema()
        ]
    })
}

fn tech_schema() -> serde_json::Value {
    json!({
        "enum": [
            "auto",
            "general",
            "rust",
            "node",
            "go",
            "python",
            "maven",
            "gradle",
            "dotnet"
        ]
    })
}

fn runtime_schema() -> serde_json::Value {
    json!({ "enum": ["auto", "podman", "docker"] })
}

fn git_mode_schema() -> serde_json::Value {
    json!({ "enum": ["host", "auto", "alias", "flatpak", "custom"] })
}

fn git_command_schema() -> serde_json::Value {
    json!({
        "oneOf": [
            { "type": "string" },
            string_array_schema()
        ]
    })
}

fn string_array_schema() -> serde_json::Value {
    json!({
        "type": "array",
        "items": { "type": "string" }
    })
}

fn string_map_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "additionalProperties": { "type": "string" }
    })
}
