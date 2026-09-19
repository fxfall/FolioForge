# Cross-client parity fixture

`input.epub`, `edit-plan.json`, and `conversion.json` form the shared source,
edit, and conversion request. `run.sh` exercises the CLI, the same C ABI used by
the macOS bridge, and the local Web/service API. It compares stable Core report
fields, re-inspected Semantic IR, and deterministic output bytes.

The test is local-only. It starts a temporary service with online providers
disabled and stores all generated outputs and job data in a temporary directory.
