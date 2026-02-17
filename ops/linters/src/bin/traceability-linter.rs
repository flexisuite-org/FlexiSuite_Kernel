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

    // REQ ID must consist of alphanumeric segments separated by single hyphens
    let re_req = Regex::new(r"REQ-[A-Z0-9]+(?:-[A-Z0-9]+)*").unwrap();

    // 1. Load Matrix REQs
    let matrix_path = root.join("docs/verification_matrix.md");
    if !matrix_path.exists() {
        eprintln!("Error: docs/verification_matrix.md not found");
        std::process::exit(1);
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
        if path.components().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            s.starts_with('.') && s != "." && s != ".."
        }) || path.components().any(|c| c.as_os_str() == "target") ||
           path == matrix_path ||
           path == impl_path ||
           path.to_string_lossy().contains("ops/linters") {
            continue;
        }

        if path.is_file() {
            // Check text files only roughly based on extension or content
            // We'll trust utf8 read for now
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

    if errors {
        std::process::exit(1);
    }

    println!("Traceability Linter Passed");
    Ok(())
}
