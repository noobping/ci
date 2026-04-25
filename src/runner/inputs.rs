use std::collections::BTreeMap;

pub(crate) fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub(crate) fn input_value<'a>(
    values: &'a BTreeMap<String, String>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| values.get(*key).map(String::as_str))
}

pub(crate) fn input_bool(values: &BTreeMap<String, String>, keys: &[&str], default: bool) -> bool {
    input_value(values, keys).map(parse_bool).unwrap_or(default)
}

pub(crate) fn parse_path_list(value: &str) -> Vec<String> {
    value
        .split(['\n', ','])
        .map(str::trim)
        .map(|item| item.strip_prefix("- ").unwrap_or(item).trim())
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
