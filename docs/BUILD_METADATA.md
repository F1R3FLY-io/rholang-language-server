# Build Metadata and Version Tracking

The rholang-language-server embeds build metadata directly into the compiled binary to help track which version is running.

## What's Embedded

Each build includes:
- **Git commit hash** (8 chars): Short hash of the commit used to build
- **Git branch**: The branch name at build time
- **Git dirty flag**: Whether there were uncommitted changes
- **Build timestamp**: When the binary was compiled (UTC)
- **Build ID**: A unique 8-character hex identifier for this specific build

## How It Works

The `build.rs` script runs during compilation and:
1. Queries git to get the current commit, branch, and working directory status
2. Generates a build timestamp and unique build ID
3. Embeds these values as compile-time constants using `cargo:rustc-env`

The `env!()` macro reads these **at compile time** and embeds them as string literals in the binary. Despite the name, these are **not** runtime environment variables - they are compile-time constants baked directly into the executable.

Example: When you run `env!("BUILD_ID")`, the Rust compiler replaces it with the actual string `"ae12bab6"` in the compiled code. The binary contains this string and doesn't need to read any environment at runtime.

### Viewing Build Metadata

**Via command line:**
```bash
$ rholang-language-server --version
rholang-language-server 0.1.0
Build: c1a0a24f (dylon/metta-integration-dirty) built at 2025-10-28 03:03:02 UTC
Build ID: d129ce1d
```

**Via server logs:**
```
[INFO] Initializing rholang-language-server with log level debug ...
[INFO] Build: c1a0a24f (dylon/metta-integration-dirty) built at 2025-10-28 03:03:02 UTC [build-id: d129ce1d]
```

## Checking Version Mismatch

If you experience issues and suspect you're running an old build, use the version check script:

```bash
# Check if the latest log matches the current build
./scripts/check-server-version.sh

# Check a specific log file
./scripts/check-server-version.sh ~/.cache/f1r3fly-io/rholang-language-server/session-20251028-024319-4009700.log
```

### Example: Version Mismatch

```
$ ./scripts/check-server-version.sh
Extracting build metadata from current binary...
Current build ID: ae12bab6
  Full info: Build: c1a0a24f (dylon/metta-integration-dirty) built at 2025-10-28 02:55:24 UTC [build-id: ae12bab6]
Checking log: /home/user/.cache/f1r3fly-io/rholang-language-server/session-20251028-024319-4009700.log
Running server build ID: 3392e153
✗ Server is running an older build
  Latest build:  ae12bab6
  Running build: 3392e153

Recommendation: Rebuild and restart the language server
  cargo build --release
  # Restart VSCode or kill the server process
```

### Example: Version Match

```
$ ./scripts/check-server-version.sh
Extracting build metadata from current binary...
Current build ID: ae12bab6
  Full info: Build: c1a0a24f (dylon/metta-integration-dirty) built at 2025-10-28 02:55:24 UTC [build-id: ae12bab6]
Checking log: /home/user/.cache/f1r3fly-io/rholang-language-server/session-20251028-025818-4044906.log
Running server build ID: ae12bab6
✓ Server is running the latest build
```

## Interpreting Build Metadata

- **Clean build**: `c1a0a24f (main) built at 2025-10-28 02:55:24 UTC`
  - Built from commit c1a0a24f on main branch with no uncommitted changes

- **Dirty build**: `c1a0a24f (dylon/metta-integration-dirty) built at 2025-10-28 02:55:24 UTC`
  - Built with uncommitted changes (indicated by `-dirty` suffix)
  - Useful for development but should not be deployed

## Build ID Uniqueness

The build ID is a hash of the build timestamp and git commit. This ensures:
- Different commits produce different IDs
- Rebuilding the same commit at different times produces different IDs
- Uncommitted changes result in different IDs due to different timestamps

This makes it easy to definitively identify which binary is running, even when working with multiple development builds.

## VSCode Integration

When VSCode starts the language server, it automatically logs the build metadata. Check the logs to see:
- Which commit the server was built from
- Whether there were uncommitted changes
- Exactly when it was compiled

Log location: `~/.cache/f1r3fly-io/rholang-language-server/session-*.log`
