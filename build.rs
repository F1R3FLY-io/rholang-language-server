use std::process::Command;
use std::path::Path;
use std::fs;

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

    // Get build timestamp
    let build_timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();

    // Generate a unique build ID (first 8 chars of hash of timestamp + git hash)
    let build_id = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        format!("{}{}", build_timestamp, git_hash).hash(&mut hasher);
        format!("{:08x}", hasher.finish() & 0xFFFFFFFF)
    };

    // Export as environment variables for the compiler
    println!("cargo:rustc-env=BUILD_GIT_HASH={}", git_hash);
    println!("cargo:rustc-env=BUILD_GIT_BRANCH={}", git_branch);
    println!("cargo:rustc-env=BUILD_GIT_DIRTY={}", if git_dirty { "-dirty" } else { "" });
    println!("cargo:rustc-env=BUILD_TIMESTAMP={}", build_timestamp);
    println!("cargo:rustc-env=BUILD_ID={}", build_id);

    // Rerun if git state changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    println!("cargo:warning=Build metadata: {} ({}{}) [{}]",
             git_hash,
             git_branch,
             if git_dirty { "-dirty" } else { "" },
             build_id);

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
