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
        Regex::new(r"(?i)SET\s+((SESSION|LOCAL)\s+)?search_path\s*(=|TO)\s*flexi\s*,\s*pg_catalog\s*,\s*pg_temp").unwrap();
    // Use (?s) to allow '.' to match newlines for multi-line REVOKE statements
    let re_revoke = Regex::new(r"(?si)REVOKE\s+.*?FROM\s+PUBLIC").unwrap();
    let re_kernel_mode = Regex::new(r"flexi\.kernel_mode").unwrap();

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

                    // Check SECURITY DEFINER template vs REVOKE count
                    let security_definer_count = re_security_definer.find_iter(&content).count();
                    let revoke_count = re_revoke.find_iter(&content).count();

                    if security_definer_count > revoke_count {
                        eprintln!("{}: SECURITY DEFINER found {} times but 'REVOKE ... FROM PUBLIC' found only {} times.", path.display(), security_definer_count, revoke_count);
                        errors = true;
                    }

                    for cap in re_security_definer.find_iter(&content) {
                        let start = cap.start();
                        let line = get_line_number(&content, start);

                        // Check search_path in the vicinity (bidirectional 500 chars)
                        let mut start_search = if start > 500 { start - 500 } else { 0 };
                        while !content.is_char_boundary(start_search) {
                            start_search = start_search.saturating_sub(1);
                        }

                        let mut end_search = std::cmp::min(content.len(), start + 500);
                        // Ensure we slice at a valid char boundary
                        while !content.is_char_boundary(end_search) {
                            end_search = end_search.saturating_sub(1);
                        }
                        let window = &content[start_search..end_search];

                        if !re_search_path.is_match(window) {
                            eprintln!("{}:{}: SECURITY DEFINER found without 'SET search_path = flexi, pg_catalog, pg_temp' nearby", path.display(), line);
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
