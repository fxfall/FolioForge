# FolioForge 0.1 Provenance Audit

Audit date: 2026-09-19  
Audited revision: `4dc28714aa0f48c8e0d07a9244282aa79270020`  
Scope: the tracked public tree and the local Git object database. Private book
corpora, generated build roots and the external historical archive are outside
the public tree and were not copied into this repository.

## Conclusion

No Calibre, Bōkō, Kindle Previewer, or other converter source file, executable,
patch, archive, or extracted book bytes were found in the tracked tree.

The repository contains FolioForge implementation code, public contracts, the
user-supplied FolioForge logo, and normal crates.io dependencies declared
through Cargo. Synthetic fixtures and external-oracle scripts are kept in the
ignored local `.folioforge-dev/` bundle. Calibre and Bōkō are used as explicitly
external compatibility references/oracles; they are not runtime dependencies
and are not bundled.

This is a repository-content audit, not a formal source-similarity proof
against every historical release of every external project. The repository was
initialized locally during 0.1 consolidation, so no upstream development
history is available for a line-by-line ancestry claim.

## Evidence

### 1. Tracked-tree inventory

- The tracked tree was audited after the local validation bundle was removed
  from the public checkout.
- No tracked `vendor/`, `third_party/`, `upstream/`, `calibre/` or `boko/`
  source directory exists.
- The only tracked project license file is the FolioForge MIT `LICENSE`.
- No tracked patch/diff file, foreign copyright header, SPDX notice or GPL
  source header was found in Rust, Swift, Python or shell source.
- The repository audit passed for private-corpus names, generated output,
  machine-specific absolute paths, high-signal credentials and oversized
  files.

### 2. Calibre/Bōkō/Kindle references

References occur in documentation, the ignored local oracle bundle and a small
number of behavior-reference comments. Examples are:

- `.folioforge-dev/tools/dev/kfx_probe.py` invokes installed Kindle Previewer
  and Calibre only when a developer explicitly runs the probe.
- `.folioforge-dev/tools/oracle/kfx/` contains comparison and evidence scripts
  that invoke externally supplied Calibre/Bōkō executables; generated book
  output is kept outside the repository.
- `docs/0.1/FORMAT_SUPPORT.md` describes them as compatibility references, not
  authorities or dependencies.
- Comments in `crates/folio-kfx/src/amazon/mod.rs` and
  `crates/folio-epub/src/lib.rs` identify observed behavior being compared.

The runtime Rust and Swift source has no Calibre/Bōkō import, subprocess
invocation, vendored module, copied class, or embedded converter payload. The
runtime manifests contain only FolioForge workspace paths and crates.io
dependencies.

### 3. Cargo dependency provenance

`Cargo.lock` contains 198 package records: FolioForge workspace packages and
registry packages from `https://github.com/rust-lang/crates.io-index`. There
are no Git URL dependencies, local paths outside the FolioForge workspace, or
vendored registry source trees. Cargo checksums remain in the lockfile; the
third-party crate source is resolved by Cargo at build time and is not copied
into this repository.

The workspace MIT declaration applies to FolioForge code. Each external crate
retains its own license and notice obligations when distributing binaries;
those obligations are not converted into FolioForge source ownership.

### 4. Non-text files

- `assets/folioforge-logo.png` and
  `macos/FolioForge/Resources/folioforge-logo.png` are byte-identical
  (`SHA-256 6030e378edb0cd42c5c63b2ddc3ab091fdb86e850073f9243ec5bbf6d47bc5c9`)
  copies of the logo supplied for this project.
- Synthetic EPUB/SVG/KFX probes and the deterministic font fixture live only
  under `.folioforge-dev/`. No private book, Calibre EPUB, Bōkō EPUB, KPF or
  KFX output is tracked.

## Limitations and follow-up

This audit cannot prove that no independently written line resembles an
external implementation. If a future contributor imports upstream code, they
must add the original project, version/commit, license, files and modification
notice to a dedicated provenance record before committing it. External
converter outputs must remain in an ignored, user-selected validation root.
