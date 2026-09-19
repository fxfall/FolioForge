# Expected results

Expected behavior for synthetic cases is asserted by the `folio-kfx` tests:

- exact `$165 location` / `$417 fid` identity resolves once;
- a resource in another container of the same file set is visible to lookup;
- duplicate exact identities remain unresolved;
- equal dimensions, MIME, byte length, bytes, or nearby ordering never create a
  binding;
- numeric symbol IDs stay scoped to their own container and content-symbol
  offsets are tested separately from fragment-identity IDs.
