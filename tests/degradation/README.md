# Degradation golden corpus

These fixtures exercise the semantic compatibility planner rather than
comparing opaque output bytes.  Each directory contains a source XHTML file,
an `expected.json` assertion, and optional CSS/resources.  The runner wraps the
source in a deterministic EPUB package and checks the planned feature,
compatibility quality, selected fallback, and diagnostic path.

Run from the repository root after building the CLI:

```bash
cargo build -p folio-cli
tests/scripts/run_degradation_golden.sh
```

The expected quality names map directly to `QualityLevel`:

| Quality | Meaning |
| --- | --- |
| `Exact` | L0 native representation |
| `Equivalent` | L1 equivalent target representation |
| `CompatibleApproximation` | L2 compatible approximation |
| `StructuralFallback` | L3 readable structural fallback |
| `Drop` | L4 last-resort removal, always diagnosed |

The current KF7 SVG fixture deliberately expects an accessible alt-text
fallback because this repository does not claim to contain a binary SVG
rasterizer.  A future rasterizer may add a new deterministic golden case; it
must change both the selected representation and its quality together.
