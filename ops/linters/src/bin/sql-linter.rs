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

fn main() -> Result<()> {
    let args = Args::parse();
    let root = Path::new(&args.path);

    let re_security_definer = Regex::new(r"(?i)SECURITY\s+DEFINER").unwrap();
    // Allow variations in spacing and newlines
    let re_search_path =
        Regex::new(r"(?i)SET\s+(?:(?:SESSION|LOCAL)\s+)?search_path\s*(?:=|TO)\s*flexi\s*,\s*pg_catalog\s*,\s*pg_temp").unwrap();
    // Use (?s) to allow '.' to match newlines for multi-line REVOKE statements
    let re_revoke = Regex::new(r"(?si)REVOKE\s+.*?FROM\s+PUBLIC").unwrap();
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
                if ext_str == "rs" || ext_str == "sql" {
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

                    let re_func_parts = Regex::new(r"(?si)CREATE\s+(?:OR\s+REPLACE\s+)?FUNCTION\s+([\w.]+)\s*\((.*?)\)").unwrap();

                    for cap in re_security_definer.find_iter(&content) {
                        let start = cap.start();
                        let line = get_line_number(&content, start);

                        // Check search_path and REVOKE in the vicinity (bidirectional 1000 chars total)
                        let mut start_lookback = if start > 500 { start - 500 } else { 0 };
                        while !content.is_char_boundary(start_lookback) {
                            start_lookback = start_lookback.saturating_sub(1);
                        }

                        let mut end_lookahead = std::cmp::min(content.len(), start + 1000);
                        while !content.is_char_boundary(end_lookahead) {
                            end_lookahead = end_lookahead.saturating_sub(1);
                        }
                        let window = &content[start_lookback..end_lookahead];

                        if !re_search_path.is_match(window) {
                            eprintln!("{}:{}: SECURITY DEFINER found without a valid search_path reset nearby. Accepted forms: SET [SESSION|LOCAL] search_path {{=|TO}} flexi, pg_catalog, pg_temp", path.display(), line);
                            errors = true;
                        }

                        // Targeted REVOKE check
                        if let Some(_func_cap) = re_func_parts.captures(&content[..start]) {
                            // Find the *last* CREATE FUNCTION before the SECURITY DEFINER
                            let last_func = re_func_parts.find_iter(&content[..start]).last();
                            if let Some(func_match) = last_func {
                                let func_cap = re_func_parts.captures(func_match.as_str()).unwrap();
                                let func_name = func_cap.get(1).unwrap().as_str();
                                let args_list = func_cap.get(2).unwrap().as_str();

                                // Extract just the types from args_list (e.g. "token_val text, other int" -> "text, int")
                                let arg_types: Vec<String> = args_list
                                    .split(',')
                                    .map(|s| s.trim().split_whitespace().last().unwrap_or("").to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                                let arg_signature = arg_types.join(r"\s*,\s*");

                                // Escape dots in func_name for regex
                                let func_name_regex = func_name.replace('.', r"\.");

                                let revoke_pattern = format!(
                                    r"(?si)REVOKE\s+.*?\s+ON\s+FUNCTION\s+{}\s*\(\s*{}\s*\)\s+FROM\s+PUBLIC",
                                    func_name_regex, arg_signature
                                );

                                let re_targeted_revoke = Regex::new(&revoke_pattern).unwrap();
                                if !re_targeted_revoke.is_match(window) {
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
                            } else {
                                // Fallback to broad check if for some reason we can't find the function name
                                if !re_revoke.is_match(window) {
                                    eprintln!("{}:{}: SECURITY DEFINER found without nearby 'REVOKE ... FROM PUBLIC'", path.display(), line);
                                    errors = true;
                                }
                            }
                        } else {
                            if !re_revoke.is_match(window) {
                                eprintln!("{}:{}: SECURITY DEFINER found without nearby 'REVOKE ... FROM PUBLIC'", path.display(), line);
                                errors = true;
                            }
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
