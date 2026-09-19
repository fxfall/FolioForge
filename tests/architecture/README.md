# Architecture checks

Run `python3 tests/architecture/check_boundaries.py`. It checks Cargo
workspace edges and prevents semantic/client crates from becoming a second
format-capability authority. CI runs it before the Rust test suites.
