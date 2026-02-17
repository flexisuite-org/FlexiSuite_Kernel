use std::fs;
use std::path::Path;
use std::collections::HashSet;
use walkdir::WalkDir;
use regex::Regex;
use anyhow::{Result, Context};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = ".")]
    path: String,
}

fn extract_reqs(content: &str, regex: &Regex) -> HashSet<String> {
    regex.find_iter(content)
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
    let matrix_content = fs::read_to_string(&matrix_path).context("Failed to read verification_matrix.md")?;
    let matrix_reqs = extract_reqs(&matrix_content, &re_req);

    // 2. Load Implementation Plan REQs
    let impl_path = root.join("docs/implementation_plan.md");
    let impl_reqs = if impl_path.exists() {
        let impl_content = fs::read_to_string(&impl_path).context("Failed to read implementation_plan.md")?;
        extract_reqs(&impl_content, &re_req)
    } else {
        HashSet::new()
    };

    let mut errors = false;

    // 3. Check Consistency (Matrix == Impl)
    // The existing script enforced strict equality. We will maintain that.
    // However, if implementation_plan doesn't exist, we might skip or warn.
    // Assuming strict equality if impl exists.
    if impl_path.exists() {
        let missing_in_matrix: Vec<_> = impl_reqs.difference(&matrix_reqs).collect();
        let missing_in_impl: Vec<_> = matrix_reqs.difference(&impl_reqs).collect();

        if !missing_in_matrix.is_empty() {
            eprintln!("Error: REQs found in implementation_plan but missing in verification_matrix:");
            for req in missing_in_matrix {
                eprintln!("  - {}", req);
            }
            errors = true;
        }

        if !missing_in_impl.is_empty() {
            eprintln!("Error: REQs found in verification_matrix but missing in implementation_plan:");
            for req in missing_in_impl {
                eprintln!("  - {}", req);
            }
            errors = true;
        }
    }

    // 4. Scan Codebase
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();

        // Skip hidden directories, target, source docs, and linters source
        if path.components().any(|c| c.as_os_str() == "target") ||
            // Check for ops/linters sequence more robustly
            path.components().collect::<Vec<_>>().windows(2).any(|w| w[0].as_os_str() == "ops" && w[1].as_os_str() == "linters") ||
            path.components().any(|c| { // Hidden files
                let s = c.as_os_str().to_string_lossy();
                s.starts_with('.') && s != "." && s != ".."
            }) ||
           path == matrix_path ||
           path == impl_path {
            continue;
        }

        if path.is_file() {
            // Check text files only roughly based on extension or content
            // We'll trust utf8 read for now
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy();
                // Whitelist text extensions to skip binary files
                if ["rs", "sql", "md", "txt", "toml", "sh", "yaml", "yml", "json"].contains(&ext_str.as_ref()) {
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
                            eprintln!("Error: Undefined REQ ID '{}' found in {}", req, path.display());
                            errors = true;
                        }
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
