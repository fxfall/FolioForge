# FolioForge development contract

For 0.1 maintenance, read `docs/0.1/DEVELOPMENT.md` first; it remains the
frozen 0.1 source of truth. For 0.2 comic work, read
`docs/0.2/COMIC_CORE_DEVELOPMENT.md` and its linked architecture and editing
contracts. For 0.2.1 Reader work, also read
`docs/0.2/READER_RUNTIME.md`. The 0.1 freeze applies to the 0.1 release line;
0.2 work is additive and must preserve existing 0.1 behavior.

For any change that touches conversion, the Semantic IR, an importer, a
compatibility rule, an exporter, FFI, batch execution, or the service boundary:

1. Read the affected contract in `docs/0.1/DEVELOPMENT.md` and
   `docs/0.1/API.md`.
2. Read the format contract under `docs/formats/` or the Library contract under
   `docs/library/` for the affected layer.
3. Read `docs/0.1/AUDIT.md`, `docs/0.1/KNOWN_LIMITS.md` and
   `CHANGELOG.md` before accepting a release-bound change.
4. Inspect only the source files required for the change. Do not start with an
   unbounded repository-wide refactor.
5. State the intended boundary in the change notes or pull request.
6. Classify the change as major or minor under the frozen-scope rules in
   `docs/0.1/DEVELOPMENT.md`; record the classification in the audit/change
   notes.
7. Run targeted Rust tests from the ignored repository-root `tests/` tree and
   the canonical local validation bundle when available. Test sources and
   fixtures must not be tracked or pushed to GitHub.
8. Run the required A/B comparison before accepting a behavior-changing
   change, then update the behavior/performance baseline and API changelog.
9. Remove temporary code and keep build/scratch data under the path selected by
   `FOLIOFORGE_VALIDATION_ROOT`, outside the repository.

Local regression tests and fixtures live in the ignored repository-root
`tests/` directory. Python/oracle tooling, private comparison notes, benchmark
seeds and chronological process records live in the ignored `.folioforge-dev/`
directory; project status lives in ignored `.codex/`. These directories are
inside this checkout but are excluded from GitHub, release archives and Docker
build contexts. Runtime code must never import or depend on them.

If the implementation and a contract disagree, stop and record the mismatch as
Architecture Drift before deciding whether the code or the document is wrong.
The document is a design/behavior contract; it is not a reason to hide a
regression.

The 0.1 release is frozen: do not change its product scope or claim new 0.1
capabilities. FolioForge 0.2 adds only the comic/manga Core scope defined in
`docs/0.2/`; it does not add Library or Reader features, pagination,
annotations, cloud sync, or a second format-specific exporter pipeline. Client
layers consume Core/Reader contracts; they do not become capability or
compatibility authorities. Private project status and process notes live in
ignored `.codex/`; maintainer-only tests and fixtures live in ignored root
`tests/`; private tools, benchmarks and chronological process logs live in
ignored `.folioforge-dev/`.
