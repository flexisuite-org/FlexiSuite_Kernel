use anyhow::Result;
use clap::Parser;
use regex::Regex;
use std::fs;
use std::path::{Component, Path};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = ".")]
    path: String,
}

fn get_line_number(content: &str, byte_offset: usize) -> usize {
    content[..byte_offset]
        .chars()
        .filter(|&c| c == '\n')
        .count()
        + 1
}

fn tokenize_args(args_list: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    let mut in_quote = false;
    let chars = args_list.chars();

    for c in chars {
        match c {
            '\'' => {
                in_quote = !in_quote;
                current.push(c);
            }
            '(' | '[' if !in_quote => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' if !in_quote => {
                if depth == 0 {
                    return Err(format!(
                        "Unbalanced parentheses/brackets in args list: {}",
                        args_list
                    ));
                }
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 && !in_quote => {
                args.push(current.trim().to_string());
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }

    if in_quote {
        return Err(format!("Unterminated quote in args list: {}", args_list));
    }
    if depth != 0 {
        return Err(format!(
            "Unbalanced parentheses/brackets in args list: {}",
            args_list
        ));
    }

    if !current.trim().is_empty() {
        args.push(current.trim().to_string());
    }

    // Extract types
    let mut types = Vec::new();
    for s in args {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        let mut parts_slice = parts.as_slice();
        let leading_mode = parts_slice[0].to_ascii_uppercase();
        if matches!(leading_mode.as_str(), "IN" | "OUT" | "INOUT" | "VARIADIC") {
            if leading_mode == "OUT" {
                continue;
            }
            parts_slice = &parts_slice[1..];
            if parts_slice.is_empty() {
                continue;
            }
        }

        let default_pos = parts_slice
            .iter()
            .position(|&p| p.eq_ignore_ascii_case("DEFAULT"));
        let type_parts = if let Some(pos) = default_pos {
            &parts_slice[..pos]
        } else {
            parts_slice
        };

        if type_parts.len() >= 2 {
            // Likely "name type" or "name type[]"
            types.push(type_parts[1..].join(" "));
        } else if type_parts.len() == 1 {
            // just "type"
            types.push(type_parts[0].to_string());
        }
    }

    Ok(types)
}

fn skip_whitespace(input: &str, mut idx: usize) -> usize {
    while idx < input.len() {
        let Some(ch) = input[idx..].chars().next() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }
        idx += ch.len_utf8();
    }
    idx
}

fn parse_identifier_part(input: &str, idx: usize) -> Result<(String, usize), String> {
    if idx >= input.len() {
        return Err("Unexpected end while parsing function identifier".to_string());
    }

    let rest = &input[idx..];
    let Some(first) = rest.chars().next() else {
        return Err("Unexpected end while parsing function identifier".to_string());
    };

    if first == '"' {
        let mut i = idx + first.len_utf8();
        while i < input.len() {
            let Some(ch) = input[i..].chars().next() else {
                break;
            };
            if ch == '"' {
                let next = i + ch.len_utf8();
                if next < input.len() && input[next..].starts_with('"') {
                    i = next + '"'.len_utf8();
                    continue;
                }
                let end = i + ch.len_utf8();
                return Ok((input[idx..end].to_string(), end));
            }
            i += ch.len_utf8();
        }
        return Err("Unterminated double-quoted identifier in CREATE FUNCTION".to_string());
    }

    if first.is_ascii_alphabetic() || first == '_' {
        let mut i = idx + first.len_utf8();
        while i < input.len() {
            let Some(ch) = input[i..].chars().next() else {
                break;
            };
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
                i += ch.len_utf8();
            } else {
                break;
            }
        }
        return Ok((input[idx..i].to_string(), i));
    }

    Err("Invalid function identifier in CREATE FUNCTION".to_string())
}

fn parse_create_function_signature(
    input: &str,
    start_idx: usize,
) -> Result<(String, String), String> {
    let mut idx = skip_whitespace(input, start_idx);
    let mut identifier_parts = Vec::new();

    let (first_part, next_idx) = parse_identifier_part(input, idx)?;
    identifier_parts.push(first_part);
    idx = next_idx;

    loop {
        idx = skip_whitespace(input, idx);
        if idx >= input.len() || !input[idx..].starts_with('.') {
            break;
        }
        idx += '.'.len_utf8();
        idx = skip_whitespace(input, idx);
        let (part, part_end) = parse_identifier_part(input, idx)?;
        identifier_parts.push(part);
        idx = part_end;
    }

    idx = skip_whitespace(input, idx);
    if idx >= input.len() || !input[idx..].starts_with('(') {
        return Err("Expected '(' after CREATE FUNCTION name".to_string());
    }

    let mut i = idx + '('.len_utf8();
    let args_start = i;
    let mut depth = 1usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < input.len() {
        let Some(ch) = input[i..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if in_single_quote {
            if ch == '\'' {
                let next = i + ch_len;
                if next < input.len() && input[next..].starts_with('\'') {
                    i = next + '\''.len_utf8();
                    continue;
                }
                in_single_quote = false;
            }
            i += ch_len;
            continue;
        }

        if in_double_quote {
            if ch == '"' {
                let next = i + ch_len;
                if next < input.len() && input[next..].starts_with('"') {
                    i = next + '"'.len_utf8();
                    continue;
                }
                in_double_quote = false;
            }
            i += ch_len;
            continue;
        }

        match ch {
            '\'' => in_single_quote = true,
            '"' => in_double_quote = true,
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Err(
                        "Unbalanced closing parenthesis while parsing CREATE FUNCTION arguments"
                            .to_string(),
                    );
                }
                depth -= 1;
                if depth == 0 {
                    let args = input[args_start..i].to_string();
                    let func_name = identifier_parts.join(".");
                    return Ok((func_name, args));
                }
            }
            _ => {}
        }

        i += ch_len;
    }

    if in_single_quote || in_double_quote {
        return Err("Unterminated quote while parsing CREATE FUNCTION arguments".to_string());
    }

    Err("Unmatched parentheses while parsing CREATE FUNCTION arguments".to_string())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let root = Path::new(&args.path);

    let re_security_definer = Regex::new(r"(?i)SECURITY\s+DEFINER").unwrap();
    // Allow variations in spacing and newlines
    let re_search_path =
        Regex::new(r"(?i)SET\s+(?:(?:SESSION|LOCAL)\s+)?search_path\s*(?:=|TO)\s*flexi\s*,\s*pg_catalog\s*,\s*pg_temp").unwrap();
    let re_create_function = Regex::new(r"(?i)CREATE\s+(?:OR\s+REPLACE\s+)?FUNCTION\s+").unwrap();
    let re_kernel_mode = Regex::new(r"(?i)flexi\.kernel_mode").unwrap();

    let mut errors = false;

    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let components: Vec<Component<'_>> = path.components().collect();
        let in_ops_linters = components
            .windows(2)
            .any(|w| w[0].as_os_str() == "ops" && w[1].as_os_str() == "linters");

        // Skip hidden directories, target directory, and linters source
        // We must check that the component is not strictly "." or ".."
        if components.iter().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            s.starts_with('.') && s != "." && s != ".."
        }) || components.iter().any(|c| c.as_os_str() == "target")
            || in_ops_linters
        {
            continue;
        }

        if path.is_file() {
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy();
                // Rust scanning is intentionally narrowed to migration paths: SECURITY
                // DEFINER SQL lives there, and broader .rs scanning creates false
                // positives from examples and doc comments.
                if ext_str == "sql"
                    || (ext_str == "rs" && path.to_string_lossy().contains("migration"))
                {
                    let content = match fs::read_to_string(path) {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("Error: Failed to read file {}: {}", path.display(), e);
                            errors = true;
                            continue;
                        }
                    };

                    let is_migration = path.to_string_lossy().contains("migration");

                    // Check forbidden kernel_mode
                    if is_migration {
                        for cap in re_kernel_mode.find_iter(&content) {
                            let line = get_line_number(&content, cap.start());
                            eprintln!(
                                "{}:{}: forbidden usage of 'flexi.kernel_mode' in migration",
                                path.display(),
                                line
                            );
                            errors = true;
                        }
                    }

                    for cap in re_security_definer.find_iter(&content) {
                        let start = cap.start();
                        let line = get_line_number(&content, start);

                        let last_func_span = re_create_function
                            .find_iter(&content[..start])
                            .last()
                            .map(|m| (m.start(), m.end()));

                        // search_path must be set from CREATE FUNCTION through SECURITY DEFINER.
                        let mut start_lookback = last_func_span
                            .map(|(func_start, _)| func_start)
                            .unwrap_or(0);
                        while !content.is_char_boundary(start_lookback) {
                            start_lookback = start_lookback.saturating_sub(1);
                        }

                        let mut end_lookahead = std::cmp::min(content.len(), start + 500);
                        while !content.is_char_boundary(end_lookahead) {
                            end_lookahead = end_lookahead.saturating_sub(1);
                        }
                        let window = &content[start_lookback..end_lookahead];

                        if !re_search_path.is_match(window) {
                            eprintln!("{}:{}: SECURITY DEFINER found without a valid search_path reset nearby. Accepted forms: SET [SESSION|LOCAL] search_path {{=|TO}} flexi, pg_catalog, pg_temp", path.display(), line);
                            errors = true;
                        }

                        // Targeted REVOKE check
                        let Some((last_func_start, last_func_end)) = last_func_span else {
                            eprintln!(
                                "{}:{}: SECURITY DEFINER found but no preceding CREATE FUNCTION could be located for targeted REVOKE validation",
                                path.display(),
                                line
                            );
                            errors = true;
                            continue;
                        };

                        let (func_name, args_list) = match parse_create_function_signature(
                            &content,
                            last_func_end,
                        ) {
                            Ok(parsed) => parsed,
                            Err(e) => {
                                eprintln!(
                                    "{}:{}: failed to parse CREATE FUNCTION signature near SECURITY DEFINER: {}",
                                    path.display(),
                                    line,
                                    e
                                );
                                errors = true;
                                continue;
                            }
                        };

                        // Extract just the types from args_list (e.g. "token_val text, other int" -> ["text", "int"])
                        let arg_types = match tokenize_args(&args_list) {
                            Ok(types) => types,
                            Err(e) => {
                                eprintln!("{}:{}: {}", path.display(), line, e);
                                errors = true;
                                continue;
                            }
                        };
                        let arg_signature = arg_types
                            .iter()
                            .map(|t| regex::escape(t))
                            .collect::<Vec<_>>()
                            .join(r"\s*,\s*");

                        // Escape func_name for regex
                        let func_name_regex = regex::escape(&func_name);

                        let revoke_pattern = format!(
                            r"(?si)REVOKE\s+.*?\s+ON\s+FUNCTION\s+{}\s*\(\s*{}\s*\)\s+FROM\s+PUBLIC",
                            func_name_regex, arg_signature
                        );

                        let mut revoke_search_start = last_func_start;
                        while !content.is_char_boundary(revoke_search_start) {
                            revoke_search_start = revoke_search_start.saturating_sub(1);
                        }
                        let revoke_window = &content[revoke_search_start..];

                        let re_targeted_revoke = Regex::new(&revoke_pattern).unwrap();
                        if !re_targeted_revoke.is_match(revoke_window) {
                            eprintln!(
                                "{}:{}: SECURITY DEFINER found for function '{}' without nearby 'REVOKE ... ON FUNCTION {}({}) FROM PUBLIC'",
                                path.display(),
                                line,
                                func_name,
                                func_name,
                                arg_types.join(", ")
                            );
                            errors = true;
                        }
                    }
                }
            }
        }
    }

    if errors {
        anyhow::bail!("SQL Security Linter found errors");
    }

    println!("SQL Security Linter Passed");
    Ok(())
}
