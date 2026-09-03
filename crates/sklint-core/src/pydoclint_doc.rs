//! Structured docstring parsing for the pydoclint-compatible rule family.
//!
//! Python structure comes from `rustpython-parser`; this module parses only the
//! NumPy/Google/Sphinx mini-languages inside an already identified docstring.
//! The parser deliberately keeps type text as text: Python syntax ownership
//! stays in the shared Rust AST layer.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocStyle {
    Numpy,
    Google,
    Sphinx,
}

impl DocStyle {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "numpy" => Some(Self::Numpy),
            "google" => Some(Self::Google),
            "sphinx" => Some(Self::Sphinx),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocItem {
    pub name: String,
    pub ty: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedDocstring {
    pub params: Vec<DocItem>,
    pub attrs: Vec<DocItem>,
    pub returns: Vec<String>,
    pub yields: Vec<String>,
    pub raises: Vec<String>,
    pub has_args_section: bool,
    pub has_attributes_section: bool,
    pub has_returns_section: bool,
    pub has_yields_section: bool,
    pub has_raises_section: bool,
    pub is_short: bool,
    pub parse_error: Option<String>,
}

/// Parse a docstring in one explicitly configured style.
pub fn parse_docstring(docstring: &str, style: DocStyle) -> ParsedDocstring {
    let lines = normalized_doc_lines(docstring);
    parse_docstring_lines(&lines, style)
}

/// Parse in the style selected by pydoclint-style detection and report whether
/// that detected style differs from the user's configured style.
///
/// Detection is intentionally narrower than parser acceptance. For example,
/// the Google parser accepts `Parameters:`, while upstream style detection does
/// not use it as a Google signal. Keeping those concepts separate prevents
/// false DOC003/SKD003 cascades.
pub fn parse_docstring_with_style_detection(
    docstring: &str,
    configured_style: DocStyle,
) -> (ParsedDocstring, bool) {
    let lines = normalized_doc_lines(docstring);
    let (numpy, google, sphinx) = style_signals(&lines);

    let detected = if numpy > 0 {
        // An underlined NumPy header is the strongest signal used by upstream.
        Some(DocStyle::Numpy)
    } else {
        match (google > 0, sphinx > 0) {
            (true, false) => Some(DocStyle::Google),
            (false, true) => Some(DocStyle::Sphinx),
            _ => None,
        }
    };

    match detected {
        Some(style) => {
            let parsed = parse_docstring_lines(&lines, style);
            let mismatch = style != configured_style
                || (style == DocStyle::Google && parsed.parse_error.is_some());
            (parsed, mismatch)
        }
        None if google > 0 && sphinx > 0 => {
            // Mixed non-NumPy signals are suspicious, but there is no reliable
            // alternate parser to prefer. Keep configured parsing and only
            // surface the style mismatch.
            (parse_docstring_lines(&lines, configured_style), true)
        }
        None => (parse_docstring_lines(&lines, configured_style), false),
    }
}

/// Return a uniquely detectable style, if any.
///
/// Kept as a small public helper for callers/tests; the semantic visitor should
/// prefer `parse_docstring_with_style_detection` so detection and parsing are
/// atomic.
pub fn likely_style(docstring: &str) -> Option<DocStyle> {
    let lines = normalized_doc_lines(docstring);
    let (numpy, google, sphinx) = style_signals(&lines);
    if numpy > 0 {
        return Some(DocStyle::Numpy);
    }
    match (google > 0, sphinx > 0) {
        (true, false) => Some(DocStyle::Google),
        (false, true) => Some(DocStyle::Sphinx),
        _ => None,
    }
}

fn parse_docstring_lines(lines: &[String], style: DocStyle) -> ParsedDocstring {
    let non_empty = lines.iter().filter(|line| !line.trim().is_empty()).count();
    let mut parsed = match style {
        DocStyle::Numpy => parse_numpy(lines),
        DocStyle::Google => parse_google(lines),
        DocStyle::Sphinx => parse_sphinx(lines),
    };
    parsed.is_short = non_empty <= 1
        || !(has_any_structured_section(&parsed) || has_nonshort_aux_meta(lines, style));
    parsed
}

fn style_signals(lines: &[String]) -> (usize, usize, usize) {
    let mut numpy = 0usize;
    let mut google = 0usize;
    let mut sphinx = 0usize;

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if lines
            .get(index + 1)
            .is_some_and(|next| is_numpy_underline(next.trim()))
            && numpy_detection_heading(trimmed)
        {
            numpy += 1;
        }

        // Google and Sphinx signals count only at the docstring's base
        // indentation, matching upstream `_detectDocstringIndent` semantics.
        if indentation(line) != 0 {
            continue;
        }

        if [
            "Args:",
            "Returns:",
            "Yields:",
            "Raises:",
            "Examples:",
            "Notes:",
        ]
        .iter()
        .any(|keyword| trimmed.starts_with(keyword))
        {
            google += 1;
        }

        if trimmed.starts_with(":param ")
            || trimmed.starts_with(":type ")
            || trimmed.starts_with(":raises ")
            || trimmed.starts_with(":return:")
            || trimmed.starts_with(":rtype:")
            || trimmed.starts_with(":yield:")
            || trimmed.starts_with(":ytype:")
        {
            sphinx += 1;
        }
    }

    (numpy, google, sphinx)
}

fn numpy_detection_heading(text: &str) -> bool {
    let heading = text
        .strip_suffix(':')
        .unwrap_or(text)
        .trim()
        .to_ascii_lowercase();
    matches!(
        heading.as_str(),
        "arg"
            | "args"
            | "argument"
            | "arguments"
            | "parameter"
            | "parameters"
            | "param"
            | "return"
            | "returns"
            | "yield"
            | "yields"
            | "raise"
            | "raises"
            | "example"
            | "examples"
            | "note"
            | "notes"
            | "see also"
            | "reference"
            | "references"
    )
}

/// Normalize a type expression written in documentation.
///
/// A single physical-line trailing `\\` is discarded before whitespace
/// normalization, matching Markdown's explicit line-break spelling. Embedded
/// backslashes are preserved, and quote spelling is normalized so single and
/// double quoted literals compare equivalently.
pub fn normalize_type_text(text: &str) -> String {
    let mut joined = String::new();
    for raw_line in text.lines() {
        let mut line = strip_type_comment(raw_line).trim_end().to_string();
        if physical_line_continues(&line) {
            line.pop();
            line = line.trim_end().to_string();
        }
        if !joined.is_empty() && !line.trim().is_empty() {
            joined.push(' ');
        }
        joined.push_str(line.trim());
    }

    let mut out = String::with_capacity(joined.len());
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in joined.chars() {
        if let Some(active) = quote {
            if escaped {
                out.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                out.push(ch);
                escaped = true;
                continue;
            }
            if ch == active {
                out.push('\'');
                quote = None;
            } else {
                out.push(ch);
            }
            continue;
        }

        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                out.push('\'');
            }
            ch if ch.is_whitespace() => {}
            _ => out.push(ch),
        }
    }
    out.trim_matches('`').to_string()
}

fn parse_numpy(lines: &[String]) -> ParsedDocstring {
    let mut out = ParsedDocstring::default();

    let unsupported = unsupported_numpy_sections(lines);
    if unsupported.len() == 1 {
        out.parse_error = Some(format!(
            "Unsupported numpy docstring section: \"{}\"",
            unsupported[0]
        ));
        return out;
    }
    if unsupported.len() > 1 {
        out.parse_error = Some(format!(
            "Unsupported numpy docstring sections: {}",
            unsupported
                .iter()
                .map(|section| format!("\"{section}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        return out;
    }

    let mut index = 0usize;
    while index < lines.len() {
        if indentation(&lines[index]) != 0 {
            index += 1;
            continue;
        }
        let heading = lines[index].trim();
        let Some(kind) = numpy_section_kind(heading) else {
            index += 1;
            continue;
        };
        if !lines.get(index + 1).is_some_and(|line| {
            indentation(line) == 0 && is_numpy_section_underline(heading, line.trim())
        }) {
            index += 1;
            continue;
        }

        set_section_flag(&mut out, kind);
        let start = index + 2;
        let end = find_next_numpy_section(lines, start);
        let body_non_empty = lines[start..end].iter().any(|line| !line.trim().is_empty());

        let parsed_count = match kind {
            SectionKind::Args => match parse_numpy_named_items(lines, start, end, false) {
                Ok(items) => {
                    let count = items.len();
                    out.params.extend(items);
                    count
                }
                Err(error) => {
                    out.parse_error = Some(error);
                    return out;
                }
            },
            SectionKind::Attributes => match parse_numpy_named_items(lines, start, end, true) {
                Ok(items) => {
                    let count = items.len();
                    out.attrs.extend(items);
                    count
                }
                Err(error) => {
                    out.parse_error = Some(error);
                    return out;
                }
            },
            SectionKind::Returns => {
                let items = parse_numpy_type_items(lines, start, end);
                let count = items.len();
                out.returns.extend(items);
                count
            }
            SectionKind::Yields => {
                let items = parse_numpy_type_items(lines, start, end);
                let count = items.len();
                out.yields.extend(items);
                count
            }
            SectionKind::Raises => {
                let items = parse_numpy_raises(lines, start, end);
                let count = items.len();
                out.raises.extend(items);
                count
            }
        };

        if body_non_empty && parsed_count == 0 {
            out.parse_error = Some(format!(
                "Section '{heading}' is not empty but nothing was parsed."
            ));
            return out;
        }
        index = end;
    }
    out
}

fn unsupported_numpy_sections(lines: &[String]) -> Vec<String> {
    let mut unsupported = Vec::new();
    let mut index = 0usize;
    while index + 1 < lines.len() {
        let heading = lines[index].trim();
        if !heading.is_empty()
            && !heading.ends_with(':')
            && is_numpy_underline(lines[index + 1].trim())
            && !is_allowed_numpy_section(heading)
            && !unsupported.iter().any(|existing| existing == heading)
        {
            unsupported.push(heading.to_string());
        }
        index += 1;
    }
    unsupported
}

fn is_allowed_numpy_section(text: &str) -> bool {
    matches!(
        text,
        "Parameters"
            | "Params"
            | "Arguments"
            | "Args"
            | "Other Parameters"
            | "Other Params"
            | "Other Arguments"
            | "Other Args"
            | "Receives"
            | "Receive"
            | "Raises"
            | "Raise"
            | "Warns"
            | "Warn"
            | "Attributes"
            | "Attribute"
            | "Returns"
            | "Return"
            | "Yields"
            | "Yield"
            | "Examples"
            | "Example"
            | "Warnings"
            | "Warning"
            | "See Also"
            | "Related"
            | "Notes"
            | "Note"
            | "References"
            | "Reference"
            | "deprecated"
    )
}

fn parse_numpy_named_items(
    lines: &[String],
    start: usize,
    end: usize,
    attribute: bool,
) -> Result<Vec<DocItem>, String> {
    let mut items = Vec::new();
    let mut index = start;
    while index < end {
        let line = &lines[index];
        if line.trim().is_empty() || indentation(line) > 0 {
            index += 1;
            continue;
        }

        let trimmed = line.trim();
        let (names, ty, consumed) = if let Some(colon) = find_top_level_colon(trimmed) {
            let names = trimmed[..colon].trim();
            let initial = trimmed[colon + 1..].trim();
            let (ty, consumed) = collect_type_continuation(lines, index, end, initial, 0);
            (names, ty, consumed)
        } else {
            // NumPy parameters are allowed to omit a type entirely.
            (trimmed, String::new(), index)
        };

        if names.is_empty() {
            return Err(if attribute {
                "Parsed docstring attribute has an empty name".to_string()
            } else {
                "Parsed docstring parameter has an empty name".to_string()
            });
        }

        let description = collect_item_description(lines, consumed + 1, end, 0, "");
        items.push(DocItem {
            name: normalize_doc_name(names),
            ty,
            description,
        });
        index = consumed + 1;
    }
    Ok(items)
}

fn parse_numpy_type_items(lines: &[String], start: usize, end: usize) -> Vec<String> {
    let mut items = Vec::new();
    let mut index = start;
    while index < end {
        let line = &lines[index];
        let trimmed = line.trim();
        if trimmed.is_empty() || indentation(line) > 0 {
            index += 1;
            continue;
        }

        let initial = find_top_level_colon(trimmed)
            .map(|colon| trimmed[colon + 1..].trim())
            .unwrap_or(trimmed);
        let (ty, consumed) = collect_type_continuation(lines, index, end, initial, 0);
        // The NumPy fork treats `name :` as a valid return/yield item whose
        // documented type is empty. Keeping the empty item is what lets DOC203
        // report a type mismatch instead of turning the section into DOC001.
        items.push(ty);
        index = consumed + 1;
    }
    items
}

fn parse_numpy_raises(lines: &[String], start: usize, end: usize) -> Vec<String> {
    let mut items = Vec::new();
    for line in &lines[start..end] {
        if indentation(line) != 0 {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        items.push(trimmed.to_string());
    }
    items
}

fn parse_google(lines: &[String]) -> ParsedDocstring {
    let mut out = ParsedDocstring::default();
    let mut index = 0usize;

    while index < lines.len() {
        if indentation(&lines[index]) != 0 {
            index += 1;
            continue;
        }
        let Some(kind) = google_section_kind(lines[index].trim()) else {
            index += 1;
            continue;
        };
        set_section_flag(&mut out, kind);
        let heading_indent = indentation(&lines[index]);
        let start = index + 1;
        let end = find_next_google_section(lines, start, heading_indent);

        let result = match kind {
            SectionKind::Args => parse_google_named_items(lines, start, end, false)
                .map(|items| out.params.extend(items)),
            SectionKind::Attributes => parse_google_named_items(lines, start, end, true)
                .map(|items| out.attrs.extend(items)),
            SectionKind::Returns => parse_google_return_or_yield(lines, start, end)
                .map(|items| out.returns.extend(items)),
            SectionKind::Yields => parse_google_return_or_yield(lines, start, end)
                .map(|items| out.yields.extend(items)),
            SectionKind::Raises => {
                parse_google_raises(lines, start, end).map(|items| out.raises.extend(items))
            }
        };

        if let Err(error) = result {
            out.parse_error = Some(error);
            return out;
        }
        index = end;
    }
    out
}

fn parse_google_named_items(
    lines: &[String],
    start: usize,
    end: usize,
    attribute: bool,
) -> Result<Vec<DocItem>, String> {
    let Some(item_indent) = section_item_indent(lines, start, end) else {
        return Ok(Vec::new());
    };
    let mut items = Vec::new();
    let mut index = start;
    while index < end {
        if lines[index].trim().is_empty() || indentation(&lines[index]) != item_indent {
            index += 1;
            continue;
        }

        let original = lines[index].trim();
        let Some((declaration, consumed)) = collect_until_top_level_colon(lines, index, end) else {
            return Err(format!("Expected a colon in '{original}'."));
        };
        let Some(colon) = find_top_level_colon(&declaration) else {
            return Err(format!("Expected a colon in '{original}'."));
        };
        let head = declaration[..colon].trim();
        let inline_description = declaration[colon + 1..].trim();
        let (name, ty) = split_google_name_and_type(head);
        if name.is_empty() {
            return Err(if attribute {
                "Parsed docstring attribute has an empty name".to_string()
            } else {
                "Parsed docstring parameter has an empty name".to_string()
            });
        }
        let description =
            collect_item_description(lines, consumed + 1, end, item_indent, inline_description);
        items.push(DocItem {
            name,
            ty,
            description,
        });
        index = consumed + 1;
    }
    Ok(items)
}

pub(crate) fn split_google_name_and_type(head: &str) -> (String, String) {
    let head = head.trim();
    if !head.ends_with(')') {
        return (normalize_doc_name(head), String::new());
    }

    // Find the opening parenthesis of the final type group. The type itself may
    // contain nested brackets and parentheses.
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut open = None;
    for (index, ch) in head.char_indices().rev() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    open = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }

    let Some(open) = open else {
        return (normalize_doc_name(head), String::new());
    };
    let inner = head[open + 1..head.len() - 1].trim();
    if inner.is_empty() {
        // GOOGLE_TYPED_ARG_REGEX requires at least one non-whitespace type
        // character; empty parentheses remain part of the argument name.
        return (normalize_doc_name(head), String::new());
    }
    let name = normalize_doc_name(head[..open].trim());
    let mut ty = inner.to_string();
    if let Some(stripped) = ty.strip_suffix(", optional") {
        ty = stripped.to_string();
    } else if let Some(stripped) = ty.strip_suffix('?') {
        ty = stripped.to_string();
    }
    (name, ty)
}

fn parse_google_return_or_yield(
    lines: &[String],
    start: usize,
    end: usize,
) -> Result<Vec<String>, String> {
    let Some(item_indent) = section_item_indent(lines, start, end) else {
        return Ok(Vec::new());
    };
    let Some(first_index) = (start..end).find(|index| {
        !lines[*index].trim().is_empty() && indentation(&lines[*index]) == item_indent
    }) else {
        return Ok(Vec::new());
    };

    // docstring_parser_fork treats Google Returns/Yields as
    // SINGULAR_OR_MULTIPLE but builds exactly one meta item from the complete
    // section chunk. No colon anywhere means a singular description with no
    // type. If a structural colon exists, the text before the first such colon
    // is the one documented type.
    let mut chunk = String::new();
    for line in &lines[first_index..end] {
        if !chunk.is_empty() {
            chunk.push('\n');
        }
        chunk.push_str(line.trim());
    }
    let Some(colon) = find_top_level_colon(&chunk) else {
        return Ok(vec![String::new()]);
    };
    let ty = chunk[..colon].trim();
    Ok(vec![ty.to_string()])
}

fn parse_google_raises(lines: &[String], start: usize, end: usize) -> Result<Vec<String>, String> {
    let Some(item_indent) = section_item_indent(lines, start, end) else {
        return Ok(Vec::new());
    };
    let mut items = Vec::new();
    for line in &lines[start..end] {
        if line.trim().is_empty() || indentation(line) != item_indent {
            continue;
        }
        let original = line.trim();
        let Some(colon) = find_top_level_colon(original) else {
            return Err(format!("Expected a colon in '{original}'."));
        };
        let exception = original[..colon].trim();
        if !exception.is_empty() {
            items.push(exception.to_string());
        }
    }
    Ok(items)
}

#[derive(Debug)]
pub(crate) struct SphinxField {
    pub(crate) key: String,
    pub(crate) args: Vec<String>,
    pub(crate) description: String,
}

fn parse_sphinx(lines: &[String]) -> ParsedDocstring {
    let mut out = ParsedDocstring::default();
    let mut param_types = HashMap::<String, String>::new();
    let mut param_items = Vec::<(String, Option<String>, String)>::new();
    let mut attr_items = Vec::<DocItem>::new();
    let mut return_items = Vec::<Option<String>>::new();
    let mut yield_items = Vec::<Option<String>>::new();
    let mut return_types = Vec::<(Option<String>, String)>::new();
    let mut yield_types = Vec::<(Option<String>, String)>::new();
    let mut index = 0usize;

    while index < lines.len() {
        let trimmed = lines[index].trim();

        if trimmed.starts_with(".. attribute ::") {
            let Some(name) = sphinx_attribute_directive_name(trimmed) else {
                out.parse_error = Some("Parsed docstring attribute has an empty name".to_string());
                return out;
            };
            out.has_attributes_section = true;
            let directive_indent = indentation(&lines[index]);
            let mut attr_type = String::new();
            let mut attr_description = Vec::new();
            let mut inner = index + 1;
            let mut previous_was_blank = false;
            while inner < lines.len() {
                let inner_trimmed = lines[inner].trim();
                if inner_trimmed.is_empty() {
                    if previous_was_blank {
                        break;
                    }
                    previous_was_blank = true;
                    inner += 1;
                    continue;
                }
                previous_was_blank = false;
                if indentation(&lines[inner]) <= directive_indent {
                    break;
                }
                if let Some(initial) = inner_trimmed.strip_prefix(":type:") {
                    let (ty, consumed) = collect_type_continuation(
                        lines,
                        inner,
                        lines.len(),
                        initial.trim(),
                        indentation(&lines[inner]),
                    );
                    attr_type = ty;
                    inner = consumed + 1;
                    continue;
                }
                attr_description.push(inner_trimmed.to_string());
                inner += 1;
            }
            attr_items.push(DocItem {
                name: normalize_doc_name(name),
                ty: attr_type,
                description: attr_description.join(" ").trim().to_string(),
            });
            index = inner.max(index + 1);
            continue;
        }

        // ReST field lists are recognized only at base indentation (`^:` in
        // docstring_parser_fork). Indented colon-prefixed text is description.
        if indentation(&lines[index]) != 0 {
            index += 1;
            continue;
        }

        let Some(field) = parse_sphinx_field(trimmed) else {
            if trimmed.starts_with(':') {
                out.parse_error = Some(format!(
                    "Error parsing meta information near \"{}\".",
                    trimmed.replace('\n', " ")
                ));
                return out;
            }
            index += 1;
            continue;
        };
        let key = field.key.as_str();

        if is_sphinx_param_key(key) {
            out.has_args_section = true;
            let description = collect_sphinx_field_description(lines, index, &field.description);
            match field.args.as_slice() {
                [name] => param_items.push((normalize_doc_name(name), None, description)),
                [type_name, name] => {
                    let type_name = type_name.strip_suffix('?').unwrap_or(type_name).to_string();
                    param_items.push((normalize_doc_name(name), Some(type_name), description));
                }
                _ => {
                    out.parse_error = Some(format!(
                        "Expected one or two arguments for a {key} keyword."
                    ));
                    return out;
                }
            }
        } else if key == "type" {
            if field.args.len() != 1 {
                out.parse_error = Some("Expected one argument for a type keyword.".to_string());
                return out;
            }
            let name = normalize_doc_name(&field.args[0]);
            let (ty, consumed) = collect_type_continuation(
                lines,
                index,
                lines.len(),
                field.description.trim(),
                indentation(&lines[index]),
            );
            // The fork stores `:type name:` separately and only uses it for
            // params whose inline type is absent. Repeated type fields use the
            // last value, matching Python dict assignment.
            param_types.insert(name, ty);
            index = consumed + 1;
            continue;
        } else if is_sphinx_return_key(key) {
            if field.args.len() > 1 {
                out.parse_error =
                    Some(format!("Expected one or no arguments for a {key} keyword."));
                return out;
            }
            out.has_returns_section = true;
            return_items.push(field.args.first().cloned());
        } else if key == "rtype" {
            if field.args.len() > 1 {
                out.parse_error =
                    Some("Expected one or no arguments for a rtype keyword.".to_string());
                return out;
            }
            let (ty, consumed) = collect_type_continuation(
                lines,
                index,
                lines.len(),
                field.description.trim(),
                indentation(&lines[index]),
            );
            upsert_named_type(&mut return_types, field.args.first().cloned(), ty);
            index = consumed + 1;
            continue;
        } else if is_sphinx_yield_key(key) {
            if field.args.len() > 1 {
                out.parse_error =
                    Some(format!("Expected one or no arguments for a {key} keyword."));
                return out;
            }
            out.has_yields_section = true;
            yield_items.push(field.args.first().cloned());
        } else if key == "ytype" {
            if field.args.len() > 1 {
                out.parse_error =
                    Some("Expected one or no arguments for a ytype keyword.".to_string());
                return out;
            }
            let (ty, consumed) = collect_type_continuation(
                lines,
                index,
                lines.len(),
                field.description.trim(),
                indentation(&lines[index]),
            );
            upsert_named_type(&mut yield_types, field.args.first().cloned(), ty);
            index = consumed + 1;
            continue;
        } else if is_sphinx_raises_key(key) {
            if field.args.len() > 1 {
                out.parse_error =
                    Some(format!("Expected one or no arguments for a {key} keyword."));
                return out;
            }
            out.has_raises_section = true;
            if let Some(exception) = field.args.first() {
                out.raises.push(exception.clone());
            } else if let Some((exception, _)) = field.description.split_once(':') {
                // pydoclint has a Sphinx-specific compatibility path for
                // ``:raises: ValueError: explanation``. The ReST parser
                // stores ValueError inside the description (type_name=None),
                // and Visitor.checkRaises recovers the leading token when it
                // contains no whitespace.
                let exception = exception.trim();
                if !exception.is_empty() && !exception.chars().any(char::is_whitespace) {
                    out.raises.push(exception.to_string());
                }
            }
        }

        index += 1;
    }

    out.params = param_items
        .into_iter()
        .map(|(name, inline_type, description)| DocItem {
            ty: inline_type
                .or_else(|| param_types.get(&name).cloned())
                .unwrap_or_default(),
            name,
            description,
        })
        .collect();
    out.attrs = attr_items;

    if return_items.is_empty() {
        // ReST uniquely synthesizes return metadata from rtype declarations.
        if !return_types.is_empty() {
            out.has_returns_section = true;
            out.returns
                .extend(return_types.iter().map(|(_, ty)| ty.clone()));
        }
    } else {
        let unnamed_rtype = named_type(&return_types, None);
        out.returns.extend(
            return_items.into_iter().map(|inline| {
                inline.unwrap_or_else(|| unnamed_rtype.unwrap_or_default().to_string())
            }),
        );
    }

    // ytype only enriches an existing yield meta item; it never creates one.
    if !yield_items.is_empty() {
        let unnamed_ytype = named_type(&yield_types, None);
        out.yields.extend(
            yield_items.into_iter().map(|inline| {
                inline.unwrap_or_else(|| unnamed_ytype.unwrap_or_default().to_string())
            }),
        );
    }

    out
}

fn upsert_named_type(items: &mut Vec<(Option<String>, String)>, name: Option<String>, ty: String) {
    if let Some((_, existing)) = items.iter_mut().find(|(key, _)| key == &name) {
        *existing = ty;
    } else {
        items.push((name, ty));
    }
}

fn named_type<'a>(items: &'a [(Option<String>, String)], name: Option<&str>) -> Option<&'a str> {
    items
        .iter()
        .find(|(key, _)| key.as_deref() == name)
        .map(|(_, ty)| ty.as_str())
}

pub(crate) fn parse_sphinx_field(trimmed: &str) -> Option<SphinxField> {
    let body = trimmed.strip_prefix(':')?;
    let (head, description) = body.split_once(':')?;
    let mut parts = head.split_whitespace();
    let key = parts.next()?.to_ascii_lowercase();
    Some(SphinxField {
        key,
        args: parts.map(ToString::to_string).collect(),
        description: description.trim().to_string(),
    })
}

pub(crate) fn sphinx_attribute_directive_name(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix(".. attribute ::")?.trim();
    (!rest.is_empty()).then_some(rest)
}

pub(crate) fn is_sphinx_param_key(key: &str) -> bool {
    matches!(
        key,
        "param" | "parameter" | "arg" | "argument" | "key" | "keyword"
    )
}

fn is_sphinx_return_key(key: &str) -> bool {
    matches!(key, "return" | "returns")
}

fn is_sphinx_yield_key(key: &str) -> bool {
    matches!(key, "yield" | "yields")
}

fn is_sphinx_raises_key(key: &str) -> bool {
    matches!(key, "raise" | "raises" | "except" | "exception")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionKind {
    Args,
    Attributes,
    Returns,
    Yields,
    Raises,
}

fn numpy_section_kind(text: &str) -> Option<SectionKind> {
    match text {
        "Parameters" | "Params" | "Arguments" | "Args" | "Other Parameters" | "Other Params"
        | "Other Arguments" | "Other Args" | "Receives" | "Receive" => Some(SectionKind::Args),
        "Attributes" | "Attribute" => Some(SectionKind::Attributes),
        "Returns" | "Return" => Some(SectionKind::Returns),
        "Yields" | "Yield" => Some(SectionKind::Yields),
        "Raises" | "Raise" | "Warns" | "Warn" => Some(SectionKind::Raises),
        _ => None,
    }
}

fn google_section_kind(text: &str) -> Option<SectionKind> {
    match text {
        "Args:" | "Arguments:" | "Parameters:" | "Params:" => Some(SectionKind::Args),
        "Attributes:" => Some(SectionKind::Attributes),
        "Returns:" => Some(SectionKind::Returns),
        "Yields:" => Some(SectionKind::Yields),
        "Raises:" | "Exceptions:" | "Except:" => Some(SectionKind::Raises),
        _ => None,
    }
}

fn set_section_flag(out: &mut ParsedDocstring, kind: SectionKind) {
    match kind {
        SectionKind::Args => out.has_args_section = true,
        SectionKind::Attributes => out.has_attributes_section = true,
        SectionKind::Returns => out.has_returns_section = true,
        SectionKind::Yields => out.has_yields_section = true,
        SectionKind::Raises => out.has_raises_section = true,
    }
}

fn has_any_structured_section(doc: &ParsedDocstring) -> bool {
    doc.has_args_section
        || doc.has_attributes_section
        || doc.has_returns_section
        || doc.has_yields_section
        || doc.has_raises_section
}

fn has_nonshort_aux_meta(lines: &[String], style: DocStyle) -> bool {
    match style {
        DocStyle::Google => lines
            .iter()
            .any(|line| indentation(line) == 0 && matches!(line.trim(), "Example:" | "Examples:")),
        DocStyle::Numpy => {
            let mut index = 0usize;
            while index < lines.len() {
                if indentation(&lines[index]) != 0 {
                    index += 1;
                    continue;
                }
                let heading = lines[index].trim();
                if is_numpy_deprecated_directive(heading) {
                    return true;
                }
                if matches!(heading, "Example" | "Examples")
                    && lines.get(index + 1).is_some_and(|underline| {
                        indentation(underline) == 0
                            && is_numpy_section_underline(heading, underline.trim())
                    })
                {
                    let start = index + 2;
                    let end = find_next_numpy_section(lines, start);
                    if lines[start..end].iter().any(|line| !line.trim().is_empty()) {
                        return true;
                    }
                    index = end;
                    continue;
                }
                index += 1;
            }
            false
        }
        DocStyle::Sphinx => lines.iter().any(|line| {
            if indentation(line) != 0 {
                return false;
            }
            parse_sphinx_field(line.trim())
                .is_some_and(|field| matches!(field.key.as_str(), "deprecation" | "deprecated"))
        }),
    }
}

fn find_next_numpy_section(lines: &[String], start: usize) -> usize {
    (start..lines.len())
        .find(|index| is_numpy_parser_section_start(lines, *index))
        .unwrap_or(lines.len())
}

fn is_numpy_parser_section_start(lines: &[String], index: usize) -> bool {
    if indentation(&lines[index]) != 0 {
        return false;
    }
    let heading = lines[index].trim();
    if is_numpy_deprecated_directive(heading) {
        return true;
    }
    is_allowed_numpy_section(heading)
        && lines.get(index + 1).is_some_and(|underline| {
            indentation(underline) == 0 && is_numpy_section_underline(heading, underline.trim())
        })
}

fn is_numpy_deprecated_directive(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("..") else {
        return false;
    };
    let rest = rest.trim_start();
    let Some(rest) = rest.strip_prefix("deprecated") else {
        return false;
    };
    rest.trim_start().starts_with("::")
}

fn find_next_google_section(lines: &[String], start: usize, heading_indent: usize) -> usize {
    lines
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, line)| !line.trim().is_empty() && indentation(line) <= heading_indent)
        .map(|(index, _)| index)
        .unwrap_or(lines.len())
}

fn collect_item_description(
    lines: &[String],
    start: usize,
    end: usize,
    item_indent: usize,
    inline: &str,
) -> String {
    let mut parts = Vec::new();
    if !inline.trim().is_empty() {
        parts.push(inline.trim().to_string());
    }
    let mut index = start;
    while index < end {
        let line = &lines[index];
        let trimmed = line.trim();
        if !trimmed.is_empty() && indentation(line) <= item_indent {
            break;
        }
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
        index += 1;
    }
    parts.join(" ").trim().to_string()
}

fn collect_sphinx_field_description(lines: &[String], index: usize, inline: &str) -> String {
    collect_item_description(lines, index + 1, lines.len(), 0, inline)
}

fn section_item_indent(lines: &[String], start: usize, end: usize) -> Option<usize> {
    lines[start..end]
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| indentation(line))
        .min()
}

fn collect_until_top_level_colon(
    lines: &[String],
    start: usize,
    end: usize,
) -> Option<(String, usize)> {
    let base_indent = indentation(&lines[start]);
    let mut text = String::new();
    let mut index = start;
    while index < end {
        if index > start && indentation(&lines[index]) <= base_indent && bracket_balance(&text) <= 0
        {
            break;
        }
        let part = lines[index].trim();
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(part);
        if find_top_level_colon(&text).is_some() {
            return Some((text, index));
        }
        if bracket_balance(&text) <= 0 && !physical_line_continues(part) {
            return None;
        }
        index += 1;
    }
    None
}

fn collect_type_continuation(
    lines: &[String],
    start: usize,
    end: usize,
    initial: &str,
    base_indent: usize,
) -> (String, usize) {
    let mut text = initial.to_string();
    let mut depth = bracket_balance(initial);
    let mut continued = physical_line_continues(initial) || depth > 0;
    let mut index = start;

    while continued && index + 1 < end {
        let next_index = index + 1;
        let next = &lines[next_index];
        if next.trim().is_empty() {
            break;
        }
        if depth <= 0 && indentation(next) <= base_indent {
            break;
        }
        let part = next.trim();
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(part);
        depth += bracket_balance(part);
        continued = physical_line_continues(part) || depth > 0;
        index = next_index;
    }
    (text, index)
}

fn physical_line_continues(text: &str) -> bool {
    let text = text.trim_end();
    text.ends_with('\\') && !ends_with_escaped_backslash(text)
}

fn ends_with_escaped_backslash(text: &str) -> bool {
    let count = text.chars().rev().take_while(|ch| *ch == '\\').count();
    count > 0 && count % 2 == 0
}

fn bracket_balance(text: &str) -> i32 {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in text.chars() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

pub(crate) fn find_top_level_colon(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ':' if depth == 0 => return Some(index),
            _ => {}
        }
    }
    None
}

fn strip_type_comment(text: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '#' => return &text[..index],
            _ => {}
        }
    }
    text
}

fn is_numpy_underline(text: &str) -> bool {
    text.len() >= 3 && text.chars().all(|ch| ch == '-')
}

fn is_numpy_section_underline(heading: &str, underline: &str) -> bool {
    underline.len() == heading.len() && underline.chars().all(|ch| ch == '-')
}

fn normalized_doc_lines(docstring: &str) -> Vec<String> {
    let raw = docstring.lines().collect::<Vec<_>>();
    let common = raw
        .iter()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| indentation(line))
        .min()
        .unwrap_or(0);
    raw.into_iter()
        .map(|line| strip_indent(line, common).to_string())
        .collect()
}

fn strip_indent(text: &str, width: usize) -> &str {
    let mut bytes = 0usize;
    let mut columns = 0usize;
    for (index, ch) in text.char_indices() {
        if columns >= width || !matches!(ch, ' ' | '\t') {
            bytes = index;
            break;
        }
        columns += if ch == '\t' { 4 } else { 1 };
        bytes = index + ch.len_utf8();
    }
    &text[bytes.min(text.len())..]
}

fn indentation(text: &str) -> usize {
    text.chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum()
}

pub(crate) fn normalize_doc_name(name: &str) -> String {
    name.replace('\\', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numpy_multiline_type_is_preserved() {
        let doc = r#"Summary.

Parameters
----------
value : dict[
    str,
    list[int],
]
    Description.
"#;
        let parsed = parse_docstring(doc, DocStyle::Numpy);
        assert_eq!(parsed.params.len(), 1);
        assert_eq!(
            normalize_type_text(&parsed.params[0].ty),
            "dict[str,list[int],]"
        );
    }

    #[test]
    fn numpy_untyped_parameter_is_parsed() {
        let doc = "Parameters\n----------\nvalue\n    Description.\n";
        let parsed = parse_docstring(doc, DocStyle::Numpy);
        assert_eq!(
            parsed.params,
            vec![DocItem {
                name: "value".into(),
                ty: String::new(),
                description: "Description.".into()
            }]
        );
        assert_eq!(parsed.parse_error, None);
    }

    #[test]
    fn numpy_named_return_keeps_only_the_type() {
        let doc = "Returns\n-------\nresult : tuple[int, str]\n    Description.\n";
        let parsed = parse_docstring(doc, DocStyle::Numpy);
        assert_eq!(parsed.returns, vec!["tuple[int, str]"]);
    }

    #[test]
    fn numpy_named_return_with_empty_type_preserves_empty_type() {
        let doc = "Returns\n-------\nint :\n    Description.\n";
        let parsed = parse_docstring(doc, DocStyle::Numpy);
        assert_eq!(parsed.returns, vec![String::new()]);
        assert_eq!(parsed.parse_error, None);
    }

    #[test]
    fn numpy_malformed_nonempty_section_reports_parse_error() {
        let doc = "Parameters\n----------\n    This has no parameter name.\n";
        let parsed = parse_docstring(doc, DocStyle::Numpy);
        assert!(parsed.parse_error.is_some());
    }

    #[test]
    fn numpy_unsupported_sections_are_rejected_together() {
        let doc = "Inputs\n------\nx\n\nOutputs\n-------\ny\n";
        let parsed = parse_docstring(doc, DocStyle::Numpy);
        let error = parsed.parse_error.unwrap();
        assert!(error.contains("Inputs"));
        assert!(error.contains("Outputs"));
    }

    #[test]
    fn numpy_style_signal_allows_colon_before_underline() {
        assert_eq!(likely_style("Args:\n-----\nvalue\n"), Some(DocStyle::Numpy));
    }

    #[test]
    fn numpy_comma_separated_parameter_spelling_is_one_name_like_upstream() {
        let parsed = parse_docstring(
            "Parameters\n----------\nx, y : int\n    Values.\n",
            DocStyle::Numpy,
        );
        assert_eq!(parsed.params.len(), 1);
        assert_eq!(parsed.params[0].name, "x, y");
    }

    #[test]
    fn numpy_comma_separated_raise_spelling_is_one_exception() {
        let parsed = parse_docstring(
            "Raises\n------\nValueError, TypeError\n    Bad.\n",
            DocStyle::Numpy,
        );
        assert_eq!(parsed.raises, vec!["ValueError, TypeError"]);
    }

    #[test]
    fn google_multiline_type_is_preserved() {
        let doc = r#"Summary.

Args:
    value (dict[
        str,
        list[int],
    ]): Description.
"#;
        let parsed = parse_docstring(doc, DocStyle::Google);
        assert_eq!(parsed.params.len(), 1);
        assert_eq!(
            normalize_type_text(&parsed.params[0].ty),
            "dict[str,list[int],]"
        );
    }

    #[test]
    fn google_optional_suffix_is_removed_from_type() {
        let parsed = parse_docstring(
            "Summary.\n\nArgs:\n    value (int, optional): Value.\n    other (str?): Other.\n",
            DocStyle::Google,
        );
        assert_eq!(parsed.params[0].ty, "int");
        assert_eq!(parsed.params[1].ty, "str");
    }

    #[test]
    fn escaped_docstring_argument_name_is_unescaped() {
        let parsed = parse_docstring(
            "Parameters\n----------\narg\\_\\_ : int\n    Value.\n",
            DocStyle::Numpy,
        );
        assert_eq!(parsed.params[0].name, "arg__");
    }

    #[test]
    fn google_malformed_parameter_reports_parse_error() {
        let parsed = parse_docstring("Summary.\n\nArgs:\n    value\n", DocStyle::Google);
        assert!(parsed
            .parse_error
            .as_deref()
            .is_some_and(|error| error.contains("Expected a colon")));
    }

    #[test]
    fn google_empty_attribute_name_reports_attribute_parse_error() {
        let parsed = parse_docstring("Summary.\n\nAttributes:\n    : missing\n", DocStyle::Google);
        assert_eq!(
            parsed.parse_error.as_deref(),
            Some("Parsed docstring attribute has an empty name")
        );
    }

    #[test]
    fn google_singular_return_description_is_not_treated_as_type() {
        let parsed = parse_docstring(
            "Summary.\n\nReturns:\n    Human readable description.\n",
            DocStyle::Google,
        );
        assert!(parsed.has_returns_section);
        assert_eq!(parsed.returns, vec![""]);
        assert_eq!(parsed.parse_error, None);
    }

    #[test]
    fn google_unknown_base_indent_text_terminates_section() {
        let doc = "Args:\n    value (int): Description.\nNotes:\n    Free-form notes.\n";
        let parsed = parse_docstring(doc, DocStyle::Google);
        assert_eq!(parsed.params.len(), 1);
        assert_eq!(parsed.parse_error, None);
    }

    #[test]
    fn google_params_and_exceptions_aliases_are_parsed() {
        let doc = "Params:\n    value (int): Description.\nExceptions:\n    ValueError: Invalid.\n";
        let parsed = parse_docstring(doc, DocStyle::Google);
        assert_eq!(parsed.params[0].name, "value");
        assert_eq!(parsed.raises, vec!["ValueError"]);
    }

    #[test]
    fn parser_alias_does_not_become_google_style_signal() {
        assert_eq!(
            likely_style("Parameters:\n    value (int): Description.\n"),
            None
        );
    }

    #[test]
    fn google_style_signal_uses_upstream_startswith_semantics() {
        assert_eq!(
            likely_style("Args: malformed suffix\n"),
            Some(DocStyle::Google)
        );
    }

    #[test]
    fn detected_google_style_parses_arguments_before_reporting_mismatch() {
        let doc = "Summary.\n\nArgs:\n    value (int): Description.\n";
        let (parsed, mismatch) = parse_docstring_with_style_detection(doc, DocStyle::Numpy);
        assert!(mismatch);
        assert_eq!(parsed.params[0].name, "value");
    }

    #[test]
    fn google_returns_chunk_creates_one_meta_item_like_upstream() {
        let doc = "Summary.\n\nReturns:\n    int: First.\n    str: Second.\n";
        let parsed = parse_docstring(doc, DocStyle::Google);
        assert_eq!(parsed.returns, vec!["int"]);
    }

    #[test]
    fn numpy_auxiliary_section_terminates_parameters() {
        let doc =
            "Parameters\n----------\nvalue : int\n    Value.\n\nExamples\n--------\n>>> f(1)\n";
        let parsed = parse_docstring(doc, DocStyle::Numpy);
        assert_eq!(parsed.params.len(), 1);
        assert_eq!(parsed.params[0].name, "value");
        assert!(!parsed.is_short);
    }

    #[test]
    fn numpy_warns_is_exposed_as_raises_like_upstream_fork() {
        let doc = "Warns\n-----\nUserWarning\n    Warning.\n";
        let parsed = parse_docstring(doc, DocStyle::Numpy);
        assert!(parsed.has_raises_section);
        assert_eq!(parsed.raises, vec!["UserWarning"]);
    }

    #[test]
    fn sphinx_raises_type_can_be_recovered_from_description_like_upstream() {
        let parsed = parse_docstring(
            "Summary.\n\n:raises: ValueError: Invalid value.",
            DocStyle::Sphinx,
        );
        assert!(parsed.has_raises_section);
        assert_eq!(parsed.raises, vec!["ValueError"]);
    }

    #[test]
    fn sphinx_raises_description_with_spaces_is_not_treated_as_exception_type() {
        let parsed = parse_docstring(
            "Summary.\n\n:raises: invalid value: explanation.",
            DocStyle::Sphinx,
        );
        assert!(parsed.has_raises_section);
        assert!(parsed.raises.is_empty());
    }

    #[test]
    fn sphinx_types_and_raises_are_parsed() {
        let doc = r#"Summary.

:param value: Value.
:type value: dict[
    str, int]
:returns: Result.
:rtype: list[int]
:raises ValueError: Invalid.
"#;
        let parsed = parse_docstring(doc, DocStyle::Sphinx);
        assert_eq!(parsed.params[0].name, "value");
        assert_eq!(normalize_type_text(&parsed.params[0].ty), "dict[str,int]");
        assert_eq!(parsed.returns, vec!["list[int]"]);
        assert_eq!(parsed.raises, vec!["ValueError"]);
    }

    #[test]
    fn sphinx_aliases_inline_types_and_yields_are_parsed() {
        let doc = "Summary.\n\n:argument str value: Value.\n:yield tuple[str,int]: Item.\n:exception ValueError: Invalid.\n";
        let parsed = parse_docstring(doc, DocStyle::Sphinx);
        assert_eq!(
            parsed.params[0],
            DocItem {
                name: "value".into(),
                ty: "str".into(),
                description: "Value.".into()
            }
        );
        assert_eq!(parsed.yields, vec!["tuple[str,int]"]);
        assert_eq!(parsed.raises, vec!["ValueError"]);
    }

    #[test]
    fn sphinx_attribute_directive_is_parsed_as_class_attribute() {
        let doc = "Summary.\n\n.. attribute :: name\n    :type: str | None\n\n    Description.\n";
        let parsed = parse_docstring(doc, DocStyle::Sphinx);
        assert_eq!(
            parsed.attrs[0],
            DocItem {
                name: "name".into(),
                ty: "str | None".into(),
                description: "Description.".into()
            }
        );
    }

    #[test]
    fn sphinx_type_directive_can_precede_parameter() {
        let doc = ":type value: int\n:param value: Value.\n";
        let parsed = parse_docstring(doc, DocStyle::Sphinx);
        assert_eq!(parsed.params[0].ty, "int");
    }

    #[test]
    fn sphinx_missing_param_name_reports_parse_error() {
        let parsed = parse_docstring(":param: missing\n", DocStyle::Sphinx);
        assert!(parsed.parse_error.is_some());
    }

    #[test]
    fn sphinx_inline_optional_suffix_is_removed() {
        let parsed = parse_docstring(":param int? value: Value.\n", DocStyle::Sphinx);
        assert_eq!(parsed.params[0].ty, "int");
    }

    #[test]
    fn sphinx_duplicate_params_are_preserved() {
        let parsed = parse_docstring(
            ":param value: First.\n:param value: Duplicate.\n",
            DocStyle::Sphinx,
        );
        assert_eq!(parsed.params.len(), 2);
        assert_eq!(parsed.params[0].name, "value");
        assert_eq!(parsed.params[1].name, "value");
    }

    #[test]
    fn sphinx_inline_param_type_wins_over_separate_type_field() {
        let parsed = parse_docstring(
            ":param int value: Value.\n:type value: str\n",
            DocStyle::Sphinx,
        );
        assert_eq!(parsed.params[0].ty, "int");
    }

    #[test]
    fn sphinx_duplicate_attribute_directives_are_preserved() {
        let parsed = parse_docstring(
            ".. attribute :: value\n    :type: int\n\n.. attribute :: value\n    :type: str\n",
            DocStyle::Sphinx,
        );
        assert_eq!(parsed.attrs.len(), 2);
        assert_eq!(parsed.attrs[0].ty, "int");
        assert_eq!(parsed.attrs[1].ty, "str");
    }

    #[test]
    fn sphinx_empty_attribute_directive_reports_parse_error() {
        let parsed = parse_docstring(".. attribute ::\n", DocStyle::Sphinx);
        assert_eq!(
            parsed.parse_error.as_deref(),
            Some("Parsed docstring attribute has an empty name")
        );
    }

    #[test]
    fn sphinx_rtype_accepts_optional_name_and_synthesizes_return() {
        let parsed = parse_docstring(":rtype result: list[int]\n", DocStyle::Sphinx);
        assert!(parsed.has_returns_section);
        assert_eq!(parsed.returns, vec!["list[int]"]);
        assert_eq!(parsed.parse_error, None);
    }

    #[test]
    fn sphinx_ytype_alone_does_not_create_yields_section() {
        let parsed = parse_docstring(":ytype item: int\n", DocStyle::Sphinx);
        assert!(!parsed.has_yields_section);
        assert!(parsed.yields.is_empty());
    }

    #[test]
    fn sphinx_ytype_enriches_untyped_yield() {
        let parsed = parse_docstring(":yield: Item.\n:ytype: int\n", DocStyle::Sphinx);
        assert!(parsed.has_yields_section);
        assert_eq!(parsed.yields, vec!["int"]);
    }

    #[test]
    fn sphinx_inline_return_type_wins_over_rtype() {
        let parsed = parse_docstring(":return str: Result.\n:rtype: int\n", DocStyle::Sphinx);
        assert_eq!(parsed.returns, vec!["str"]);
    }

    #[test]
    fn indented_sphinx_field_is_meta_after_cleandoc() {
        let parsed = parse_docstring(
            "Summary.\n\n    :param ghost: example text\n",
            DocStyle::Sphinx,
        );
        assert!(parsed.has_args_section);
        assert_eq!(parsed.params.len(), 1);
        assert_eq!(parsed.params[0].name, "ghost");
    }

    #[test]
    fn indented_google_heading_is_a_section_after_cleandoc() {
        let parsed = parse_docstring(
            "Summary.\n\n    Args:\n        ghost (int): example\n",
            DocStyle::Google,
        );
        assert!(parsed.has_args_section);
        assert_eq!(parsed.params.len(), 1);
        assert_eq!(parsed.params[0].name, "ghost");
    }

    #[test]
    fn indented_numpy_heading_is_parsed_after_cleandoc() {
        let parsed = parse_docstring(
            "Summary.\n\n    Returns\n    -------\n    int\n",
            DocStyle::Numpy,
        );
        assert!(parsed.has_returns_section);
        assert_eq!(parsed.returns, vec!["int"]);
    }

    #[test]
    fn examples_and_deprecation_prevent_false_short_docstring_classification() {
        let google = parse_docstring(
            "Summary.\n\nExamples:\n    Example text.\n",
            DocStyle::Google,
        );
        assert!(!google.is_short);
        let sphinx = parse_docstring("Summary.\n\n:deprecated 1.0: Old API.\n", DocStyle::Sphinx);
        assert!(!sphinx.is_short);
    }

    #[test]
    fn trailing_markdown_backslash_is_ignored_only_at_line_boundary() {
        assert_eq!(
            normalize_type_text("dict[str, \\\n    list[int]]"),
            "dict[str,list[int]]"
        );
        assert_eq!(normalize_type_text(r"Literal['a\\b']"), r"Literal['a\\b']");
    }

    #[test]
    fn numpy_unsupported_section_error_matches_upstream_wording() {
        let singular = parse_docstring(
            "Summary.\n\nInputs\n------\nvalue : int\n    Value.\n",
            DocStyle::Numpy,
        );
        assert_eq!(
            singular.parse_error.as_deref(),
            Some("Unsupported numpy docstring section: \"Inputs\"")
        );

        let plural = parse_docstring(
            "Summary.\n\nInputs\n------\nvalue : int\n    Value.\n\nOutputs\n-------\nint\n    Value.\n",
            DocStyle::Numpy,
        );
        assert_eq!(
            plural.parse_error.as_deref(),
            Some("Unsupported numpy docstring sections: \"Inputs\", \"Outputs\"")
        );
    }
}
