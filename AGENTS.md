# FolioForge 0.1 development contract

Read `docs/0.1/DEVELOPMENT.md` first. It is the current 0.1 source of truth;
historical stage notes are kept outside the public working tree and are not
authoritative.

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
7. Run the targeted Rust tests and the canonical local validation bundle when
   it is available; the bundle is intentionally outside the public checkout.
8. Run the required A/B comparison before accepting a behavior-changing
   change, then update the behavior/performance baseline and API changelog.
9. Remove temporary code and keep build/scratch data under the path selected by
   `FOLIOFORGE_VALIDATION_ROOT`, outside the repository.

The maintainer-only validation bundle lives in the ignored `.folioforge-dev/`
directory. It contains Python/oracle tooling, synthetic corpora, private
comparison notes and development records. It is not part of the public GitHub
checkout, release archives or Docker build context. Runtime code must never
import or depend on it.

If the implementation and a contract disagree, stop and record the mismatch as
Architecture Drift before deciding whether the code or the document is wrong.
The document is a design/behavior contract; it is not a reason to hide a
regression.

The 0.1 release is frozen: do not add formats, Library features, Reader
features, pagination, annotations, cloud sync, or a second conversion pipeline.
Client layers consume Core contracts; they do not become capability or
compatibility authorities.
