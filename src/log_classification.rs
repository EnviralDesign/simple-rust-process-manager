pub(crate) fn contains_error_indicator(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();

    if ["critical", "fatal", "panic", "traceback", "exception"]
        .iter()
        .any(|keyword| contains_token(&lower, keyword))
    {
        return true;
    }

    contains_singular_error(&lower) || contains_plural_errors_indicator(&lower)
}

fn contains_singular_error(lower: &str) -> bool {
    let mut search_start = 0;
    while let Some(relative) = lower[search_start..].find("error") {
        let start = search_start + relative;
        let end = start + "error".len();
        if is_token_start(lower, start)
            && lower[end..].starts_with("_count")
            && !plural_errors_is_significant(&lower[end + "_count".len()..])
        {
            search_start = end;
            continue;
        }
        if is_token_start(lower, start) && is_token_end(lower, end) {
            return true;
        }
        search_start = end;
    }
    false
}

fn contains_plural_errors_indicator(lower: &str) -> bool {
    let mut search_start = 0;
    while let Some(relative) = lower[search_start..].find("errors") {
        let start = search_start + relative;
        let end = start + "errors".len();
        if is_token_start(lower, start)
            && is_token_end(lower, end)
            && plural_errors_is_significant(&lower[end..])
        {
            return true;
        }
        search_start = end;
    }
    false
}

fn plural_errors_is_significant(rest: &str) -> bool {
    let after_name = rest.trim_start_matches(|c: char| c == '\'' || c == '"' || c.is_whitespace());
    let Some(after_separator) = after_name
        .strip_prefix(':')
        .or_else(|| after_name.strip_prefix('='))
    else {
        return true;
    };

    let value = after_separator.trim_start();
    !(value.starts_with("[]")
        || value.starts_with("{}")
        || value.starts_with('0')
        || value.starts_with("false")
        || value.starts_with("none")
        || value.starts_with("null"))
}

fn contains_token(lower: &str, token: &str) -> bool {
    let mut search_start = 0;
    while let Some(relative) = lower[search_start..].find(token) {
        let start = search_start + relative;
        let end = start + token.len();
        if is_token_start(lower, start) && is_token_end(lower, end) {
            return true;
        }
        search_start = end;
    }
    false
}

fn is_token_start(value: &str, index: usize) -> bool {
    value[..index]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_ascii_alphanumeric())
}

fn is_token_end(value: &str, index: usize) -> bool {
    value[index..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::contains_error_indicator;

    #[test]
    fn ignores_empty_error_aggregate_fields() {
        let line = "[stderr] INFO worker: report: {'dry_run': False, 'errors': []}";
        assert!(!contains_error_indicator(line));
    }

    #[test]
    fn ignores_zero_error_aggregate_fields() {
        assert!(!contains_error_indicator("INFO report errors=0"));
        assert!(!contains_error_indicator("INFO report error_count: 0"));
    }

    #[test]
    fn detects_real_error_indicators() {
        assert!(contains_error_indicator("ERROR failed to bind port"));
        assert!(contains_error_indicator("worker fatal exception"));
        assert!(contains_error_indicator("report errors: ['failed row']"));
    }
}
