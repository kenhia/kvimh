# Windows: `just` runs recipes through `sh`, which Windows does not ship — put
# Git for Windows' `usr\bin` on PATH (it holds `sh.exe`) or run from Git Bash.
# (Upstream's own requirement: "sh must be available in the PATH".)

# List available recipes
default:
    @just --list

# Run CI gates (lint, typecheck, tests)
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

# Apply formatting
fmt:
    cargo fmt
