use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::config::Architecture;
use crate::error::Result;
use crate::workflow::canonical_events;

pub(crate) type ConditionCommandProbe<'a> = dyn Fn(&str) -> Result<bool> + 'a;

#[derive(Clone, Debug)]
pub(crate) struct ExpressionContext<'a> {
    pub event: &'a str,
    pub branch: Option<&'a str>,
    pub root: &'a Path,
    pub env: &'a BTreeMap<String, String>,
    pub matrix: &'a BTreeMap<String, String>,
    pub inputs: &'a BTreeMap<String, String>,
    pub success: bool,
    pub previous_failed: bool,
}

pub(crate) fn evaluate_condition(expr: Option<&str>, ctx: &ExpressionContext<'_>) -> bool {
    evaluate_condition_with_probe(expr, ctx, None).unwrap_or(false)
}

pub(crate) fn evaluate_condition_with_probe(
    expr: Option<&str>,
    ctx: &ExpressionContext<'_>,
    command_probe: Option<&ConditionCommandProbe<'_>>,
) -> Result<bool> {
    let Some(expr) = expr else {
        return Ok(ctx.success);
    };
    let expr = trim_expr(expr);

    if let Some(parts) = split_logical_operator(expr, "||", "or") {
        for part in parts {
            if evaluate_condition_with_probe(Some(part), ctx, command_probe)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if let Some(parts) = split_logical_operator(expr, "&&", "and") {
        for part in parts {
            if !evaluate_condition_with_probe(Some(part), ctx, command_probe)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    if let Some(rest) = expr.strip_prefix('!') {
        return Ok(!evaluate_condition_with_probe(
            Some(rest),
            ctx,
            command_probe,
        )?);
    }

    match expr {
        "true" | "always" | "always()" => return Ok(true),
        "false" | "cancelled" | "cancelled()" => return Ok(false),
        "success" | "success()" => return Ok(ctx.success),
        "failure" | "failure()" => return Ok(ctx.previous_failed),
        _ => {}
    }

    if let Some(target) = function_arg_any(expr, &["exists", "has", "is"]) {
        return condition_target_exists(&resolve_condition_target(target, ctx), ctx, command_probe);
    }
    if let Some(target) = function_arg_any(expr, &["missing", "not"]) {
        return condition_target_exists(&resolve_condition_target(target, ctx), ctx, command_probe)
            .map(|exists| !exists);
    }
    if let Some(target) = function_arg(expr, "arch") {
        return Ok(condition_arch_matches(target, ctx));
    }

    if let Some(target) = word_function_arg(expr, "arch") {
        return Ok(condition_arch_matches(target, ctx));
    }
    if let Some(target) = word_function_arg_any(expr, &["exists", "has"]) {
        return condition_target_exists(&resolve_condition_target(target, ctx), ctx, command_probe);
    }
    if let Some(target) = word_function_arg(expr, "missing") {
        return condition_target_exists(&resolve_condition_target(target, ctx), ctx, command_probe)
            .map(|exists| !exists);
    }
    if let Some(rest) = word_prefix_arg(expr, "is") {
        if !is_nested_condition_expr(rest) {
            return condition_target_exists(
                &resolve_condition_target(rest, ctx),
                ctx,
                command_probe,
            );
        }
        return evaluate_condition_with_probe(Some(rest), ctx, command_probe);
    }
    if let Some(rest) = word_prefix_arg(expr, "not") {
        if !is_nested_condition_expr(rest) {
            return condition_target_exists(
                &resolve_condition_target(rest, ctx),
                ctx,
                command_probe,
            )
            .map(|exists| !exists);
        }
        return Ok(!evaluate_condition_with_probe(
            Some(rest),
            ctx,
            command_probe,
        )?);
    }

    if let Some(rest) = expr
        .strip_prefix("startsWith(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let mut parts = rest.splitn(2, ',');
        let left = parts.next().unwrap_or("").trim();
        let right = parts
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('\'')
            .trim_matches('"');
        return Ok(resolve_expr_value(left, ctx)
            .map(|value| value.starts_with(right))
            .unwrap_or(false));
    }

    if let Some((left, right)) = expr.split_once("==") {
        return Ok(resolve_expr_value(left.trim(), ctx)
            .map(|value| value == trim_literal(right))
            .unwrap_or(false));
    }
    if let Some((left, right)) = expr.split_once("!=") {
        return Ok(resolve_expr_value(left.trim(), ctx)
            .map(|value| value != trim_literal(right))
            .unwrap_or(false));
    }

    Ok(resolve_expr_value(expr, ctx)
        .map(|value| !value.is_empty() && value != "false")
        .unwrap_or(false))
}

pub(crate) fn interpolate_expressions(value: &str, ctx: &ExpressionContext<'_>) -> String {
    let mut rendered = String::new();
    let mut remaining = value;

    while let Some(start) = remaining.find("${{") {
        rendered.push_str(&remaining[..start]);
        let after = &remaining[start + 3..];
        if let Some(end) = after.find("}}") {
            let expr = after[..end].trim();
            rendered.push_str(&resolve_expr_value(expr, ctx).unwrap_or_default());
            remaining = &after[end + 2..];
        } else {
            rendered.push_str(&remaining[start..]);
            return rendered;
        }
    }

    rendered.push_str(remaining);
    rendered
}

pub(crate) fn resolve_expr_value(expr: &str, ctx: &ExpressionContext<'_>) -> Option<String> {
    match trim_expr(expr) {
        "github.ref" | "gitea.ref" => ctx.branch.map(|branch| format!("refs/heads/{branch}")),
        "github.ref_name" | "gitea.ref_name" => ctx.branch.map(ToOwned::to_owned),
        "github.event_name" | "gitea.event_name" => Some(
            canonical_events(ctx.event)
                .last()
                .cloned()
                .unwrap_or_else(|| ctx.event.to_string()),
        ),
        "github.workspace" | "gitea.workspace" => ctx.env.get("CI_REPO").cloned(),
        value if value.starts_with("env.") => {
            ctx.env.get(value.trim_start_matches("env.")).cloned()
        }
        value if value.starts_with("matrix.") => {
            ctx.matrix.get(value.trim_start_matches("matrix.")).cloned()
        }
        value if value.starts_with("inputs.") => {
            ctx.inputs.get(value.trim_start_matches("inputs.")).cloned()
        }
        value if ctx.inputs.contains_key(value) => ctx.inputs.get(value).cloned(),
        value => Some(trim_literal(value)),
    }
}

pub(crate) fn executable_exists(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let path = Path::new(name);
    if path.components().count() > 1 {
        return executable_file_exists(path);
    }

    env::var_os("PATH")
        .map(|value| env::split_paths(&value).any(|dir| executable_file_exists(&dir.join(name))))
        .unwrap_or(false)
}

fn split_logical_operator<'a>(expr: &'a str, symbol: &str, word: &str) -> Option<Vec<&'a str>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut quote = None;
    let mut chars = expr.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }

        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 && expr[index..].starts_with(symbol) => {
                parts.push(expr[start..index].trim());
                start = index + symbol.len();
                for _ in 1..symbol.chars().count() {
                    chars.next();
                }
            }
            _ if depth == 0 && word_operator_at(expr, index, word) => {
                parts.push(expr[start..index].trim());
                start = index + word.len();
                for _ in 1..word.chars().count() {
                    chars.next();
                }
            }
            _ => {}
        }
    }

    if parts.is_empty() {
        None
    } else {
        parts.push(expr[start..].trim());
        Some(parts)
    }
}

fn word_operator_at(expr: &str, index: usize, word: &str) -> bool {
    expr[index..].starts_with(word)
        && expr[..index]
            .chars()
            .next_back()
            .map(|ch| !is_condition_word_char(ch))
            .unwrap_or(true)
        && expr[index + word.len()..]
            .chars()
            .next()
            .map(|ch| !is_condition_word_char(ch))
            .unwrap_or(true)
}

fn is_condition_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-')
}

fn condition_arch_matches(target: &str, ctx: &ExpressionContext<'_>) -> bool {
    let current = ctx
        .env
        .get("CI_ARCH")
        .cloned()
        .or_else(|| env::var("CI_ARCH").ok())
        .unwrap_or_else(|| Architecture::host().to_string());
    let Ok(current) = current.parse::<Architecture>() else {
        return false;
    };

    parse_condition_list(target).iter().any(|value| {
        let value = trim_literal(value);
        let value = resolve_expr_value(&value, ctx).unwrap_or(value);
        if matches!(
            value
                .trim()
                .to_ascii_lowercase()
                .replace(['-', ' '], "_")
                .as_str(),
            "host" | "host_arch" | "native"
        ) {
            return Architecture::host() == current;
        }
        value
            .parse::<Architecture>()
            .map(|arch| arch == current)
            .unwrap_or(false)
    })
}

fn trim_expr(expr: &str) -> &str {
    let trimmed = expr.trim();
    trimmed
        .strip_prefix("${{")
        .and_then(|value| value.strip_suffix("}}"))
        .map(str::trim)
        .unwrap_or(trimmed)
}

fn trim_literal(value: &str) -> String {
    value
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .to_string()
}

enum ConditionTarget {
    Generic(String),
    Path(String),
    File(String),
    Directory(String),
    Env(String),
    Command(String),
}

fn resolve_condition_target(value: &str, ctx: &ExpressionContext<'_>) -> ConditionTarget {
    let value = trim_literal(value);
    if let Some((kind, target)) = value.split_once(':') {
        let target = resolve_expr_value(target, ctx).unwrap_or_default();
        match kind {
            "env" => return ConditionTarget::Env(target),
            "path" => return ConditionTarget::Path(target),
            "file" => return ConditionTarget::File(target),
            "dir" | "directory" => return ConditionTarget::Directory(target),
            "cmd" | "command" | "exe" | "executable" => {
                return ConditionTarget::Command(target);
            }
            _ => {}
        }
    }
    ConditionTarget::Generic(resolve_expr_value(&value, ctx).unwrap_or_default())
}

fn function_arg<'a>(expr: &'a str, name: &str) -> Option<&'a str> {
    expr.strip_prefix(name)
        .and_then(|value| value.strip_prefix('('))
        .and_then(|value| value.strip_suffix(')'))
        .map(str::trim)
}

fn function_arg_any<'a>(expr: &'a str, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|name| function_arg(expr, name))
}

fn word_prefix_arg<'a>(expr: &'a str, word: &str) -> Option<&'a str> {
    if word_operator_at(expr, 0, word) {
        Some(expr[word.len()..].trim())
    } else {
        None
    }
}

fn word_function_arg<'a>(expr: &'a str, word: &str) -> Option<&'a str> {
    let target = word_prefix_arg(expr, word)?;
    if target.is_empty() || target.starts_with('(') {
        None
    } else {
        Some(target)
    }
}

fn word_function_arg_any<'a>(expr: &'a str, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|name| word_function_arg(expr, name))
}

fn is_nested_condition_expr(expr: &str) -> bool {
    let expr = trim_expr(expr);
    matches!(
        expr,
        "true"
            | "false"
            | "always"
            | "always()"
            | "cancelled"
            | "cancelled()"
            | "success"
            | "success()"
            | "failure"
            | "failure()"
    ) || expr.starts_with('!')
        || expr.contains("==")
        || expr.contains("!=")
        || expr
            .strip_prefix("startsWith(")
            .and_then(|value| value.strip_suffix(')'))
            .is_some()
        || function_arg_any(expr, &["exists", "has", "is", "missing", "not", "arch"]).is_some()
        || word_function_arg_any(expr, &["exists", "has", "missing", "arch"]).is_some()
        || word_prefix_arg(expr, "is").is_some()
        || word_prefix_arg(expr, "not").is_some()
}

fn condition_target_exists(
    target: &ConditionTarget,
    ctx: &ExpressionContext<'_>,
    command_probe: Option<&ConditionCommandProbe<'_>>,
) -> Result<bool> {
    match target {
        ConditionTarget::Generic(value) => target_exists(value, ctx.root, command_probe),
        ConditionTarget::Path(value) => Ok(path_target_exists(value, ctx.root)),
        ConditionTarget::File(value) => Ok(file_target_exists(value, ctx.root)),
        ConditionTarget::Directory(value) => Ok(directory_target_exists(value, ctx.root)),
        ConditionTarget::Env(name) => Ok(env_target_exists(name, ctx.env)),
        ConditionTarget::Command(value) => command_target_exists(value, ctx.root, command_probe),
    }
}

fn target_exists(
    name: &str,
    root: &Path,
    command_probe: Option<&ConditionCommandProbe<'_>>,
) -> Result<bool> {
    if name.is_empty() {
        return Ok(false);
    }

    let candidate = Path::new(name);
    if candidate.is_absolute() {
        return Ok(filesystem_entry_exists(candidate));
    }
    if candidate.components().count() > 1 || name.starts_with('.') {
        return Ok(filesystem_entry_exists(&root.join(candidate)));
    }

    if filesystem_entry_exists(&root.join(candidate)) {
        return Ok(true);
    }

    if let Some(command_probe) = command_probe {
        return command_probe(name);
    }

    Ok(executable_exists(name))
}

fn command_target_exists(
    name: &str,
    root: &Path,
    command_probe: Option<&ConditionCommandProbe<'_>>,
) -> Result<bool> {
    if name.is_empty() {
        return Ok(false);
    }

    let candidate = Path::new(name);
    if candidate.is_absolute() {
        return Ok(executable_file_exists(candidate));
    }
    if candidate.components().count() > 1 || name.starts_with('.') {
        return Ok(executable_file_exists(&root.join(candidate)));
    }

    if let Some(command_probe) = command_probe {
        return command_probe(name);
    }

    Ok(executable_exists(name))
}

fn path_target_exists(name: &str, root: &Path) -> bool {
    if name.is_empty() {
        return false;
    }

    let candidate = Path::new(name);
    if candidate.is_absolute() {
        filesystem_entry_exists(candidate)
    } else {
        filesystem_entry_exists(&root.join(candidate))
    }
}

fn file_target_exists(name: &str, root: &Path) -> bool {
    path_target_has_kind(name, root, |path| {
        fs::metadata(path)
            .map(|meta| meta.is_file())
            .unwrap_or(false)
    })
}

fn directory_target_exists(name: &str, root: &Path) -> bool {
    path_target_has_kind(name, root, |path| {
        fs::metadata(path)
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
    })
}

fn path_target_has_kind(name: &str, root: &Path, predicate: impl FnOnce(&Path) -> bool) -> bool {
    if name.is_empty() {
        return false;
    }

    let candidate = Path::new(name);
    if candidate.is_absolute() {
        predicate(candidate)
    } else {
        predicate(&root.join(candidate))
    }
}

fn env_target_exists(name: &str, env: &BTreeMap<String, String>) -> bool {
    if name.is_empty() {
        return false;
    }

    env.get(name)
        .map(|value| !value.is_empty())
        .or_else(|| std::env::var_os(name).map(|value| !value.is_empty()))
        .unwrap_or(false)
}

fn filesystem_entry_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn executable_file_exists(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn parse_condition_list(value: &str) -> Vec<String> {
    value
        .split(['\n', ','])
        .map(str::trim)
        .map(|item| item.strip_prefix("- ").unwrap_or(item).trim())
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
