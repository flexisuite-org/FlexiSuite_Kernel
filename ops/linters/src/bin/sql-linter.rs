use std::fs;
use std::path::Path;
use walkdir::WalkDir;
use regex::Regex;
use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = ".")]
    path: String,
}

fn get_line_number(content: &str, byte_offset: usize) -> usize {
    content[..byte_offset].chars().filter(|&c| c == '\n').count() + 1
}

fn main() -> Result<()> {
    let args = Args::parse();
    let root = Path::new(&args.path);

    let re_security_definer = Regex::new(r"(?i)SECURITY\s+DEFINER").unwrap();
    // Allow variations in spacing and newlines
    let re_search_path = Regex::new(r"(?i)SET\s+search_path\s*=\s*flexi\s*,\s*pg_catalog\s*,\s*pg_temp").unwrap();
    let re_revoke = Regex::new(r"(?i)REVOKE\s+.*FROM\s+PUBLIC").unwrap();
    let re_kernel_mode = Regex::new(r"flexi\.kernel_mode").unwrap();

    let mut errors = false;

    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();

        // Skip hidden directories, target directory, and linters source
        // We must check that the component is not strictly "." or ".."
        if path.components().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            s.starts_with('.') && s != "." && s != ".."
        }) || path.components().any(|c| c.as_os_str() == "target")
           || path.to_string_lossy().contains("ops/linters") {
            continue;
        }

        if path.is_file() {
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy();
                if ext_str == "rs" || ext_str == "sql" {
                    let content = match fs::read_to_string(path) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };

                    let is_migration = path.to_string_lossy().contains("migration");

                    // Check forbidden kernel_mode
                    if is_migration {
                        for cap in re_kernel_mode.find_iter(&content) {
                            let line = get_line_number(&content, cap.start());
                            eprintln!("{}:{}: forbidden usage of 'flexi.kernel_mode' in migration", path.display(), line);
                            errors = true;
                        }
                    }

                    // Check SECURITY DEFINER template
                    let mut security_definer_found = false;
                    for cap in re_security_definer.find_iter(&content) {
                        security_definer_found = true;
                        let start = cap.start();
                        let line = get_line_number(&content, start);

                        // Check search_path in the vicinity (e.g., next 500 chars)
                        // This assumes the SET clause is near the SECURITY DEFINER keyword
                        let end_search = std::cmp::min(content.len(), start + 500);
                        let window = &content[start..end_search];

                        if !re_search_path.is_match(window) {
                            eprintln!("{}:{}: SECURITY DEFINER found without 'SET search_path = flexi, pg_catalog, pg_temp' nearby", path.display(), line);
                            errors = true;
                        }
                    }

                    if security_definer_found {
                        // Check for REVOKE in the whole file if SECURITY DEFINER is present
                        if !re_revoke.is_match(&content) {
                             eprintln!("{}: SECURITY DEFINER used but 'REVOKE ... FROM PUBLIC' is missing in file", path.display());
                             errors = true;
                        }
                    }
                }
            }
        }
    }

    if errors {
        std::process::exit(1);
    }

    println!("SQL Security Linter Passed");
    Ok(())
}
