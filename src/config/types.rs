use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use clap::ValueEnum;
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Architecture(String);

impl Architecture {
    pub fn host() -> Self {
        Self::from_alias(std::env::consts::ARCH).unwrap_or_else(|| Self("x64".to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn runner_suffix(&self) -> String {
        self.0
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '-'
                }
            })
            .collect()
    }

    pub fn platform(&self) -> String {
        match self.0.as_str() {
            "x64" => "linux/amd64".to_string(),
            "arm64" => "linux/arm64".to_string(),
            value if value.starts_with("linux/") => value.to_string(),
            value => format!("linux/{value}"),
        }
    }

    fn from_alias(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }

        let value = value.to_ascii_lowercase().replace('-', "_");
        let value = value.strip_prefix("linux/").unwrap_or(&value);
        let canonical = match value {
            "amd64" | "x64" | "x86_64" => "x64".to_string(),
            "arm64" | "aarch64" => "arm64".to_string(),
            other => other.replace('_', "-"),
        };
        Some(Self(canonical))
    }
}

impl Default for Architecture {
    fn default() -> Self {
        Self::host()
    }
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Architecture {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Self::from_alias(value).ok_or_else(|| "architecture must not be empty".to_string())
    }
}

impl Serialize for Architecture {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Architecture {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ColorWhen {
    #[default]
    Auto,
    Always,
    Never,
}

impl fmt::Display for ColorWhen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        };
        f.write_str(value)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ContainerRuntime {
    #[default]
    Auto,
    Podman,
    Docker,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum GitMode {
    Host,
    #[default]
    Auto,
    Alias,
    Flatpak,
    Custom,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GitCommand(Vec<String>);

impl GitCommand {
    pub fn new(parts: Vec<String>) -> std::result::Result<Self, String> {
        if parts.is_empty() {
            return Err("git command must not be empty".to_string());
        }
        if parts.iter().any(|part| part.trim().is_empty()) {
            return Err("git command parts must not be empty".to_string());
        }
        Ok(Self(parts))
    }

    pub fn parts(&self) -> &[String] {
        &self.0
    }

    pub fn render(&self) -> String {
        self.0.join(" ")
    }
}

impl FromStr for GitCommand {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Self::new(value.split_whitespace().map(str::to_string).collect())
    }
}

impl<'de> Deserialize<'de> for GitCommand {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct GitCommandVisitor;

        impl<'de> Visitor<'de> for GitCommandVisitor {
            type Value = GitCommand;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a git command string or list of command parts")
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                GitCommand::from_str(value).map_err(E::custom)
            }

            fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut parts = Vec::new();
                while let Some(part) = seq.next_element::<String>()? {
                    parts.push(part);
                }
                GitCommand::new(parts).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_any(GitCommandVisitor)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum InstallMode {
    #[default]
    Link,
    Copy,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactMode {
    #[default]
    Keep,
    Move,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ContainerType {
    Auto,
    General,
    Rust,
    #[serde(
        alias = "javascript",
        alias = "js",
        alias = "npm",
        alias = "yarn",
        alias = "pnpm"
    )]
    #[value(
        alias = "javascript",
        alias = "js",
        alias = "npm",
        alias = "yarn",
        alias = "pnpm"
    )]
    Node,
    #[serde(alias = "golang")]
    #[value(alias = "golang")]
    Go,
    #[serde(alias = "py")]
    #[value(alias = "py")]
    Python,
    #[serde(alias = "java")]
    #[value(alias = "java")]
    Maven,
    Gradle,
    #[serde(alias = "dot-net", alias = ".net", alias = "csharp", alias = "cs")]
    #[value(alias = "dot-net", alias = ".net", alias = "csharp", alias = "cs")]
    Dotnet,
}

impl ContainerType {
    pub fn as_name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::General => "general",
            Self::Rust => "rust",
            Self::Node => "node",
            Self::Go => "go",
            Self::Python => "python",
            Self::Maven => "maven",
            Self::Gradle => "gradle",
            Self::Dotnet => "dotnet",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(untagged)]
pub enum EventFilter {
    #[default]
    None,
    Single(String),
    Many(Vec<String>),
}

impl EventFilter {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::Single(value) => vec![value.clone()],
            Self::Many(values) => values.clone(),
        }
    }

    pub fn merged(&self, other: &Self) -> Self {
        match other {
            Self::None => self.clone(),
            _ => other.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ArchFilter {
    #[default]
    None,
    Single(Architecture),
    Many(Vec<Architecture>),
}

impl ArchFilter {
    pub fn to_vec(&self) -> Vec<Architecture> {
        match self {
            Self::None => Vec::new(),
            Self::Single(value) => vec![value.clone()],
            Self::Many(values) => values.clone(),
        }
    }

    pub fn merged(&self, other: &Self) -> Self {
        match other {
            Self::None => self.clone(),
            _ => other.clone(),
        }
    }

    pub fn allows(&self, arch: &Architecture) -> bool {
        match self {
            Self::None => true,
            Self::Many(values) if values.is_empty() => true,
            Self::Single(value) => value == arch,
            Self::Many(values) => values.iter().any(|value| value == arch),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::None => true,
            Self::Many(values) => values.is_empty(),
            Self::Single(_) => false,
        }
    }
}

pub fn format_arches(arches: &[Architecture]) -> String {
    if arches.is_empty() {
        return Architecture::host().to_string();
    }
    arches
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ArtifactConfig {
    #[serde(default)]
    pub paths: Vec<String>,

    pub mode: Option<ArtifactMode>,

    pub destination: Option<PathBuf>,
}
