# Security and malformed-input smoke

`folio-core/tests/security_regression.rs` is the bounded, deterministic gate
for the current release. It exercises ZIP traversal, macro-enabled DOCX,
malformed XML/container inputs, and 32 mutations of each seed while asserting
that every public import path fails closed or returns a normal result without a
panic.

This is not a replacement for long-running coverage-guided fuzzing. A local
environment may run `cargo-fuzz` against the same adapter entry points; such a
campaign is not a 0.1 clean-clone gate.
