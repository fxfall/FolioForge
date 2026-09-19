# KFX resource identity regression

This suite protects the KFX invariant that only an exact native identity may
bind an external resource to raw media. The production key is the decoded
`$165 location` matched byte-for-byte with a `$417 fid` across the complete
parsed file set. Dimensions, media type, hashes, ordering, and proximity are
diagnostic evidence only.

The repository contains no copyrighted book fixtures. Synthetic identity,
symbol-scope, duplicate-key, negative-candidate, and cross-container cases live
in the `folio-kfx` unit tests. Local DRM-free inputs may be audited without
copying them into the repository:

```sh
cargo build --locked --offline -p folio-cli
python3 tests/scripts/audit_kfx_resource_identity.py \
  /path/to/local/kfx-corpus \
  --output /path/to/private/kfx-resource-identity.json
```

Set `FOLIOFORGE_KFX_CORPUS` or pass an external corpus path explicitly. The
script prints only counts and writes anonymized IDs, source-content SHA-256
fingerprints, container/entity totals, fragment observations, and classifications;
it never records source paths, book names, metadata, or text.

`fixtures-private/` is ignored by Git. Do not add copyrighted books, extracted
book text, or raw media to tracked fixtures. `expected/` documents synthetic
unit-test expectations; reproducible corpus and oracle reports belong in the
external validation workspace, with any corpus-specific report verified for
privacy before it is considered for publication.

Current product support and identity boundaries are documented in
[`../../docs/0.1/FORMAT_SUPPORT.md`](../../docs/0.1/FORMAT_SUPPORT.md) and
[`../../docs/formats/kfx.md`](../../docs/formats/kfx.md). Local anonymized
reports are release inputs only when explicitly reviewed and must remain
outside the public tree.
