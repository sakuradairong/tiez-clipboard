//! Bounded, read-only draft preview. Never reads settings/history or touches the OS clipboard.
use crate::{CoreError, CoreResult};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;

const MAX_INPUT: usize = 16 * 1024;
const MAX_RULE_BYTES: usize = 16 * 1024;
const MAX_RULES: usize = 32;
const MAX_OUTPUT: usize = 64 * 1024;
const MAX_MATCHES: usize = 4096;

#[derive(Serialize)]
pub(crate) struct CleanupPreview {
    text: String,
    ignored: bool,
    rules: usize,
    matches: usize,
}
fn limit() -> CoreError {
    CoreError::new(
        "preview_limit",
        "Preview exceeds its sample, rules, matches or output limit",
    )
}
fn append(output: &mut String, value: &str) -> CoreResult<()> {
    if value.len() > MAX_OUTPUT - output.len() {
        return Err(limit());
    }
    output.push_str(value);
    Ok(())
}
fn replace(
    input: &str,
    regex: &Regex,
    replacement: &str,
    matches: &mut usize,
) -> CoreResult<String> {
    let names: HashMap<_, _> = regex
        .capture_names()
        .enumerate()
        .filter_map(|(index, name)| name.map(|name| (name, index)))
        .collect();
    let mut output = String::new();
    let mut last = 0;
    for captures in regex.captures_iter(input) {
        *matches += 1;
        if *matches > MAX_MATCHES {
            return Err(limit());
        }
        let matched = captures.get(0).expect("full match");
        append(&mut output, &input[last..matched.start()])?;
        // Use the regex engine's interpolation parser to count capture references first.
        // Its scratch output contains only literal bytes, bounded by replacement.len().
        let mut literals = String::new();
        let mut capture_bytes = 0usize;
        regex_automata::util::interpolate::string(
            replacement,
            |index, _| {
                if let Some(value) = captures.get(index) {
                    capture_bytes = capture_bytes.saturating_add(value.len());
                }
            },
            |name| names.get(name).copied(),
            &mut literals,
        );
        let expanded = capture_bytes.saturating_add(literals.len());
        if expanded > MAX_OUTPUT - output.len() {
            return Err(limit());
        }
        captures.expand(replacement, &mut output);
        last = matched.end();
    }
    append(&mut output, &input[last..])?;
    Ok(output)
}
pub(crate) fn preview(raw_rules: &str, sample: &str) -> CoreResult<CleanupPreview> {
    if raw_rules.len() > MAX_RULE_BYTES || sample.len() > MAX_INPUT {
        return Err(limit());
    }
    let count = raw_rules
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .count();
    if count > MAX_RULES {
        return Err(limit());
    }
    let rules = tiez_content::utils::parse_cleanup_rules(raw_rules);
    // The capture parser deliberately ignores malformed lines; an editor must report them.
    if rules.len() != count {
        return Err(CoreError::new(
            "invalid_rule",
            "A cleanup rule has an invalid regex or is missing =>",
        ));
    }
    let mut text = sample.replace("\r\n", "\n").replace('\r', "\n");
    let mut matches = 0;
    for (regex, replacement) in &rules {
        text = replace(&text, regex, replacement, &mut matches)?;
    }
    // Capture checks this after the entire rule group, before normalizing replacement CRs.
    let ignored = !rules.is_empty() && (text.trim().is_empty() || text == "__IGNORE_CAPTURE__");
    text = text.replace("\r\n", "\n").replace('\r', "\n");
    Ok(CleanupPreview {
        text,
        ignored,
        rules: rules.len(),
        matches,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn follows_shared_parser_replacements_and_capture_interpolation() {
        for rules in [
            r"(?P<word>foo) => ${word}:$0:$$:$missing:$1x:${1}x",
            r"(中)(文)? => $2$1",
            r"x => \n\tend\r",
            "# comment\n foo => bar\nbar => result",
            r"^ => prefix",
            r"$ => suffix",
            r" => invalid",
        ] {
            if rules.starts_with(' ') {
                assert!(preview(rules, "foo").is_err());
                continue;
            }
            let sample = "  foo 中文 x\r\n  ";
            let normalized = sample.replace("\r\n", "\n");
            let expected = tiez_content::utils::apply_cleanup_rules(
                &normalized,
                &tiez_content::utils::parse_cleanup_rules(rules),
            )
            .replace("\r\n", "\n")
            .replace('\r', "\n");
            assert_eq!(preview(rules, sample).unwrap().text, expected);
        }
    }
    #[test]
    fn distinguishes_no_rules_empty_results_and_ignore_sentinel() {
        assert!(!preview("# only comments", " \r\n ").unwrap().ignored);
        assert!(preview("x =>", "x").unwrap().ignored);
        assert!(preview("x => __IGNORE_CAPTURE__", "x").unwrap().ignored);
        assert!(
            !preview("x => __IGNORE_CAPTURE__\n__IGNORE_CAPTURE__ => kept", "x")
                .unwrap()
                .ignored
        );
        let result = preview("foo => bar\nbar => baz", " foo ").unwrap();
        assert_eq!(
            (result.text.as_str(), result.rules, result.matches),
            (" baz ", 2, 2)
        );
    }
    #[test]
    fn rejects_invalid_rules_instead_of_showing_partial_results() {
        for rules in [
            "x => y\n[ => bad",
            "missing separator",
            " => blank pattern",
            "(?=x) => y",
        ] {
            assert_eq!(preview(rules, "x").err().unwrap().code, "invalid_rule");
        }
    }
    #[test]
    fn bounds_input_rule_count_matches_and_amplification() {
        for (rules, sample) in [
            ("x => y".into(), "x".repeat(MAX_INPUT + 1)),
            ("#".repeat(MAX_RULE_BYTES + 1), "x".into()),
            ("x => y\n".repeat(MAX_RULES + 1), "x".into()),
            ("x => y".into(), "x".repeat(MAX_MATCHES + 1)),
            (format!("(.*) => {}", "$1".repeat(200)), "x".repeat(1024)),
            (format!("x => {}", "中".repeat(1000)), "x".repeat(100)),
        ] {
            assert_eq!(
                preview(&rules, &sample).err().unwrap().code,
                "preview_limit"
            );
        }
        assert_eq!(
            preview("x => xxx", &"x".repeat(4096)).unwrap().text.len(),
            12288
        );
    }
}
