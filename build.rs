use std::process::Command;
use std::path::Path;
use std::fs;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/repl.proto")?;
    tonic_build::compile_protos("proto/lsp.proto")?;

    // Ensure tree-sitter grammar is regenerated with named comments for LSP use
    ensure_rholang_parser_with_named_comments()?;

    // Embed build metadata for version tracking
    embed_build_metadata()?;

    Ok(())
}

fn embed_build_metadata() -> Result<(), Box<dyn std::error::Error>> {
    // Get git commit hash
    let git_hash = Command::new("git")
        .args(&["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
        .trim()
        .to_string();

    // Get git branch
    let git_branch = Command::new("git")
        .args(&["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
        .trim()
        .to_string();

    // Check if working directory is dirty
    let git_dirty = Command::new("git")
        .args(&["status", "--porcelain"])
        .output()
        .ok()
        .map(|output| !output.stdout.is_empty())
        .unwrap_or(false);

    // Generate build ID based on source code and dependencies
    // This changes when source files or Cargo.lock changes
    let source_hash = compute_source_hash();
    let build_id = if git_dirty {
        format!("{}-{}", git_hash, &source_hash[..8])
    } else {
        format!("{}-{}", git_hash, &source_hash[..8])
    };

    // Export as environment variables for the compiler
    println!("cargo:rustc-env=BUILD_GIT_HASH={}", git_hash);
    println!("cargo:rustc-env=BUILD_GIT_BRANCH={}", git_branch);
    println!("cargo:rustc-env=BUILD_GIT_DIRTY={}", if git_dirty { "-dirty" } else { "" });
    println!("cargo:rustc-env=BUILD_ID={}", build_id);

    // Only rerun build script if git state actually changes
    // This prevents unnecessary recompilation from timestamp changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    Ok(())
}

fn ensure_rholang_parser_with_named_comments() -> Result<(), Box<dyn std::error::Error>> {
    let tree_sitter_path = "../rholang-rs/rholang-tree-sitter";
    let grammar_path = Path::new(tree_sitter_path).join("grammar.js");
    let parser_path = Path::new(tree_sitter_path).join("src/parser.c");
    let marker_path = Path::new(tree_sitter_path).join(".named_comments_enabled");

    // Tell cargo to rerun if the grammar file changes
    println!("cargo:rerun-if-changed={}", grammar_path.display());
    println!("cargo:rerun-if-changed={}", marker_path.display());

    // Check if regeneration is needed:
    // 1. Marker file doesn't exist (indicates parser wasn't built with named comments)
    // 2. Parser doesn't exist
    // 3. Grammar is newer than parser
    let needs_regeneration = !marker_path.exists()
        || !parser_path.exists()
        || is_file_newer(&grammar_path, &parser_path)?;

    if needs_regeneration {
        println!("cargo:warning=Regenerating Tree-Sitter grammar with named comments enabled...");

        // Check if tree-sitter CLI is available
        let tree_sitter_check = Command::new("npx")
            .args(&["tree-sitter", "--version"])
            .output();

        if tree_sitter_check.is_err() {
            println!("cargo:warning=tree-sitter CLI not found via npx, trying direct command...");
            let tree_sitter_direct = Command::new("tree-sitter")
                .args(&["--version"])
                .output();

            if tree_sitter_direct.is_err() {
                return Err(
                    "tree-sitter CLI not found. Install it with: npm install -g tree-sitter-cli".into()
                );
            }
        }

        // Regenerate the grammar with RHOLANG_NAMED_COMMENTS=1
        let mut cmd = Command::new("npx");
        cmd.args(&["tree-sitter", "generate"])
            .current_dir(tree_sitter_path)
            .env("RHOLANG_NAMED_COMMENTS", "1");

        let status = cmd.status()?;

        if !status.success() {
            return Err("Failed to regenerate tree-sitter grammar with named comments".into());
        }

        // Create marker file to indicate successful regeneration with named comments
        fs::write(&marker_path, "named-comments-enabled\n")?;

        println!("cargo:warning=Tree-sitter grammar regenerated successfully with named comments");
    } else {
        println!("cargo:warning=Tree-sitter grammar is up-to-date with named comments");
    }

    // Verify that named comments are actually enabled in the parser
    verify_named_comments_enabled(tree_sitter_path)?;

    Ok(())
}

fn verify_named_comments_enabled(tree_sitter_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:warning=Verifying named comments are enabled...");

    // Create a temporary test file with both comment types
    let test_file = Path::new(tree_sitter_path).join(".test_named_comments.rho");
    let test_code = "// line comment\n/* block comment */\nNil";
    fs::write(&test_file, test_code)?;

    // Parse the test file and capture the parse tree
    let output = Command::new("npx")
        .args(&["tree-sitter", "parse", ".test_named_comments.rho"])
        .current_dir(tree_sitter_path)
        .output();

    // Clean up test file immediately
    let _ = fs::remove_file(&test_file);

    let output = output?;

    if !output.status.success() {
        return Err("Failed to parse test file for verification".into());
    }

    let parse_tree = String::from_utf8_lossy(&output.stdout);

    // Check if line_comment and block_comment appear as named nodes in the parse tree
    // When named comments are enabled, they appear as: (line_comment) and (block_comment)
    // When disabled, they're just unnamed nodes and won't appear in the tree output
    let has_line_comment = parse_tree.contains("line_comment");
    let has_block_comment = parse_tree.contains("block_comment");

    if !has_line_comment || !has_block_comment {
        let error_msg = format!(
            "Named comments verification failed!\n\
             Expected both 'line_comment' and 'block_comment' nodes in parse tree.\n\
             Found: line_comment={}, block_comment={}\n\
             Parse tree:\n{}",
            has_line_comment, has_block_comment, parse_tree
        );
        return Err(error_msg.into());
    }

    println!("cargo:warning=✓ Verified: Named comments are enabled (line_comment and block_comment nodes found)");
    Ok(())
}

fn is_file_newer(file1: &Path, file2: &Path) -> Result<bool, std::io::Error> {
    let metadata1 = fs::metadata(file1)?;
    let metadata2 = fs::metadata(file2)?;
    Ok(metadata1.modified()? > metadata2.modified()?)
}

fn compute_source_hash() -> String {
    let mut hasher = DefaultHasher::new();

    // Hash Cargo.lock to detect dependency changes
    if let Ok(cargo_lock) = fs::read_to_string("Cargo.lock") {
        cargo_lock.hash(&mut hasher);
    }

    // Hash Cargo.toml to detect direct dependency changes
    if let Ok(cargo_toml) = fs::read_to_string("Cargo.toml") {
        cargo_toml.hash(&mut hasher);
    }

    // Hash all source files
    if let Ok(entries) = fs::read_dir("src") {
        let mut files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "rs"))
            .collect();
        files.sort_by_key(|e| e.path());

        for entry in files {
            if let Ok(contents) = fs::read_to_string(entry.path()) {
                entry.path().to_string_lossy().hash(&mut hasher);
                contents.hash(&mut hasher);
            }
        }
    }

    // Tell Cargo to rerun when these files change
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=src");

    format!("{:016x}", hasher.finish())
}
