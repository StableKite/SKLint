use super::*;
use crate::analyzer::{analyze, AnalysisInput};
use crate::config::VscodeConfig;
use std::path::PathBuf;

fn diagnostics(source: &str, code: &str) -> Vec<Diagnostic> {
    analyze(AnalysisInput {
        path: PathBuf::from("evaluation.py"),
        source: source.to_string(),
        vscode_config: VscodeConfig {
            strict: Some(true),
            ..VscodeConfig::default()
        },
    })
    .diagnostics
    .into_iter()
    .filter(|d| d.code == code)
    .collect()
}

#[test]
fn sk801_deadline_is_not_recomputed_by_a_while_test() {
    let source = "def poll():\n    deadline = monotonic() + HCI_COMMAND_TIMEOUT\n    while (remaining := deadline - monotonic()) > 0:\n        poll_socket(remaining)\n";
    assert!(diagnostics(source, "SK801").is_empty());
}

#[test]
fn sk801_repeated_and_deferred_expression_regions_keep_the_binding() {
    for expression in [
        "[value for _ in range(3)]",
        "{value for _ in range(3)}",
        "{value: i for i in range(3)}",
        "{i: value for i in range(3)}",
        "[i for i in range(3) if value]",
        "[j for i in range(3) for j in value]",
        "(value for _ in range(3))",
        "lambda: value",
        "consume(lambda: value)",
        "(lambda: (yield value))()",
        "[j for _ in range(3) for j in [i for i in value]]",
        "[i for _ in range(3) for i in (lambda cached=value: cached)()]",
    ] {
        let source = format!("def make():\n    value = produce()\n    return {expression}\n");
        assert!(diagnostics(&source, "SK801").is_empty(), "{expression}");
    }
}

#[test]
fn sk801_once_only_outer_iterables_defaults_and_calls_still_inline() {
    for expression in [
        "[i for i in value]",
        "{i for i in value}",
        "{i: i for i in value}",
        "(i for i in value)",
        "lambda cached=value: cached",
        "consume(value)",
        "(lambda: value)()",
        "[i for i in (lambda: value)()]",
    ] {
        let source = format!("def make():\n    value = produce()\n    return {expression}\n");
        let reports = diagnostics(&source, "SK801");
        assert_eq!(reports.len(), 1, "{expression}");
        assert!(
            reports[0].fix.as_ref().is_some_and(|fix| fix.safe),
            "{expression}"
        );
    }
}

#[test]
fn sk801_assignment_and_use_in_the_same_iteration_still_inline() {
    for source in [
        "def make():\n    for item in items:\n        value = produce(item)\n        return consume(value)\n",
        "def make():\n    while keep_going():\n        value = produce()\n        return consume(value)\n",
        "def make():\n    value = produce()\n    def callback(cached=value):\n        return cached\n",
        "def make():\n    value = produce()\n    for item in (items := value):\n        consume(item)\n",
    ] {
        assert_eq!(diagnostics(source, "SK801").len(), 1, "{source}");
    }
}

#[test]
fn sk802_groups_complete_tuple_returns_in_both_branches() {
    let source =
        "def choose(flag):\n    if flag:\n        return 1, 2, 3, 4\n    return 5, 6, 7, 8\n";
    let reports = diagnostics(source, "SK802");
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].fix.as_ref().unwrap().replacement,
        "    return (1, 2, 3, 4) if flag else (5, 6, 7, 8)"
    );
    assert!(reports[0].fix.as_ref().unwrap().safe);
}

#[test]
fn sk802_none_and_tuple_results_do_not_become_nested_tuples() {
    for (a, b, expected) in [
        (
            "None",
            "width, height",
            "    return None if flag else (width, height)",
        ),
        (
            "samples, gain",
            "read()",
            "    return (samples, gain) if flag else read()",
        ),
        (
            "samples, gain",
            "None",
            "    return (samples, gain) if flag else None",
        ),
        (
            "*items, 1",
            "*others, 2",
            "    return (*items, 1) if flag else (*others, 2)",
        ),
    ] {
        let source =
            format!("def choose(flag):\n    if flag:\n        return {a}\n    return {b}\n");
        let reports = diagnostics(&source, "SK802");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].fix.as_ref().unwrap().replacement, expected);
    }
}

#[test]
fn sk802_multiline_and_nested_conditional_expressions_have_safe_fixes() {
    for source in [
        "def choose(flag):\n    if flag:\n        return first(), second()\n    return (\n        third(),\n        fourth(),\n    )\n",
        "def choose(flag):\n    if (flag if outer else other):\n        return first if inner else second\n    return third if alternate else fourth\n",
        "def choose(flag):\n    if flag:\n        return lambda: first()\n    return lambda: second()\n",
        "def choose(flag):\n    if flag:\n        return 'literal#not-comment'\n    return 'other'\n",
    ] {
        let reports = diagnostics(source, "SK802");
        assert_eq!(reports.len(), 1, "{source}");
        assert!(reports[0].fix.as_ref().is_some_and(|fix| fix.safe), "{source}");
    }
}

#[test]
fn sk802_preserves_comments_and_diagnostics_without_a_lossy_fix() {
    for source in [
        "def choose(flag):\n    if flag:\n        return first()  # type: ignore[return-value]\n    return second()\n",
        "def choose(flag):\n    if flag:\n        return first()\n    return second()  # explain fallback\n",
    ] {
        let reports = diagnostics(source, "SK802");
        assert_eq!(reports.len(), 1, "{source}");
        assert!(reports[0].fix.is_none(), "Comment must remain in place: {source}");
    }
}

#[test]
fn sk802_collapses_a_complete_terminal_chain_in_one_fix() {
    let source = "def choose(flag):\n    if flag == 1:\n        return 1, 2\n    if flag == 2:\n        return 3, 4\n    if flag == 3:\n        return 5, 6\n    return 7, 8\n";
    let reports = diagnostics(source, "SK802");
    assert_eq!(reports.len(), 1);
    let fix = reports[0].fix.as_ref().unwrap();
    assert!(fix.safe);
    assert_eq!(reports[0].line, 6);
    assert_eq!(fix.start_line, 2);
    assert_eq!(fix.end_line, 8);
    assert_eq!(fix.replacement, "    return (1, 2) if flag == 1 else (3, 4) if flag == 2 else (5, 6) if flag == 3 else (7, 8)");
}

#[test]
fn sk802_does_not_expand_the_original_diagnostic_eligibility() {
    for source in [
        "def choose(flag):\n    if flag:\n        return first()\n\n    return second()\n",
        "def choose(flag):\n    if flag:\n        return (\n            first(),\n            second(),\n        )\n    return third()\n",
        "def choose(flag):\n    if (\n        flag\n    ):\n        return first()\n    return second()\n",
        "def choose(flag):\n    if flag:\n        return first()\n    # explain fallback\n    return second()\n",
    ] {
        assert!(diagnostics(source, "SK802").is_empty(), "{source}");
    }
}

#[test]
fn sk801_keeps_safe_immutable_constants_in_repeated_regions() {
    for value in [
        "1", "None", "'value'", "b'value'", "(1, 2)", "-1", "~1", "not None",
    ] {
        for expression in [
            "[value for _ in range(3)]",
            "lambda: value",
            "(value for _ in range(3))",
        ] {
            let source = format!("def make():\n    value = {value}\n    return {expression}\n");
            assert_eq!(diagnostics(&source, "SK801").len(), 1, "{source}");
        }
    }
    for value in ["[]", "{}", "set()", "([],)"] {
        let source =
            format!("def make():\n    value = {value}\n    return [value for _ in range(3)]\n");
        assert!(diagnostics(&source, "SK801").is_empty(), "{source}");
    }
}

#[test]
fn sk801_class_names_are_not_captured_as_function_locals() {
    for expression in [
        "lambda: value",
        "(lambda: value)()",
        "[value for _ in range(3)]",
    ] {
        let source = format!("class Owner:\n    value = 1\n    callback = {expression}\n");
        assert!(diagnostics(&source, "SK801").is_empty(), "{source}");
    }
    for expression in ["lambda cached=value: cached", "[i for i in value]"] {
        let source = format!("class Owner:\n    value = (1, 2)\n    callback = {expression}\n");
        assert_eq!(diagnostics(&source, "SK801").len(), 1, "{source}");
    }
}
