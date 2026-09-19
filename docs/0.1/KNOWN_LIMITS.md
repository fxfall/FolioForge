# FolioForge 0.1 Known Limits

## Supported with diagnostics

- Real KFX input is limited to DRM-free, reflowable content whose native
  container can be parsed. The importer preserves visible semantics when the
  source relationship is proven and records `input_loss` or unresolved
  diagnostics otherwise.
- KFX field identifiers, symbols, positions and resource hashes are not
  semantic proof by themselves. A visible string is never recovered from a
  general string pool merely because it matches output text.
- Plain-text chapter boundaries and some legacy markup decisions are
  heuristics. The selected policy is included in the text import report.
- Target-specific layout, font and complex-table behavior can be an
  approximation even when the text is exact.

## Not exercised by the public release gate

- Private real-book KFX corpus comparisons, Calibre plugin output and Bōkō
  comparisons. These are local research inputs and are never required by a
  clean clone or GitHub CI.
- Kindle Previewer, proprietary KPF/KFX authoring tools and device screenshots.
- Very large Library scale runs and private benchmark books.
- Developer ID signing, notarization and Gatekeeper acceptance.

## Unsupported or out of scope

- DRM decryption, key recovery or bypass. Protected content is rejected.
- Fixed-layout KFX, comic/manga page composition and device-specific page
  rendering as a general import promise.
- Cloud synchronization, remote book storage, reader pagination/annotations
  and a second conversion pipeline.
- A claim that FolioForge's internal KFX compatibility container is an Amazon
  production KFX file.

## User-visible behavior

Strict mode fails on an unrepresentable required feature. Compatible mode
chooses a declared target fallback and reports it. Readable mode allows a
broader approximation. Diagnostics are part of the conversion result and must
not be replaced by a silent guess.
