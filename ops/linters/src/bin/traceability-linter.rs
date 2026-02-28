use anyhow::{Context, Result};
use clap::Parser;
use regex::Regex;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const EXTENSIONLESS_TEXT_MAX_BYTES: u64 = 256 * 1024;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = ".")]
    path: String,
}

fn extract_reqs(content: &str, regex: &Regex) -> HashSet<String> {
    regex
        .find_iter(content)
        .map(|m| m.as_str().to_string())
        .collect()
}

fn main() -> Result<()> {
    let args = Args::parse();
    let root = Path::new(&args.path);

    // REQ ID must consist of alphanumeric segments separated by single hyphens, anchored to word boundaries
    let re_req = Regex::new(r"\bREQ-[A-Z0-9]+(?:-[A-Z0-9]+)*\b").unwrap();

    // 1. Load Matrix REQs
    let matrix_path = root.join("docs/verification_matrix.md");
    if !matrix_path.exists() {
        anyhow::bail!("Error: docs/verification_matrix.md not found");
    }
    let matrix_content =
        fs::read_to_string(&matrix_path).context("Failed to read verification_matrix.md")?;
    let matrix_reqs = extract_reqs(&matrix_content, &re_req);

    // 2. Load Implementation Plan REQs
    let impl_path = root.join("docs/implementation_plan.md");
    let (impl_reqs, impl_loaded) = match fs::read_to_string(&impl_path) {
        Ok(impl_content) => (extract_reqs(&impl_content, &re_req), true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (HashSet::new(), false),
        Err(err) => {
            return Err(err).context("Failed to read implementation_plan.md");
        }
    };

    let mut errors = false;

    // 3. Check Consistency (Matrix == Impl)
    // The existing script enforced strict equality. We will maintain that.
    // However, if implementation_plan doesn't exist, we might skip or warn.
    // Assuming strict equality if impl exists.
    if impl_loaded {
        let mut missing_in_matrix: Vec<String> =
            impl_reqs.difference(&matrix_reqs).cloned().collect();
        missing_in_matrix.sort();
        let mut missing_in_impl: Vec<String> =
            matrix_reqs.difference(&impl_reqs).cloned().collect();
        missing_in_impl.sort();

        if !missing_in_matrix.is_empty() {
            eprintln!(
                "Error: REQs found in implementation_plan but missing in verification_matrix:"
            );
            for req in missing_in_matrix {
                eprintln!("  - {}", req);
            }
            errors = true;
        }

        if !missing_in_impl.is_empty() {
            eprintln!(
                "Error: REQs found in verification_matrix but missing in implementation_plan:"
            );
            for req in missing_in_impl {
                eprintln!("  - {}", req);
            }
            errors = true;
        }
    }

    // 4. Scan Codebase
    for entry in WalkDir::new(root).into_iter().filter_entry(|entry| {
        let p = entry.path();
        let rel = match p.strip_prefix(root) {
            Ok(rel) => rel,
            Err(_) => return false,
        };
        let mut components = rel.components();
        let mut prev = None;
        let mut is_ops_linters_path = false;
        let mut has_target = false;
        let mut has_hidden_dot = false;

        for component in components.by_ref() {
            let current = component.as_os_str();
            if prev == Some(OsStr::new("ops")) && current == OsStr::new("linters") {
                is_ops_linters_path = true;
            }

            if current == OsStr::new("target") {
                has_target = true;
            }

            let s = current.to_string_lossy();
            if s.starts_with('.') && s != "." && s != ".." {
                has_hidden_dot = true;
            }

            prev = Some(current);
        }

        !has_target && !is_ops_linters_path && !has_hidden_dot
    }) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                let is_benign = err
                    .io_error()
                    .map(|io_err| {
                        matches!(
                            io_err.kind(),
                            std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
                        )
                    })
                    .unwrap_or(false);

                if is_benign {
                    eprintln!("Warning: Failed to traverse directory: {err}");
                } else {
                    eprintln!("Error: Failed to traverse directory: {err}");
                    errors = true;
                }
                continue;
            }
        };

        let path = entry.path();
        if path == matrix_path || path == impl_path {
            continue;
        }

        if path.is_file() {
            // Scan likely text files: whitelisted extensions, or extensionless files whose size is
            // <= EXTENSIONLESS_TEXT_MAX_BYTES. If read_to_string() fails, we emit a warning and continue.
            let should_scan = match path.extension().and_then(|ext| ext.to_str()) {
                // Whitelist text extensions to skip binary files.
                Some(ext_str) => [
                    "rs", "sql", "md", "txt", "toml", "sh", "yaml", "yml", "json",
                ]
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(ext_str)),
                // Also scan small files without extensions (e.g., Makefile-like files).
                None => path
                    .metadata()
                    .map(|m| m.len() <= EXTENSIONLESS_TEXT_MAX_BYTES)
                    .unwrap_or(false),
            };

            if should_scan {
                let content = match fs::read_to_string(path) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Warning: Failed to read file {}: {}", path.display(), e);
                        continue;
                    }
                };

                let file_reqs = extract_reqs(&content, &re_req);
                for req in file_reqs {
                    if !matrix_reqs.contains(&req) {
                        eprintln!(
                            "Error: Undefined REQ ID '{}' found in {}",
                            req,
                            path.display()
                        );
                        errors = true;
                    }
                }
            }
        }
    }

    if errors {
        anyhow::bail!("Traceability Linter found errors");
    }

    println!("Traceability Linter Passed");
    Ok(())
}
