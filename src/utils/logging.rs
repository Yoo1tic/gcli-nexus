use serde::Serialize;

pub(crate) fn with_pretty_json_debug<T, F>(value: &T, log_action: F)
where
    T: Serialize,
    F: FnOnce(&str),
{
    if !tracing::enabled!(tracing::Level::DEBUG) {
        return;
    }

    let pretty_json = serde_json::to_string_pretty(value)
        .unwrap_or_else(|error| format!("<pretty serialize failed: {error}>"));
    log_action(pretty_json.as_str());
}
