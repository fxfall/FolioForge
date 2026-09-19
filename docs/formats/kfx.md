# KFX

FolioForge treats KFX input and the internal KFX writer identity separately.

## Real KFX input

`folio-kfx` enters the native KFX container/Ion path for DRM-free reflowable
books. It recovers content, styles, resources, navigation and anchor evidence
into the shared IR. It records parser warnings, unresolved relationships and
input loss in the conversion report. A relationship is accepted only when the
native source structure and the semantic path agree; string-pool matches,
numeric field IDs and resource hashes are not enough.

Protected content is rejected without decryption. Fixed-layout/comic KFX,
device page geometry and other proprietary presentation variants are outside
the 0.1 input promise.

## FolioForge KFX compatibility output

The KFX target writer currently produces an explicit FolioForge compatibility
container used for deterministic writer/round-trip and fixture tests. Its
signature is intentionally distinguishable from Amazon KFX. It must not be
described as an Amazon-publishing KFX writer or used as proof that arbitrary
Amazon KFX can be generated.

## External references

Kindle Previewer, Calibre KFX Input and Bōkō are private compatibility
references. They can expose a missing test case, but raw KFX source evidence
and the IR contract decide meaning. Their executables and book outputs are not
runtime or CI dependencies.
