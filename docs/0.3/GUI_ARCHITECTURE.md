# FolioForge 0.3 Desktop GUI Architecture

Status: local implementation complete; hosted and interactive visual validation pending
Target product version: `0.3.0`

This document turns the 0.3 Internationalized GUI and Cross-Platform Slint
Frontend specification into the permanent feature-portability contract. It is
additive: the macOS client remains SwiftUI, existing format/Core behavior stays
authoritative, and no Reader or Comic algorithms move into either frontend.

## Ownership

```text
SwiftUI (macOS) ── Swift adapter ── C FFI ──┐
                                            ├── Rust Core / folio-reader / folio-comic
Slint (Windows/Linux) ── Rust adapter ──────┘
```

- Core owns input detection, supported formats and targets, conversion
  defaults, capability decisions, Semantic IR, diagnostics, progress,
  cancellation, Reader semantics, Comic identity/order/preview/editing and
  output behavior.
- Each GUI owns transient view state, dialogs, accessibility, keyboard or
  pointer gestures, layout, resource-to-widget transport, and localized text.
- The macOS frontend continues to call the public FFI. The Rust-native Slint
  frontend calls public `folio-core` APIs directly; it must not call the C ABI
  back into its own process.
- Do not add a presentation-contract crate unless a concrete parity gap cannot
  be solved by reusing the existing Core/FFI/Reader/Comic DTOs.
- Core and format crates remain locale-independent. Machine-readable error
  codes and diagnostic codes are never translated or rewritten.

## Portability contract for each feature

Before implementation, document the following in the feature change:

| Contract field | Conversion | Reader | Comic |
| --- | --- | --- | --- |
| State | Sources, Core descriptors, options, output selection, progress and result | Core Reader session, location, current render model and diagnostics | Core Comic session, page DTOs, Reader projection, selected page and output result |
| Actions | Open, inspect/preflight, select Core target/options, choose output, convert, cancel | Open, first/previous/next/last, select a Core navigation target, change viewport/direction/spread mode | Open/close, select Core page, navigate through Reader, preview, convert supported output, cancel |
| Events/results | Structured Core report, diagnostics and progress stage | Reader location/model or typed Reader/Core error | Core page/preview/output DTO, diagnostics and progress |
| Capabilities | Only `FormatRegistry` and Core preflight | Only `folio-reader`/Core | Only `folio-comic`/Core and existing Reader |
| Localized presentation | GUI keys map stage/severity/code and parameters to locale resources | GUI keys map typed Reader error/diagnostics to locale resources | GUI keys map typed Comic/Reader errors and progress to locale resources |

No frontend may infer reading order, compatibility, format support, device
profiles, crop geometry, or conversion defaults. Platform-specific dialogs
must return ordinary source/output paths to the same Core action.

## Localization

- SwiftUI uses the Apple String Catalog in the Swift package resource bundle.
  Keys are semantic and stable; source English is the development language and
  Simplified Chinese is the initial translation. Plural/parameter formatting
  belongs to localization resources, not string concatenation.
- Slint strings are marked with `@tr` and stable semantic translation
  contexts. Bundled gettext catalogs keep EN and `zh-Hans` resources available
  on all four target architectures without a runtime gettext installation.
- Only GUI-facing text is translated. File extensions, format IDs, Core error
  codes, diagnostic IDs, raw metadata and technical detail remain unchanged.
- Every key must have the source value, translation, context/comment where
  needed, and a passing missing-key/placeholder check.
- UI must tolerate long translations through flexible layout, wrapping,
  truncation only for secondary metadata, and localization stress fixtures.

## Slint target and feature parity

The official 0.3 Slint targets are Windows x86_64, Windows ARM64, Linux
x86_64, and Linux ARM64. macOS remains the official SwiftUI application.
Slint files define components, layout, bindings and callbacks. Rust adapter
code owns Core calls, asynchronous work, cancellation, state projection and
event-loop delivery. Use platform-appropriate Slint widget styling and native
file/folder dialogs; do not imitate macOS titlebar behavior on Windows/Linux.

Port in this order and compare each feature with its SwiftUI reference:

1. Shell, navigation, sidebar/main/inspector/status hierarchy.
2. Standard conversion, Core-provided formats/options, progress and cancel.
3. Structured diagnostics and localization.
4. Existing Reader API and render models.
5. Existing Comic Core page browser, Reader-backed preview and controls.
6. Settings and platform adaptations.

Behavioral A/B compares Core request values, actions, event/result meaning,
capability data, diagnostics codes and cancellation semantics. It does not
require pixel or widget identity.

## Required checks

- Validate both localization catalogs for missing/duplicate keys, untranslated
  required values, malformed placeholders, and placeholder mismatch.
- Compare normalized SwiftUI and Slint conversion requests for the same
  settings against the public Core request types.
- Exercise error, progress, cancellation, Reader navigation/resource identity,
  and Comic page identity/order via Core contracts.
- Build macOS ARM64 SwiftUI and all four Slint target architectures in CI.
- Keep maintainer-only regression fixtures and process records in ignored
  local directories; public docs may contain only sanitized contracts and
  aggregate results.
