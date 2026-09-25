//! Clean-room Amazon KFX input frontend.
//!
//! The public compatibility container remains in `lib.rs` for old internal
//! fixtures. Real files beginning with `CONT` enter this module only.

mod fragment;
mod ion;
mod resource;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    convert::TryFrom,
};

use folio_model::{
    Anchor, AnchorEdge, AnchorId, AnchorRelation, Book, ComputedStyle, Confidence, Document,
    DocumentId, MemoryResourceLoader, Metadata, NavPoint, Node, NodeId, NodeKind,
    PresentationFeature, PresentationIntent, Resource, ResourceId, ResourceKind, SemanticRole,
    StyleId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

use self::fragment::FragmentKey;
use self::ion::{as_int, as_string, decode_one, struct_get, IonError, IonValue, VERSION_MARKER};
pub use self::resource::KfxResourceDiagnostic;
use self::resource::{
    classify_resource_shape, normalize_kfx_resource_path, resolve_native_resource_paths,
    validate_resource_resolution, NativeResourcePathResolution, ResourceBinding,
};

const CONT_HEADER_MIN: usize = 18;
const CONT_VERSION: u16 = 2;
const ENTITY_INDEX_ENTRY: usize = 24;
const ENTY_HEADER_MIN: usize = 10;
const ENTY_HEADER_LEN: usize = 21;
const MAX_ENTITIES: usize = 1_000_000;
const MAX_STRING_ITEMS: usize = 200_000;

// These are system-table IDs observed in the public Ion data model and used
// by the CONT framing itself. KFX content field IDs stay numeric in the
// Native model until a decoder has enough evidence to assign a semantic role.
const FIELD_CONTAINER_ID: u64 = 409;
const FIELD_COMPRESSION: u64 = 410;
const FIELD_DRM_SCHEME: u64 = 411;
const FIELD_CHUNK_SIZE: u64 = 412;
const FIELD_INDEX_OFFSET: u64 = 413;
const FIELD_INDEX_LENGTH: u64 = 414;
const FIELD_DOC_SYMBOL_OFFSET: u64 = 415;
const FIELD_DOC_SYMBOL_LENGTH: u64 = 416;
const FIELD_CAPABILITIES_OFFSET: u64 = 594;
const FIELD_CAPABILITIES_LENGTH: u64 = 595;
const FIELD_SYMBOL_TABLE_IMPORTS: u64 = 6;
const FIELD_SYMBOL_TABLE_SYMBOLS: u64 = 7;
const FIELD_SYMBOL_TABLE_MAX_ID: u64 = 8;
const FIELD_SYMBOL_TABLE_IMPORT_NAME: u64 = 4;
const FIELD_SYMBOL_TABLE_IMPORT_VERSION: u64 = 5;
const FIELD_SYMBOL_TABLE_IMPORT_MAX_ID: u64 = 8;
const ION_SYSTEM_SYMBOL_COUNT: u64 = 9;

const FIELD_METADATA_ENTRIES: u64 = 491;
const FIELD_METADATA_KIND: u64 = 495;
const FIELD_METADATA_PAIRS: u64 = 258;
const FIELD_METADATA_KEY: u64 = 492;
const FIELD_METADATA_VALUE: u64 = 307;
const FIELD_TEXT_VALUES: u64 = 146;
const FIELD_CONTENT_STRING_REFERENCE: u64 = 145;
const FIELD_TEXT_STYLE_EVENTS: u64 = 142;
const FIELD_TEXT_STYLE_OFFSET: u64 = 143;
const FIELD_TEXT_STYLE_LENGTH: u64 = 144;
// `$179` is the link target on a KFX content node or text-style event. The
// target symbol is resolved through the book's `$266` anchor fragments.
const FIELD_TEXT_STYLE_LINK_TARGET: u64 = 179;
const FIELD_TEXT_STYLE_KIND: u64 = 616;
const FIELD_CONTENT_FRAGMENT_SYMBOL: u64 = 176;
const FIELD_READING_ORDERS: u64 = 169;
const FIELD_READING_ORDER_SECTIONS: u64 = 170;
const FIELD_CONTENT_RESOURCE_SYMBOL: u64 = 157;
const FIELD_CONTENT_ORACLE_RESOURCE_SYMBOL: u64 = 175;
const FIELD_CONTENT_NODE_TYPE: u64 = 159;
// [PROBE-OBSERVED] Kindle Previewer stores a MathML occurrence as a `$683`
// pair: accessible text at index 0 and serialized MathML at index 1. The
// relationship is established by MATH-01/02/04, not by the number alone.
const FIELD_MATH_CONTENT: u64 = 683;
// KFX ruby content is stored as fragment type `$756`. Content style events
// reference that fragment through `$757` and identify an entry with `$758`.
// `$759` is the multi-range form used by some books.
const KFX_RUBY_CONTENT_FRAGMENT_TYPE: u32 = 756;
const FIELD_RUBY_FRAGMENT_SYMBOL: u64 = 757;
const FIELD_RUBY_ID: u64 = 758;
const FIELD_RUBY_ID_LIST: u64 = 759;
// [OBSERVED] `$790` is the source block heading level in the local Kindle
// corpus.  It is stronger than a style SID because style names are scoped to
// each book and are not a portable heading taxonomy.
const FIELD_CONTENT_HEADING_LEVEL: u64 = 790;
// [OBSERVED] In the C082 real-Kindle sample, node type 270 is an image
// container and type 269 is its caption/layout child. Bare strings are
// accepted only inside that proven image-caption subtree when the caption
// child also carries a non-empty `$142` text-style event list. This keeps
// table/grid labels such as C042's `共计`/`桶蜂蜜` out of body text.
const KFX_IMAGE_CONTAINER_NODE_TYPE: u64 = 270;
const KFX_IMAGE_CAPTION_NODE_TYPE: u64 = 269;
const KFX_IMAGE_RECORD_NODE_TYPE: u64 = 271;
const FIELD_SECTION_ENTRIES: u64 = 141;
const FIELD_FRAGMENT_TEXT: u64 = 584;
const FIELD_FRAGMENT_STRING_INDEX: u64 = 403;
const FIELD_FRAGMENT_ID: u64 = 4;
const FIELD_FRAGMENT_TARGET: u64 = 246;
const FIELD_FRAGMENT_LABEL: u64 = 244;
const FIELD_NAV_TARGET_ID: u64 = 155;
const FIELD_ANCHOR_NAME: u64 = 180;
const FIELD_ANCHOR_POSITION: u64 = 183;
const FIELD_ANCHOR_URI: u64 = 186;
const FIELD_NAVIGATION_TYPE: u64 = 235;
const FIELD_NAVIGATION_LANDMARK_TYPE: u64 = 238;
const FIELD_NAVIGATION_ROOT: u64 = 392;
const FIELD_NAVIGATION_LABEL: u64 = 241;
const FIELD_NAVIGATION_CHILDREN: u64 = 247;
const FIELD_NAVIGATION_ENTRY_SETS: u64 = 248;
const FIELD_NAVIGATION_REPRESENTATION: u64 = 241;
const KFX_NAVIGATION_TOC: u64 = 212;
const KFX_NAVIGATION_HEADINGS: u64 = 798;
const KFX_NAVIGATION_HEADING_LEVEL_BASE: u64 = 799;
const KFX_NAVIGATION_HEADING_LEVEL_MAX: u64 = 804;
const FIELD_RESOURCE_METADATA_PATH: u64 = 165;
const FIELD_RESOURCE_FORMAT: u64 = 161;
const FIELD_RESOURCE_MIME: u64 = 162;
const FIELD_RESOURCE_TILE_GRID: u64 = 636;
const FIELD_RESOURCE_OVERLAPPED_TILES: u64 = 797;
const FIELD_RESOURCE_WIDTH_FALLBACK: u64 = 66;
const FIELD_RESOURCE_HEIGHT_FALLBACK: u64 = 67;
// [OBSERVED] Across the local DRM-free KFX corpus these values match the
// decoded raster width and height respectively.
const FIELD_RESOURCE_PIXEL_WIDTH: u64 = 422;
const FIELD_RESOURCE_PIXEL_HEIGHT: u64 = 423;
// [OBSERVED] With the corpus' YJ_symbols@10#851 import, symbol 271 (offset
// 261 within that shared table) is used by direct-text resource placeholders.
const WHITESPACE_RESOURCE_RECORD_SYMBOL_OFFSET: u64 = 261;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseMode {
    Strict,
    #[default]
    Compatible,
    Recovery,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct InputReport {
    pub detected_format: String,
    pub parser_version: String,
    pub container_count: usize,
    pub entity_count: usize,
    pub fragment_count: usize,
    pub symbol_count: usize,
    pub resource_count: usize,
    pub document_count: usize,
    pub drm_detected: bool,
    pub warnings: Vec<String>,
    pub unknown_features: Vec<String>,
    pub recovery_actions: Vec<String>,
    pub input_loss: Vec<String>,
    #[serde(default)]
    pub resource_identity_diagnostics: Vec<KfxResourceDiagnostic>,
}

#[derive(Clone, Debug)]
pub struct AmazonKfxImport {
    pub book: Book,
    pub input: InputReport,
}

/// Privacy-conscious, deterministic observations for investigating KFX
/// resource identity. Digests are audit-only and are never used to bind an
/// external resource to media in production conversion.
#[derive(Clone, Debug, Serialize)]
pub struct KfxResourceAudit {
    pub schema_version: u32,
    pub source_id: String,
    pub parser_version: String,
    pub containers: Vec<KfxAuditContainer>,
    pub external_resources: Vec<KfxAuditExternalResource>,
    pub raw_media: Vec<KfxAuditRawMedia>,
    pub composite_fragments: Vec<KfxAuditCompositeFragment>,
    pub summary: KfxAuditSummary,
}

/// Content-free, occurrence-preserving trace of KFX image placements.
/// Resource aliases are local to this report; raw KFX names, text, and paths
/// are intentionally omitted. This is a diagnostic API, never an import rule.
#[derive(Clone, Debug, Serialize)]
pub struct KfxPlacementAudit {
    pub schema_version: u32,
    pub input_sha256_audit_only: String,
    pub resource_identity: KfxPlacementIdentitySummary,
    pub raw_reference: KfxPlacementStage,
    pub native_placement: KfxPlacementStage,
    pub semantic_placement: KfxPlacementStage,
    pub ir_image_nodes: KfxPlacementStage,
    pub epub_resource_map: Vec<KfxPlacementEpubResource>,
}

/// Content-free corpus report for checking KFX import coverage after parser
/// changes. It intentionally excludes metadata values other than a validated
/// language tag, body text, resource names/paths, and diagnostic messages.
#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelityAudit {
    pub schema_version: u32,
    pub parser_version: String,
    pub input_sha256_audit_only: String,
    pub input_bytes: u64,
    pub status: String,
    pub error_code: Option<String>,
    pub source: KfxFidelitySource,
    pub native: KfxFidelityNative,
    pub content: KfxFidelityContent,
    pub text: KfxFidelityText,
    pub inline: KfxFidelityInline,
    pub links: KfxFidelityLinks,
    pub navigation: KfxFidelityNavigation,
    pub resources: KfxFidelityResources,
    pub placements: KfxFidelityPlacements,
    pub styles: KfxFidelityStyles,
    pub fonts: KfxFidelityFonts,
    pub conditional: KfxConditionalEvidence,
    pub diagnostics: BTreeMap<String, usize>,
}

/// Content-free observations of the original Ion tree. This report is a
/// developer evidence surface for controlled KFX probes; it is deliberately not used by
/// semantic decoding. Numeric fields and symbols remain observations until a
/// controlled source relationship proves their meaning.
#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxSemanticEvidenceAudit {
    pub schema_version: u32,
    pub parser_version: String,
    pub input_sha256_audit_only: String,
    pub status: String,
    pub error_code: Option<String>,
    pub native: KfxSemanticEvidenceNative,
    pub candidate_field_occurrence_counts: BTreeMap<String, usize>,
    pub candidate_samples: Vec<KfxSemanticEvidenceSample>,
    pub conditional: KfxConditionalEvidence,
    pub render_inline: KfxRenderInlineEvidence,
    pub diagnostics: BTreeMap<String, usize>,
}

/// Detection-only evidence for S3. A candidate is reported without changing
/// the reading output; field numbers remain observations until a controlled
/// source relation proves their meaning.
#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxConditionalEvidence {
    pub candidate_field_occurrence_counts: BTreeMap<String, usize>,
    pub candidate_fragment_type_counts: BTreeMap<String, usize>,
    pub evaluation: Option<KfxConditionalEvaluation>,
    pub status: String,
}

/// A conditional branch is never selected by default. `Unknown` is the only
/// value emitted while S3 remains detection-only and the source relationship
/// has not been proved.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum KfxConditionalEvaluation {
    True,
    False,
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxRenderInlineEvidence {
    pub candidate_field_occurrence_counts: BTreeMap<String, usize>,
    pub candidate_fragment_type_counts: BTreeMap<String, usize>,
    pub status: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxSemanticEvidenceNative {
    pub fragment_count: usize,
    pub fragment_type_counts: BTreeMap<String, usize>,
    pub field_occurrence_counts_by_fragment: BTreeMap<String, usize>,
    pub field_value_kind_counts_by_fragment: BTreeMap<String, BTreeMap<String, usize>>,
    pub value_kind_counts_by_fragment: BTreeMap<String, BTreeMap<String, usize>>,
    pub annotation_symbol_counts: BTreeMap<String, usize>,
    pub scalar_string_count: usize,
    pub scalar_string_max_utf8_bytes: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxSemanticEvidenceSample {
    pub fragment_type: u32,
    pub container_index: usize,
    pub entity_id: u32,
    pub field_path: Vec<u64>,
    pub value_kind: String,
    pub list_item_count: Option<usize>,
    pub annotation_symbol_ids: Vec<u64>,
    /// Present only when the candidate field itself is a scalar symbol. This
    /// keeps the audit numeric and content-free while making probe evidence
    /// such as `$616 = 617` directly reviewable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_symbol_id: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelitySource {
    pub format: String,
    pub container_version_counts: BTreeMap<String, usize>,
    pub language: Option<String>,
    /// The current importer does not reliably decode source layout metadata.
    pub layout: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelityNative {
    pub container_count: usize,
    pub entity_count: usize,
    pub fragment_count: usize,
    pub fragment_type_counts: BTreeMap<String, usize>,
    pub unknown_fragment_count: usize,
    pub symbol_count: usize,
    pub string_table_count: usize,
    pub raw_resource_count: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelityContent {
    pub document_count: usize,
    pub content_fragment_count: usize,
    pub ir_node_counts: BTreeMap<String, usize>,
    pub special_feature_counts: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelityText {
    pub text_node_count: usize,
    pub unicode_scalar_count: usize,
    pub normalized_sha256_audit_only: String,
    pub source_text_segment_count: usize,
    pub unresolved_text_reference_count: usize,
    pub source_unicode_scalar_count: usize,
    pub source_normalized_sha256_audit_only: String,
    pub semantic_text_segment_count: usize,
    pub semantic_unicode_scalar_count: usize,
    pub semantic_normalized_sha256_audit_only: String,
    pub source_fragments: Vec<KfxFidelityTextUnit>,
    pub semantic_documents: Vec<KfxFidelityTextUnit>,
    pub ir_documents: Vec<KfxFidelityTextUnit>,
    pub normalization: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelityTextUnit {
    pub content_fragment_order: Option<usize>,
    pub document_order: Option<usize>,
    pub text_segment_count: usize,
    pub text_node_count: usize,
    pub unicode_scalar_count: usize,
    pub normalized_sha256_audit_only: String,
}

/// Developer-only KFX text provenance. Event metadata is content-free unless
/// a caller explicitly asks for private text in a local diagnostic run.
#[derive(Clone, Debug, Serialize)]
pub struct KfxTextEventAudit {
    pub schema_version: u32,
    pub input_sha256_audit_only: String,
    pub status: String,
    pub source_features: KfxTextSourceFeatures,
    pub stream: KfxTextStreamSummary,
    pub events: Vec<KfxTextEvent>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxTextSourceFeatures {
    pub has_stories: Option<bool>,
    pub story_definitions: usize,
    pub story_references: usize,
    pub stories_traversed_unique: usize,
    pub stories_emitted_more_than_once: usize,
    pub story_definitions_not_emitted: usize,
    pub has_conditional_content: Option<bool>,
    pub has_illustrated_layout: Option<bool>,
    pub has_image_occurrences: bool,
    pub has_footnotes: Option<bool>,
    pub has_non_image_render_inline: Option<bool>,
    pub has_ruby: Option<bool>,
    pub has_mathml: Option<bool>,
    pub creator: Option<String>,
    pub creator_version: Option<String>,
    /// Per-input disposition for semantic candidates observed in the native
    /// tree.  Corpus-level `NotExercised` is reported by the probe ledger;
    /// this map is intentionally limited to features encountered in this
    /// input so an absent feature is never mistaken for support.
    pub feature_states: BTreeMap<String, String>,
    pub undecoded_features: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxTextStreamSummary {
    pub event_count: usize,
    pub unresolved_text_reference_count: usize,
    pub raw_unicode_scalar_count: usize,
    pub raw_sha256_audit_only: String,
    pub normalized_unicode_scalar_count: usize,
    pub normalized_sha256_audit_only: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxTextEvent {
    pub trace_id: String,
    pub section_id: Option<String>,
    pub story_id: Option<String>,
    pub source_fragment: String,
    pub source_fragment_order: usize,
    pub eid: u32,
    pub source_path: String,
    pub source_value_kind: String,
    pub source_position: Option<u32>,
    pub source_position_kind: String,
    pub text_len: usize,
    pub text_hash_raw_audit_only: String,
    pub context: String,
    pub context_basis: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjacent_style_events: Vec<KfxTextStyleEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxTextStyleEvent {
    pub list_index: usize,
    pub text_offset: Option<i64>,
    pub text_length: Option<i64>,
    pub style_symbol_id: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelityInline {
    pub inline_node_counts: BTreeMap<String, usize>,
    pub image_alt_present_count: usize,
    pub image_alt_empty_count: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelityLinks {
    pub link_node_count: usize,
    pub anchor_node_count: usize,
    pub anchor_count: usize,
    pub anchor_graph_edge_count: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelityNavigation {
    pub toc_point_count: usize,
    pub landmark_point_count: usize,
    pub page_list_point_count: usize,
    pub has_start_location: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelityResources {
    pub ir_resource_count: usize,
    pub resource_kind_counts: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelityPlacements {
    pub raw_reference_occurrence_count: usize,
    pub native_occurrence_count: usize,
    pub semantic_occurrence_count: usize,
    pub ir_image_node_count: usize,
    pub exact_image_identity_count: usize,
    pub unresolved_or_ambiguous_identity_count: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelityStyles {
    pub style_count: usize,
    pub style_property_count: usize,
    pub distinct_style_property_count: usize,
    pub nodes_with_nondefault_style_count: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct KfxFidelityFonts {
    pub declared_font_resource_record_count: usize,
    pub font_resource_count: usize,
    pub font_face_count: usize,
    pub mapped_font_resource_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxPlacementIdentitySummary {
    pub external_resource_records: usize,
    pub raw_image_reference_names: usize,
    pub exact_image_identities: usize,
    pub unresolved_or_ambiguous_identities: usize,
    pub resource_alias_policy: String,
    pub resource_alias_catalog: Vec<String>,
    pub raw_reference_resolution_counts: Vec<KfxPlacementResolutionCount>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxPlacementResolutionCount {
    pub category: String,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxPlacementStage {
    pub occurrence_count: usize,
    pub identity_verified_occurrence_count: usize,
    pub unresolved_identity_occurrence_count: usize,
    pub resource_frequencies: Vec<KfxPlacementFrequency>,
    pub occurrences: Vec<KfxPlacementOccurrence>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxPlacementFrequency {
    pub resource: String,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxPlacementOccurrence {
    pub source_order: usize,
    pub resource: Option<String>,
    pub alternate_symbol_resources_audit_only: Vec<String>,
    /// `$155` target token, not the cumulative Kindle position in the oracle.
    pub source_position_target_id: Option<u32>,
    pub identity_status: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxPlacementEpubResource {
    pub resource: String,
    /// FolioForge's EPUB writer uses this opaque manifest ID for the ResourceId.
    pub manifest_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxAuditContainer {
    pub origin_index: usize,
    pub input_index: usize,
    pub byte_offset: usize,
    pub entity_count: usize,
    pub symbol_count: usize,
    pub symbol_import_count: usize,
    pub duplicate_symbol_ids: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxAuditExternalResource {
    pub fragment_key: KfxAuditFragmentKey,
    pub location: Option<String>,
    pub location_symbol_id: Option<u64>,
    pub location_source: String,
    pub normalized_location: Option<String>,
    pub declared_mime: Option<String>,
    pub declared_format: Option<String>,
    pub declared_dimensions: Option<(u32, u32)>,
    pub tile_field_ids: Option<Vec<u64>>,
    pub tile_layout: Option<KfxAuditTileLayout>,
    pub metadata_field_ids: Vec<u64>,
    pub linked_symbols_by_field: Vec<KfxAuditSymbolReference>,
    pub reference_count: usize,
    pub visible_placement_count: usize,
    pub reference_count_standard_ion_audit_only: usize,
    pub visible_placement_count_standard_ion_audit_only: usize,
    pub production_binding_status: String,
    pub production_diagnostic_code: Option<String>,
    pub identity_status: String,
    pub identity_reason: String,
    pub identity_classification: String,
    pub resource_shape_classification: String,
    pub candidate_raw_media: Vec<KfxAuditFragmentKey>,
    pub candidate_raw_media_legacy_unadjusted_audit_only: Vec<KfxAuditFragmentKey>,
    pub dimension_candidate_count_audit_only: usize,
    pub prior_dimension_only_match_audit_only: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxAuditRawMedia {
    pub fragment_key: KfxAuditFragmentKey,
    pub media_type: String,
    pub byte_length: usize,
    pub sha256_audit_only: String,
    pub dimensions: Option<(u32, u32)>,
    pub symbol_name: Option<String>,
    pub symbol_name_legacy_unadjusted_audit_only: Option<String>,
    pub entity_header_field_ids: Vec<u64>,
    pub inner_fid_value: Option<KfxAuditScalar>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxAuditCompositeFragment {
    pub fragment_key: KfxAuditFragmentKey,
    pub classification: String,
    pub field_ids: Vec<u64>,
    pub embedded_blob_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxAuditTileLayout {
    pub row_count: usize,
    pub max_column_count: usize,
    pub overlapped_tiles_field_present: bool,
    pub locations: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxAuditSymbolReference {
    pub field_id: u64,
    pub symbol_id: u64,
    pub resolved_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxAuditScalar {
    pub kind: String,
    pub integer: Option<i64>,
    pub symbol_id: Option<u64>,
    pub string: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxAuditFragmentKey {
    pub container_origin: usize,
    pub fragment_type: u32,
    pub fid: Option<String>,
    pub native_entity_id: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KfxAuditSummary {
    pub external_resource_count: usize,
    pub raw_media_count: usize,
    pub exact_identity_resolved: usize,
    pub unresolved: usize,
    pub duplicate_identity: usize,
    pub raw_media_exists_but_lookup_failed: usize,
    pub unresolved_referenced_records: usize,
    pub referenced_unresolved_occurrences: usize,
    pub unresolved_visibly_placed_records: usize,
    pub visibly_placed_unresolved_positions: usize,
    pub prior_dimension_only_matches_audit_only: usize,
    pub composite_fragments_unknown: usize,
    pub resource_shapes_unknown: usize,
    pub legacy_unadjusted_symbol_offset_matches_audit_only: usize,
    pub standard_ion_reference_occurrences_audit_only: usize,
    pub standard_ion_visible_placement_positions_audit_only: usize,
    pub production_exact_bindings: usize,
    pub production_unresolved_bindings: usize,
    pub production_unresolved_visible_placement_positions: usize,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum AmazonKfxError {
    #[error("Amazon KFX CONT container is truncated at byte {0}")]
    Truncated(usize),
    #[error("Amazon KFX CONT header is invalid: {0}")]
    InvalidHeader(String),
    #[error("Amazon KFX CONT version {0} is unsupported")]
    UnsupportedVersion(u16),
    #[error("Amazon KFX entity index is invalid: {0}")]
    InvalidEntityIndex(String),
    #[error("Amazon KFX entity {0} is invalid: {1}")]
    InvalidEntity(u32, String),
    #[error("Amazon KFX Ion data is invalid: {0}")]
    Ion(#[from] IonError),
    #[error("Amazon KFX symbol {0} is not available")]
    UnknownSymbol(u64),
    #[error("Amazon KFX input is protected by DRM")]
    ProtectedContent,
    #[error("KFX input contains unresolved visible resources ({0} placements)")]
    UnresolvedVisibleResources(usize),
    #[error("Amazon KFX semantic decoding failed: {0}")]
    Semantic(String),
}

#[derive(Clone, Debug)]
struct ContainerHeader {
    version: u16,
    header_len: usize,
    info_offset: usize,
    info_length: usize,
}

#[derive(Clone, Debug)]
struct NativeContainer<'a> {
    header: ContainerHeader,
    info: IonValue,
    doc_symbols: Option<IonValue>,
    capabilities: Option<IonValue>,
    entities: Vec<NativeEntity<'a>>,
    kfxgen_info: &'a [u8],
}

#[derive(Clone, Debug)]
struct NativeEntity<'a> {
    id: u32,
    type_id: u32,
    payload: Option<IonValue>,
    raw_payload: Option<&'a [u8]>,
    info: IonValue,
    drm_scheme: i64,
}

#[derive(Clone, Debug, Default)]
struct SymbolTable {
    // IDs are kept as IDs with a best-effort textual resolution. No KFX
    // numeric field is promoted to an IR field merely because it is known.
    names: BTreeMap<u64, String>,
    imports: Vec<SymbolTableImport>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SymbolTableImport {
    name: Option<String>,
    version: Option<u32>,
    max_id: Option<u32>,
}

#[derive(Clone, Debug, Default)]
struct NativeModel {
    containers: Vec<NativeContainerOwned>,
    symbols: SymbolTable,
    metadata: Metadata,
    string_tables: BTreeMap<u32, Vec<String>>,
    fragments: Vec<NativeFragment>,
    resources: Vec<NativeResource>,
    cover_resource_ids: BTreeSet<u32>,
    /// Amazon metadata stores `cover_image` as the external-resource fid
    /// (for example `e6`), not as the numeric entity id of the raw `$417`
    /// media. Keep the fid until the exact `$164` -> `$165` -> `$417`
    /// resolution pass can prove the resource identity.
    cover_resource_fids: BTreeSet<String>,
    /// `$179` link symbols are declared by `$266` anchor fragments. Keep the
    /// declaration scoped to its CONT container because KFX local symbol IDs
    /// are not globally unique.
    link_targets: BTreeMap<(usize, u64), KfxLinkTarget>,
    navigation: Vec<NavPointDraft>,
    warnings: Vec<String>,
    unknown_features: BTreeSet<String>,
    input_loss: Vec<String>,
    resource_identity_diagnostics: Vec<KfxResourceDiagnostic>,
    drm_detected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum KfxLinkTarget {
    Position(u32),
    Uri(String),
}

#[derive(Clone, Debug)]
struct NativeContainerOwned {
    input_index: usize,
    byte_offset: usize,
    entity_count: usize,
    symbols: SymbolTable,
}

#[derive(Clone, Debug)]
struct NativeFragment {
    container_index: usize,
    entity_id: u32,
    fragment_type: u32,
    value: IonValue,
}

#[derive(Clone, Debug)]
struct NativeResource {
    container_index: usize,
    fragment_type: u32,
    entity_id: Option<u32>,
    media_type: String,
    bytes: Vec<u8>,
    entity_info: Option<IonValue>,
    properties: BTreeSet<String>,
}

#[derive(Clone, Debug)]
struct NavPointDraft {
    label: String,
    target: u32,
    /// Heading navigation (`$798`) carries a level separately from its
    /// presentation label, which is commonly the synthetic
    /// `heading-nav-unit` string. It must not be inferred from that label.
    heading_level: Option<u8>,
    children: Vec<NavPointDraft>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetLocation {
    document: DocumentId,
    node: NodeId,
    href: String,
    anchor_name: String,
}

#[derive(Clone, Debug)]
struct ContentTextSegment {
    position: Option<u32>,
    /// A directly enclosing KFX block may carry the navigation target while
    /// its child text run has its own local position. Keep that parent
    /// position so navigation can anchor the heading without relabeling all
    /// descendants as headings.
    parent_position: Option<u32>,
    /// `$798` can target a structural content block whose first readable
    /// leaf has a different local `$155` (or no `$155` at all). Keep that
    /// proven heading target separate from the leaf position so only the
    /// first text leaf is promoted.
    navigation_heading_position: Option<u32>,
    /// The explicit source heading level, when KFX carries `$790` on this
    /// block or on a directly enclosing single-text block.
    source_heading_level: Option<u8>,
    /// Text ranges whose `$142` style event carries `$179`.
    link_ranges: Vec<KfxTextLinkRange>,
    /// A block-level `$179` applies to the complete text leaf when no more
    /// specific style-event link range overrides it.
    link_target_symbol_id: Option<u64>,
    text: String,
    style_name: Option<String>,
    style_events: Vec<KfxTextStyleEvent>,
    ruby_annotations: Vec<KfxRubyAnnotation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct KfxTextLinkRange {
    text_offset: usize,
    text_length: usize,
    target_symbol_id: u64,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KfxNoteKind {
    Footnote,
    Endnote,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct KfxNoteProvenance {
    display_style_symbol_id: u64,
    link_target_symbol_id: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct KfxNoteReference {
    kind: KfxNoteKind,
    href: String,
    provenance: KfxNoteProvenance,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct KfxNote {
    kind: KfxNoteKind,
    target: TargetLocation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct KfxRubyAnnotation {
    text_offset: usize,
    text_length: usize,
    text: String,
}

#[derive(Clone, Debug)]
struct KfxMathProvenance {
    content_field_id: u64,
    /// A visual fragment family was observed in the controlled corpus, but
    /// no source-backed co-reference binds one particular visual fragment to
    /// this occurrence.  Keep that relation unresolved instead of guessing.
    visual_fragment_type: Option<u32>,
}

#[derive(Clone, Debug)]
struct KfxMathOccurrence {
    mathml: String,
    alt: Option<String>,
    display: bool,
    provenance: KfxMathProvenance,
}

#[derive(Clone, Debug)]
enum ContentSegment {
    Text(ContentTextSegment),
    Math {
        position: Option<u32>,
        parent_position: Option<u32>,
        occurrence: KfxMathOccurrence,
        style_name: Option<String>,
    },
    Image {
        position: Option<u32>,
        resource: ResourceId,
        alt: String,
        style_name: Option<String>,
        inline: bool,
        link_target_symbol_id: Option<u64>,
    },
}

#[derive(Clone, Copy, Debug)]
struct CapturedImagePlacement {
    position_target_id: Option<u32>,
    resource: ResourceId,
}

#[derive(Default)]
struct PlacementCapture {
    native: Vec<CapturedImagePlacement>,
    semantic: Vec<CapturedImagePlacement>,
    ir: Vec<CapturedImagePlacement>,
    text: Option<TextTraceCapture>,
    text_events: Option<TextEventCapture>,
}

#[derive(Default)]
struct TextTraceCapture {
    source_fragments: Vec<CapturedTextUnit>,
    semantic_documents: Vec<CapturedTextUnit>,
    unresolved_text_reference_count: usize,
}

struct CapturedTextUnit {
    content_fragment_order: usize,
    document_order: Option<usize>,
    text_segment_count: usize,
    text: String,
}

#[derive(Default)]
struct TextEventCapture {
    include_private_text: bool,
    unresolved_text_reference_count: usize,
    events: Vec<CapturedKfxTextEvent>,
}

struct CapturedKfxTextEvent {
    section_id: Option<String>,
    story_id: Option<String>,
    source_fragment: String,
    source_fragment_order: usize,
    eid: u32,
    source_path: String,
    source_position: Option<u32>,
    text_kind: &'static str,
    text: String,
    adjacent_style_events: Vec<KfxTextStyleEvent>,
}

#[derive(Clone, Debug, Default)]
struct ContentTraceSource {
    section_id: Option<String>,
    story_id: Option<String>,
    source_fragment: String,
    source_fragment_order: usize,
    eid: u32,
}

struct ContentTextTraceState<'a> {
    capture: &'a mut TextEventCapture,
    source: Option<&'a ContentTraceSource>,
    source_path: &'a mut Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct ContentTraceLocation {
    section_id: Option<String>,
    story_id: Option<String>,
}

#[derive(Clone, Debug)]
struct RawImageReference {
    resource_name: Option<String>,
    alternate_names_audit_only: Vec<String>,
    position_target_id: Option<u32>,
    resolution_category: String,
}

#[derive(Clone, Debug)]
struct ExternalPlacementIdentity {
    status: String,
    resource: Option<ResourceId>,
}

struct ContentDecodeContext<'a> {
    container_index: usize,
    string_tables: &'a BTreeMap<u32, Vec<String>>,
    ruby_contents: &'a RubyContentMap,
    resource_symbols: &'a BTreeMap<u64, (ResourceId, bool)>,
    image_resource_symbols: &'a BTreeMap<u64, (ResourceId, bool)>,
    style_names: Option<&'a BTreeMap<u64, String>>,
    image_record_symbol: Option<u64>,
    navigation_heading_levels: Option<&'a BTreeMap<u32, u8>>,
    link_targets: &'a BTreeMap<(usize, u64), KfxLinkTarget>,
}

type RubyContentMap = BTreeMap<(usize, u64), BTreeMap<u64, String>>;

#[derive(Clone, Copy, Debug, Default)]
struct DecodeOptions {
    mode: ParseMode,
}

pub fn import(bytes: &[u8], mode: ParseMode) -> Result<AmazonKfxImport, AmazonKfxError> {
    import_file_set(&[bytes], mode)
}

/// Import one logical KFX file-set. Every member is parsed into the same
/// native model before any semantic pass or resource identity resolution.
/// Input order is retained as provenance; it is never used to guess identity.
pub fn import_file_set(
    inputs: &[&[u8]],
    mode: ParseMode,
) -> Result<AmazonKfxImport, AmazonKfxError> {
    let mut model = parse_native_sources(inputs, DecodeOptions { mode })?;
    if model.drm_detected {
        return Err(AmazonKfxError::ProtectedContent);
    }
    record_unresolved_conditional_evidence(&mut model);
    let resource_resolution = resolve_native_resource_paths(&model);
    record_resource_identity_limitations(&mut model, &resource_resolution);
    validate_resource_resolution(mode, &resource_resolution.diagnostics)?;
    let book = decode_semantic(&mut model, mode, &resource_resolution, None)?;
    let recovery_actions = recovery_actions(mode, &model);
    let input = InputReport {
        detected_format: "Amazon KFX CONT file-set".to_owned(),
        parser_version: "folioforge-kfx-cont/0.4".to_owned(),
        container_count: model.containers.len(),
        entity_count: model
            .containers
            .iter()
            .map(|container| container.entity_count)
            .sum(),
        fragment_count: model.fragments.len(),
        symbol_count: model.symbols.names.len(),
        resource_count: model.resources.len(),
        document_count: book.documents.len(),
        drm_detected: model.drm_detected,
        warnings: model.warnings,
        unknown_features: model.unknown_features.iter().cloned().collect(),
        recovery_actions,
        input_loss: model.input_loss,
        resource_identity_diagnostics: resource_resolution.diagnostics,
    };
    Ok(AmazonKfxImport { book, input })
}

pub fn inspect(bytes: &[u8], mode: ParseMode) -> Result<InputReport, AmazonKfxError> {
    let mut model = parse_native(bytes, DecodeOptions { mode })?;
    let resource_resolution = resolve_native_resource_paths(&model);
    record_resource_identity_limitations(&mut model, &resource_resolution);
    let recovery_actions = recovery_actions(mode, &model);
    Ok(InputReport {
        detected_format: "Amazon KFX CONT".to_owned(),
        parser_version: "folioforge-kfx-cont/0.3".to_owned(),
        container_count: model.containers.len(),
        entity_count: model
            .containers
            .iter()
            .map(|container| container.entity_count)
            .sum(),
        fragment_count: model.fragments.len(),
        symbol_count: model.symbols.names.len(),
        resource_count: model.resources.len(),
        document_count: model
            .fragments
            .iter()
            .filter(|fragment| fragment.fragment_type == 259)
            .count(),
        drm_detected: model.drm_detected,
        warnings: model.warnings,
        unknown_features: model.unknown_features.iter().cloned().collect(),
        recovery_actions,
        input_loss: model.input_loss,
        resource_identity_diagnostics: model.resource_identity_diagnostics,
    })
}

/// Emit content-free identity observations for one real Amazon CONT input.
/// This path is diagnostic only: SHA-256 values are not consulted by the
/// importer or by any resource resolver.
pub fn audit_resources(bytes: &[u8]) -> Result<KfxResourceAudit, AmazonKfxError> {
    audit_resources_file_set(&[bytes])
}

/// Audit a logical KFX file-set after aggregating every member. File names and
/// book metadata are intentionally excluded from the report.
pub fn audit_resources_file_set(inputs: &[&[u8]]) -> Result<KfxResourceAudit, AmazonKfxError> {
    let model = parse_native_sources(inputs, DecodeOptions::default())?;
    let production_resolution = resolve_native_resource_paths(&model);
    let mut raw_media = Vec::new();
    let mut raw_by_name = BTreeMap::<FragmentKey, Vec<KfxAuditFragmentKey>>::new();
    let mut raw_by_legacy_name = BTreeMap::<FragmentKey, Vec<KfxAuditFragmentKey>>::new();

    for resource in model
        .resources
        .iter()
        .filter(|item| item.fragment_type == 417)
    {
        let Some(fid) = resource.entity_id else {
            continue;
        };
        let symbol_name = model
            .containers
            .get(resource.container_index)
            .and_then(|container| container.symbols.names.get(&u64::from(fid)))
            .cloned();
        let legacy_symbol_name = resource.entity_id.and_then(|entity_id| {
            let container = model.containers.get(resource.container_index)?;
            let shift = import_symbol_id_shift(&container.symbols.imports)?;
            let legacy_id = u64::from(entity_id).checked_sub(shift)?;
            container.symbols.names.get(&legacy_id).cloned()
        });
        let key = KfxAuditFragmentKey {
            container_origin: resource.container_index,
            fragment_type: resource.fragment_type,
            fid: symbol_name.clone(),
            native_entity_id: Some(fid),
        };
        if let Some(fid) = symbol_name.as_ref() {
            raw_by_name
                .entry(FragmentKey {
                    fragment_type: 417,
                    fid: fid.clone(),
                })
                .or_default()
                .push(key.clone());
        }
        if let Some(fid) = legacy_symbol_name.as_ref() {
            raw_by_legacy_name
                .entry(FragmentKey {
                    fragment_type: 417,
                    fid: fid.clone(),
                })
                .or_default()
                .push(key.clone());
        }
        let digest = Sha256::digest(&resource.bytes);
        let entity_header_field_ids = resource
            .entity_info
            .as_ref()
            .and_then(struct_fields)
            .map(|fields| {
                let mut ids = fields.iter().map(|(field, _)| *field).collect::<Vec<_>>();
                ids.sort_unstable();
                ids.dedup();
                ids
            })
            .unwrap_or_default();
        raw_media.push(KfxAuditRawMedia {
            fragment_key: key,
            media_type: resource.media_type.clone(),
            byte_length: resource.bytes.len(),
            sha256_audit_only: hex_lower(&digest),
            dimensions: raster_dimensions(&resource.bytes),
            symbol_name,
            symbol_name_legacy_unadjusted_audit_only: legacy_symbol_name,
            entity_header_field_ids,
            inner_fid_value: resource
                .entity_info
                .as_ref()
                .and_then(|info| struct_get(info, FIELD_FRAGMENT_ID))
                .and_then(audit_scalar),
        });
    }
    raw_media.sort_by(|left, right| audit_key_sort(&left.fragment_key, &right.fragment_key));
    for candidates in raw_by_name.values_mut() {
        candidates.sort_by(audit_key_sort);
    }
    for candidates in raw_by_legacy_name.values_mut() {
        candidates.sort_by(audit_key_sort);
    }

    let references = count_resource_references(&model);
    let standard_ion_references = count_resource_references_standard_ion(&model);
    let placements = count_observed_resource_placements(&model);
    let standard_ion_placements = count_observed_resource_placements_standard_ion(&model);
    let mut external_resources = Vec::new();
    for fragment in model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 164)
    {
        let (location, location_symbol_id, location_source) = resource_location(&model, fragment);
        let normalized_location = location.as_deref().and_then(normalize_kfx_resource_path);
        let candidates = location
            .as_ref()
            .and_then(|fid| {
                raw_by_name.get(&FragmentKey {
                    fragment_type: 417,
                    fid: fid.clone(),
                })
            })
            .cloned()
            .unwrap_or_default();
        let legacy_candidates = location
            .as_ref()
            .and_then(|fid| {
                raw_by_legacy_name.get(&FragmentKey {
                    fragment_type: 417,
                    fid: fid.clone(),
                })
            })
            .cloned()
            .unwrap_or_default();
        let declared_dimensions = audit_dimension(&fragment.value, FIELD_RESOURCE_PIXEL_WIDTH)
            .or_else(|| audit_dimension(&fragment.value, FIELD_RESOURCE_WIDTH_FALLBACK))
            .zip(
                audit_dimension(&fragment.value, FIELD_RESOURCE_PIXEL_HEIGHT)
                    .or_else(|| audit_dimension(&fragment.value, FIELD_RESOURCE_HEIGHT_FALLBACK)),
            );
        let dimension_candidate_indices = declared_dimensions
            .map(|dimensions| {
                model
                    .resources
                    .iter()
                    .enumerate()
                    .filter_map(|(index, resource)| {
                        (raster_dimensions(&resource.bytes) == Some(dimensions)).then_some(index)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // Reconstruct the former (system-symbol-unadjusted) index only for
        // audit comparison. This never contributes to the production lookup.
        let prior_dimension_only_match_audit_only = legacy_candidates.is_empty()
            && !dimension_candidate_indices.is_empty()
            && dimension_candidate_indices.iter().all(|index| {
                model.resources[*index].bytes
                    == model.resources[dimension_candidate_indices[0]].bytes
            });
        let (identity_status, identity_reason) = match normalized_location.as_ref() {
            None if location.is_none() => ("unresolved", "MissingLocation"),
            None => ("unresolved", "InvalidReference"),
            Some(_) if candidates.is_empty() && !model.resources.is_empty() => (
                "RawMediaExistsButLookupFailed",
                "MissingExactLocationToRawFid",
            ),
            Some(_) if candidates.is_empty() => ("unresolved", "MissingRawMedia"),
            Some(_) if candidates.len() > 1 => ("unresolved", "DuplicateRawMedia"),
            Some(_) => ("ResolvedByExactIdentity", "ExactLocationToRawFid"),
        };
        let mut metadata_field_ids = struct_fields(&fragment.value)
            .map(|fields| fields.iter().map(|(field, _)| *field).collect::<Vec<_>>())
            .unwrap_or_default();
        metadata_field_ids.sort_unstable();
        metadata_field_ids.dedup();
        let linked_symbols_by_field = [161_u64, 162, 165, 175]
            .into_iter()
            .filter_map(|field| match struct_get(&fragment.value, field) {
                Some(IonValue::Symbol(symbol)) => Some(KfxAuditSymbolReference {
                    field_id: field,
                    symbol_id: *symbol,
                    resolved_name: model
                        .containers
                        .get(fragment.container_index)
                        .and_then(|container| container.symbols.names.get(symbol))
                        .cloned(),
                }),
                _ => None,
            })
            .collect();
        let lookup_key = location.as_deref().unwrap_or_default();
        let reference_count = references.get(lookup_key).copied().unwrap_or_default();
        let visible_placement_count = placements.get(lookup_key).copied().unwrap_or_default();
        let reference_count_standard_ion_audit_only = standard_ion_references
            .get(lookup_key)
            .copied()
            .unwrap_or_default();
        let visible_placement_count_standard_ion_audit_only = standard_ion_placements
            .get(lookup_key)
            .copied()
            .unwrap_or_default();
        let production_binding = production_resolution.bindings.iter().find(|record| {
            record.container_origin == fragment.container_index
                && record.external_resource_entity_id == fragment.entity_id
        });
        let format = audit_string_or_symbol(
            struct_get(&fragment.value, FIELD_RESOURCE_FORMAT),
            model.containers.get(fragment.container_index),
        );
        let mime = audit_string_or_symbol(
            struct_get(&fragment.value, FIELD_RESOURCE_MIME),
            model.containers.get(fragment.container_index),
        );
        let tile_layout = audit_tile_layout(
            struct_get(&fragment.value, FIELD_RESOURCE_TILE_GRID),
            struct_get(&fragment.value, FIELD_RESOURCE_OVERLAPPED_TILES).is_some(),
            model.containers.get(fragment.container_index),
        );
        let tile_field_ids = [FIELD_RESOURCE_TILE_GRID, FIELD_RESOURCE_OVERLAPPED_TILES]
            .into_iter()
            .filter(|field| struct_get(&fragment.value, *field).is_some())
            .collect::<Vec<_>>();
        let resource_shape_classification = classify_resource_shape(
            format.as_deref(),
            mime.as_deref(),
            !tile_field_ids.is_empty(),
            struct_get(&fragment.value, FIELD_RESOURCE_OVERLAPPED_TILES).is_some(),
        );
        external_resources.push(KfxAuditExternalResource {
            fragment_key: KfxAuditFragmentKey {
                container_origin: fragment.container_index,
                fragment_type: fragment.fragment_type,
                fid: resolved_fragment_fid(&model, fragment),
                native_entity_id: Some(fragment.entity_id),
            },
            location,
            location_symbol_id,
            location_source,
            normalized_location: normalized_location.clone(),
            declared_mime: mime,
            declared_format: format,
            declared_dimensions,
            tile_field_ids: (!tile_field_ids.is_empty()).then_some(tile_field_ids),
            tile_layout,
            metadata_field_ids,
            linked_symbols_by_field,
            reference_count,
            visible_placement_count,
            reference_count_standard_ion_audit_only,
            visible_placement_count_standard_ion_audit_only,
            production_binding_status: if production_binding
                .is_some_and(|record| record.binding.is_exact())
            {
                "Exact".to_owned()
            } else {
                "Unresolved".to_owned()
            },
            production_diagnostic_code: production_binding
                .and_then(|record| record.binding.unresolved_diagnostic_code())
                .map(ToOwned::to_owned),
            identity_status: identity_status.to_owned(),
            identity_reason: identity_reason.to_owned(),
            identity_classification: if identity_status == "ResolvedByExactIdentity" {
                "ResolvedByExactIdentity"
            } else {
                "StillUnknown"
            }
            .to_owned(),
            resource_shape_classification,
            candidate_raw_media: candidates,
            candidate_raw_media_legacy_unadjusted_audit_only: legacy_candidates,
            dimension_candidate_count_audit_only: dimension_candidate_indices.len(),
            prior_dimension_only_match_audit_only,
        });
    }
    external_resources
        .sort_by(|left, right| audit_key_sort(&left.fragment_key, &right.fragment_key));

    let exact_identity_resolved = external_resources
        .iter()
        .filter(|record| record.identity_status == "ResolvedByExactIdentity")
        .count();
    let standard_ion_reference_occurrences_audit_only = external_resources
        .iter()
        .map(|record| record.reference_count_standard_ion_audit_only)
        .sum();
    let standard_ion_visible_placement_positions_audit_only = external_resources
        .iter()
        .map(|record| record.visible_placement_count_standard_ion_audit_only)
        .sum();
    let production_exact_bindings = production_resolution.resolved_records;
    let production_unresolved_bindings = production_resolution.unresolved_records;
    let production_unresolved_visible_placement_positions = production_resolution
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.visible_placement_count)
        .sum();
    let unresolved_records = external_resources
        .iter()
        .filter(|record| record.identity_status != "ResolvedByExactIdentity")
        .collect::<Vec<_>>();
    let duplicate_identity = unresolved_records
        .iter()
        .filter(|record| record.identity_reason == "DuplicateRawMedia")
        .count();
    let unresolved_referenced_records = unresolved_records
        .iter()
        .filter(|record| record.reference_count > 0)
        .count();
    let referenced_unresolved_occurrences = unresolved_records
        .iter()
        .map(|record| record.reference_count)
        .sum();
    let unresolved_visibly_placed_records = unresolved_records
        .iter()
        .filter(|record| record.visible_placement_count > 0)
        .count();
    let visibly_placed_unresolved_positions = unresolved_records
        .iter()
        .map(|record| record.visible_placement_count)
        .sum();
    let raw_media_exists_but_lookup_failed = external_resources
        .iter()
        .filter(|record| record.identity_status == "RawMediaExistsButLookupFailed")
        .count();
    let prior_dimension_only_matches_audit_only = external_resources
        .iter()
        .filter(|record| record.prior_dimension_only_match_audit_only)
        .count();
    let resource_shapes_unknown = external_resources
        .iter()
        .filter(|record| record.resource_shape_classification == "Unknown")
        .count();
    let legacy_unadjusted_symbol_offset_matches_audit_only = external_resources
        .iter()
        .filter(|record| {
            record.candidate_raw_media.is_empty()
                && !record
                    .candidate_raw_media_legacy_unadjusted_audit_only
                    .is_empty()
        })
        .count();
    let mut composite_fragments = model
        .fragments
        .iter()
        .filter(|fragment| ion_blob_count(&fragment.value) > 0)
        .map(|fragment| {
            let mut field_ids = struct_fields(&fragment.value)
                .map(|fields| fields.iter().map(|(field, _)| *field).collect::<Vec<_>>())
                .unwrap_or_default();
            field_ids.sort_unstable();
            field_ids.dedup();
            KfxAuditCompositeFragment {
                fragment_key: KfxAuditFragmentKey {
                    container_origin: fragment.container_index,
                    fragment_type: fragment.fragment_type,
                    fid: resolved_fragment_fid(&model, fragment),
                    native_entity_id: Some(fragment.entity_id),
                },
                classification: "Unknown".to_owned(),
                field_ids,
                embedded_blob_count: ion_blob_count(&fragment.value),
            }
        })
        .collect::<Vec<_>>();
    composite_fragments
        .sort_by(|left, right| audit_key_sort(&left.fragment_key, &right.fragment_key));
    let composite_fragments_unknown = composite_fragments.len();
    let external_resource_count = external_resources.len();
    let raw_media_count = raw_media.len();
    let unresolved = unresolved_records.len();
    let containers = model
        .containers
        .iter()
        .enumerate()
        .map(|(origin_index, container)| {
            let earlier_ids = model.containers[..origin_index]
                .iter()
                .flat_map(|earlier| earlier.symbols.names.keys().copied())
                .collect::<BTreeSet<_>>();
            KfxAuditContainer {
                origin_index,
                input_index: container.input_index,
                byte_offset: container.byte_offset,
                entity_count: container.entity_count,
                symbol_count: container.symbols.names.len(),
                symbol_import_count: container
                    .symbols
                    .imports
                    .iter()
                    .filter(|import| import.max_id.is_some())
                    .count(),
                duplicate_symbol_ids: container
                    .symbols
                    .names
                    .keys()
                    .filter(|symbol| earlier_ids.contains(symbol))
                    .count(),
            }
        })
        .collect();
    let mut hasher = Sha256::new();
    for input in inputs {
        hasher.update((input.len() as u64).to_le_bytes());
        hasher.update(input);
    }
    let digest = hasher.finalize();
    Ok(KfxResourceAudit {
        schema_version: 1,
        source_id: format!("KFX-RI-{}", &hex_lower(&digest)[..16]),
        parser_version: "folioforge-kfx-cont/0.4-resource-audit".to_owned(),
        containers,
        external_resources,
        raw_media,
        composite_fragments,
        summary: KfxAuditSummary {
            external_resource_count,
            raw_media_count,
            exact_identity_resolved,
            unresolved,
            duplicate_identity,
            raw_media_exists_but_lookup_failed,
            unresolved_referenced_records,
            referenced_unresolved_occurrences,
            unresolved_visibly_placed_records,
            visibly_placed_unresolved_positions,
            prior_dimension_only_matches_audit_only,
            composite_fragments_unknown,
            resource_shapes_unknown,
            legacy_unadjusted_symbol_offset_matches_audit_only,
            standard_ion_reference_occurrences_audit_only,
            standard_ion_visible_placement_positions_audit_only,
            production_exact_bindings,
            production_unresolved_bindings,
            production_unresolved_visible_placement_positions,
        },
    })
}

/// Trace image-position occurrences through source references, the current
/// native recognizer, Semantic IR placement, and final IR image nodes. All
/// resource labels are anonymous; this function never changes production
/// resource resolution or placement behavior.
pub fn audit_placements(bytes: &[u8]) -> Result<KfxPlacementAudit, AmazonKfxError> {
    if !bytes.starts_with(b"CONT") {
        return Err(AmazonKfxError::InvalidHeader(
            "placement audit requires an Amazon CONT KFX input".to_owned(),
        ));
    }
    let mut model = parse_native(bytes, DecodeOptions::default())?;
    if model.drm_detected {
        return Err(AmazonKfxError::ProtectedContent);
    }
    let resolution = resolve_native_resource_paths(&model);
    record_resource_identity_limitations(&mut model, &resolution);
    validate_resource_resolution(ParseMode::Compatible, &resolution.diagnostics)?;
    let (_, _, resource_ids_by_path) = materialize_ir_resources(&model, &resolution);
    let (identity_by_name, name_by_resource_id, name_by_location) =
        placement_identity_index(&model, &resolution, &resource_ids_by_path);
    let raw_references = collect_raw_image_references(&model, &identity_by_name, &name_by_location);
    let raw_reference_resolution_counts = raw_references
        .iter()
        .fold(BTreeMap::<String, usize>::new(), |mut counts, reference| {
            *counts
                .entry(reference.resolution_category.clone())
                .or_default() += 1;
            counts
        })
        .into_iter()
        .map(|(category, count)| KfxPlacementResolutionCount { category, count })
        .collect();
    let raw_names = raw_references
        .iter()
        .flat_map(|reference| {
            reference
                .resource_name
                .iter()
                .chain(reference.alternate_names_audit_only.iter())
                .cloned()
        })
        .collect::<BTreeSet<_>>();
    let alias_names = identity_by_name
        .iter()
        .filter(|(_, identity)| identity.status == "exact_image_identity")
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    let aliases = alias_names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), format!("img_{:04}", index + 1)))
        .collect::<BTreeMap<_, _>>();

    let mut capture = PlacementCapture::default();
    let _book = decode_semantic(
        &mut model,
        ParseMode::Compatible,
        &resolution,
        Some(&mut capture),
    )?;
    let raw_reference = placement_stage_from_raw(&raw_references, &aliases, &identity_by_name);
    let native_placement = placement_stage_from_capture(
        &capture.native,
        &aliases,
        &identity_by_name,
        &name_by_resource_id,
    );
    let semantic_placement = placement_stage_from_capture(
        &capture.semantic,
        &aliases,
        &identity_by_name,
        &name_by_resource_id,
    );
    let ir_image_nodes = placement_stage_from_capture(
        &capture.ir,
        &aliases,
        &identity_by_name,
        &name_by_resource_id,
    );

    let epub_resource_map = name_by_resource_id
        .iter()
        .filter_map(|(resource_id, name)| {
            Some(KfxPlacementEpubResource {
                resource: aliases.get(name)?.clone(),
                manifest_id: format!("res{}", resource_id.get()),
            })
        })
        .collect::<Vec<_>>();
    let external_resource_records = model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 164)
        .count();
    let exact_image_identities = identity_by_name
        .values()
        .filter(|identity| identity.status == "exact_image_identity")
        .count();
    let unresolved_or_ambiguous_identities = raw_names
        .iter()
        .filter(|name| {
            identity_by_name
                .get(*name)
                .is_none_or(|identity| identity.status != "exact_image_identity")
        })
        .count();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let input_sha256_audit_only = hex_lower(&hasher.finalize());

    Ok(KfxPlacementAudit {
        schema_version: 1,
        input_sha256_audit_only,
        resource_identity: KfxPlacementIdentitySummary {
            external_resource_records,
            raw_image_reference_names: raw_names.len(),
            exact_image_identities,
            unresolved_or_ambiguous_identities,
            resource_alias_policy:
                "lexically sorted exact-image KFX $164 FID names; aliases are diagnostic only"
                    .to_owned(),
            resource_alias_catalog: alias_names
                .iter()
                .filter_map(|name| aliases.get(name).cloned())
                .collect(),
            raw_reference_resolution_counts,
        },
        raw_reference,
        native_placement,
        semantic_placement,
        ir_image_nodes,
        epub_resource_map,
    })
}

/// Produce a privacy-safe, content-free semantic coverage report for one real
/// Amazon CONT KFX input. Unlike `audit_placements`, parse failures are
/// represented by fixed diagnostic codes so corpus runs can retain every
/// input without persisting parser messages that might contain source data.
pub fn audit_fidelity(bytes: &[u8]) -> KfxFidelityAudit {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut report = KfxFidelityAudit {
        schema_version: 1,
        parser_version: "folioforge-kfx-cont/0.5-fidelity-audit".to_owned(),
        input_sha256_audit_only: hex_lower(&hasher.finalize()),
        input_bytes: bytes.len() as u64,
        source: KfxFidelitySource {
            format: "Amazon KFX CONT".to_owned(),
            layout: "unknown_not_decoded".to_owned(),
            ..KfxFidelitySource::default()
        },
        text: KfxFidelityText {
            normalization: "IR text-node order; CRLF/CR to LF; Unicode NFC; SHA-256 over normalized UTF-8; scalar count after normalization".to_owned(),
            ..KfxFidelityText::default()
        },
        ..KfxFidelityAudit::default()
    };
    if !bytes.starts_with(b"CONT") {
        report.status = "Failed".to_owned();
        report.error_code = Some("KFX_INPUT_NOT_CONT".to_owned());
        increment_count(&mut report.diagnostics, "input_not_cont");
        return report;
    }

    let mut model = match parse_native(bytes, DecodeOptions::default()) {
        Ok(model) => model,
        Err(error) => {
            report.status = "Failed".to_owned();
            report.error_code = Some(fidelity_error_code(&error).to_owned());
            increment_count(
                &mut report.diagnostics,
                report.error_code.as_deref().unwrap(),
            );
            return report;
        }
    };

    report.native.container_count = model.containers.len();
    report.native.entity_count = model
        .containers
        .iter()
        .map(|container| container.entity_count)
        .sum();
    report.native.fragment_count = model.fragments.len();
    report.native.symbol_count = model.symbols.names.len();
    report.native.string_table_count = model.string_tables.len();
    report.native.raw_resource_count = model.resources.len();
    report.native.unknown_fragment_count = model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 0)
        .count();
    for fragment in &model.fragments {
        *report
            .native
            .fragment_type_counts
            .entry(fragment.fragment_type.to_string())
            .or_default() += 1;
    }
    for container in &model.containers {
        if let Ok(header) = read_container_header(bytes, container.byte_offset) {
            *report
                .source
                .container_version_counts
                .entry(header.version.to_string())
                .or_default() += 1;
        }
    }
    report.source.language = model
        .metadata
        .language
        .as_deref()
        .and_then(validated_language_tag);
    report.conditional = audit_conditional_evidence(&model);
    refresh_fidelity_diagnostics(&model, &mut report);

    if model.drm_detected {
        report.status = "Failed".to_owned();
        report.error_code = Some("KFX_DRM_PROTECTED".to_owned());
        increment_count(&mut report.diagnostics, "drm_protected");
        return report;
    }

    record_unresolved_conditional_evidence(&mut model);
    let resolution = resolve_native_resource_paths(&model);
    record_resource_identity_limitations(&mut model, &resolution);
    refresh_fidelity_diagnostics(&model, &mut report);
    for diagnostic in &resolution.diagnostics {
        let key = format!("resource_identity_{}", diagnostic.code);
        increment_count(&mut report.diagnostics, &key);
    }
    if let Err(error) = validate_resource_resolution(ParseMode::Compatible, &resolution.diagnostics)
    {
        let code = fidelity_error_code(&error);
        increment_count(&mut report.diagnostics, code);
    }

    // Match the deterministic path order used by `materialize_ir_resources`
    // without cloning resource byte buffers merely to construct the audit.
    let mut resource_ids_by_path = BTreeMap::new();
    for (path, candidate) in &resolution.by_path {
        if candidate.is_some() {
            let id = ResourceId::new(resource_ids_by_path.len() as u32);
            resource_ids_by_path.insert(path.clone(), id);
        }
    }
    let (identity_by_name, _name_by_resource_id, name_by_location) =
        placement_identity_index(&model, &resolution, &resource_ids_by_path);
    let raw_references = collect_raw_image_references(&model, &identity_by_name, &name_by_location);
    let raw_names = raw_references
        .iter()
        .flat_map(|reference| {
            reference
                .resource_name
                .iter()
                .chain(reference.alternate_names_audit_only.iter())
                .cloned()
        })
        .collect::<BTreeSet<_>>();
    report.placements.raw_reference_occurrence_count = raw_references.len();
    report.placements.exact_image_identity_count = identity_by_name
        .values()
        .filter(|identity| identity.status == "exact_image_identity")
        .count();
    report.placements.unresolved_or_ambiguous_identity_count = raw_names
        .iter()
        .filter(|name| {
            identity_by_name
                .get(*name)
                .is_none_or(|identity| identity.status != "exact_image_identity")
        })
        .count();

    let mut capture = PlacementCapture {
        text: Some(TextTraceCapture::default()),
        ..PlacementCapture::default()
    };
    let book = match decode_semantic(
        &mut model,
        ParseMode::Compatible,
        &resolution,
        Some(&mut capture),
    ) {
        Ok(book) => book,
        Err(error) => {
            report.status = "Partial".to_owned();
            report.error_code = Some(fidelity_error_code(&error).to_owned());
            refresh_fidelity_diagnostics(&model, &mut report);
            increment_count(
                &mut report.diagnostics,
                report.error_code.as_deref().unwrap(),
            );
            report.placements.native_occurrence_count = capture.native.len();
            report.placements.semantic_occurrence_count = capture.semantic.len();
            return report;
        }
    };

    report.content.document_count = book.documents.len();
    report.content.content_fragment_count = model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 259)
        .count();
    report.content.ir_node_counts = book.feature_summary();
    report.content.special_feature_counts = report
        .content
        .ir_node_counts
        .iter()
        .filter(|(kind, _)| !matches!(kind.as_str(), "section" | "paragraph" | "text"))
        .map(|(kind, count)| (kind.clone(), *count))
        .collect();
    report.placements.native_occurrence_count = capture.native.len();
    report.placements.semantic_occurrence_count = capture.semantic.len();
    report.placements.ir_image_node_count = capture.ir.len();
    if let Some(text_trace) = capture.text.as_ref() {
        summarize_captured_text(text_trace, &mut report.text);
    }
    summarize_book(&book, &mut report);

    let mut navigation_point_count = 0;
    let mut landmark_point_count = 0;
    let mut page_list_point_count = 0;
    for point in &book.navigation.toc {
        navigation_point_count += count_navigation_points(point);
    }
    for point in &book.navigation.landmarks {
        landmark_point_count += count_navigation_points(point);
    }
    for point in &book.navigation.page_list {
        page_list_point_count += count_navigation_points(point);
    }
    report.navigation = KfxFidelityNavigation {
        toc_point_count: navigation_point_count,
        landmark_point_count,
        page_list_point_count,
        has_start_location: book.navigation.start_location.is_some(),
    };
    report.resources.ir_resource_count = book.resources.len();
    for resource in &book.resources {
        increment_count(
            &mut report.resources.resource_kind_counts,
            resource_kind_label(&resource.kind),
        );
    }
    report.fonts.font_resource_count = book
        .resources
        .iter()
        .filter(|resource| resource.kind == ResourceKind::Font)
        .count();
    report.fonts.font_face_count = book.font_faces.len();
    report.fonts.mapped_font_resource_count = book
        .font_faces
        .iter()
        .map(|face| face.resource)
        .collect::<BTreeSet<_>>()
        .len();
    report.fonts.declared_font_resource_record_count = model
        .fragments
        .iter()
        .filter(|fragment| {
            if fragment.fragment_type != 164 {
                return false;
            }
            let container = model.containers.get(fragment.container_index);
            let format = audit_string_or_symbol(
                struct_get(&fragment.value, FIELD_RESOURCE_FORMAT),
                container,
            );
            let mime =
                audit_string_or_symbol(struct_get(&fragment.value, FIELD_RESOURCE_MIME), container);
            looks_like_font_descriptor(format.as_deref(), mime.as_deref())
        })
        .count();
    refresh_fidelity_diagnostics(&model, &mut report);

    report.status = if model.warnings.is_empty()
        && model.unknown_features.is_empty()
        && model.input_loss.is_empty()
        && resolution.diagnostics.is_empty()
    {
        "Complete"
    } else {
        "CompleteWithWarnings"
    }
    .to_owned();
    report
}

/// Produce a content-free structural audit of the original KFX Ion tree.
/// This is intentionally separate from `audit_fidelity`: the latter describes
/// FolioForge's recovered semantic book, while this report records only what
/// the native source actually contains. It never emits Ion strings, symbol
/// names, resource paths, or body text.
pub fn audit_semantic_evidence(bytes: &[u8]) -> KfxSemanticEvidenceAudit {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let input_sha256_audit_only = hex_lower(&hasher.finalize());
    let mut report = KfxSemanticEvidenceAudit {
        schema_version: 1,
        parser_version: "folioforge-kfx-cont/0.6-semantic-evidence".to_owned(),
        input_sha256_audit_only,
        ..KfxSemanticEvidenceAudit::default()
    };
    if !bytes.starts_with(b"CONT") {
        report.status = "Failed".to_owned();
        report.error_code = Some("KFX_INPUT_NOT_CONT".to_owned());
        increment_count(&mut report.diagnostics, "input_not_cont");
        return report;
    }
    let model = match parse_native(bytes, DecodeOptions::default()) {
        Ok(model) => model,
        Err(error) => {
            report.status = "Failed".to_owned();
            report.error_code = Some(fidelity_error_code(&error).to_owned());
            increment_count(
                &mut report.diagnostics,
                report.error_code.as_deref().unwrap(),
            );
            return report;
        }
    };
    if model.drm_detected {
        report.status = "Failed".to_owned();
        report.error_code = Some("KFX_DRM_PROTECTED".to_owned());
        increment_count(&mut report.diagnostics, "drm_protected");
        return report;
    }

    report.native.fragment_count = model.fragments.len();
    let mut candidates = Vec::new();
    for fragment in &model.fragments {
        *report
            .native
            .fragment_type_counts
            .entry(fragment.fragment_type.to_string())
            .or_default() += 1;
        collect_semantic_evidence_value(
            &fragment.value,
            fragment,
            &mut Vec::new(),
            &mut report,
            &mut candidates,
        );
    }
    report.conditional = audit_conditional_evidence(&model);
    report.render_inline = audit_render_inline_evidence(&model);
    report.candidate_samples = candidates;
    report
        .diagnostics
        .insert("parser_warning_count".to_owned(), model.warnings.len());
    report.diagnostics.insert(
        "unknown_feature_count".to_owned(),
        model.unknown_features.len(),
    );
    report
        .diagnostics
        .insert("input_loss_count".to_owned(), model.input_loss.len());
    report.status = if model.warnings.is_empty()
        && model.unknown_features.is_empty()
        && model.input_loss.is_empty()
    {
        "Complete"
    } else {
        "CompleteWithWarnings"
    }
    .to_owned();
    report
}

const SEMANTIC_EVIDENCE_CANDIDATE_FIELDS: [u64; 16] = [
    159, 171, 179, 281, 491, 590, 591, 592, 616, 618, 619, 683, 690, 757, 758, 759,
];
const CONDITIONAL_EVIDENCE_FIELDS: [u64; 3] = [171, 591, 592];
const MAX_SEMANTIC_EVIDENCE_DEPTH: usize = 32;
const MAX_SEMANTIC_EVIDENCE_SAMPLES: usize = 256;

fn audit_conditional_evidence(model: &NativeModel) -> KfxConditionalEvidence {
    let mut evidence = KfxConditionalEvidence::default();
    for fragment in &model.fragments {
        for field in CONDITIONAL_EVIDENCE_FIELDS {
            let count = count_field_occurrences(&fragment.value, field);
            if count > 0 {
                *evidence
                    .candidate_field_occurrence_counts
                    .entry(field.to_string())
                    .or_default() += count;
                *evidence
                    .candidate_fragment_type_counts
                    .entry(fragment.fragment_type.to_string())
                    .or_default() += count;
            }
        }
    }
    evidence.evaluation = (!evidence.candidate_field_occurrence_counts.is_empty())
        .then_some(KfxConditionalEvaluation::Unknown);
    evidence.status = if evidence.candidate_field_occurrence_counts.is_empty() {
        "NotObservedInNativeTree".to_owned()
    } else {
        "CandidateObservedRequiresSourceRelation".to_owned()
    };
    evidence
}

fn record_unresolved_conditional_evidence(model: &mut NativeModel) {
    let evidence = audit_conditional_evidence(model);
    if evidence.candidate_field_occurrence_counts.is_empty() {
        return;
    }
    model
        .unknown_features
        .insert("kfx-conditional-unknown".to_owned());
    model.input_loss.push(
        "KFX conditional-content candidates were observed, but no source-backed branch relation has been proved; branches were not selected or duplicated.".to_owned(),
    );
}

fn audit_render_inline_evidence(model: &NativeModel) -> KfxRenderInlineEvidence {
    let mut evidence = KfxRenderInlineEvidence::default();
    for fragment in &model.fragments {
        let count = count_field_occurrences(&fragment.value, 690);
        if count > 0 {
            *evidence
                .candidate_field_occurrence_counts
                .entry("690".to_owned())
                .or_default() += count;
            *evidence
                .candidate_fragment_type_counts
                .entry(fragment.fragment_type.to_string())
                .or_default() += count;
        }
    }
    evidence.status = if evidence.candidate_field_occurrence_counts.is_empty() {
        "NotObservedInNativeTree".to_owned()
    } else {
        "CandidateObservedRequiresSourceRelation".to_owned()
    };
    evidence
}

fn collect_semantic_evidence_value(
    value: &IonValue,
    fragment: &NativeFragment,
    path: &mut Vec<u64>,
    report: &mut KfxSemanticEvidenceAudit,
    samples: &mut Vec<KfxSemanticEvidenceSample>,
) {
    if path.len() > MAX_SEMANTIC_EVIDENCE_DEPTH {
        increment_count(&mut report.diagnostics, "evidence_depth_limit");
        return;
    }
    *report
        .native
        .value_kind_counts_by_fragment
        .entry(fragment.fragment_type.to_string())
        .or_default()
        .entry(ion_value_kind(value).to_owned())
        .or_default() += 1;
    match value {
        IonValue::String(text) => {
            report.native.scalar_string_count += 1;
            report.native.scalar_string_max_utf8_bytes =
                report.native.scalar_string_max_utf8_bytes.max(text.len());
        }
        IonValue::Annotation { symbols, value } => {
            for symbol in symbols {
                *report
                    .native
                    .annotation_symbol_counts
                    .entry(symbol.to_string())
                    .or_default() += 1;
            }
            collect_semantic_evidence_value(value, fragment, path, report, samples);
        }
        IonValue::Struct(fields) => {
            for (field, child) in fields {
                *report
                    .native
                    .field_occurrence_counts_by_fragment
                    .entry(format!("{}:${field}", fragment.fragment_type))
                    .or_default() += 1;
                *report
                    .native
                    .field_value_kind_counts_by_fragment
                    .entry(format!("{}:${field}", fragment.fragment_type))
                    .or_default()
                    .entry(ion_value_kind(child).to_owned())
                    .or_default() += 1;
                if SEMANTIC_EVIDENCE_CANDIDATE_FIELDS.contains(field) {
                    *report
                        .candidate_field_occurrence_counts
                        .entry(field.to_string())
                        .or_default() += 1;
                    if samples.len() < MAX_SEMANTIC_EVIDENCE_SAMPLES {
                        let mut child_path = path.clone();
                        child_path.push(*field);
                        let (list_item_count, annotation_symbol_ids, candidate_symbol_id) =
                            match child {
                                IonValue::List(items) | IonValue::SExp(items) => {
                                    (Some(items.len()), Vec::new(), None)
                                }
                                IonValue::Annotation { symbols, .. } => {
                                    (None, symbols.clone(), None)
                                }
                                IonValue::Symbol(symbol) => (None, Vec::new(), Some(*symbol)),
                                _ => (None, Vec::new(), None),
                            };
                        samples.push(KfxSemanticEvidenceSample {
                            fragment_type: fragment.fragment_type,
                            container_index: fragment.container_index,
                            entity_id: fragment.entity_id,
                            field_path: child_path,
                            value_kind: ion_value_kind(child).to_owned(),
                            list_item_count,
                            annotation_symbol_ids,
                            candidate_symbol_id,
                        });
                    }
                }
                path.push(*field);
                collect_semantic_evidence_value(child, fragment, path, report, samples);
                path.pop();
            }
        }
        IonValue::List(items) | IonValue::SExp(items) => {
            for item in items {
                collect_semantic_evidence_value(item, fragment, path, report, samples);
            }
        }
        _ => {}
    }
}

/// Build an audit-only KFX text event stream with numeric source paths.
/// `include_private_text` is deliberately opt-in and intended only for a
/// temporary local comparison process; normal audit output contains hashes
/// and lengths, never source text.
pub fn audit_text_events(
    bytes: &[u8],
    include_private_text: bool,
) -> Result<KfxTextEventAudit, AmazonKfxError> {
    if !bytes.starts_with(b"CONT") {
        return Err(AmazonKfxError::InvalidHeader(
            "text-event audit requires an Amazon CONT KFX input".to_owned(),
        ));
    }
    let mut model = parse_native(bytes, DecodeOptions::default())?;
    if model.drm_detected {
        return Err(AmazonKfxError::ProtectedContent);
    }
    let resolution = resolve_native_resource_paths(&model);
    let content_indices = ordered_content_fragment_indices(&model).unwrap_or_else(|| {
        model
            .fragments
            .iter()
            .enumerate()
            .filter_map(|(index, fragment)| (fragment.fragment_type == 259).then_some(index))
            .collect()
    });
    let has_ruby = !collect_ruby_contents(&model).is_empty();
    let trace_locations = ordered_content_trace_locations(&model);
    let story_definitions = model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 259)
        .count();
    let story_references = model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 260)
        .map(|fragment| count_field_occurrences(&fragment.value, FIELD_CONTENT_FRAGMENT_SYMBOL))
        .sum::<usize>();
    let mut emitted_story_counts = BTreeMap::<String, usize>::new();
    for fragment_index in &content_indices {
        if let Some(story_id) = trace_locations
            .get(fragment_index)
            .and_then(|location| location.story_id.as_ref())
        {
            *emitted_story_counts.entry(story_id.clone()).or_default() += 1;
        }
    }
    let stories_traversed_unique = emitted_story_counts.len();
    let conditional_candidate = !audit_conditional_evidence(&model)
        .candidate_field_occurrence_counts
        .is_empty();
    let render_inline_candidate = !audit_render_inline_evidence(&model)
        .candidate_field_occurrence_counts
        .is_empty();
    let note_candidate = model
        .fragments
        .iter()
        .any(|fragment| contains_linked_note_display(&fragment.value));
    let math_candidate = model
        .fragments
        .iter()
        .any(|fragment| count_field_occurrences(&fragment.value, FIELD_MATH_CONTENT) > 0);
    let illustrated_candidate = model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 259)
        .any(|fragment| contains_illustrated_image_structure(&fragment.value));
    let source_features = KfxTextSourceFeatures {
        has_stories: Some(story_definitions > 0 || story_references > 0),
        story_definitions,
        story_references,
        stories_traversed_unique,
        stories_emitted_more_than_once: emitted_story_counts
            .values()
            .filter(|count| **count > 1)
            .count(),
        story_definitions_not_emitted: story_definitions.saturating_sub(stories_traversed_unique),
        has_conditional_content: Some(conditional_candidate),
        has_illustrated_layout: None,
        has_image_occurrences: false,
        has_footnotes: None,
        has_non_image_render_inline: Some(render_inline_candidate),
        has_ruby: Some(has_ruby),
        has_mathml: None,
        creator: None,
        creator_version: None,
        feature_states: BTreeMap::new(),
        undecoded_features: Vec::new(),
    };

    let mut capture = PlacementCapture {
        text_events: Some(TextEventCapture {
            include_private_text,
            ..TextEventCapture::default()
        }),
        ..PlacementCapture::default()
    };
    let book = decode_semantic(
        &mut model,
        ParseMode::Compatible,
        &resolution,
        Some(&mut capture),
    )?;
    let has_footnotes = book_has_semantic_role(&book, SemanticRole::Footnote)
        || book_has_semantic_role(&book, SemanticRole::Endnote);
    let has_mathml = book_has_semantic_role(&book, SemanticRole::Math);
    let has_illustrated_layout = book_has_semantic_role(&book, SemanticRole::Figure);
    let has_approximate_illustrated_output = illustrated_candidate
        && !has_illustrated_layout
        && book_has_semantic_role(&book, SemanticRole::Image)
        && book_has_semantic_role(&book, SemanticRole::Text);
    let event_capture = capture.text_events.unwrap_or_default();
    let raw_text = event_capture
        .events
        .iter()
        .map(|event| event.text.as_str())
        .collect::<String>();
    let (normalized_sha256_audit_only, normalized_unicode_scalar_count) =
        normalized_text_digest(&raw_text);
    let raw_unicode_scalar_count = raw_text.chars().count();
    let raw_sha256_audit_only = {
        let mut hasher = Sha256::new();
        hasher.update(raw_text.as_bytes());
        hex_lower(&hasher.finalize())
    };
    let unresolved_text_reference_count = event_capture.unresolved_text_reference_count;
    let include_private_text = event_capture.include_private_text;
    let events: Vec<KfxTextEvent> = event_capture
        .events
        .into_iter()
        .enumerate()
        .map(|(index, event)| {
            let mut hasher = Sha256::new();
            hasher.update(event.text.as_bytes());
            KfxTextEvent {
                trace_id: format!("T{:06}", index + 1),
                section_id: event.section_id,
                context: if event.story_id.is_some() {
                    "Story".to_owned()
                } else {
                    "Body".to_owned()
                },
                context_basis: if event.story_id.is_some() {
                    "reading_order_section_story_reference".to_owned()
                } else {
                    "KFX_type_259_content_fragment".to_owned()
                },
                story_id: event.story_id,
                source_fragment: event.source_fragment,
                source_fragment_order: event.source_fragment_order,
                eid: event.eid,
                source_path: event.source_path,
                source_value_kind: event.text_kind.to_owned(),
                source_position: event.source_position,
                source_position_kind: if event.source_position.is_some() {
                    "field_155_kfx_navigation_target_id".to_owned()
                } else {
                    "not_present".to_owned()
                },
                text_len: event.text.chars().count(),
                text_hash_raw_audit_only: hex_lower(&hasher.finalize()),
                text: include_private_text.then_some(event.text),
                adjacent_style_events: event.adjacent_style_events,
            }
        })
        .collect();
    let mut source_features = source_features;
    source_features.has_footnotes = Some(has_footnotes);
    source_features.has_mathml = Some(has_mathml || math_candidate);
    source_features.has_illustrated_layout = Some(has_illustrated_layout || illustrated_candidate);
    let mut feature_states = BTreeMap::new();
    if conditional_candidate {
        feature_states.insert("conditional_content".to_owned(), "Unknown".to_owned());
    }
    if render_inline_candidate {
        feature_states.insert("non_image_render_inline".to_owned(), "Unknown".to_owned());
    }
    if illustrated_candidate || has_illustrated_layout {
        feature_states.insert(
            "illustrated_layout".to_owned(),
            if has_illustrated_layout {
                "Mapped"
            } else if has_approximate_illustrated_output {
                "Approximated"
            } else {
                "Unknown"
            }
            .to_owned(),
        );
    }
    if note_candidate || has_footnotes {
        feature_states.insert(
            "footnotes".to_owned(),
            if has_footnotes { "Mapped" } else { "Unknown" }.to_owned(),
        );
    }
    if math_candidate || has_mathml {
        feature_states.insert(
            "mathml".to_owned(),
            if has_mathml { "Mapped" } else { "Unknown" }.to_owned(),
        );
    }
    if has_ruby {
        feature_states.insert("ruby".to_owned(), "Mapped".to_owned());
    }
    source_features.feature_states = feature_states;
    let mut undecoded_features = vec!["creator".to_owned(), "creator_version".to_owned()];
    if conditional_candidate {
        undecoded_features.push("conditional_content".to_owned());
    }
    if illustrated_candidate
        && source_features
            .feature_states
            .get("illustrated_layout")
            .is_some_and(|state| state == "Unknown")
    {
        undecoded_features.push("illustrated_layout".to_owned());
    }
    if note_candidate && !has_footnotes {
        undecoded_features.push("footnotes".to_owned());
    }
    if render_inline_candidate {
        undecoded_features.push("non_image_render_inline".to_owned());
    }
    if math_candidate && !has_mathml {
        undecoded_features.push("mathml".to_owned());
    }
    source_features.undecoded_features = undecoded_features;
    source_features.has_image_occurrences = !capture.native.is_empty();
    Ok(KfxTextEventAudit {
        schema_version: 1,
        input_sha256_audit_only: {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            hex_lower(&hasher.finalize())
        },
        status: "Complete".to_owned(),
        source_features,
        stream: KfxTextStreamSummary {
            event_count: events.len(),
            unresolved_text_reference_count,
            raw_unicode_scalar_count,
            raw_sha256_audit_only,
            normalized_unicode_scalar_count,
            normalized_sha256_audit_only,
        },
        events,
    })
}

/// Build a content-free inventory of decoded Ion string values in KFX
/// fragment payloads. Private values are returned only when explicitly
/// requested for a temporary local root-cause comparison.
pub fn audit_source_strings(
    bytes: &[u8],
    include_private_strings: bool,
) -> Result<serde_json::Value, AmazonKfxError> {
    audit_source_strings_impl(bytes, include_private_strings, None)
}

/// Return only source-string inventory rows containing one or more private
/// query strings. Query values are never copied into the returned report.
pub fn audit_source_string_matches(
    bytes: &[u8],
    queries: &[(String, String, bool)],
) -> Result<serde_json::Value, AmazonKfxError> {
    audit_source_strings_impl(bytes, false, Some(queries))
}

fn audit_source_strings_impl(
    bytes: &[u8],
    include_private_strings: bool,
    queries: Option<&[(String, String, bool)]>,
) -> Result<serde_json::Value, AmazonKfxError> {
    if !bytes.starts_with(b"CONT") {
        return Err(AmazonKfxError::InvalidHeader(
            "source-string audit requires an Amazon CONT KFX input".to_owned(),
        ));
    }
    let model = parse_native(bytes, DecodeOptions::default())?;
    if model.drm_detected {
        return Err(AmazonKfxError::ProtectedContent);
    }

    let content_indices = ordered_content_fragment_indices(&model).unwrap_or_else(|| {
        model
            .fragments
            .iter()
            .enumerate()
            .filter_map(|(index, fragment)| (fragment.fragment_type == 259).then_some(index))
            .collect()
    });
    let content_aliases: BTreeMap<usize, String> = content_indices
        .iter()
        .filter(|index| model.fragments[**index].fragment_type == 259)
        .enumerate()
        .map(|(order, index)| (*index, format!("F{:03}", order + 1)))
        .collect();
    let query_matcher = queries.map(SourceStringQueryMatcher::new);

    let mut references = BTreeMap::<(u32, usize), Vec<serde_json::Value>>::new();
    for (fragment_index, fragment) in model.fragments.iter().enumerate() {
        let alias = content_aliases.get(&fragment_index).map(String::as_str);
        let context = SourceStringFragmentContext {
            fragment_index,
            fragment_type: fragment.fragment_type,
            entity_id: fragment.entity_id,
            content_fragment_alias: alias,
            string_table_id: None,
        };
        let mut walk = SourceStringReferenceWalk {
            string_tables: &model.string_tables,
            path: Vec::new(),
            parent_field_ids: Vec::new(),
            references: BTreeMap::new(),
        };
        collect_source_string_references(&fragment.value, context, &mut walk);
        for (key, rows) in walk.references {
            references.entry(key).or_default().extend(rows);
        }
    }

    let mut entries = Vec::new();
    for (fragment_index, fragment) in model.fragments.iter().enumerate() {
        let alias = content_aliases.get(&fragment_index).map(String::as_str);
        let table_id = (fragment.fragment_type == 145).then(|| {
            struct_get(&fragment.value, FIELD_FRAGMENT_ID)
                .and_then(|value| match value {
                    IonValue::Symbol(id) => u32::try_from(*id).ok(),
                    _ => None,
                })
                .unwrap_or(fragment.entity_id)
        });
        let context = SourceStringFragmentContext {
            fragment_index,
            fragment_type: fragment.fragment_type,
            entity_id: fragment.entity_id,
            content_fragment_alias: alias,
            string_table_id: table_id,
        };
        let mut walk = SourceStringEntryWalk {
            query_matcher: query_matcher.as_ref(),
            ..SourceStringEntryWalk::default()
        };
        collect_source_string_entries(&fragment.value, context, &mut walk);
        entries.extend(walk.entries);
    }

    let strings: Vec<serde_json::Value> = entries
        .into_iter()
        .enumerate()
        .map(|(ordinal, entry)| {
            let linked_references = entry
                .string_table_id
                .zip(entry.string_table_index)
                .and_then(|key| references.get(&key))
                .cloned()
                .unwrap_or_default();
            source_string_inventory_row(entry, ordinal, linked_references, include_private_strings)
        })
        .collect();
    let matched_query_id_set: BTreeSet<String> = strings
        .iter()
        .flat_map(|row| {
            row.get("matched_query_ids")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect();
    let matched_query_ids: Vec<String> = matched_query_id_set.iter().cloned().collect();
    let unmatched_query_ids: Vec<String> = queries
        .into_iter()
        .flatten()
        .filter(|(query_id, _, _)| !matched_query_id_set.contains(query_id))
        .map(|(query_id, _, _)| query_id.clone())
        .collect();

    Ok(serde_json::json!({
        "schema_version": 1,
        "input_sha256_audit_only": hex_lower(&Sha256::digest(bytes)),
        "coverage": "decoded Ion strings in parsed fragment payloads; opaque/raw payloads and undecoded entity metadata are excluded",
        "inventory_mode": if queries.is_some() { "query_matches_only" } else { "complete" },
        "query_count": queries.map(|items| items.len()),
        "matched_query_ids": matched_query_ids,
        "unmatched_query_ids": unmatched_query_ids,
        "private_strings_included": include_private_strings,
        "fragment_count": model.fragments.len(),
        "string_count": strings.len(),
        "strings": strings,
    }))
}

struct SourceStringEntry {
    fragment_index: usize,
    fragment_type: u32,
    entity_id: u32,
    content_fragment_alias: Option<String>,
    source_path: String,
    parent_field_ids: Vec<u64>,
    string_table_id: Option<u32>,
    string_table_index: Option<usize>,
    matched_query_ids: Vec<String>,
    exact_query_ids: Vec<String>,
    text: String,
}

#[derive(Clone, Copy)]
struct SourceStringFragmentContext<'a> {
    fragment_index: usize,
    fragment_type: u32,
    entity_id: u32,
    content_fragment_alias: Option<&'a str>,
    string_table_id: Option<u32>,
}

#[derive(Default)]
struct SourceStringEntryWalk<'a> {
    query_matcher: Option<&'a SourceStringQueryMatcher>,
    path: Vec<IonAuditPathToken>,
    parent_field_ids: Vec<u64>,
    entries: Vec<SourceStringEntry>,
}

#[derive(Default)]
struct SourceStringQueryMatcher {
    exact: HashMap<String, Vec<String>>,
    contains: Vec<(String, Vec<String>)>,
}

impl SourceStringQueryMatcher {
    fn new(queries: &[(String, String, bool)]) -> Self {
        let mut exact = HashMap::<String, Vec<String>>::new();
        let mut contains = BTreeMap::<String, Vec<String>>::new();
        for (query_id, query, exact_only) in queries {
            if query.is_empty() {
                continue;
            }
            if *exact_only {
                exact
                    .entry(query.clone())
                    .or_default()
                    .push(query_id.clone());
            } else {
                contains
                    .entry(query.clone())
                    .or_default()
                    .push(query_id.clone());
            }
        }
        Self {
            exact,
            contains: contains.into_iter().collect(),
        }
    }
}

struct SourceStringReferenceWalk<'a> {
    string_tables: &'a BTreeMap<u32, Vec<String>>,
    path: Vec<IonAuditPathToken>,
    parent_field_ids: Vec<u64>,
    references: BTreeMap<(u32, usize), Vec<serde_json::Value>>,
}

fn source_string_inventory_row(
    entry: SourceStringEntry,
    ordinal: usize,
    linked_references: Vec<serde_json::Value>,
    include_private_strings: bool,
) -> serde_json::Value {
    let mut hasher = Sha256::new();
    hasher.update(entry.text.as_bytes());
    let digest = hex_lower(&hasher.finalize());
    let role_candidate = if entry.string_table_id.is_some() {
        "string_table_entry_candidate"
    } else if entry.source_path.ends_with(".$584") {
        "direct_text_field_584_candidate"
    } else {
        "unclassified_raw_string"
    };
    let mut row = serde_json::json!({
        "string_id": format!("S{:07}", ordinal + 1),
        "fragment_index": entry.fragment_index,
        "fragment_type": entry.fragment_type,
        "entity_id": entry.entity_id,
        "content_fragment_alias": entry.content_fragment_alias,
        "source_path": entry.source_path,
        "parent_field_ids": entry.parent_field_ids,
        "role_candidate": role_candidate,
        "unicode_scalar_length": entry.text.chars().count(),
        "sha256_raw_audit_only": digest,
        "string_table_id": entry.string_table_id,
        "string_table_index": entry.string_table_index,
        "matched_query_ids": entry.matched_query_ids,
        "exact_query_ids": entry.exact_query_ids,
        "reference_count": linked_references.len(),
        "references": linked_references,
    });
    if include_private_strings {
        row["private_text"] = serde_json::Value::String(entry.text);
    }
    row
}

fn collect_source_string_entries(
    value: &IonValue,
    context: SourceStringFragmentContext<'_>,
    walk: &mut SourceStringEntryWalk<'_>,
) {
    match value {
        IonValue::String(text) => {
            let string_table_index = if context.fragment_type == 145 && walk.path.len() == 2 {
                match (&walk.path[0], &walk.path[1]) {
                    (
                        IonAuditPathToken::Field(FIELD_TEXT_VALUES),
                        IonAuditPathToken::Index(index),
                    ) => Some(*index),
                    _ => None,
                }
            } else {
                None
            };
            let mut matched_query_ids = Vec::new();
            let mut exact_query_ids = Vec::new();
            if let Some(matcher) = walk.query_matcher {
                if let Some(query_ids) = matcher.exact.get(text) {
                    matched_query_ids.extend(query_ids.iter().cloned());
                    exact_query_ids.extend(query_ids.iter().cloned());
                }
                for (query, query_ids) in &matcher.contains {
                    if text.contains(query) {
                        matched_query_ids.extend(query_ids.iter().cloned());
                        if text == query {
                            exact_query_ids.extend(query_ids.iter().cloned());
                        }
                    }
                }
                matched_query_ids.sort();
                matched_query_ids.dedup();
                exact_query_ids.sort();
                exact_query_ids.dedup();
            }
            if walk.query_matcher.is_none() || !matched_query_ids.is_empty() {
                walk.entries.push(SourceStringEntry {
                    fragment_index: context.fragment_index,
                    fragment_type: context.fragment_type,
                    entity_id: context.entity_id,
                    content_fragment_alias: context.content_fragment_alias.map(ToOwned::to_owned),
                    source_path: format_ion_audit_path(&walk.path),
                    parent_field_ids: walk.parent_field_ids.clone(),
                    string_table_id: string_table_index.and(context.string_table_id),
                    string_table_index,
                    matched_query_ids,
                    exact_query_ids,
                    text: text.clone(),
                });
            }
        }
        IonValue::Struct(fields) => {
            for (field, child) in fields {
                walk.path.push(IonAuditPathToken::Field(*field));
                walk.parent_field_ids.push(*field);
                collect_source_string_entries(child, context, walk);
                walk.parent_field_ids.pop();
                walk.path.pop();
            }
        }
        IonValue::List(items) | IonValue::SExp(items) => {
            for (index, child) in items.iter().enumerate() {
                walk.path.push(IonAuditPathToken::Index(index));
                collect_source_string_entries(child, context, walk);
                walk.path.pop();
            }
        }
        IonValue::Annotation { value, .. } => collect_source_string_entries(value, context, walk),
        _ => {}
    }
}

fn collect_source_string_references(
    value: &IonValue,
    context: SourceStringFragmentContext<'_>,
    walk: &mut SourceStringReferenceWalk<'_>,
) {
    if resolve_string_reference(value, walk.string_tables).is_some() {
        if let (Some(IonValue::Symbol(table_id)), Some(string_index)) = (
            struct_get(value, FIELD_FRAGMENT_ID),
            struct_get(value, FIELD_FRAGMENT_STRING_INDEX)
                .and_then(as_int)
                .and_then(|index| usize::try_from(index).ok()),
        ) {
            if let Ok(table_id) = u32::try_from(*table_id) {
                walk.references
                    .entry((table_id, string_index))
                    .or_default()
                    .push(serde_json::json!({
                        "fragment_index": context.fragment_index,
                        "fragment_type": context.fragment_type,
                        "entity_id": context.entity_id,
                        "content_fragment_alias": context.content_fragment_alias,
                        "source_path": format_ion_audit_path(&walk.path),
                        "parent_field_ids": walk.parent_field_ids,
                        "role_candidate": if context.fragment_type == 259 {
                            "content_fragment_string_reference_candidate"
                        } else {
                            "non_content_string_reference"
                        },
                    }));
            }
        }
    }
    match value {
        IonValue::Struct(fields) => {
            for (field, child) in fields {
                walk.path.push(IonAuditPathToken::Field(*field));
                walk.parent_field_ids.push(*field);
                collect_source_string_references(child, context, walk);
                walk.parent_field_ids.pop();
                walk.path.pop();
            }
        }
        IonValue::List(items) | IonValue::SExp(items) => {
            for (index, child) in items.iter().enumerate() {
                walk.path.push(IonAuditPathToken::Index(index));
                collect_source_string_references(child, context, walk);
                walk.path.pop();
            }
        }
        IonValue::Annotation { value, .. } => {
            collect_source_string_references(value, context, walk)
        }
        _ => {}
    }
}

/// Inspect one emitted text event's native Ion source, without dumping the
/// containing book. The fragment selector is the anonymous `Fnnn` alias from
/// `audit_text_events`; `source_path` is the event's numeric Ion path. String
/// values are represented by scalar length and digest unless private text is
/// explicitly requested for a local diagnostic.
pub fn audit_text_fragment_source(
    bytes: &[u8],
    fragment_alias: &str,
    source_path: &str,
    include_private_text: bool,
    include_private_symbol_names: bool,
) -> Result<serde_json::Value, AmazonKfxError> {
    if !bytes.starts_with(b"CONT") {
        return Err(AmazonKfxError::InvalidHeader(
            "KFX source dump requires an Amazon CONT KFX input".to_owned(),
        ));
    }
    let Some(alias_number) = fragment_alias
        .strip_prefix('F')
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
    else {
        return Err(AmazonKfxError::Semantic(
            "fragment must be an anonymous text-event alias such as F006".to_owned(),
        ));
    };
    let path = parse_ion_audit_path(source_path)?;
    let model = parse_native(bytes, DecodeOptions::default())?;
    if model.drm_detected {
        return Err(AmazonKfxError::ProtectedContent);
    }

    let content_indices = ordered_content_fragment_indices(&model).unwrap_or_else(|| {
        model
            .fragments
            .iter()
            .enumerate()
            .filter_map(|(index, fragment)| (fragment.fragment_type == 259).then_some(index))
            .collect()
    });
    let ordered = content_indices
        .iter()
        .filter(|index| model.fragments[**index].fragment_type == 259)
        .copied()
        .collect::<Vec<_>>();
    let fragment_index = *ordered.get(alias_number - 1).ok_or_else(|| {
        AmazonKfxError::Semantic(format!(
            "content fragment alias {fragment_alias} was not found"
        ))
    })?;
    let fragment = &model.fragments[fragment_index];
    let symbol_table = model
        .containers
        .get(fragment.container_index)
        .map(|container| &container.symbols);
    let target = resolve_ion_audit_path(&fragment.value, &path).ok_or_else(|| {
        AmazonKfxError::Semantic(format!(
            "source path {source_path} does not resolve in fragment {fragment_alias}"
        ))
    })?;
    let adjacent_style_events = path
        .last()
        .filter(|token| {
            matches!(
                token,
                IonAuditPathToken::Field(FIELD_CONTENT_STRING_REFERENCE)
            )
        })
        .and_then(|_| resolve_ion_audit_path(&fragment.value, &path[..path.len() - 1]))
        .and_then(audit_adjacent_style_events);
    let trace_locations = ordered_content_trace_locations(&model);
    let location = trace_locations.get(&fragment_index);

    let text_audit = audit_text_events(bytes, include_private_text)?;
    let event = text_audit
        .events
        .iter()
        .find(|event| event.source_fragment == fragment_alias && event.source_path == source_path);
    let event_value = event
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| AmazonKfxError::Semantic(error.to_string()))?;
    let (resolved_text, string_reference) = match target {
        IonValue::String(text) => (Some(text.as_str()), None),
        value if source_path.ends_with(&format!("${FIELD_CONTENT_STRING_REFERENCE}")) => {
            let table_id = struct_get(value, FIELD_FRAGMENT_ID).and_then(|value| match value {
                IonValue::Symbol(id) => u32::try_from(*id).ok(),
                _ => None,
            });
            let string_index = struct_get(value, FIELD_FRAGMENT_STRING_INDEX)
                .and_then(as_int)
                .and_then(|value| usize::try_from(value).ok());
            let text = resolve_string_reference(value, &model.string_tables);
            (
                text,
                Some(serde_json::json!({
                    "table_symbol_id": table_id,
                    "string_index": string_index,
                    "resolved": text.is_some(),
                })),
            )
        }
        _ => (None, None),
    };
    let resolved_text_summary = resolved_text.map(|text| {
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        let mut summary = serde_json::json!({
            "unicode_scalar_count": text.chars().count(),
            "sha256_raw_audit_only": hex_lower(&hasher.finalize()),
        });
        if include_private_text {
            summary["private_text"] = serde_json::Value::String(text.to_owned());
        }
        summary
    });

    let mut context = Vec::new();
    let mut current = &fragment.value;
    let mut current_path = Vec::new();
    for token in &path {
        let Some((child, siblings)) = ion_audit_step(current, token) else {
            return Err(AmazonKfxError::Semantic(format!(
                "source path {source_path} does not resolve in fragment {fragment_alias}"
            )));
        };
        context.push(serde_json::json!({
            "parent_path": format_ion_audit_path(&current_path),
            "parent": ion_audit_value_summary(current, include_private_text, symbol_table, include_private_symbol_names),
            "selected_edge": ion_audit_edge(token),
            "sibling_summaries": siblings.into_iter().map(|(edge, value)| serde_json::json!({
                "edge": edge,
                "value": ion_audit_value_summary(value, include_private_text, symbol_table, include_private_symbol_names),
            })).collect::<Vec<_>>(),
        }));
        current = child;
        current_path.push(token.clone());
    }

    let input_hash = Sha256::digest(bytes);
    let (container_symbol, content_symbol_id) = model
        .containers
        .get(fragment.container_index)
        .map(|container| {
            let symbol =
                struct_get(&fragment.value, FIELD_CONTENT_FRAGMENT_SYMBOL).and_then(|value| {
                    match value {
                        IonValue::Symbol(id) => Some(*id),
                        _ => None,
                    }
                });
            (container.symbols.names.len(), symbol)
        })
        .unwrap_or_default();

    Ok(serde_json::json!({
        "schema_version": 1,
        "input_sha256_audit_only": hex_lower(&input_hash),
        "fragment": {
            "alias": fragment_alias,
            "entity_id": fragment.entity_id,
            "fragment_type": fragment.fragment_type,
            "container_symbol_count": container_symbol,
            "content_symbol_id": content_symbol_id,
            "section_id": location.and_then(|item| item.section_id.clone()),
            "story_id": location.and_then(|item| item.story_id.clone()),
            "relationship_basis": if location.and_then(|item| item.story_id.as_ref()).is_some() {
                "reading_order_to_section_to_story_reference"
            } else {
                "content_fragment_without_proven_story_reference"
            },
        },
        "event": event_value,
        "source_path": source_path,
        "path_context": context,
        "target": ion_audit_value_summary(target, include_private_text, symbol_table, include_private_symbol_names),
        "adjacent_style_events": adjacent_style_events,
        "string_reference": string_reference,
        "resolved_text": resolved_text_summary,
        "private_text_included": include_private_text,
        "private_symbol_names_included": include_private_symbol_names,
    }))
}

fn audit_adjacent_style_events(parent: &IonValue) -> Option<serde_json::Value> {
    let items = match struct_get(parent, FIELD_TEXT_STYLE_EVENTS)? {
        IonValue::List(items) | IonValue::SExp(items) => items,
        _ => return None,
    };
    let entries = summarize_text_style_events(items);
    Some(serde_json::json!({
        "item_count": items.len(),
        "events": entries,
    }))
}

fn summarize_text_style_events(items: &[IonValue]) -> Vec<KfxTextStyleEvent> {
    items
        .iter()
        .enumerate()
        .map(|(list_index, item)| KfxTextStyleEvent {
            list_index,
            text_offset: struct_get(item, FIELD_TEXT_STYLE_OFFSET).and_then(as_int),
            text_length: struct_get(item, FIELD_TEXT_STYLE_LENGTH).and_then(as_int),
            style_symbol_id: struct_get(item, FIELD_TEXT_STYLE_KIND).and_then(
                |value| match value {
                    IonValue::Symbol(symbol_id) => Some(*symbol_id),
                    _ => None,
                },
            ),
        })
        .collect()
}

fn adjacent_text_style_events(parent: &IonValue) -> Vec<KfxTextStyleEvent> {
    match struct_get(parent, FIELD_TEXT_STYLE_EVENTS) {
        Some(IonValue::List(items) | IonValue::SExp(items)) => summarize_text_style_events(items),
        _ => Vec::new(),
    }
}

fn content_link_ranges(parent: &IonValue) -> Vec<KfxTextLinkRange> {
    let Some(IonValue::List(items) | IonValue::SExp(items)) =
        struct_get(parent, FIELD_TEXT_STYLE_EVENTS)
    else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| {
            let target_symbol_id =
                struct_get(item, FIELD_TEXT_STYLE_LINK_TARGET).and_then(ion_symbol_id)?;
            let (start, end) = text_style_range(item)?;
            Some(KfxTextLinkRange {
                text_offset: start,
                text_length: end - start,
                target_symbol_id,
            })
        })
        .collect()
}

/// Link ranges on a structural content container use the scalar offsets of
/// the container's concatenated text. Nested `$146` items are emitted as
/// separate IR text nodes, so translate the intersecting part into the local
/// offset space before lowering the link.
fn localize_kfx_link_ranges(
    ranges: &[KfxTextLinkRange],
    segment_offset: usize,
    segment_length: usize,
) -> Vec<KfxTextLinkRange> {
    let Some(segment_end) = segment_offset.checked_add(segment_length) else {
        return Vec::new();
    };
    ranges
        .iter()
        .filter_map(|range| {
            let range_end = range.text_offset.checked_add(range.text_length)?;
            let start = range.text_offset.max(segment_offset);
            let end = range_end.min(segment_end);
            if start >= end {
                return None;
            }
            Some(KfxTextLinkRange {
                text_offset: start - segment_offset,
                text_length: end - start,
                target_symbol_id: range.target_symbol_id,
            })
        })
        .collect()
}

/// Advance a nested `$146` leaf in the parent `$142` coordinate space. KFX
/// reserves one scalar at each leaf boundary; that separator is not emitted as
/// reading text but remains part of the source range coordinates.
fn advance_kfx_nested_text_offset(segment_offset: usize, segment_length: usize) -> usize {
    segment_offset
        .saturating_add(segment_length)
        .saturating_add(1)
}

/// A link range on an image-only `$269` block has no text leaf to consume its
/// `$143/$144` span. Calibre applies that block-level range to the contained
/// image run. Only accept the unambiguous single-target form; multiple
/// image-target ranges need per-child source evidence and remain unresolved.
fn image_link_target(block_target: Option<u64>, link_ranges: &[KfxTextLinkRange]) -> Option<u64> {
    block_target.or_else(|| {
        if link_ranges.len() == 1 && link_ranges[0].text_offset == 0 {
            Some(link_ranges[0].target_symbol_id)
        } else {
            None
        }
    })
}

#[derive(Clone, Debug)]
enum IonAuditPathToken {
    Field(u64),
    Index(usize),
}

fn parse_ion_audit_path(path: &str) -> Result<Vec<IonAuditPathToken>, AmazonKfxError> {
    let bytes = path.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'$' => {
                index += 1;
                let start = index;
                while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                    index += 1;
                }
                let field = path.get(start..index).and_then(|value| value.parse().ok());
                let Some(field) = field else {
                    return Err(AmazonKfxError::Semantic(
                        "Ion source path contains an invalid field selector".to_owned(),
                    ));
                };
                tokens.push(IonAuditPathToken::Field(field));
            }
            b'[' => {
                index += 1;
                let start = index;
                while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                    index += 1;
                }
                if bytes.get(index) != Some(&b']') {
                    return Err(AmazonKfxError::Semantic(
                        "Ion source path contains an invalid list selector".to_owned(),
                    ));
                }
                let item = path.get(start..index).and_then(|value| value.parse().ok());
                let Some(item) = item else {
                    return Err(AmazonKfxError::Semantic(
                        "Ion source path contains an invalid list index".to_owned(),
                    ));
                };
                tokens.push(IonAuditPathToken::Index(item));
                index += 1;
            }
            b'.' => index += 1,
            _ => {
                return Err(AmazonKfxError::Semantic(
                    "Ion source path contains an unsupported character".to_owned(),
                ));
            }
        }
    }
    if tokens.is_empty() {
        return Err(AmazonKfxError::Semantic(
            "Ion source path must select at least one field or list item".to_owned(),
        ));
    }
    Ok(tokens)
}

fn resolve_ion_audit_path<'a>(
    mut value: &'a IonValue,
    path: &[IonAuditPathToken],
) -> Option<&'a IonValue> {
    for token in path {
        while let IonValue::Annotation { value: inner, .. } = value {
            value = inner;
        }
        value = match value {
            IonValue::Struct(fields) => match token {
                IonAuditPathToken::Field(field) => fields
                    .iter()
                    .find_map(|(id, child)| (*id == *field).then_some(child))?,
                IonAuditPathToken::Index(_) => return None,
            },
            IonValue::List(items) | IonValue::SExp(items) => match token {
                IonAuditPathToken::Index(item) => items.get(*item)?,
                IonAuditPathToken::Field(_) => return None,
            },
            _ => return None,
        };
    }
    Some(value)
}

fn ion_audit_step<'a>(
    value: &'a IonValue,
    token: &IonAuditPathToken,
) -> Option<(&'a IonValue, Vec<(serde_json::Value, &'a IonValue)>)> {
    match (value, token) {
        (IonValue::Annotation { value, .. }, token) => ion_audit_step(value, token),
        (IonValue::Struct(fields), IonAuditPathToken::Field(field)) => {
            let selected = fields.iter().position(|(id, _)| id == field)?;
            let siblings = fields
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != selected)
                .map(|(_, (id, value))| (serde_json::json!({"field_id": id}), value))
                .collect();
            Some((&fields[selected].1, siblings))
        }
        (IonValue::List(items) | IonValue::SExp(items), IonAuditPathToken::Index(item)) => {
            let selected = items.get(*item)?;
            let start = item.saturating_sub(2);
            let end = (*item + 3).min(items.len());
            let siblings = items[start..end]
                .iter()
                .enumerate()
                .filter(|(offset, _)| start + offset != *item)
                .map(|(offset, value)| (serde_json::json!({"index": start + offset}), value))
                .collect();
            Some((selected, siblings))
        }
        _ => None,
    }
}

fn ion_audit_edge(token: &IonAuditPathToken) -> serde_json::Value {
    match token {
        IonAuditPathToken::Field(field) => serde_json::json!({"field_id": field}),
        IonAuditPathToken::Index(index) => serde_json::json!({"list_index": index}),
    }
}

fn format_ion_audit_path(path: &[IonAuditPathToken]) -> String {
    let mut output = String::new();
    for token in path {
        match token {
            IonAuditPathToken::Field(field) => {
                if !output.is_empty() {
                    output.push('.');
                }
                output.push('$');
                output.push_str(&field.to_string());
            }
            IonAuditPathToken::Index(index) => {
                output.push('[');
                output.push_str(&index.to_string());
                output.push(']');
            }
        }
    }
    if output.is_empty() {
        "$".to_owned()
    } else {
        output
    }
}

fn ion_audit_value_summary(
    value: &IonValue,
    include_private_text: bool,
    symbols: Option<&SymbolTable>,
    include_private_symbol_names: bool,
) -> serde_json::Value {
    match value {
        IonValue::Null => serde_json::json!({"kind": "null"}),
        IonValue::Nop => serde_json::json!({"kind": "nop"}),
        IonValue::Bool(value) => serde_json::json!({"kind": "bool", "value": value}),
        IonValue::Int(value) => serde_json::json!({"kind": "int", "value": value}),
        IonValue::Symbol(value) => {
            let mut summary = serde_json::json!({"kind": "symbol", "symbol_id": value});
            if include_private_symbol_names {
                summary["resolved_name"] = symbols
                    .and_then(|table| table.names.get(value))
                    .cloned()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null);
            }
            summary
        }
        IonValue::Float(bytes) => serde_json::json!({"kind": "float", "byte_length": bytes.len()}),
        IonValue::Decimal(bytes) => {
            serde_json::json!({"kind": "decimal", "byte_length": bytes.len()})
        }
        IonValue::Timestamp(bytes) => {
            serde_json::json!({"kind": "timestamp", "byte_length": bytes.len()})
        }
        IonValue::String(text) => {
            let mut hasher = Sha256::new();
            hasher.update(text.as_bytes());
            let mut summary = serde_json::json!({
                "kind": "string",
                "unicode_scalar_count": text.chars().count(),
                "sha256_raw_audit_only": hex_lower(&hasher.finalize()),
            });
            if include_private_text {
                summary["private_text"] = serde_json::Value::String(text.clone());
            }
            summary
        }
        IonValue::Clob(bytes) | IonValue::Blob(bytes) | IonValue::Reserved { bytes, .. } => {
            let digest = Sha256::digest(bytes);
            serde_json::json!({"kind": ion_value_kind(value), "byte_length": bytes.len(), "sha256_audit_only": hex_lower(&digest)})
        }
        IonValue::List(items) | IonValue::SExp(items) => serde_json::json!({
            "kind": ion_value_kind(value),
            "item_count": items.len(),
        }),
        IonValue::Struct(fields) => serde_json::json!({
            "kind": "struct",
            "field_count": fields.len(),
            "fields": fields.iter().take(32).map(|(field, value)| serde_json::json!({
                "field_id": field,
                "value": ion_audit_value_summary(value, include_private_text, symbols, include_private_symbol_names),
            })).collect::<Vec<_>>(),
            "fields_truncated": fields.len() > 32,
        }),
        IonValue::Annotation {
            symbols: annotation_symbols,
            value,
        } => serde_json::json!({
            "kind": "annotation",
            "annotation_symbol_ids": annotation_symbols,
            "value": ion_audit_value_summary(value, include_private_text, symbols, include_private_symbol_names),
        }),
    }
}

fn ion_value_kind(value: &IonValue) -> &'static str {
    match value {
        IonValue::Null => "null",
        IonValue::Nop => "nop",
        IonValue::Bool(_) => "bool",
        IonValue::Int(_) => "int",
        IonValue::Float(_) => "float",
        IonValue::Decimal(_) => "decimal",
        IonValue::Timestamp(_) => "timestamp",
        IonValue::Symbol(_) => "symbol",
        IonValue::String(_) => "string",
        IonValue::Clob(_) => "clob",
        IonValue::Blob(_) => "blob",
        IonValue::List(_) => "list",
        IonValue::SExp(_) => "sexp",
        IonValue::Struct(_) => "struct",
        IonValue::Annotation { .. } => "annotation",
        IonValue::Reserved { .. } => "reserved",
    }
}

fn refresh_fidelity_diagnostics(model: &NativeModel, report: &mut KfxFidelityAudit) {
    report
        .diagnostics
        .insert("parser_warning_count".to_owned(), model.warnings.len());
    report.diagnostics.insert(
        "unknown_feature_count".to_owned(),
        model.unknown_features.len(),
    );
    report
        .diagnostics
        .insert("input_loss_count".to_owned(), model.input_loss.len());
    report.diagnostics.insert(
        "recovery_action_count".to_owned(),
        recovery_actions(ParseMode::Compatible, model).len(),
    );
    report.diagnostics.insert(
        "broken_reference_feature_count".to_owned(),
        model
            .unknown_features
            .iter()
            .filter(|feature| feature.contains("unresolved") || feature.contains("dangling"))
            .count(),
    );
    report.diagnostics.insert(
        "missing_symbol_feature_count".to_owned(),
        model
            .unknown_features
            .iter()
            .filter(|feature| feature.contains("symbol") && feature.contains("unresolved"))
            .count(),
    );
    report.diagnostics.insert(
        "unsupported_structure_feature_count".to_owned(),
        model
            .unknown_features
            .iter()
            .filter(|feature| {
                feature.contains("unsupported") || feature.contains("unknown-fragment")
            })
            .count(),
    );
    for (diagnostic, feature) in [
        (
            "unresolved_text_reference_feature_count",
            "kfx-text-reference-unresolved",
        ),
        (
            "unresolved_navigation_target_feature_count",
            "kfx-navigation-target-unresolved",
        ),
        (
            "navigation_not_recovered_feature_count",
            "kfx-navigation-not-recovered",
        ),
        (
            "navigation_shape_unsupported_feature_count",
            "kfx-navigation-fragment-shape-unsupported",
        ),
        (
            "navigation_entries_unsupported_feature_count",
            "kfx-navigation-entry-shape-unsupported",
        ),
        (
            "inferred_image_placement_feature_count",
            "kfx-image-placeholder-placement-inferred",
        ),
        (
            "resource_identity_incomplete_feature_count",
            "kfx-resource-byte-association-incomplete",
        ),
    ] {
        report.diagnostics.insert(
            diagnostic.to_owned(),
            usize::from(model.unknown_features.contains(feature)),
        );
    }
}

fn summarize_captured_text(capture: &TextTraceCapture, text: &mut KfxFidelityText) {
    text.unresolved_text_reference_count = capture.unresolved_text_reference_count;
    let mut source_text = String::new();
    for unit in &capture.source_fragments {
        source_text.push_str(&unit.text);
        text.source_text_segment_count += unit.text_segment_count;
        let (digest, scalar_count) = normalized_text_digest(&unit.text);
        text.source_fragments.push(KfxFidelityTextUnit {
            content_fragment_order: Some(unit.content_fragment_order),
            document_order: None,
            text_segment_count: unit.text_segment_count,
            text_node_count: 0,
            unicode_scalar_count: scalar_count,
            normalized_sha256_audit_only: digest,
        });
    }
    let (source_digest, source_scalar_count) = normalized_text_digest(&source_text);
    text.source_unicode_scalar_count = source_scalar_count;
    text.source_normalized_sha256_audit_only = source_digest;

    let mut semantic_text = String::new();
    for unit in &capture.semantic_documents {
        semantic_text.push_str(&unit.text);
        text.semantic_text_segment_count += unit.text_segment_count;
        let (digest, scalar_count) = normalized_text_digest(&unit.text);
        text.semantic_documents.push(KfxFidelityTextUnit {
            content_fragment_order: Some(unit.content_fragment_order),
            document_order: unit.document_order,
            text_segment_count: unit.text_segment_count,
            text_node_count: 0,
            unicode_scalar_count: scalar_count,
            normalized_sha256_audit_only: digest,
        });
    }
    let (semantic_digest, semantic_scalar_count) = normalized_text_digest(&semantic_text);
    text.semantic_unicode_scalar_count = semantic_scalar_count;
    text.semantic_normalized_sha256_audit_only = semantic_digest;
}

fn summarize_book(book: &Book, report: &mut KfxFidelityAudit) {
    let mut text = String::new();
    for (document_order, document) in book.documents.iter().enumerate() {
        let text_node_count_before = report.text.text_node_count;
        let mut document_text = String::new();
        for node in &document.nodes {
            summarize_node(node, &mut document_text, report);
        }
        text.push_str(&document_text);
        let (document_digest, document_scalar_count) = normalized_text_digest(&document_text);
        report.text.ir_documents.push(KfxFidelityTextUnit {
            content_fragment_order: None,
            document_order: Some(document_order),
            text_segment_count: 0,
            text_node_count: report.text.text_node_count - text_node_count_before,
            unicode_scalar_count: document_scalar_count,
            normalized_sha256_audit_only: document_digest,
        });
    }
    let (digest, scalar_count) = normalized_text_digest(&text);
    report.text.normalized_sha256_audit_only = digest;
    report.text.unicode_scalar_count = scalar_count;
    for (id, style) in book.styles.iter() {
        let _ = id;
        report.styles.style_count += 1;
        report.styles.style_property_count += style.properties.len();
    }
    report.styles.distinct_style_property_count = book
        .styles
        .iter()
        .flat_map(|(_, style)| style.properties.keys().cloned())
        .collect::<BTreeSet<_>>()
        .len();
    report.links.anchor_count = book.anchors.len();
    report.links.anchor_graph_edge_count = book.navigation.anchor_graph.edges.len();
}

fn summarize_node(node: &folio_model::Node, text: &mut String, report: &mut KfxFidelityAudit) {
    if let NodeKind::Text { value } = &node.kind {
        report.text.text_node_count += 1;
        text.push_str(value);
    }
    let name = node.kind.name();
    if matches!(
        name,
        "inline" | "emphasis" | "strong" | "code" | "ruby" | "generic-inline" | "footnote"
    ) {
        *report
            .inline
            .inline_node_counts
            .entry(name.to_owned())
            .or_default() += 1;
    }
    match &node.kind {
        NodeKind::Image { alt, .. } | NodeKind::Svg { alt, .. } => {
            if alt.is_empty() {
                report.inline.image_alt_empty_count += 1;
            } else {
                report.inline.image_alt_present_count += 1;
            }
        }
        NodeKind::Link { .. } => report.links.link_node_count += 1,
        NodeKind::Anchor { .. } => report.links.anchor_node_count += 1,
        _ => {}
    }
    if node.style.get() != 0 {
        report.styles.nodes_with_nondefault_style_count += 1;
    }
    for child in &node.children {
        summarize_node(child, text, report);
    }
}

fn count_navigation_points(point: &NavPoint) -> usize {
    1 + point
        .children
        .iter()
        .map(count_navigation_points)
        .sum::<usize>()
}

fn normalized_text_digest(text: &str) -> (String, usize) {
    let line_normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = line_normalized.nfc().collect::<String>();
    let scalar_count = normalized.chars().count();
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    (hex_lower(&hasher.finalize()), scalar_count)
}

fn validated_language_tag(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 35
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || !value.bytes().any(|byte| byte.is_ascii_alphabetic())
    {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

fn looks_like_font_descriptor(format: Option<&str>, mime: Option<&str>) -> bool {
    let format = format.unwrap_or_default().to_ascii_lowercase();
    let mime = mime.unwrap_or_default().to_ascii_lowercase();
    matches!(format.as_str(), "ttf" | "otf" | "woff" | "woff2" | "font")
        || mime.starts_with("font/")
        || matches!(
            mime.as_str(),
            "application/font-woff"
                | "application/font-woff2"
                | "application/vnd.ms-opentype"
                | "application/x-font-ttf"
                | "application/x-font-opentype"
                | "application/x-font-truetype"
        )
}

fn resource_kind_label(kind: &ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Jpeg => "jpeg",
        ResourceKind::Png => "png",
        ResourceKind::Gif => "gif",
        ResourceKind::Svg => "svg",
        ResourceKind::Font => "font",
        ResourceKind::Stylesheet => "stylesheet",
        ResourceKind::Audio => "audio",
        ResourceKind::Unknown => "unknown",
    }
}

fn increment_count(counts: &mut BTreeMap<String, usize>, key: &str) {
    *counts.entry(key.to_owned()).or_default() += 1;
}

fn fidelity_error_code(error: &AmazonKfxError) -> &'static str {
    match error {
        AmazonKfxError::Truncated(_) => "KFX_PARSE_TRUNCATED",
        AmazonKfxError::InvalidHeader(_) => "KFX_PARSE_INVALID_HEADER",
        AmazonKfxError::UnsupportedVersion(_) => "KFX_PARSE_UNSUPPORTED_VERSION",
        AmazonKfxError::InvalidEntityIndex(_) => "KFX_PARSE_INVALID_ENTITY_INDEX",
        AmazonKfxError::InvalidEntity(_, _) => "KFX_PARSE_INVALID_ENTITY",
        AmazonKfxError::Ion(_) => "KFX_PARSE_INVALID_ION",
        AmazonKfxError::UnknownSymbol(_) => "KFX_PARSE_UNKNOWN_SYMBOL",
        AmazonKfxError::ProtectedContent => "KFX_DRM_PROTECTED",
        AmazonKfxError::UnresolvedVisibleResources(_) => "KFX_UNRESOLVED_VISIBLE_RESOURCES",
        AmazonKfxError::Semantic(_) => "KFX_SEMANTIC_DECODE_FAILED",
    }
}

fn struct_fields(value: &IonValue) -> Option<&[(u64, IonValue)]> {
    unwrap_struct(value)
}

fn audit_dimension(value: &IonValue, field: u64) -> Option<u32> {
    struct_get(value, field)
        .and_then(as_int)
        .and_then(|value| u32::try_from(value).ok())
}

fn resolved_fragment_fid(model: &NativeModel, fragment: &NativeFragment) -> Option<String> {
    let container = model.containers.get(fragment.container_index)?;
    container
        .symbols
        .names
        .get(&u64::from(fragment.entity_id))
        .cloned()
}

fn audit_string_or_symbol(
    value: Option<&IonValue>,
    container: Option<&NativeContainerOwned>,
) -> Option<String> {
    match value? {
        IonValue::String(value) => Some(value.clone()),
        IonValue::Symbol(symbol) => container?.symbols.names.get(symbol).cloned(),
        _ => None,
    }
}

fn resource_location(
    model: &NativeModel,
    fragment: &NativeFragment,
) -> (Option<String>, Option<u64>, String) {
    match struct_get(&fragment.value, FIELD_RESOURCE_METADATA_PATH) {
        Some(IonValue::String(value)) => (Some(value.clone()), None, "ion_string".to_owned()),
        Some(IonValue::Symbol(symbol)) => {
            let name = model
                .containers
                .get(fragment.container_index)
                .and_then(|container| container.symbols.names.get(symbol))
                .cloned();
            let source = if name.is_some() {
                "container_symbol_table"
            } else {
                "unresolved_symbol"
            };
            (name, Some(*symbol), source.to_owned())
        }
        Some(_) => (None, None, "unsupported_value_type".to_owned()),
        None => (None, None, "missing_field".to_owned()),
    }
}

fn audit_tile_layout(
    value: Option<&IonValue>,
    overlapped_tiles_field_present: bool,
    container: Option<&NativeContainerOwned>,
) -> Option<KfxAuditTileLayout> {
    let value = value?;
    let rows = match value {
        IonValue::List(rows) => rows,
        _ => {
            return Some(KfxAuditTileLayout {
                row_count: 0,
                max_column_count: 0,
                overlapped_tiles_field_present,
                locations: Vec::new(),
            })
        }
    };
    let max_column_count = rows
        .iter()
        .map(|row| match row {
            IonValue::List(columns) => columns.len(),
            _ => 0,
        })
        .max()
        .unwrap_or_default();
    let mut locations = Vec::new();
    for row in rows {
        collect_tile_locations(row, container, &mut locations);
    }
    Some(KfxAuditTileLayout {
        row_count: rows.len(),
        max_column_count,
        overlapped_tiles_field_present,
        locations,
    })
}

fn collect_tile_locations(
    value: &IonValue,
    container: Option<&NativeContainerOwned>,
    output: &mut Vec<String>,
) {
    match value {
        IonValue::String(location) if location.starts_with("resource/") => {
            output.push(location.clone());
        }
        IonValue::Symbol(symbol) => {
            if let Some(location) = container
                .and_then(|container| container.symbols.names.get(symbol))
                .filter(|location| location.starts_with("resource/"))
            {
                output.push(location.clone());
            }
        }
        IonValue::Struct(fields) => {
            for (_, child) in fields {
                collect_tile_locations(child, container, output);
            }
        }
        IonValue::List(items) | IonValue::SExp(items) => {
            for item in items {
                collect_tile_locations(item, container, output);
            }
        }
        IonValue::Annotation { value, .. } => collect_tile_locations(value, container, output),
        _ => {}
    }
}

fn audit_scalar(value: &IonValue) -> Option<KfxAuditScalar> {
    Some(match value {
        IonValue::Int(integer) => KfxAuditScalar {
            kind: "int".to_owned(),
            integer: Some(*integer),
            symbol_id: None,
            string: None,
        },
        IonValue::Symbol(symbol_id) => KfxAuditScalar {
            kind: "symbol".to_owned(),
            integer: None,
            symbol_id: Some(*symbol_id),
            string: None,
        },
        IonValue::String(value) => KfxAuditScalar {
            kind: "string".to_owned(),
            integer: None,
            symbol_id: None,
            string: value.starts_with("resource/rsrc").then(|| value.clone()),
        },
        _ => return None,
    })
}

fn ion_blob_count(value: &IonValue) -> usize {
    match value {
        IonValue::Blob(_) => 1,
        IonValue::Struct(fields) => fields.iter().map(|(_, child)| ion_blob_count(child)).sum(),
        IonValue::List(items) | IonValue::SExp(items) => items.iter().map(ion_blob_count).sum(),
        IonValue::Annotation { value, .. } => ion_blob_count(value),
        _ => 0,
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn placement_identity_index(
    model: &NativeModel,
    resolution: &NativeResourcePathResolution,
    resource_ids_by_path: &BTreeMap<String, ResourceId>,
) -> (
    BTreeMap<String, ExternalPlacementIdentity>,
    BTreeMap<ResourceId, String>,
    BTreeMap<String, String>,
) {
    let mut candidates = BTreeMap::<String, Vec<ExternalPlacementIdentity>>::new();
    let mut names_by_location = BTreeMap::<String, Vec<String>>::new();
    for fragment in model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 164)
    {
        let Some(name) = resolved_fragment_fid(model, fragment) else {
            continue;
        };
        let binding = resolution.bindings.iter().find(|binding| {
            binding.container_origin == fragment.container_index
                && binding.external_resource_entity_id == fragment.entity_id
        });
        let identity = match binding.map(|binding| &binding.binding) {
            Some(ResourceBinding::Exact(native_index)) => {
                let native = model.resources.get(*native_index);
                if native.is_some_and(|resource| resource.media_type.starts_with("image/")) {
                    let location = binding.and_then(|record| record.location.as_deref());
                    let normalized_location = location.and_then(normalize_kfx_resource_path);
                    if let Some(path) = normalized_location.as_ref() {
                        names_by_location
                            .entry(path.clone())
                            .or_default()
                            .push(name.clone());
                    }
                    let id = normalized_location
                        .as_ref()
                        .and_then(|path| resource_ids_by_path.get(path).copied());
                    ExternalPlacementIdentity {
                        status: if id.is_some() {
                            "exact_image_identity".to_owned()
                        } else {
                            "exact_image_identity_not_materialized".to_owned()
                        },
                        resource: id,
                    }
                } else {
                    ExternalPlacementIdentity {
                        status: "exact_non_image_identity".to_owned(),
                        resource: None,
                    }
                }
            }
            Some(ResourceBinding::Unresolved(unresolved)) => ExternalPlacementIdentity {
                status: unresolved.reason.diagnostic_code().to_owned(),
                resource: None,
            },
            None => ExternalPlacementIdentity {
                status: "unresolved_external_resource_record".to_owned(),
                resource: None,
            },
        };
        candidates.entry(name).or_default().push(identity);
    }

    let mut identities = BTreeMap::new();
    let mut names_by_resource = BTreeMap::<ResourceId, Vec<String>>::new();
    for (name, entries) in candidates {
        if entries.len() != 1 {
            identities.insert(
                name,
                ExternalPlacementIdentity {
                    status: "ambiguous_external_resource_name".to_owned(),
                    resource: None,
                },
            );
            continue;
        }
        let identity = entries.into_iter().next().expect("length checked");
        if identity.status == "exact_image_identity" {
            if let Some(resource_id) = identity.resource {
                names_by_resource
                    .entry(resource_id)
                    .or_default()
                    .push(name.clone());
            }
        }
        identities.insert(name, identity);
    }
    let name_by_resource = names_by_resource
        .into_iter()
        .filter_map(|(resource, names)| {
            (names.len() == 1).then(|| (resource, names.into_iter().next().unwrap()))
        })
        .collect();
    let name_by_location = names_by_location
        .into_iter()
        .filter_map(|(path, names)| {
            (names.len() == 1).then(|| (path, names.into_iter().next().unwrap()))
        })
        .collect();
    (identities, name_by_resource, name_by_location)
}

fn collect_raw_image_references(
    model: &NativeModel,
    identities: &BTreeMap<String, ExternalPlacementIdentity>,
    names_by_location: &BTreeMap<String, String>,
) -> Vec<RawImageReference> {
    fn resource_name_from_symbol(
        container: &NativeContainerOwned,
        symbol: u64,
        identities: &BTreeMap<String, ExternalPlacementIdentity>,
        names_by_location: &BTreeMap<String, String>,
    ) -> (Option<String>, Vec<String>, String) {
        // Ion symbols are resolved using the SID in the current container's
        // symbol table. The legacy content-import adjustment is retained only
        // as an audit observation and must not override the actual Ion SID.
        let direct_value = container.symbols.names.get(&symbol);
        let (resource_name, category) = resolve_resource_symbol_name(
            direct_value.map(String::as_str),
            identities,
            names_by_location,
        );
        let shift = import_symbol_id_shift(&container.symbols.imports);
        let alternate_value = shift
            .and_then(|shift| symbol.checked_sub(shift))
            .and_then(|id| container.symbols.names.get(&id));
        let alternate_name = resolve_resource_symbol_name(
            alternate_value.map(String::as_str),
            identities,
            names_by_location,
        )
        .0;
        let alternate_names = alternate_name.into_iter().collect();
        (resource_name, alternate_names, category)
    }

    fn resource_name_from_string(
        name: &str,
        identities: &BTreeMap<String, ExternalPlacementIdentity>,
        names_by_location: &BTreeMap<String, String>,
    ) -> (Option<String>, Vec<String>, String) {
        let (resource_name, category) =
            resolve_resource_symbol_name(Some(name), identities, names_by_location);
        let category = match category.as_str() {
            "symbol_ion_sid_exact_fid" => "string_exact_fid".to_owned(),
            "symbol_ion_sid_exact_location_bridge" => "string_exact_location_bridge".to_owned(),
            "symbol_ion_sid_name_without_exact_identity" => {
                "string_without_exact_identity".to_owned()
            }
            _ => "string_without_exact_identity".to_owned(),
        };
        (resource_name, Vec::new(), category)
    }

    fn resolve_resource_symbol_name(
        value: Option<&str>,
        identities: &BTreeMap<String, ExternalPlacementIdentity>,
        names_by_location: &BTreeMap<String, String>,
    ) -> (Option<String>, String) {
        let Some(value) = value else {
            return (None, "symbol_ion_sid_not_in_container_scope".to_owned());
        };
        if identities.contains_key(value) {
            return (
                Some(value.to_owned()),
                "symbol_ion_sid_exact_fid".to_owned(),
            );
        }
        if let Some(path) = normalize_kfx_resource_path(value) {
            if let Some(name) = names_by_location.get(&path) {
                return (
                    Some(name.clone()),
                    "symbol_ion_sid_exact_location_bridge".to_owned(),
                );
            }
        }
        (
            None,
            "symbol_ion_sid_name_without_exact_identity".to_owned(),
        )
    }

    fn visit(
        value: &IonValue,
        container: &NativeContainerOwned,
        image_record_symbol: u64,
        identities: &BTreeMap<String, ExternalPlacementIdentity>,
        names_by_location: &BTreeMap<String, String>,
        inherited_position: Option<u32>,
        output: &mut Vec<RawImageReference>,
    ) {
        match value {
            IonValue::Struct(fields) => {
                let position = struct_get(value, FIELD_NAV_TARGET_ID)
                    .and_then(as_int)
                    .and_then(|value| u32::try_from(value).ok())
                    .or(inherited_position);
                let is_image_record = struct_get(value, FIELD_CONTENT_NODE_TYPE).is_some_and(
                    |value| {
                        matches!(value, IonValue::Symbol(actual) if *actual == image_record_symbol)
                    },
                );
                if is_image_record {
                    let (resource_name, alternate_names_audit_only, resolution_category) =
                        match struct_get(value, FIELD_CONTENT_ORACLE_RESOURCE_SYMBOL) {
                            Some(IonValue::Symbol(symbol)) => resource_name_from_symbol(
                                container,
                                *symbol,
                                identities,
                                names_by_location,
                            ),
                            Some(IonValue::String(name)) => {
                                resource_name_from_string(name, identities, names_by_location)
                            }
                            None => (None, Vec::new(), "field_175_missing".to_owned()),
                            Some(_) => (
                                None,
                                Vec::new(),
                                "field_175_unsupported_value_type".to_owned(),
                            ),
                        };
                    output.push(RawImageReference {
                        resource_name,
                        alternate_names_audit_only,
                        position_target_id: position,
                        resolution_category,
                    });
                }
                for (_, child) in fields {
                    visit(
                        child,
                        container,
                        image_record_symbol,
                        identities,
                        names_by_location,
                        position,
                        output,
                    );
                }
            }
            IonValue::List(items) | IonValue::SExp(items) => {
                for item in items {
                    visit(
                        item,
                        container,
                        image_record_symbol,
                        identities,
                        names_by_location,
                        inherited_position,
                        output,
                    );
                }
            }
            IonValue::Annotation { value, .. } => visit(
                value,
                container,
                image_record_symbol,
                identities,
                names_by_location,
                inherited_position,
                output,
            ),
            _ => {}
        }
    }

    let mut output = Vec::new();
    for fragment in model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 259)
    {
        let Some(container) = model.containers.get(fragment.container_index) else {
            continue;
        };
        let Some(image_record_symbol) =
            observed_whitespace_resource_record_symbol_for_imports_content(
                &container.symbols.imports,
            )
        else {
            continue;
        };
        visit(
            &fragment.value,
            container,
            image_record_symbol,
            identities,
            names_by_location,
            None,
            &mut output,
        );
    }
    output
}

fn placement_stage_from_raw(
    raw: &[RawImageReference],
    aliases: &BTreeMap<String, String>,
    identities: &BTreeMap<String, ExternalPlacementIdentity>,
) -> KfxPlacementStage {
    let occurrences = raw
        .iter()
        .map(|reference| {
            let identity_status = reference
                .resource_name
                .as_ref()
                .and_then(|name| identities.get(name))
                .map(|identity| identity.status.clone())
                .unwrap_or_else(|| "unresolved_external_resource_reference".to_owned());
            KfxPlacementOccurrence {
                source_order: 0,
                resource: reference
                    .resource_name
                    .as_ref()
                    .and_then(|name| aliases.get(name))
                    .cloned(),
                alternate_symbol_resources_audit_only: reference
                    .alternate_names_audit_only
                    .iter()
                    .filter_map(|name| aliases.get(name).cloned())
                    .collect(),
                source_position_target_id: reference.position_target_id,
                identity_status,
            }
        })
        .collect();
    finalize_placement_stage(occurrences)
}

fn placement_stage_from_capture(
    captured: &[CapturedImagePlacement],
    aliases: &BTreeMap<String, String>,
    identities: &BTreeMap<String, ExternalPlacementIdentity>,
    names_by_resource: &BTreeMap<ResourceId, String>,
) -> KfxPlacementStage {
    let occurrences = captured
        .iter()
        .map(|placement| {
            let name = names_by_resource.get(&placement.resource);
            let identity_status = name
                .and_then(|name| identities.get(name))
                .map(|identity| identity.status.clone())
                .unwrap_or_else(|| "unresolved_ir_resource_identity".to_owned());
            KfxPlacementOccurrence {
                source_order: 0,
                resource: name.and_then(|name| aliases.get(name)).cloned(),
                alternate_symbol_resources_audit_only: Vec::new(),
                source_position_target_id: placement.position_target_id,
                identity_status,
            }
        })
        .collect();
    finalize_placement_stage(occurrences)
}

fn finalize_placement_stage(mut occurrences: Vec<KfxPlacementOccurrence>) -> KfxPlacementStage {
    let mut frequencies = BTreeMap::<String, usize>::new();
    let mut identity_verified_occurrence_count = 0usize;
    for (index, occurrence) in occurrences.iter_mut().enumerate() {
        occurrence.source_order = index;
        if occurrence.identity_status == "exact_image_identity" {
            identity_verified_occurrence_count += 1;
        }
        if let Some(resource) = occurrence.resource.as_ref() {
            *frequencies.entry(resource.clone()).or_default() += 1;
        }
    }
    let resource_frequencies = frequencies
        .into_iter()
        .map(|(resource, count)| KfxPlacementFrequency { resource, count })
        .collect();
    let occurrence_count = occurrences.len();
    KfxPlacementStage {
        occurrence_count,
        identity_verified_occurrence_count,
        unresolved_identity_occurrence_count: occurrence_count
            .saturating_sub(identity_verified_occurrence_count),
        resource_frequencies,
        occurrences,
    }
}

fn audit_key_sort(left: &KfxAuditFragmentKey, right: &KfxAuditFragmentKey) -> std::cmp::Ordering {
    (
        left.container_origin,
        left.fragment_type,
        &left.fid,
        left.native_entity_id,
    )
        .cmp(&(
            right.container_origin,
            right.fragment_type,
            &right.fid,
            right.native_entity_id,
        ))
}

fn count_resource_references(model: &NativeModel) -> BTreeMap<String, usize> {
    count_resource_references_with_symbol_policy(model, true)
}

fn count_resource_references_standard_ion(model: &NativeModel) -> BTreeMap<String, usize> {
    count_resource_references_with_symbol_policy(model, false)
}

fn count_resource_references_with_symbol_policy(
    model: &NativeModel,
    content_symbol_ids: bool,
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for fragment in model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 259)
    {
        let Some(symbols) = model
            .containers
            .get(fragment.container_index)
            .map(|container| &container.symbols.names)
        else {
            continue;
        };
        let import_shift = if content_symbol_ids {
            model
                .containers
                .get(fragment.container_index)
                .and_then(|container| import_symbol_id_shift(&container.symbols.imports))
                .unwrap_or_default()
        } else {
            0
        };
        count_reference_symbols(&fragment.value, symbols, import_shift, &mut counts);
    }
    counts
}

fn count_reference_symbols(
    value: &IonValue,
    symbols: &BTreeMap<u64, String>,
    content_import_shift: u64,
    counts: &mut BTreeMap<String, usize>,
) {
    match value {
        IonValue::Struct(fields) => {
            for (field, child) in fields {
                if *field == FIELD_CONTENT_RESOURCE_SYMBOL {
                    if let IonValue::Symbol(symbol) = child {
                        let decoded_id = symbol.checked_sub(content_import_shift);
                        if let Some(fid) = decoded_id.and_then(|id| symbols.get(&id)) {
                            *counts.entry(fid.clone()).or_default() += 1;
                        }
                    }
                }
                count_reference_symbols(child, symbols, content_import_shift, counts);
            }
        }
        IonValue::List(items) | IonValue::SExp(items) => {
            for item in items {
                count_reference_symbols(item, symbols, content_import_shift, counts);
            }
        }
        IonValue::Annotation { value, .. } => {
            count_reference_symbols(value, symbols, content_import_shift, counts)
        }
        _ => {}
    }
}

fn count_observed_resource_placements_standard_ion(model: &NativeModel) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for fragment in model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 259)
    {
        let Some(container) = model.containers.get(fragment.container_index) else {
            continue;
        };
        let image_symbol =
            observed_whitespace_resource_record_symbol_for_imports(&container.symbols.imports);
        if let Some(image_symbol) = image_symbol {
            count_placement_shapes(
                &fragment.value,
                image_symbol,
                &container.symbols.names,
                &mut counts,
            );
        }
    }
    counts
}

fn count_observed_resource_placements(model: &NativeModel) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for fragment in model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 259)
    {
        let Some(container) = model.containers.get(fragment.container_index) else {
            continue;
        };
        let Some(image_symbol) = observed_whitespace_resource_record_symbol_for_imports_content(
            &container.symbols.imports,
        ) else {
            continue;
        };
        let Some(shift) = import_symbol_id_shift(&container.symbols.imports) else {
            continue;
        };
        let content_names = container
            .symbols
            .names
            .iter()
            .filter_map(|(symbol, name)| symbol.checked_add(shift).map(|id| (id, name.clone())))
            .collect::<BTreeMap<_, _>>();
        count_placement_shapes(&fragment.value, image_symbol, &content_names, &mut counts);
    }
    counts
}

fn count_placement_shapes(
    value: &IonValue,
    image_record_symbol: u64,
    symbols: &BTreeMap<u64, String>,
    counts: &mut BTreeMap<String, usize>,
) {
    match value {
        IonValue::Struct(fields) => {
            let is_placeholder = struct_get(value, FIELD_CONTENT_NODE_TYPE).is_some_and(
                |value| matches!(value, IonValue::Symbol(actual) if *actual == image_record_symbol),
            ) && struct_get(value, FIELD_CONTENT_RESOURCE_SYMBOL)
                .is_some_and(|value| matches!(value, IonValue::Symbol(_)))
                && struct_get(value, FIELD_NAV_TARGET_ID)
                    .and_then(as_int)
                    .is_some()
                && struct_get(value, FIELD_FRAGMENT_TEXT)
                    .and_then(as_string)
                    .is_some_and(|text| text.trim().is_empty())
                && struct_get(value, FIELD_CONTENT_STRING_REFERENCE).is_none()
                && struct_get(value, 142).is_none()
                && struct_get(value, FIELD_TEXT_VALUES).is_none();
            if is_placeholder {
                if let Some(IonValue::Symbol(symbol)) =
                    struct_get(value, FIELD_CONTENT_RESOURCE_SYMBOL)
                {
                    if let Some(fid) = symbols.get(symbol) {
                        *counts.entry(fid.clone()).or_default() += 1;
                    }
                }
                return;
            }
            for (_, child) in fields {
                count_placement_shapes(child, image_record_symbol, symbols, counts);
            }
        }
        IonValue::List(items) | IonValue::SExp(items) => {
            for item in items {
                count_placement_shapes(item, image_record_symbol, symbols, counts);
            }
        }
        IonValue::Annotation { value, .. } => {
            count_placement_shapes(value, image_record_symbol, symbols, counts)
        }
        _ => {}
    }
}

fn parse_native(bytes: &[u8], options: DecodeOptions) -> Result<NativeModel, AmazonKfxError> {
    parse_native_sources(&[bytes], options)
}

fn parse_native_sources(
    inputs: &[&[u8]],
    options: DecodeOptions,
) -> Result<NativeModel, AmazonKfxError> {
    if inputs.is_empty() {
        return Err(AmazonKfxError::InvalidHeader(
            "KFX file-set contains no inputs".to_owned(),
        ));
    }
    let mut model = NativeModel::default();
    for (input_index, bytes) in inputs.iter().enumerate() {
        if !bytes.starts_with(b"CONT") {
            return Err(AmazonKfxError::InvalidHeader(format!(
                "KFX input {input_index} is missing CONT signature"
            )));
        }
        let mut offset = 0usize;
        while offset < bytes.len() {
            let (container, next) = parse_container(bytes, offset, options.mode)?;
            if next <= offset {
                return Err(AmazonKfxError::InvalidHeader(
                    "container parser made no progress".to_owned(),
                ));
            }
            model.drm_detected |= container
                .entities
                .iter()
                .any(|entity| entity.drm_scheme != 0);
            let container_index = model.containers.len();
            let symbols = absorb_container(&mut model, &container, options.mode, container_index)?;
            model.containers.push(NativeContainerOwned {
                input_index,
                byte_offset: offset,
                entity_count: container.entities.len(),
                symbols,
            });
            offset = next;
        }
    }
    decode_document_symbols(&mut model, options.mode)?;
    decode_fragment_values(&mut model, options.mode)?;
    record_native_limitations(&mut model);
    Ok(model)
}

fn parse_container<'a>(
    bytes: &'a [u8],
    start: usize,
    mode: ParseMode,
) -> Result<(NativeContainer<'a>, usize), AmazonKfxError> {
    let header = read_container_header(bytes, start)?;
    let info_end = checked_slice_end(start, header.info_offset, header.info_length, bytes.len())?;
    let (info, _) =
        decode_one(bytes, start + header.info_offset + 4, info_end).or_else(|error| {
            recover_ion(bytes, start + header.info_offset + 4, info_end, mode, error)
        })?;
    let fields = unwrap_struct(&info).ok_or_else(|| {
        AmazonKfxError::InvalidHeader("CONT container-info is not a struct".to_owned())
    })?;
    if struct_get_from_fields(fields, FIELD_CONTAINER_ID).is_none() {
        return Err(AmazonKfxError::InvalidHeader(
            "CONT container-info has no container id".to_owned(),
        ));
    }
    let container_is_protected = struct_get_from_fields(fields, FIELD_DRM_SCHEME)
        .and_then(as_int)
        .unwrap_or(0)
        != 0;
    let index_offset = required_u64(fields, FIELD_INDEX_OFFSET)?;
    let index_length = required_u64(fields, FIELD_INDEX_LENGTH)?;
    let index_start = start
        .checked_add(index_offset as usize)
        .ok_or_else(|| AmazonKfxError::InvalidEntityIndex("index offset overflow".to_owned()))?;
    let index_end = index_start
        .checked_add(index_length as usize)
        .ok_or_else(|| AmazonKfxError::InvalidEntityIndex("index length overflow".to_owned()))?;
    if index_end > bytes.len() {
        return Err(AmazonKfxError::InvalidEntityIndex(format!(
            "index range {index_start}..{index_end} exceeds input length {}",
            bytes.len()
        )));
    }
    if !(index_length as usize).is_multiple_of(ENTITY_INDEX_ENTRY) {
        return Err(AmazonKfxError::InvalidEntityIndex(
            "index length is not a multiple of 24".to_owned(),
        ));
    }
    let entity_count = index_length as usize / ENTITY_INDEX_ENTRY;
    if entity_count > MAX_ENTITIES {
        return Err(AmazonKfxError::InvalidEntityIndex(
            "entity count limit exceeded".to_owned(),
        ));
    }
    let container_end = entity_container_end(
        bytes,
        start,
        &header,
        fields,
        index_start,
        entity_count,
        index_end,
    )?;
    let doc_symbols = optional_ion_range(
        bytes,
        start,
        fields,
        FIELD_DOC_SYMBOL_OFFSET,
        FIELD_DOC_SYMBOL_LENGTH,
        mode,
    )?;
    let capabilities = optional_ion_range(
        bytes,
        start,
        fields,
        FIELD_CAPABILITIES_OFFSET,
        FIELD_CAPABILITIES_LENGTH,
        mode,
    )?;
    let mut entities = Vec::with_capacity(entity_count);
    for index in 0..entity_count {
        let entry = index_start + index * ENTITY_INDEX_ENTRY;
        let id = read_u32(bytes, entry).ok_or(AmazonKfxError::Truncated(entry))?;
        let type_id = read_u32(bytes, entry + 4).ok_or(AmazonKfxError::Truncated(entry + 4))?;
        let relative_offset =
            read_u64(bytes, entry + 8).ok_or(AmazonKfxError::Truncated(entry + 8))?;
        let length = read_u64(bytes, entry + 16).ok_or(AmazonKfxError::Truncated(entry + 16))?;
        let entity_start = start
            .checked_add(header.header_len)
            .and_then(|value| value.checked_add(usize::try_from(relative_offset).ok()?))
            .ok_or(AmazonKfxError::Truncated(entry + 8))?;
        let entity_end = entity_start
            .checked_add(
                usize::try_from(length).map_err(|_| AmazonKfxError::Truncated(entry + 16))?,
            )
            .ok_or(AmazonKfxError::Truncated(entry + 16))?;
        if entity_end > bytes.len() || entity_end > container_end {
            return Err(AmazonKfxError::InvalidEntity(
                id,
                format!("range {entity_start}..{entity_end} exceeds container end {container_end}"),
            ));
        }
        entities.push(parse_entity(
            bytes,
            entity_start,
            entity_end,
            id,
            type_id,
            mode,
            container_is_protected,
        )?);
    }
    let kfxgen_start = info_end;
    let kfxgen_end = start
        .checked_add(header.header_len)
        .ok_or(AmazonKfxError::Truncated(start))?;
    let kfxgen_info = bytes
        .get(kfxgen_start..kfxgen_end)
        .ok_or(AmazonKfxError::Truncated(kfxgen_start))?;
    Ok((
        NativeContainer {
            header,
            info,
            doc_symbols,
            capabilities,
            entities,
            kfxgen_info,
        },
        container_end,
    ))
}

fn read_container_header(bytes: &[u8], start: usize) -> Result<ContainerHeader, AmazonKfxError> {
    let minimum_end = start
        .checked_add(CONT_HEADER_MIN)
        .ok_or(AmazonKfxError::Truncated(start))?;
    let header = bytes
        .get(start..minimum_end)
        .ok_or(AmazonKfxError::Truncated(start))?;
    if &header[..4] != b"CONT" {
        return Err(AmazonKfxError::InvalidHeader(
            "missing CONT signature".to_owned(),
        ));
    }
    let version = u16::from_le_bytes([header[4], header[5]]);
    if version != 1 && version != CONT_VERSION {
        return Err(AmazonKfxError::UnsupportedVersion(version));
    }
    let header_len = usize::try_from(u32::from_le_bytes([
        header[6], header[7], header[8], header[9],
    ]))
    .map_err(|_| AmazonKfxError::InvalidHeader("header length overflows usize".to_owned()))?;
    if header_len < CONT_HEADER_MIN || start.checked_add(header_len).is_none() {
        return Err(AmazonKfxError::InvalidHeader(
            "header length is too small".to_owned(),
        ));
    }
    let info_offset = usize::try_from(u32::from_le_bytes([
        header[10], header[11], header[12], header[13],
    ]))
    .map_err(|_| AmazonKfxError::InvalidHeader("info offset overflows usize".to_owned()))?;
    let info_length = usize::try_from(u32::from_le_bytes([
        header[14], header[15], header[16], header[17],
    ]))
    .map_err(|_| AmazonKfxError::InvalidHeader("info length overflows usize".to_owned()))?;
    Ok(ContainerHeader {
        version,
        header_len,
        info_offset,
        info_length,
    })
}

fn entity_container_end(
    bytes: &[u8],
    start: usize,
    header: &ContainerHeader,
    fields: &[(u64, IonValue)],
    index_start: usize,
    entity_count: usize,
    index_end: usize,
) -> Result<usize, AmazonKfxError> {
    let mut end = start
        .checked_add(header.header_len)
        .ok_or(AmazonKfxError::Truncated(start))?
        .max(checked_slice_end(
            start,
            header.info_offset,
            header.info_length,
            bytes.len(),
        )?)
        .max(index_end);
    for index in 0..entity_count {
        let entry = index_start
            .checked_add(index.checked_mul(ENTITY_INDEX_ENTRY).ok_or_else(|| {
                AmazonKfxError::InvalidEntityIndex("index entry offset overflow".to_owned())
            })?)
            .ok_or_else(|| {
                AmazonKfxError::InvalidEntityIndex("index entry offset overflow".to_owned())
            })?;
        let relative_offset =
            read_u64(bytes, entry + 8).ok_or(AmazonKfxError::Truncated(entry + 8))?;
        let length = read_u64(bytes, entry + 16).ok_or(AmazonKfxError::Truncated(entry + 16))?;
        let entity_start = start
            .checked_add(header.header_len)
            .and_then(|value| value.checked_add(usize::try_from(relative_offset).ok()?))
            .ok_or(AmazonKfxError::Truncated(entry + 8))?;
        let entity_end = entity_start
            .checked_add(
                usize::try_from(length).map_err(|_| AmazonKfxError::Truncated(entry + 16))?,
            )
            .ok_or(AmazonKfxError::Truncated(entry + 16))?;
        if entity_end > bytes.len() {
            return Err(AmazonKfxError::InvalidEntityIndex(format!(
                "entity range {entity_start}..{entity_end} exceeds input length {}",
                bytes.len()
            )));
        }
        end = end.max(entity_end);
    }
    for (offset_id, length_id) in [
        (FIELD_DOC_SYMBOL_OFFSET, FIELD_DOC_SYMBOL_LENGTH),
        (FIELD_CAPABILITIES_OFFSET, FIELD_CAPABILITIES_LENGTH),
    ] {
        let Some(offset) = struct_get_from_fields(fields, offset_id)
            .and_then(as_int)
            .and_then(|value| usize::try_from(value).ok())
        else {
            continue;
        };
        let Some(length) = struct_get_from_fields(fields, length_id)
            .and_then(as_int)
            .and_then(|value| usize::try_from(value).ok())
        else {
            continue;
        };
        end = end.max(checked_slice_end(start, offset, length, bytes.len())?);
    }
    Ok(end)
}

fn parse_entity<'a>(
    bytes: &'a [u8],
    start: usize,
    end: usize,
    id: u32,
    type_id: u32,
    mode: ParseMode,
    container_is_protected: bool,
) -> Result<NativeEntity<'a>, AmazonKfxError> {
    let header_end = start
        .checked_add(ENTY_HEADER_MIN)
        .ok_or(AmazonKfxError::Truncated(start))?;
    let header = bytes
        .get(start..header_end)
        .ok_or(AmazonKfxError::Truncated(start))?;
    if &header[..4] != b"ENTY" {
        return Err(AmazonKfxError::InvalidEntity(
            id,
            "missing ENTY signature".to_owned(),
        ));
    }
    let version = u16::from_le_bytes([header[4], header[5]]);
    if version != 1 {
        return Err(AmazonKfxError::InvalidEntity(
            id,
            format!("unsupported ENTY version {version}"),
        ));
    }
    let header_len = usize::try_from(u32::from_le_bytes([
        header[6], header[7], header[8], header[9],
    ]))
    .map_err(|_| AmazonKfxError::InvalidEntity(id, "ENTY header length overflows".to_owned()))?;
    if header_len < ENTY_HEADER_MIN
        || start.checked_add(header_len).is_none()
        || start + header_len > end
    {
        return Err(AmazonKfxError::InvalidEntity(
            id,
            "ENTY header range is invalid".to_owned(),
        ));
    }
    let (info, _) =
        decode_one(bytes, start + ENTY_HEADER_MIN, start + header_len).or_else(|error| {
            recover_ion(
                bytes,
                start + ENTY_HEADER_MIN,
                start + header_len,
                mode,
                error,
            )
        })?;
    let info_fields = unwrap_struct(&info)
        .ok_or_else(|| AmazonKfxError::InvalidEntity(id, "ENTY info is not a struct".to_owned()))?;
    let compression = struct_get_from_fields(info_fields, FIELD_COMPRESSION)
        .and_then(as_int)
        .unwrap_or(0);
    let drm_scheme = struct_get_from_fields(info_fields, FIELD_DRM_SCHEME)
        .and_then(as_int)
        .unwrap_or(0);
    if compression != 0 && !container_is_protected && drm_scheme == 0 {
        return Err(AmazonKfxError::InvalidEntity(
            id,
            format!("unsupported compression scheme {compression}"),
        ));
    }
    let payload_start = start
        + if header_len >= ENTY_HEADER_LEN {
            header_len
        } else {
            ENTY_HEADER_MIN
        };
    let payload = bytes
        .get(payload_start..end)
        .ok_or(AmazonKfxError::Truncated(payload_start))?;
    if container_is_protected || drm_scheme != 0 {
        return Ok(NativeEntity {
            id,
            type_id,
            payload: None,
            raw_payload: None,
            info: info.clone(),
            drm_scheme: if drm_scheme == 0 { 1 } else { drm_scheme },
        });
    }
    if is_raw_payload(type_id, payload) {
        return Ok(NativeEntity {
            id,
            type_id,
            payload: None,
            raw_payload: Some(payload),
            info: info.clone(),
            drm_scheme,
        });
    }
    let ion_start = if payload.starts_with(&VERSION_MARKER) {
        payload_start + VERSION_MARKER.len()
    } else {
        payload_start
    };
    let parsed = decode_one(bytes, ion_start, end)
        .or_else(|error| recover_ion(bytes, ion_start, end, mode, error))
        .map(|(value, _)| value);
    match parsed {
        Ok(payload) => Ok(NativeEntity {
            id,
            type_id,
            payload: Some(payload),
            raw_payload: None,
            info: info.clone(),
            drm_scheme,
        }),
        Err(error) => Err(AmazonKfxError::InvalidEntity(id, error.to_string())),
    }
}

fn is_raw_payload(type_id: u32, payload: &[u8]) -> bool {
    matches!(type_id, 0x1a1 | 0x1a2)
        || payload.starts_with(&[0xff, 0xd8])
        || payload.starts_with(b"\x89PNG")
        || payload.starts_with(b"GIF8")
}

fn absorb_container(
    model: &mut NativeModel,
    container: &NativeContainer<'_>,
    _mode: ParseMode,
    container_index: usize,
) -> Result<SymbolTable, AmazonKfxError> {
    if container.header.version == 1 {
        model
            .warnings
            .push("CONT version 1 accepted with the version-2 compatibility reader".to_owned());
    }
    let container_symbols = container
        .doc_symbols
        .as_ref()
        .map(|doc_symbols| absorb_symbol_table(model, doc_symbols))
        .unwrap_or_default();
    if struct_get(&container.info, FIELD_DRM_SCHEME)
        .and_then(as_int)
        .unwrap_or(0)
        != 0
    {
        model.drm_detected = true;
    }
    if struct_get(&container.info, FIELD_COMPRESSION)
        .and_then(as_int)
        .unwrap_or(0)
        != 0
    {
        model
            .unknown_features
            .insert("container-compression".to_owned());
    }
    if struct_get(&container.info, FIELD_CHUNK_SIZE)
        .and_then(as_int)
        .is_some_and(|size| size != 4096)
    {
        model
            .unknown_features
            .insert("non-default-chunk-size".to_owned());
    }
    if container.capabilities.is_some() {
        model
            .unknown_features
            .insert("format-capabilities-preserved-native-only".to_owned());
    }
    if !container.kfxgen_info.is_empty() && !container.kfxgen_info.is_ascii() {
        model
            .unknown_features
            .insert("non-ascii-generator-metadata".to_owned());
    }
    for entity in &container.entities {
        if entity.drm_scheme != 0 {
            model.drm_detected = true;
        }
        if let Some(payload) = &entity.payload {
            model.fragments.push(NativeFragment {
                container_index,
                entity_id: entity.id,
                fragment_type: entity_type_id(entity.type_id),
                value: payload.clone(),
            });
        } else if let Some(raw) = entity.raw_payload {
            let (media_type, _) = media_type_for_bytes(raw);
            model.resources.push(NativeResource {
                container_index,
                fragment_type: entity_type_id(entity.type_id),
                entity_id: Some(entity.id),
                media_type,
                bytes: raw.to_vec(),
                entity_info: Some(entity.info.clone()),
                properties: BTreeSet::new(),
            });
        }
    }
    Ok(container_symbols)
}

fn record_native_limitations(model: &mut NativeModel) {
    if model
        .fragments
        .iter()
        .any(|fragment| fragment.fragment_type == 259)
    {
        model.unknown_features.insert(
            "kfx-advanced-inline-structure-and-computed-style-decoding-incomplete".to_owned(),
        );
        model.input_loss.push(
            "Amazon KFX block structure and observed style symbols are reconstructed; advanced inline structure and computed styles outside the supported symbol map are not fully reconstructed. `$179` hyperlinks backed by `$266` anchors are resolved when their target location is proven.".to_owned(),
        );
    }
    if !model.resources.is_empty() {
        model
            .unknown_features
            .insert("kfx-resource-role-and-placement-decoding-incomplete".to_owned());
        model.input_loss.push(
            "Amazon KFX resource roles outside the supported direct-media subset are not fully reconstructed in the semantic IR.".to_owned(),
        );
    }
    if !model.fragments.is_empty() && model.navigation.is_empty() {
        model
            .unknown_features
            .insert("kfx-navigation-not-recovered".to_owned());
        model.input_loss.push(
            "No usable Amazon KFX navigation hierarchy was recovered from the input.".to_owned(),
        );
    }
}

fn record_resource_identity_limitations(
    model: &mut NativeModel,
    resolution: &NativeResourcePathResolution,
) {
    model.resource_identity_diagnostics = resolution.diagnostics.clone();
    if resolution.unresolved_records > 0 {
        model
            .unknown_features
            .insert("kfx-resource-byte-association-incomplete".to_owned());
        model.input_loss.push(format!(
            "{} KFX resource metadata records have no unique exact `$165` location to `$417` fid association.",
            resolution.unresolved_records
        ));
    }
}

fn materialize_ir_resources(
    model: &NativeModel,
    resolution: &NativeResourcePathResolution,
) -> (
    Vec<Resource>,
    MemoryResourceLoader,
    BTreeMap<String, ResourceId>,
) {
    let mut resources = Vec::new();
    let mut loader = MemoryResourceLoader::default();
    let mut resource_ids_by_path = BTreeMap::new();
    let mut cover_native_indices = BTreeSet::new();

    // Calibre's KFX Input follows the same identity chain: metadata
    // `cover_image` names an external `$164` fid; that record's `$165`
    // location names the exact raw `$417` media. Only mark a cover after the
    // existing production resolver has proved that chain, never by image
    // order, dimensions, or filename heuristics.
    for fragment in model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 164)
    {
        let Some(fid) = resolved_fragment_fid(model, fragment) else {
            continue;
        };
        let is_metadata_cover = model.cover_resource_fids.contains(&fid);
        let binding = resolution.bindings.iter().find(|binding| {
            binding.container_origin == fragment.container_index
                && binding.external_resource_entity_id == fragment.entity_id
        });
        if is_metadata_cover {
            if let Some(ResourceBinding::Exact(native_index)) =
                binding.map(|binding| &binding.binding)
            {
                cover_native_indices.insert(*native_index);
            }
        }
    }

    for (path, candidate) in &resolution.by_path {
        let Some(index) = candidate else {
            continue;
        };
        let native = &model.resources[*index];
        let id = ResourceId::new(resources.len() as u32);
        let mut properties = native.properties.clone();
        if native
            .entity_id
            .is_some_and(|entity_id| model.cover_resource_ids.contains(&entity_id))
            || cover_native_indices.contains(index)
        {
            properties.insert("cover-image".to_owned());
        }
        loader.insert(path.clone(), native.bytes.clone());
        resource_ids_by_path.insert(path.clone(), id);
        resources.push(Resource {
            id,
            path: path.clone(),
            media_type: native.media_type.clone(),
            kind: resource_kind(&native.media_type),
            properties: properties.into_iter().collect(),
            size: Some(native.bytes.len() as u64),
        });
    }

    (resources, loader, resource_ids_by_path)
}

/// Recover the cover when a KFX book has the usual cover-only first content
/// fragment but omits the title-metadata `cover_image` declaration.  The
/// fallback is deliberately narrow: it requires that no resource is already
/// declared as a cover, the first emitted document contains exactly one
/// raster/vector image and no non-whitespace text, and that image is not used
/// anywhere else in the recovered reading order.  This avoids turning an
/// arbitrary first illustration or a repeated decoration into a cover.
fn infer_kfx_cover_resource(
    resources: &mut [Resource],
    documents: &[Document],
    model: &mut NativeModel,
) {
    if resources.iter().any(|resource| {
        resource
            .properties
            .iter()
            .any(|property| property == "cover-image")
    }) {
        return;
    }

    let Some(first_document) = documents.first() else {
        return;
    };
    let mut first_document_images = Vec::new();
    let mut has_non_whitespace_text = false;
    collect_kfx_cover_candidate_nodes(
        &first_document.nodes,
        &mut first_document_images,
        &mut has_non_whitespace_text,
    );
    if has_non_whitespace_text || first_document_images.len() != 1 {
        return;
    }

    let resource_id = first_document_images[0];
    let Some(resource) = resources
        .iter_mut()
        .find(|resource| resource.id == resource_id)
    else {
        return;
    };
    if !matches!(
        resource.kind,
        ResourceKind::Jpeg | ResourceKind::Png | ResourceKind::Gif | ResourceKind::Svg
    ) {
        return;
    }

    let reading_order_occurrences = documents
        .iter()
        .flat_map(|document| document.nodes.iter())
        .map(|node| count_kfx_resource_occurrences(node, resource_id))
        .sum::<usize>();
    if reading_order_occurrences != 1 {
        return;
    }

    resource.properties.push("cover-image".to_owned());
    model
        .unknown_features
        .insert("kfx-cover-inferred-from-image-only-first-document".to_owned());
    model.input_loss.push(
        "KFX cover metadata was absent; marked the unique image in the first image-only document as an inferred cover.".to_owned(),
    );
}

fn collect_kfx_cover_candidate_nodes(
    nodes: &[Node],
    images: &mut Vec<ResourceId>,
    has_non_whitespace_text: &mut bool,
) {
    for node in nodes {
        match &node.kind {
            NodeKind::Image { resource, .. } => images.push(*resource),
            NodeKind::Text { value } if !value.trim().is_empty() => {
                *has_non_whitespace_text = true;
            }
            _ => {}
        }
        collect_kfx_cover_candidate_nodes(&node.children, images, has_non_whitespace_text);
    }
}

fn count_kfx_resource_occurrences(node: &Node, resource_id: ResourceId) -> usize {
    let own = usize::from(matches!(
        &node.kind,
        NodeKind::Image { resource, .. } if *resource == resource_id
    ));
    own + node
        .children
        .iter()
        .map(|child| count_kfx_resource_occurrences(child, resource_id))
        .sum::<usize>()
}

fn decode_document_symbols(model: &mut NativeModel, mode: ParseMode) -> Result<(), AmazonKfxError> {
    let fragments = model.fragments.clone();
    for fragment in &fragments {
        if fragment.fragment_type != 145 {
            continue;
        }
        // The string pool is a fragment whose identity identifies the pool
        // used by content reference records.
        let Some(table_id) = struct_get(&fragment.value, FIELD_FRAGMENT_ID)
            .and_then(|value| match value {
                IonValue::Symbol(id) => u32::try_from(*id).ok(),
                _ => None,
            })
            .or(Some(fragment.entity_id))
        else {
            continue;
        };
        let Some(entries) = struct_get(&fragment.value, FIELD_TEXT_VALUES) else {
            model
                .unknown_features
                .insert("document-symbol-fragment-shape".to_owned());
            continue;
        };
        let IonValue::List(strings) = entries else {
            continue;
        };
        let values = strings
            .iter()
            .take(MAX_STRING_ITEMS)
            .filter_map(as_string)
            .map(ToOwned::to_owned)
            .collect();
        model.string_tables.insert(table_id, values);
    }
    if model.string_tables.is_empty() && mode == ParseMode::Strict {
        return Err(AmazonKfxError::Semantic(
            "CONT has no document symbol table".to_owned(),
        ));
    }
    Ok(())
}

fn absorb_symbol_table(model: &mut NativeModel, value: &IonValue) -> SymbolTable {
    let mut table = SymbolTable::default();
    let Some(fields) = unwrap_struct(value) else {
        return table;
    };
    if let Some(IonValue::List(imports)) =
        struct_get_from_fields(fields, FIELD_SYMBOL_TABLE_IMPORTS)
    {
        for import in imports {
            let Some(import_fields) = unwrap_struct(import) else {
                continue;
            };
            let import = SymbolTableImport {
                name: struct_get_from_fields(import_fields, FIELD_SYMBOL_TABLE_IMPORT_NAME)
                    .and_then(as_string)
                    .map(ToOwned::to_owned),
                version: struct_get_from_fields(import_fields, FIELD_SYMBOL_TABLE_IMPORT_VERSION)
                    .and_then(as_int)
                    .and_then(|value| u32::try_from(value).ok()),
                max_id: struct_get_from_fields(import_fields, FIELD_SYMBOL_TABLE_IMPORT_MAX_ID)
                    .and_then(as_int)
                    .and_then(|value| u32::try_from(value).ok()),
            };
            model.symbols.imports.push(import.clone());
            table.imports.push(import);
        }
    }
    let Some(local_start) = local_symbol_start(fields) else {
        model
            .unknown_features
            .insert("symbol-table-import-offset-unresolved".to_owned());
        return table;
    };
    let Some(symbols) = struct_get_from_fields(fields, FIELD_SYMBOL_TABLE_SYMBOLS) else {
        return table;
    };
    let IonValue::List(symbols) = symbols else {
        return table;
    };
    for (index, symbol) in symbols.iter().take(MAX_STRING_ITEMS).enumerate() {
        if let Some(symbol) = as_string(symbol) {
            let id = local_start.saturating_add(index as u64);
            model.symbols.names.insert(id, symbol.to_owned());
            table.names.insert(id, symbol.to_owned());
        }
    }
    if let Some(max_id) = struct_get_from_fields(fields, FIELD_SYMBOL_TABLE_MAX_ID).and_then(as_int)
    {
        if u64::try_from(max_id)
            .ok()
            .is_some_and(|value| value < local_start)
        {
            model
                .unknown_features
                .insert("symbol-table-max-id-inconsistent".to_owned());
        }
    }
    table
}

fn local_symbol_start(fields: &[(u64, IonValue)]) -> Option<u64> {
    let imported_count = match struct_get_from_fields(fields, FIELD_SYMBOL_TABLE_IMPORTS) {
        None | Some(IonValue::Symbol(3)) => 0,
        Some(IonValue::List(imports)) => imported_symbol_count(imports)?,
        Some(_) => return None,
    };
    ION_SYSTEM_SYMBOL_COUNT
        .checked_add(imported_count)?
        .checked_add(1)
}

fn import_symbol_id_shift(imports: &[SymbolTableImport]) -> Option<u64> {
    imports
        .iter()
        .filter(|import| import.max_id.is_some())
        .try_fold(0u64, |shift, _| shift.checked_add(ION_SYSTEM_SYMBOL_COUNT))
}

fn imported_symbol_count(imports: &[IonValue]) -> Option<u64> {
    imports.iter().try_fold(0u64, |total, import| {
        // Ion ignores null and non-struct entries in an import list.
        if unwrap_struct(import).is_none() {
            return Some(total);
        }
        let max_id = as_int(struct_get(import, FIELD_SYMBOL_TABLE_IMPORT_MAX_ID)?)
            .and_then(|value| u64::try_from(value).ok())?;
        total.checked_add(max_id.checked_sub(ION_SYSTEM_SYMBOL_COUNT)?)
    })
}

fn decode_fragment_values(model: &mut NativeModel, mode: ParseMode) -> Result<(), AmazonKfxError> {
    let mut fragment_ids = BTreeSet::new();
    let fragments = model.fragments.clone();
    for fragment in &fragments {
        if !fragment_ids.insert((fragment.entity_id, fragment.fragment_type)) {
            model.warnings.push(format!(
                "duplicate fragment entity/type pair {}/{}",
                fragment.entity_id, fragment.fragment_type
            ));
        }
        collect_metadata(model, &fragment.value);
        collect_string_tables(model, &fragment.value);
        collect_resources(model, &fragment.value, fragment.container_index);
        if fragment.fragment_type == 266 {
            collect_link_target(model, fragment);
        }
        if fragment.fragment_type == 389 {
            collect_navigation(model, fragment.container_index, &fragment.value);
        }
        if fragment.fragment_type == 0 {
            model
                .unknown_features
                .insert(format!("unknown-fragment-type-{}", fragment.fragment_type));
        }
    }
    if fragment_ids.is_empty() {
        let message = "CONT entity index contained no Ion fragments".to_owned();
        if mode == ParseMode::Strict {
            return Err(AmazonKfxError::Semantic(message));
        }
        model.input_loss.push(message);
    }
    Ok(())
}

fn collect_link_target(model: &mut NativeModel, fragment: &NativeFragment) {
    let Some(symbol) = struct_get(&fragment.value, FIELD_ANCHOR_NAME).and_then(ion_symbol_id)
    else {
        return;
    };

    let target = if let Some(uri) =
        struct_get(&fragment.value, FIELD_ANCHOR_URI).and_then(|value| match value {
            IonValue::String(uri) => Some(uri.clone()),
            IonValue::Symbol(symbol) => model
                .containers
                .get(fragment.container_index)
                .and_then(|container| container.symbols.names.get(symbol).cloned()),
            _ => None,
        }) {
        KfxLinkTarget::Uri(uri)
    } else {
        let Some(position) = struct_get(&fragment.value, FIELD_ANCHOR_POSITION)
            .and_then(|value| {
                as_int(value).or_else(|| struct_get(value, FIELD_NAV_TARGET_ID).and_then(as_int))
            })
            .and_then(|value| u32::try_from(value).ok())
        else {
            return;
        };
        KfxLinkTarget::Position(position)
    };

    let key = (fragment.container_index, symbol);
    if model
        .link_targets
        .get(&key)
        .is_some_and(|existing| existing != &target)
    {
        model.link_targets.remove(&key);
        model
            .unknown_features
            .insert("kfx-link-target-ambiguous".to_owned());
        return;
    }
    model.link_targets.insert(key, target);
}

fn collect_metadata(model: &mut NativeModel, value: &IonValue) {
    let Some(entries) = struct_get(value, FIELD_METADATA_ENTRIES) else {
        return;
    };
    let IonValue::List(entries) = entries else {
        return;
    };
    for entry in entries {
        let Some(kind) = struct_get(entry, FIELD_METADATA_KIND).and_then(as_string) else {
            continue;
        };
        if kind != "kindle_title_metadata" {
            continue;
        }
        let Some(pairs) = struct_get(entry, FIELD_METADATA_PAIRS) else {
            continue;
        };
        let IonValue::List(pairs) = pairs else {
            continue;
        };
        for pair in pairs {
            let Some(key) = struct_get(pair, FIELD_METADATA_KEY).and_then(as_string) else {
                continue;
            };
            let Some(value) = struct_get(pair, FIELD_METADATA_VALUE).and_then(as_string) else {
                continue;
            };
            match key {
                "title" => model.metadata.title = Some(value.to_owned()),
                "author" => model.metadata.add_author(value),
                "language" => model.metadata.language = Some(value.to_owned()),
                "publisher" => model.metadata.publisher = Some(value.to_owned()),
                "description" => model.metadata.description = Some(value.to_owned()),
                "issue_date" => {
                    if model.metadata.date.is_none() {
                        model.metadata.date = Some(value.to_owned());
                    }
                    model.metadata.dates.push(value.to_owned());
                }
                "ASIN" | "content_id" | "book_id" => {
                    model.metadata.add_identifier(value);
                }
                "cover" | "cover_image" | "cover_resource" | "cover_id" => {
                    if let Ok(resource_id) = value.parse::<u32>() {
                        model.cover_resource_ids.insert(resource_id);
                    } else if !value.is_empty() {
                        // Real KFX uses the external-resource fid here (for
                        // example `e6`). Preserve it for exact resolution in
                        // `materialize_ir_resources`.
                        model.cover_resource_fids.insert(value.to_owned());
                    }
                }
                _ => {}
            }
        }
    }
}

fn collect_string_tables(model: &mut NativeModel, value: &IonValue) {
    if let Some(IonValue::List(values)) = struct_get(value, FIELD_TEXT_VALUES) {
        let strings = values
            .iter()
            .filter_map(as_string)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !strings.is_empty() {
            model.string_tables.entry(0).or_insert(strings);
        }
    }
    match value {
        IonValue::Struct(fields) => {
            for (_, child) in fields {
                collect_string_tables(model, child);
            }
        }
        IonValue::List(values) | IonValue::SExp(values) => {
            for child in values {
                collect_string_tables(model, child);
            }
        }
        IonValue::Annotation { value, .. } => collect_string_tables(model, value),
        _ => {}
    }
}

fn collect_resources(model: &mut NativeModel, value: &IonValue, container_index: usize) {
    match value {
        IonValue::Blob(bytes) if bytes.len() > 32 => {
            if model
                .resources
                .iter()
                .any(|resource| resource.bytes == *bytes)
            {
                return;
            }
            let (media_type, _) = media_type_for_bytes(bytes);
            model.resources.push(NativeResource {
                container_index,
                fragment_type: 0,
                entity_id: None,
                media_type,
                bytes: bytes.clone(),
                entity_info: None,
                properties: BTreeSet::new(),
            });
        }
        IonValue::Struct(fields) => {
            for (_, child) in fields {
                collect_resources(model, child, container_index);
            }
        }
        IonValue::List(values) | IonValue::SExp(values) => {
            for child in values {
                collect_resources(model, child, container_index);
            }
        }
        IonValue::Annotation { value, .. } => collect_resources(model, value, container_index),
        _ => {}
    }
}

fn collect_navigation(model: &mut NativeModel, container_index: usize, value: &IonValue) {
    let Some(entries) = top_level_struct_field(value, FIELD_NAVIGATION_ROOT) else {
        model
            .unknown_features
            .insert("kfx-navigation-fragment-shape-unsupported".to_owned());
        model.input_loss.push(
            "A KFX navigation fragment did not contain the recognized navigation list.".to_owned(),
        );
        return;
    };
    let IonValue::List(entries) = entries else {
        model
            .unknown_features
            .insert("kfx-navigation-fragment-shape-unsupported".to_owned());
        model.input_loss.push(
            "A KFX navigation fragment did not contain the recognized navigation list.".to_owned(),
        );
        return;
    };

    let mut malformed_entries = 0usize;
    let has_heading_navigation = entries.iter().any(|entry| {
        navigation_fragment_value(model, container_index, entry, 391)
            .and_then(|entry| struct_get(&entry, FIELD_NAVIGATION_TYPE).and_then(ion_symbol_id))
            == Some(KFX_NAVIGATION_HEADINGS)
    });
    let mut parsed = Vec::new();
    for entry in entries {
        let Some(container) = navigation_fragment_value(model, container_index, entry, 391) else {
            // Keep the older inline shape as a bounded fallback. Some
            // compatibility fixtures place `$393` units directly under
            // `$392` rather than retaining the `$391` container wrapper.
            parsed.extend(parse_navigation_entries(
                std::slice::from_ref(entry),
                &mut malformed_entries,
            ));
            continue;
        };
        let Some(nav_type) = struct_get(&container, FIELD_NAVIGATION_TYPE).and_then(ion_symbol_id)
        else {
            // Legacy fixtures and a few older KFX producers put `$393`
            // navigation units directly under `$392`. Preserve that shape
            // instead of treating a unit as a malformed `$391` container.
            parsed.extend(parse_navigation_entries(
                std::slice::from_ref(entry),
                &mut malformed_entries,
            ));
            continue;
        };
        let Some(units) = struct_get(&container, FIELD_NAVIGATION_CHILDREN) else {
            malformed_entries = malformed_entries.saturating_add(1);
            continue;
        };
        let initial_heading_level =
            (nav_type == KFX_NAVIGATION_TOC && !has_heading_navigation).then_some(1);
        parsed.extend(parse_navigation_units(
            model,
            container_index,
            units,
            nav_type,
            initial_heading_level,
            &mut malformed_entries,
        ));
    }
    model.navigation.extend(parsed);
    if malformed_entries > 0 {
        model
            .unknown_features
            .insert("kfx-navigation-entry-shape-unsupported".to_owned());
        model.input_loss.push(format!(
            "{malformed_entries} malformed KFX navigation entries could not be retained."
        ));
    }
}

/// Resolve an inline navigation fragment or a symbol reference to the
/// corresponding native `$391`/`$393` value. Calibre expands these references
/// in its diagnostic `book.ion`; the binary KFX commonly keeps them as Ion
/// symbols, so both forms must be supported at the native boundary.
fn navigation_fragment_value(
    model: &NativeModel,
    container_index: usize,
    value: &IonValue,
    expected_type: u32,
) -> Option<IonValue> {
    match value {
        IonValue::Struct(_) | IonValue::Annotation { .. } => Some(value.clone()),
        IonValue::Symbol(symbol) => {
            let fid = model
                .containers
                .get(container_index)?
                .symbols
                .names
                .get(symbol)?;
            model
                .fragments
                .iter()
                .find(|fragment| {
                    fragment.container_index == container_index
                        && fragment.fragment_type == expected_type
                        && resolved_fragment_fid(model, fragment).as_deref() == Some(fid.as_str())
                })
                .map(|fragment| fragment.value.clone())
        }
        _ => None,
    }
}

fn ion_symbol_id(value: &IonValue) -> Option<u64> {
    match value {
        IonValue::Symbol(symbol) => Some(*symbol),
        _ => None,
    }
}

fn navigation_heading_level(value: &IonValue) -> Option<u8> {
    let symbol = struct_get(value, FIELD_NAVIGATION_LANDMARK_TYPE).and_then(ion_symbol_id)?;
    if !(KFX_NAVIGATION_HEADING_LEVEL_BASE..=KFX_NAVIGATION_HEADING_LEVEL_MAX).contains(&symbol) {
        return None;
    }
    u8::try_from(symbol - KFX_NAVIGATION_HEADING_LEVEL_BASE + 1).ok()
}

fn navigation_unit_values(
    model: &NativeModel,
    container_index: usize,
    value: &IonValue,
    malformed: &mut usize,
) -> Vec<IonValue> {
    let mut values = Vec::new();
    let Some(children) = struct_get(value, FIELD_NAVIGATION_CHILDREN) else {
        return values;
    };
    let Some(IonValue::List(children)) = (match children {
        IonValue::List(children) => Some(IonValue::List(children.clone())),
        _ => None,
    }) else {
        *malformed = malformed.saturating_add(1);
        return values;
    };
    for child in children {
        if let Some(child) = navigation_fragment_value(model, container_index, &child, 393) {
            values.push(child);
        } else {
            *malformed = malformed.saturating_add(1);
        }
    }
    if let Some(IonValue::List(entry_sets)) = struct_get(value, FIELD_NAVIGATION_ENTRY_SETS) {
        for entry_set in entry_sets {
            values.extend(navigation_unit_values(
                model,
                container_index,
                entry_set,
                malformed,
            ));
        }
    }
    values
}

fn parse_navigation_units(
    model: &NativeModel,
    container_index: usize,
    value: &IonValue,
    nav_type: u64,
    inherited_heading_level: Option<u8>,
    malformed: &mut usize,
) -> Vec<NavPointDraft> {
    let Some(raw_units) = struct_get(value, FIELD_NAVIGATION_CHILDREN)
        .and_then(|value| match value {
            IonValue::List(values) => Some(values.clone()),
            _ => None,
        })
        .or_else(|| match value {
            IonValue::List(values) => Some(values.clone()),
            _ => None,
        })
    else {
        *malformed = malformed.saturating_add(1);
        return Vec::new();
    };

    let mut output = Vec::new();
    for raw_unit in raw_units {
        let Some(unit) = navigation_fragment_value(model, container_index, &raw_unit, 393) else {
            *malformed = malformed.saturating_add(1);
            continue;
        };
        let explicit_heading_level = navigation_heading_level(&unit);
        let heading_level = (nav_type == KFX_NAVIGATION_HEADINGS)
            .then(|| explicit_heading_level.or(inherited_heading_level))
            .flatten();
        let next_heading_level = if nav_type == KFX_NAVIGATION_HEADINGS {
            if let Some(explicit) = explicit_heading_level {
                Some(explicit)
            } else {
                heading_level.and_then(|level| (level < 6).then_some(level + 1))
            }
        } else {
            inherited_heading_level.and_then(|level| (level < 6).then_some(level + 1))
        };
        let children = navigation_unit_values(model, container_index, &unit, malformed);
        let child_points = if children.is_empty() {
            Vec::new()
        } else {
            parse_navigation_units_from_values(
                model,
                container_index,
                &children,
                nav_type,
                next_heading_level,
                malformed,
            )
        };
        let label = struct_get(&unit, FIELD_NAVIGATION_REPRESENTATION)
            .and_then(|representation| struct_get(representation, FIELD_FRAGMENT_LABEL))
            .and_then(as_string)
            .map(str::trim)
            .unwrap_or_default()
            .to_owned();
        let target = navigation_target(&unit);
        if let Some(target) = target {
            output.push(NavPointDraft {
                label,
                target,
                heading_level,
                children: child_points,
            });
        } else {
            *malformed = malformed.saturating_add(1);
            output.extend(child_points);
        }
    }
    output
}

fn parse_navigation_units_from_values(
    model: &NativeModel,
    container_index: usize,
    values: &[IonValue],
    nav_type: u64,
    inherited_heading_level: Option<u8>,
    malformed: &mut usize,
) -> Vec<NavPointDraft> {
    let synthetic = IonValue::List(values.to_vec());
    parse_navigation_units(
        model,
        container_index,
        &synthetic,
        nav_type,
        inherited_heading_level,
        malformed,
    )
}

fn top_level_struct_field(value: &IonValue, field_id: u64) -> Option<&IonValue> {
    match value {
        IonValue::Struct(_) => struct_get(value, field_id),
        IonValue::List(items) | IonValue::SExp(items) => items
            .iter()
            .find_map(|item| top_level_struct_field(item, field_id)),
        IonValue::Annotation { value, .. } => top_level_struct_field(value, field_id),
        _ => None,
    }
}

fn parse_navigation_entries(entries: &[IonValue], malformed: &mut usize) -> Vec<NavPointDraft> {
    let mut parsed = Vec::new();
    for entry in entries {
        let label = struct_get(entry, FIELD_NAVIGATION_LABEL)
            .and_then(|labels| struct_get(labels, FIELD_FRAGMENT_LABEL))
            .and_then(as_string)
            .map(str::trim)
            .filter(|label| !label.is_empty());
        let target = navigation_target(entry);
        let children = match struct_get(entry, FIELD_NAVIGATION_CHILDREN) {
            Some(IonValue::List(children)) => parse_navigation_entries(children, malformed),
            Some(_) => {
                *malformed = malformed.saturating_add(1);
                Vec::new()
            }
            None => Vec::new(),
        };

        if let (Some(label), Some(target)) = (label, target) {
            parsed.push(NavPointDraft {
                label: label.to_owned(),
                target,
                heading_level: None,
                children,
            });
        } else if struct_get(entry, FIELD_NAVIGATION_CHILDREN).is_some()
            && label.is_none()
            && target.is_none()
        {
            // KFX navigation groups may wrap the actual link entries without
            // carrying a user-visible label or content position themselves.
            parsed.extend(children);
        } else {
            *malformed = malformed.saturating_add(1);
            // Preserve any decodable descendants if the parent itself has no
            // usable label/position rather than dropping the whole subtree.
            parsed.extend(children);
        }
    }
    parsed
}

fn navigation_target(value: &IonValue) -> Option<u32> {
    let target = struct_get(value, FIELD_FRAGMENT_TARGET)?;
    as_int(target)
        .or_else(|| struct_get(target, FIELD_NAV_TARGET_ID).and_then(as_int))
        .and_then(|value| u32::try_from(value).ok())
}

fn collect_navigation_targets(points: &[NavPointDraft], output: &mut BTreeSet<u32>) {
    for point in points {
        output.insert(point.target);
        collect_navigation_targets(&point.children, output);
    }
}

fn collect_navigation_labels(points: &[NavPointDraft], output: &mut BTreeMap<u32, Vec<String>>) {
    for point in points {
        output
            .entry(point.target)
            .or_default()
            .push(point.label.clone());
        collect_navigation_labels(&point.children, output);
    }
}

fn collect_navigation_heading_levels(points: &[NavPointDraft], output: &mut BTreeMap<u32, u8>) {
    for point in points {
        if let Some(level) = point.heading_level {
            output.entry(point.target).or_insert(level);
        }
        collect_navigation_heading_levels(&point.children, output);
    }
}

fn normalized_kfx_label(value: &str) -> String {
    value.split_whitespace().collect()
}

fn kfx_navigation_label_matches(
    text: &str,
    position: Option<u32>,
    navigation_labels: &BTreeMap<u32, Vec<String>>,
) -> bool {
    let Some(position) = position else {
        return false;
    };
    let normalized_text = normalized_kfx_label(text);
    navigation_labels.get(&position).is_some_and(|labels| {
        labels
            .iter()
            .any(|label| normalized_kfx_label(label) == normalized_text)
    })
}

fn collect_navigation_section_symbols(
    model: &NativeModel,
    navigation_targets: &BTreeSet<u32>,
) -> BTreeSet<u64> {
    let mut symbols = BTreeSet::new();
    for fragment in model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 260)
    {
        let Some(IonValue::List(entries)) = struct_get(&fragment.value, FIELD_SECTION_ENTRIES)
        else {
            continue;
        };
        for entry in entries {
            let target = struct_get(entry, FIELD_NAV_TARGET_ID)
                .and_then(as_int)
                .and_then(|value| u32::try_from(value).ok());
            if !target.is_some_and(|target| navigation_targets.contains(&target)) {
                continue;
            }
            if let Some(IonValue::Symbol(symbol)) = struct_get(entry, FIELD_CONTENT_FRAGMENT_SYMBOL)
            {
                symbols.insert(*symbol);
            }
        }
    }
    symbols
}

fn insert_target_location<K: Ord>(
    locations: &mut BTreeMap<K, Option<TargetLocation>>,
    target: K,
    location: TargetLocation,
) {
    use std::collections::btree_map::Entry;

    match locations.entry(target) {
        Entry::Vacant(entry) => {
            entry.insert(Some(location));
        }
        Entry::Occupied(mut entry) => match entry.get() {
            Some(existing)
                if existing.document == location.document && existing.node == location.node => {}
            Some(_) => {
                // A position reused by multiple documents is ambiguous. Do
                // not point navigation at whichever fragment happened first.
                entry.insert(None);
            }
            None => {}
        },
    }
}

fn mark_target_location_ambiguous<K: Ord>(
    locations: &mut BTreeMap<K, Option<TargetLocation>>,
    target: K,
) {
    locations.insert(target, None);
}

fn resolve_navigation(
    points: &[NavPointDraft],
    positions: &BTreeMap<u32, Option<TargetLocation>>,
    sections: &BTreeMap<u32, Option<TargetLocation>>,
    entities: &BTreeMap<u32, Option<TargetLocation>>,
    unresolved: &mut usize,
) -> Vec<NavPoint> {
    let mut resolved = Vec::new();
    for point in points {
        let children =
            resolve_navigation(&point.children, positions, sections, entities, unresolved);
        let location = match positions.get(&point.target) {
            Some(Some(location)) => Some(location),
            Some(None) => None,
            None => match sections.get(&point.target) {
                Some(Some(location)) => Some(location),
                Some(None) => None,
                None => entities.get(&point.target).and_then(Option::as_ref),
            },
        };
        if let Some(location) = location {
            resolved.push(NavPoint {
                label: point.label.clone(),
                href: location.href.clone(),
                children,
            });
        } else {
            *unresolved = unresolved.saturating_add(1);
            // Keep independently resolved descendants in document order.
            resolved.extend(children);
        }
    }
    resolved
}

fn filter_kfx_navigation(points: Vec<NavPoint>) -> Vec<NavPoint> {
    fn collect_non_start_destinations(points: &[NavPoint], output: &mut BTreeSet<String>) {
        for point in points {
            if !point.label.trim().eq_ignore_ascii_case("start") {
                output.insert(point.href.clone());
            }
            collect_non_start_destinations(&point.children, output);
        }
    }

    let mut non_start_destinations = BTreeSet::new();
    collect_non_start_destinations(&points, &mut non_start_destinations);

    fn filter(
        points: Vec<NavPoint>,
        seen: &mut BTreeSet<(String, String)>,
        non_start_destinations: &BTreeSet<String>,
    ) -> Vec<NavPoint> {
        points
            .into_iter()
            .filter_map(|point| {
                let label = point.label.trim();
                let synthetic = label.eq_ignore_ascii_case("table of contents")
                    || label.to_ascii_lowercase().ends_with("nav-unit")
                    // Kindle sometimes appends a synthetic Start entry that
                    // targets the same location as the first real chapter.
                    // Keep a genuine standalone "Start" link, but remove it
                    // when a user-visible destination already exists.
                    || (label.eq_ignore_ascii_case("start")
                        && non_start_destinations.contains(&point.href));
                if synthetic {
                    return None;
                }
                // Some Kindle navigation fragments repeat the same user-visible
                // entry when a reading-order group is also exposed as a child
                // list (C022/C075).  Remove only an exact label+destination
                // duplicate; equal destinations with different labels remain
                // valid, as do equal labels pointing at different locations.
                let key = (label.to_owned(), point.href.clone());
                if !seen.insert(key) {
                    return None;
                }
                Some(NavPoint {
                    label: point.label,
                    href: point.href,
                    children: filter(point.children, seen, non_start_destinations),
                })
            })
            .collect()
    }

    filter(points, &mut BTreeSet::new(), &non_start_destinations)
}

fn kfx_heading_level(
    style_name: Option<&str>,
    text: &str,
    source_heading_level: Option<u8>,
    navigation_heading_level: Option<u8>,
    is_navigation_target: bool,
    is_toc_document: bool,
    navigation_label_matches: bool,
) -> Option<u8> {
    if is_toc_document {
        // Calibre and Bokō keep the visible TOC title as a level-4 heading,
        // while the linked entries below it remain ordinary paragraphs. The
        // title must still be a real navigation target; do not promote an
        // arbitrary first string in a TOC-shaped fragment.
        if is_navigation_target && text.trim() == "目录" {
            return Some(4);
        }
        return None;
    }
    // `$790` is a source-level declaration and therefore takes precedence
    // over local style names and navigation heuristics.  A title may be
    // rendered as a heading even when the navigation tree does not expose a
    // link for it.
    if let Some(level) = source_heading_level {
        return Some(level);
    }
    // `$798` heading navigation is the authoritative fallback when the
    // content block itself has no `$790`. Its presentation label is usually
    // the synthetic `heading-nav-unit`, so comparing text to the label would
    // incorrectly discard every heading in that navigation container.
    if let Some(level) = navigation_heading_level {
        return Some(level);
    }
    // KFX style SIDs are local to a book.  A name such as s37 is a real
    // part-heading style in C082 but is the ordinary paragraph style in
    // C069.  Require navigation evidence for the compatibility fallbacks.
    // Do not require the first run in a fragment: large KFX content
    // fragments can contain many chapters (C008/C077), and their later
    // chapter starts are navigable too.
    if !is_navigation_target {
        return None;
    }
    match style_name.map(str::to_ascii_lowercase).as_deref() {
        Some("s1n") => Some(1),
        Some("s33" | "s3e" | "s1ze") => Some(4),
        Some("s1fb" | "s3ww") => Some(3),
        Some("s37") if kfx_is_part_heading(text) => Some(3),
        // C075 uses s3b for a repeated running title.  It is present in the
        // navigation-position stream but has no `$790` heading declaration;
        // Calibre and Bokō both keep it as body text.
        Some("s3b") => None,
        // A navigable text run with an unknown local style is still a better
        // heading candidate than flattening it into body prose, but only when
        // the navigation label independently matches the text.  This keeps
        // metadata such as C059's CIP title from becoming a heading merely
        // because it has a structural position.
        Some(_) | None if navigation_label_matches => Some(4),
        Some(_) | None => None,
    }
}

fn kfx_navigation_position(
    segment: &ContentTextSegment,
    navigation_targets: &BTreeSet<u32>,
) -> Option<u32> {
    segment
        .position
        .filter(|position| navigation_targets.contains(position))
        .or_else(|| {
            segment
                .parent_position
                .filter(|position| navigation_targets.contains(position))
        })
}

fn kfx_is_part_heading(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase();
    normalized == "part"
        || normalized
            .strip_prefix("part")
            .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
}

fn kfx_style_id(
    book: &mut Book,
    cache: &mut BTreeMap<String, StyleId>,
    style_name: Option<&str>,
    text: Option<&str>,
    is_heading: bool,
) -> StyleId {
    let Some(style_name) = style_name else {
        return book.styles.intern(ComputedStyle::default());
    };
    let key = style_name.to_ascii_lowercase();
    let variant = if is_heading {
        "heading"
    } else if key == "s1r" && text.is_some_and(|value| value.trim() == "目录") {
        "toc-title"
    } else {
        "body"
    };
    let cache_key = format!("{key}:{variant}");
    if let Some(style_id) = cache.get(&cache_key) {
        return *style_id;
    }
    let style_id = book.styles.intern(kfx_computed_style(
        &key,
        text.unwrap_or_default(),
        is_heading,
    ));
    cache.insert(cache_key, style_id);
    style_id
}

/// A compact, evidence-based translation of the KFX style symbols observed
/// in the real Kindle sample. The values intentionally mirror the computed
/// Calibre KFX Input stylesheet instead of inventing a new presentation
/// system. Unknown symbols retain a conservative reflowable-body fallback.
fn kfx_computed_style(style_name: &str, text: &str, is_heading: bool) -> ComputedStyle {
    let mut style = ComputedStyle::default();
    if style_name != "s57z" {
        style = style.with("display", "block");
    }
    if !is_heading && style_name == "s1r" {
        if text.trim() == "目录" {
            return style
                .with("font-size", "2em")
                .with("font-weight", "bold")
                .with("text-align", "center")
                .with("text-indent", "1em")
                .with("margin", "0 0 .980552em");
        }
        return style.with("margin", "1.34em 0 0");
    }
    if !is_heading
        && matches!(
            style_name,
            "s1n" | "s33" | "s37" | "s1fb" | "s3ww" | "s3e" | "s1ze"
        )
    {
        return style.with("margin", "1em 0");
    }
    match style_name {
        "s1n" => {
            style = style
                .with("font-size", "2em")
                .with("font-weight", "bold")
                .with("line-height", "1.2")
                .with("margin", ".67em 0 0")
        }
        "s33" => style = style.with("font-weight", "bold").with("margin", "2.68em 0"),
        "s3e" => style = style.with("font-weight", "bold").with("margin", "1.33em 0"),
        "s1ze" => {
            style = style
                .with("font-weight", "bold")
                .with("text-align", "center")
                .with("margin", "1.33em 0")
        }
        "s37" => {
            style = style
                .with("font-size", "1.125em")
                .with("font-weight", "bold")
                .with("line-height", "1.23071")
                .with("text-align", "center")
                .with("text-indent", "1.709em")
                .with("margin", "1.75633em 0 4.1002em")
        }
        "s1fb" => {
            style = style
                .with("font-size", "1.125em")
                .with("font-weight", "bold")
                .with("line-height", "1.23071")
                .with("text-align", "center")
                .with("text-indent", "1.709em")
                .with("margin", ".975047em 0 3.31891em")
        }
        "s3ww" => {
            style = style
                .with("font-size", "1.125em")
                .with("font-weight", "bold")
                .with("line-height", "1.23071")
                .with("text-align", "center")
                .with("text-indent", "1.709em")
                .with("margin", ".975047em 0 4.1002em")
        }
        "s5g" | "s5g1" => style = style.with("margin", "3.75em 0 0"),
        "s3k" | "s3k1" => style = style.with("margin", "0"),
        "s3g" | "s3g1" => style = style.with("margin", ".9375em 0 0"),
        "s4m" => style = style.with("text-align", "right").with("margin", "0"),
        "s9p" => {
            style = style
                .with("text-align", "center")
                .with("margin", "1.875em 0 0")
        }
        "s9s" => style = style.with("margin", "2.8125em 0 0"),
        "s16n" => {
            style = style
                .with("text-align", "right")
                .with("margin", ".9375em 0 0")
        }
        "swp" => {
            style = style
                .with("text-align", "center")
                .with("margin", "7.5em 0 0")
        }
        "sws" => style = style.with("text-align", "center").with("margin", "0"),
        "swu" => {
            style = style
                .with("text-align", "right")
                .with("margin", "1.875em 0 0")
        }
        "sww" => {
            style = style
                .with("text-align", "right")
                .with("margin", "0 0 5.625em")
        }
        "s1r"
            if text
                .trim()
                .chars()
                .all(|character| character.is_ascii_digit()) =>
        {
            style = style.with("margin", "1.34em 0 0")
        }
        "s1r" => style = style.with("margin", "1.34em 0 0"),
        "s1t" => style = style.with("text-indent", "2em").with("margin", "0"),
        "s1v" => style = style.with("margin", "0 0 0 6.25%"),
        "s" => style = style.with("margin", "1.9375em 0 0"),
        "s1" => style = style.with("margin", "2.9375em 0 0"),
        "s1g" => style = style.with("margin", "2.9375em 0 1em"),
        "ss" => style = style.with("text-indent", "2.7em").with("margin", "1em 0 0"),
        "su" => style = style.with("margin", "1em 0 0"),
        "sh" => {
            style = style
                .with("display", "block")
                .with("margin", "1.3025em auto")
        }
        "sy" => {
            style = style
                .with("margin", "1em 0 0 6.25%")
                .with("width", "25.391%")
        }
        "s57z" => style = style.with("width", "8.125em").with("height", "8.125em"),
        "s58b" => {
            style = style
                .with("text-indent", "12.812%")
                .with("margin", "1em 0 0")
        }
        "s59d" => {
            style = style
                .with("text-indent", "28.125%")
                .with("margin", "1em 0 0")
        }
        "s5a0" => style = style.with("margin", "1.3025em 0 0"),
        _ => style = style.with("margin", "1em 0"),
    }
    style
}

#[allow(clippy::too_many_arguments)]
fn kfx_text_children(
    book: &mut Book,
    next_node_id: &mut u32,
    text: &str,
    style_events: &[KfxTextStyleEvent],
    link_ranges: &[KfxTextLinkRange],
    link_target_symbol_id: Option<u64>,
    container_index: usize,
    link_targets: &BTreeMap<(usize, u64), KfxLinkTarget>,
    ruby_annotations: &[KfxRubyAnnotation],
    default_style: StyleId,
    unresolved_link_targets: &mut usize,
) -> Result<Vec<Node>, AmazonKfxError> {
    #[derive(Clone, Debug)]
    enum InlineRange {
        Sup {
            start: usize,
            end: usize,
        },
        Ruby {
            start: usize,
            end: usize,
            text: String,
        },
        Link {
            start: usize,
            end: usize,
            href: String,
        },
        Footnote {
            start: usize,
            end: usize,
            note: KfxNoteReference,
        },
    }

    let character_count = text.chars().count();
    let mut ranges = style_events
        .iter()
        .filter_map(|event| {
            (event.style_symbol_id == Some(617)).then_some((
                usize::try_from(event.text_offset?).ok()?,
                usize::try_from(event.text_length?).ok()?,
            ))
        })
        .filter_map(|(start, length)| {
            let end = start.checked_add(length)?;
            (start < end && end <= character_count).then_some(InlineRange::Sup { start, end })
        })
        .collect::<Vec<_>>();
    ranges.extend(ruby_annotations.iter().filter_map(|annotation| {
        let end = annotation.text_offset.checked_add(annotation.text_length)?;
        (annotation.text_offset < end && end <= character_count).then_some(InlineRange::Ruby {
            start: annotation.text_offset,
            end,
            text: annotation.text.clone(),
        })
    }));
    for link_range in link_ranges {
        let Some(href) = kfx_link_target_href(
            container_index,
            link_range.target_symbol_id,
            link_targets,
            unresolved_link_targets,
        ) else {
            continue;
        };
        let Some(end) = link_range.text_offset.checked_add(link_range.text_length) else {
            continue;
        };
        // Calibre's KFX Input keeps a link on one-scalar CJK entries even
        // when the source `$144` length is two (the source range units are
        // not always the same as Rust's Unicode-scalar count). Clamp only the
        // proven visible-text boundary; never extend a link past the decoded
        // text or invent text for an empty range.
        let end = end.min(character_count);
        if link_range.text_offset < end {
            let is_proven_footnote = ranges.iter().any(|range| {
                matches!(
                    range,
                    InlineRange::Sup { start, end: sup_end }
                        if *start <= link_range.text_offset && end <= *sup_end
                )
            });
            if is_proven_footnote {
                ranges.retain(|range| {
                    !matches!(
                        range,
                        InlineRange::Sup { start, end: sup_end }
                            if *start <= link_range.text_offset && end <= *sup_end
                    )
                });
                ranges.push(InlineRange::Footnote {
                    start: link_range.text_offset,
                    end,
                    note: KfxNoteReference {
                        kind: KfxNoteKind::Footnote,
                        href,
                        provenance: KfxNoteProvenance {
                            display_style_symbol_id: 617,
                            link_target_symbol_id: link_range.target_symbol_id,
                        },
                    },
                });
            } else {
                ranges.push(InlineRange::Link {
                    start: link_range.text_offset,
                    end,
                    href,
                });
            }
        }
    }
    if link_ranges.is_empty() {
        if let Some(target_symbol_id) = link_target_symbol_id {
            if let Some(href) = kfx_link_target_href(
                container_index,
                target_symbol_id,
                link_targets,
                unresolved_link_targets,
            ) {
                ranges.push(InlineRange::Link {
                    start: 0,
                    end: character_count,
                    href,
                });
            }
        }
    }
    ranges.sort_by_key(|range| match range {
        InlineRange::Link { start, end, .. } => (*start, *end, 0u8),
        InlineRange::Ruby { start, end, .. } => (*start, *end, 1u8),
        InlineRange::Footnote { start, end, .. } => (*start, *end, 2u8),
        InlineRange::Sup { start, end } => (*start, *end, 3u8),
    });

    let marker_style = book.styles.intern(
        ComputedStyle::default()
            .with("font-size", ".75em")
            .with("vertical-align", "super"),
    );
    let characters = text.chars().collect::<Vec<_>>();
    let mut children = Vec::new();
    let mut cursor = 0usize;

    for range in ranges {
        let (start, end) = match &range {
            InlineRange::Sup { start, end }
            | InlineRange::Ruby { start, end, .. }
            | InlineRange::Link { start, end, .. }
            | InlineRange::Footnote { start, end, .. } => (*start, *end),
        };
        if start < cursor {
            continue;
        }
        append_kfx_text_node(
            next_node_id,
            &mut children,
            characters[cursor..start].iter().collect(),
            default_style,
        )?;
        match range {
            InlineRange::Sup { .. } => {
                let marker = characters[start..end].iter().collect::<String>();
                let marker_id = allocate_node_id(next_node_id)?;
                let marker_text_id = allocate_node_id(next_node_id)?;
                children.push(
                    Node::new(
                        marker_id,
                        NodeKind::GenericInline {
                            tag: "sup".to_owned(),
                        },
                        marker_style,
                        vec![Node::new(
                            marker_text_id,
                            NodeKind::Text { value: marker },
                            default_style,
                            Vec::new(),
                        )
                        .with_semantics(
                            SemanticRole::Text,
                            Default::default(),
                            Confidence::StronglyInferred,
                        )],
                    )
                    .with_semantics(
                        SemanticRole::Generic,
                        Default::default(),
                        Confidence::Heuristic,
                    ),
                );
            }
            InlineRange::Link { href, .. } => {
                let linked_text = characters[start..end].iter().collect::<String>();
                let text_id = allocate_node_id(next_node_id)?;
                let link_id = allocate_node_id(next_node_id)?;
                let text_node = Node::new(
                    text_id,
                    NodeKind::Text { value: linked_text },
                    default_style,
                    Vec::new(),
                )
                .with_semantics(
                    SemanticRole::Text,
                    Default::default(),
                    Confidence::StronglyInferred,
                );
                children.push(
                    Node::new(
                        link_id,
                        NodeKind::Link { href },
                        default_style,
                        vec![text_node],
                    )
                    .with_semantics(
                        SemanticRole::Link,
                        Default::default(),
                        Confidence::Explicit,
                    ),
                );
            }
            InlineRange::Footnote { note, .. } => {
                let marker = characters[start..end].iter().collect::<String>();
                let marker_id = allocate_node_id(next_node_id)?;
                let marker_text_id = allocate_node_id(next_node_id)?;
                let marker_text = Node::new(
                    marker_text_id,
                    NodeKind::Text { value: marker },
                    default_style,
                    Vec::new(),
                )
                .with_semantics(
                    SemanticRole::Text,
                    Default::default(),
                    Confidence::StronglyInferred,
                );
                children.push(
                    Node::new(
                        marker_id,
                        NodeKind::Footnote {
                            href: Some(note.href),
                        },
                        marker_style,
                        vec![marker_text],
                    )
                    .with_semantics(
                        match note.kind {
                            KfxNoteKind::Footnote => SemanticRole::Footnote,
                            KfxNoteKind::Endnote => SemanticRole::Endnote,
                        },
                        Default::default(),
                        Confidence::Explicit,
                    ),
                );
            }
            InlineRange::Ruby {
                text: ruby_text, ..
            } => {
                let ruby_id = allocate_node_id(next_node_id)?;
                let base_id = allocate_node_id(next_node_id)?;
                let reading_id = allocate_node_id(next_node_id)?;
                let reading_text_id = allocate_node_id(next_node_id)?;
                let base = Node::new(
                    base_id,
                    NodeKind::Text {
                        value: characters[start..end].iter().collect(),
                    },
                    default_style,
                    Vec::new(),
                )
                .with_semantics(
                    SemanticRole::Text,
                    Default::default(),
                    Confidence::StronglyInferred,
                );
                let reading = Node::new(
                    reading_id,
                    NodeKind::GenericInline {
                        tag: "rt".to_owned(),
                    },
                    default_style,
                    vec![Node::new(
                        reading_text_id,
                        NodeKind::Text { value: ruby_text },
                        default_style,
                        Vec::new(),
                    )
                    .with_semantics(
                        SemanticRole::Text,
                        Default::default(),
                        Confidence::StronglyInferred,
                    )],
                )
                .with_semantics(
                    SemanticRole::Generic,
                    Default::default(),
                    Confidence::StronglyInferred,
                );
                children.push(
                    Node::new(ruby_id, NodeKind::Ruby, default_style, vec![base, reading])
                        .with_semantics(
                            SemanticRole::Ruby,
                            Default::default(),
                            Confidence::StronglyInferred,
                        ),
                );
            }
        }
        cursor = end;
    }
    append_kfx_text_node(
        next_node_id,
        &mut children,
        characters[cursor..].iter().collect(),
        default_style,
    )?;
    if children.is_empty() {
        append_kfx_text_node(next_node_id, &mut children, text.to_owned(), default_style)?;
    }
    Ok(children)
}

fn kfx_link_target_href(
    container_index: usize,
    target_symbol_id: u64,
    link_targets: &BTreeMap<(usize, u64), KfxLinkTarget>,
    unresolved_link_targets: &mut usize,
) -> Option<String> {
    let Some(target) = link_targets.get(&(container_index, target_symbol_id)) else {
        *unresolved_link_targets = unresolved_link_targets.saturating_add(1);
        return None;
    };
    Some(match target {
        KfxLinkTarget::Position(position) => format!("kfx-position:{position}"),
        KfxLinkTarget::Uri(uri) => uri.clone(),
    })
}

fn append_kfx_text_node(
    next_node_id: &mut u32,
    children: &mut Vec<Node>,
    value: String,
    style: StyleId,
) -> Result<(), AmazonKfxError> {
    if value.is_empty() {
        return Ok(());
    }
    let node_id = allocate_node_id(next_node_id)?;
    children.push(
        Node::new(node_id, NodeKind::Text { value }, style, Vec::new()).with_semantics(
            SemanticRole::Text,
            Default::default(),
            Confidence::StronglyInferred,
        ),
    );
    Ok(())
}

fn append_anchor(
    anchors: &mut Vec<Anchor>,
    document: DocumentId,
    node: NodeId,
    name: String,
) -> Result<(), AmazonKfxError> {
    let id = u32::try_from(anchors.len()).map_err(|_| {
        AmazonKfxError::Semantic("KFX produced more anchors than the IR can identify".to_owned())
    })?;
    anchors.push(Anchor {
        id: AnchorId::new(id),
        document,
        node,
        name,
    });
    Ok(())
}

fn allocate_node_id(next_id: &mut u32) -> Result<NodeId, AmazonKfxError> {
    let id = *next_id;
    *next_id = next_id.checked_add(1).ok_or_else(|| {
        AmazonKfxError::Semantic("KFX produced more nodes than the IR can identify".to_owned())
    })?;
    Ok(NodeId::new(id))
}

fn collect_navigation_positions(
    value: &IonValue,
    navigation_targets: &BTreeSet<u32>,
    output: &mut BTreeSet<u32>,
) {
    match value {
        IonValue::Struct(fields) => {
            if let Some(position) = struct_get(value, FIELD_NAV_TARGET_ID)
                .and_then(as_int)
                .and_then(|value| u32::try_from(value).ok())
                .filter(|position| navigation_targets.contains(position))
            {
                output.insert(position);
            }
            for (_, child) in fields {
                collect_navigation_positions(child, navigation_targets, output);
            }
        }
        IonValue::List(items) | IonValue::SExp(items) => {
            for item in items {
                collect_navigation_positions(item, navigation_targets, output);
            }
        }
        IonValue::Annotation { value, .. } => {
            collect_navigation_positions(value, navigation_targets, output)
        }
        _ => {}
    }
}

fn observed_whitespace_resource_record_symbol_for_imports(
    imports: &[SymbolTableImport],
) -> Option<u64> {
    let mut matches = imports.iter().enumerate().filter(|(_, import)| {
        import.name.as_deref() == Some("YJ_symbols")
            && import.version == Some(10)
            // The imported table's max id varies between Kindle producers
            // (the local corpus contains 850, 851, and 859).  The placement
            // SID is derived from the table order and the preceding imports,
            // so requiring only a present max id is the correct boundary.
            && import.max_id.is_some()
    });
    let (index, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let preceding_import_symbols = imports[..index].iter().try_fold(0u64, |total, import| {
        let max_id = u64::from(import.max_id?);
        total.checked_add(max_id.checked_sub(ION_SYSTEM_SYMBOL_COUNT)?)
    })?;
    ION_SYSTEM_SYMBOL_COUNT
        .checked_add(1)?
        .checked_add(preceding_import_symbols)?
        .checked_add(WHITESPACE_RESOURCE_RECORD_SYMBOL_OFFSET)
}

fn observed_whitespace_resource_record_symbol_for_imports_content(
    imports: &[SymbolTableImport],
) -> Option<u64> {
    let mut matches = imports.iter().enumerate().filter(|(_, import)| {
        import.name.as_deref() == Some("YJ_symbols")
            && import.version == Some(10)
            // See the corresponding note in the standard-Ion helper above.
            && import.max_id.is_some()
    });
    let (index, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let preceding_import_symbols = imports[..index].iter().try_fold(0u64, |total, import| {
        total.checked_add(u64::from(import.max_id?))
    })?;
    ION_SYSTEM_SYMBOL_COUNT
        .checked_add(1)?
        .checked_add(preceding_import_symbols)?
        .checked_add(WHITESPACE_RESOURCE_RECORD_SYMBOL_OFFSET)
}

fn resolve_content_resource_symbols(
    model: &NativeModel,
    resolution: &NativeResourcePathResolution,
    resource_ids_by_path: &BTreeMap<String, ResourceId>,
    container_index: usize,
) -> BTreeMap<u64, (ResourceId, bool)> {
    let mut resolved = BTreeMap::new();
    let Some(container) = model.containers.get(container_index) else {
        return resolved;
    };
    let Some(content_import_shift) = import_symbol_id_shift(&container.symbols.imports) else {
        return resolved;
    };
    let mut ambiguous = BTreeSet::new();
    for (symbol, name) in &container.symbols.names {
        let Some(path) = normalize_kfx_resource_path(name) else {
            continue;
        };
        let Some(native_index) = resolution.by_location.get(name).and_then(|value| *value) else {
            continue;
        };
        let Some(resource_id) = resource_ids_by_path.get(&path).copied() else {
            continue;
        };
        let is_image = model
            .resources
            .get(native_index)
            .is_some_and(|resource| resource.media_type.starts_with("image/"));
        let Some(content_symbol) = symbol.checked_add(content_import_shift) else {
            continue;
        };
        let value = (resource_id, is_image);
        if ambiguous.contains(&content_symbol) {
            continue;
        }
        if resolved
            .get(&content_symbol)
            .is_some_and(|existing| *existing != value)
        {
            resolved.remove(&content_symbol);
            ambiguous.insert(content_symbol);
            continue;
        }
        resolved.insert(content_symbol, value);
    }
    resolved
}

/// Resolve `$271/$175` image records using the Ion SID in this container's
/// symbol table. This is separate from the legacy `$157` interpretation used
/// by the earlier whitespace-placeholder recognizer.
fn resolve_content_image_symbols(
    model: &NativeModel,
    identities: &BTreeMap<String, ExternalPlacementIdentity>,
    container_index: usize,
) -> BTreeMap<u64, (ResourceId, bool)> {
    let mut resolved = BTreeMap::new();
    let Some(container) = model.containers.get(container_index) else {
        return resolved;
    };
    for (symbol, name) in &container.symbols.names {
        let Some(identity) = identities.get(name) else {
            continue;
        };
        if identity.status == "exact_image_identity" {
            if let Some(resource_id) = identity.resource {
                resolved.insert(*symbol, (resource_id, true));
            }
        }
    }
    resolved
}

fn count_content_text_leaves(value: &IonValue) -> usize {
    match value {
        IonValue::Struct(fields) => fields
            .iter()
            .map(|(field, child)| match *field {
                FIELD_CONTENT_STRING_REFERENCE => 1,
                FIELD_FRAGMENT_TEXT => {
                    usize::from(as_string(child).is_some_and(|text| !text.is_empty()))
                }
                _ => count_content_text_leaves(child),
            })
            .sum(),
        IonValue::List(items) | IonValue::SExp(items) => {
            items.iter().map(count_content_text_leaves).sum()
        }
        IonValue::Annotation { value, .. } => count_content_text_leaves(value),
        IonValue::String(text) => usize::from(!text.is_empty()),
        _ => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_content_segments_traced(
    value: &IonValue,
    context: &ContentDecodeContext<'_>,
    inherited_position: Option<u32>,
    inherited_style_name: Option<&str>,
    inherited_single_text_position: Option<u32>,
    inherited_single_text_heading_level: Option<u8>,
    inside_image_container: bool,
    inline_image_context: bool,
    emit_scoped_image_text: bool,
    allow_content_list_text: bool,
    inherited_heading_position: Option<u32>,
    inherited_link_target: Option<u64>,
    inherited_link_ranges: &[KfxTextLinkRange],
    inherited_image_link_target: Option<u64>,
    heading_target_pending: &mut bool,
    output: &mut Vec<ContentSegment>,
    unresolved: &mut usize,
    inferred_images: &mut usize,
    link_target_positions: &BTreeSet<u32>,
    link_target_segment_indices: &mut BTreeMap<u32, usize>,
    event_trace: &mut Option<ContentTextTraceState<'_>>,
) {
    match value {
        IonValue::Struct(fields) => {
            let output_start = output.len();
            let own_position = struct_get(value, FIELD_NAV_TARGET_ID)
                .and_then(as_int)
                .and_then(|value| u32::try_from(value).ok());
            let position = own_position.or(inherited_position);
            let own_heading_position = own_position.filter(|position| {
                context
                    .navigation_heading_levels
                    .is_some_and(|levels| levels.contains_key(position))
            });
            let active_heading_position = own_heading_position.or(inherited_heading_position);
            let own_link_target =
                struct_get(value, FIELD_TEXT_STYLE_LINK_TARGET).and_then(ion_symbol_id);
            let active_link_target = own_link_target.or(inherited_link_target);
            let own_link_ranges = content_link_ranges(value);
            let active_link_ranges = if own_link_ranges.is_empty() {
                inherited_link_ranges
            } else {
                own_link_ranges.as_slice()
            };
            let text_leaf_count = count_content_text_leaves(value);
            let active_image_link_target = inherited_image_link_target.or_else(|| {
                (text_leaf_count == 0)
                    .then(|| image_link_target(active_link_target, own_link_ranges.as_slice()))
                    .flatten()
            });
            let mut own_heading_pending = true;
            let heading_target_pending = if own_heading_position.is_some() {
                &mut own_heading_pending
            } else {
                heading_target_pending
            };
            let single_text_subtree = text_leaf_count == 1;
            let single_text_position = inherited_single_text_position
                .or_else(|| single_text_subtree.then_some(own_position).flatten());
            let own_heading_level = struct_get(value, FIELD_CONTENT_HEADING_LEVEL)
                .and_then(as_int)
                .and_then(|value| u8::try_from(value).ok())
                .filter(|level| (1..=6).contains(level));
            let single_text_heading_level = inherited_single_text_heading_level
                .or_else(|| single_text_subtree.then_some(own_heading_level).flatten());
            let source_heading_level = own_heading_level.or(inherited_single_text_heading_level);
            let parent_position = inherited_single_text_position.or_else(|| {
                single_text_subtree
                    .then_some(own_position)
                    .flatten()
                    .filter(|parent| own_position != Some(*parent))
            });
            let adjacent_style_events = adjacent_text_style_events(value);
            let style_name = content_style_name(value, context)
                .or_else(|| inherited_style_name.map(str::to_owned));
            let node_type =
                struct_get(value, FIELD_CONTENT_NODE_TYPE).and_then(|value| match value {
                    IonValue::Symbol(symbol) => Some(*symbol),
                    _ => None,
                });
            let inline_image_context = inline_image_context
                || (node_type == Some(KFX_IMAGE_CAPTION_NODE_TYPE) && text_leaf_count > 0);
            let is_image_text_record =
                node_type.is_some_and(|node_type| {
                    matches!(
                        node_type,
                        KFX_IMAGE_CONTAINER_NODE_TYPE
                            | KFX_IMAGE_CAPTION_NODE_TYPE
                            | KFX_IMAGE_RECORD_NODE_TYPE
                    )
                }) || struct_get(value, FIELD_CONTENT_ORACLE_RESOURCE_SYMBOL).is_some();
            let inside_image_container =
                inside_image_container || node_type == Some(KFX_IMAGE_CONTAINER_NODE_TYPE);
            if let Some(occurrence) = kfx_math_occurrence(value, context.string_tables) {
                debug_assert_eq!(occurrence.provenance.content_field_id, FIELD_MATH_CONTENT);
                debug_assert!(occurrence.provenance.visual_fragment_type.is_none());
                if let Some(trace) = event_trace.as_mut() {
                    if let Some(alt) = occurrence.alt.as_deref() {
                        let mut source_path = trace.source_path.clone();
                        source_path.push(format!(
                            "${}[0].$145",
                            occurrence.provenance.content_field_id
                        ));
                        record_captured_text_event(
                            trace.capture,
                            trace.source,
                            &source_path,
                            position,
                            "math_accessible_text",
                            alt,
                            &[],
                        );
                    }
                }
                output.push(ContentSegment::Math {
                    position,
                    parent_position,
                    occurrence,
                    style_name,
                });
                record_first_link_target_segment(
                    own_position,
                    output_start,
                    output,
                    link_target_positions,
                    link_target_segment_indices,
                );
                return;
            }
            let has_non_empty_text_style_events = struct_get(value, FIELD_TEXT_STYLE_EVENTS)
                .is_some_and(|value| matches!(value, IonValue::List(items) if !items.is_empty()));
            let emit_scoped_image_text = match node_type {
                Some(KFX_IMAGE_CONTAINER_NODE_TYPE) => false,
                Some(KFX_IMAGE_CAPTION_NODE_TYPE)
                    if inside_image_container && has_non_empty_text_style_events =>
                {
                    true
                }
                _ => emit_scoped_image_text,
            };
            let source_image_resource = context.image_record_symbol.and_then(|expected| {
                let is_source_image = struct_get(value, FIELD_CONTENT_NODE_TYPE).is_some_and(
                    |value| matches!(value, IonValue::Symbol(actual) if *actual == expected),
                );
                if !is_source_image {
                    return None;
                }
                let Some(IonValue::Symbol(symbol)) =
                    struct_get(value, FIELD_CONTENT_ORACLE_RESOURCE_SYMBOL)
                else {
                    return None;
                };
                context
                    .image_resource_symbols
                    .get(symbol)
                    .filter(|(_, is_image)| *is_image)
                    .map(|(resource, _)| *resource)
            });
            if let Some(resource) = source_image_resource {
                output.push(ContentSegment::Image {
                    position,
                    resource,
                    alt: content_image_alt(value),
                    style_name,
                    inline: inside_image_container || inline_image_context,
                    link_target_symbol_id: active_image_link_target,
                });
                record_first_link_target_segment(
                    own_position,
                    output_start,
                    output,
                    link_target_positions,
                    link_target_segment_indices,
                );
                // `$584` on a KFX image record is the source alternative
                // text/name. It belongs on the image node, never in the
                // reading-order text stream.
                return;
            }
            let is_observed_image_placeholder = source_image_resource.is_none()
                && context.image_record_symbol.is_some_and(|expected| {
                    struct_get(value, FIELD_CONTENT_NODE_TYPE).is_some_and(
                        |value| matches!(value, IonValue::Symbol(actual) if *actual == expected),
                    ) && struct_get(value, FIELD_CONTENT_RESOURCE_SYMBOL)
                        .and_then(|value| match value {
                            IonValue::Symbol(symbol) => context.resource_symbols.get(symbol),
                            _ => None,
                        })
                        .is_some_and(|(_, is_image)| *is_image)
                        && own_position.is_some()
                        && struct_get(value, FIELD_FRAGMENT_TEXT)
                            .and_then(as_string)
                            .is_some_and(|text| text.trim().is_empty())
                        && struct_get(value, FIELD_CONTENT_STRING_REFERENCE).is_none()
                        && struct_get(value, 142).is_none()
                        && struct_get(value, FIELD_TEXT_VALUES).is_none()
                });
            if is_observed_image_placeholder {
                let Some(IonValue::Symbol(symbol)) =
                    struct_get(value, FIELD_CONTENT_RESOURCE_SYMBOL)
                else {
                    return;
                };
                let Some((resource, true)) = context.resource_symbols.get(symbol).copied() else {
                    return;
                };
                output.push(ContentSegment::Image {
                    position,
                    resource,
                    alt: content_image_alt(value),
                    style_name,
                    inline: inside_image_container || inline_image_context,
                    link_target_symbol_id: active_image_link_target,
                });
                record_first_link_target_segment(
                    own_position,
                    output_start,
                    output,
                    link_target_positions,
                    link_target_segment_indices,
                );
                *inferred_images = inferred_images.saturating_add(1);
                return;
            }

            for (field, child) in fields {
                if let Some(trace) = event_trace.as_mut() {
                    trace.source_path.push(format!("${field}"));
                }
                match *field {
                    FIELD_CONTENT_STRING_REFERENCE => {
                        if let Some(text) = resolve_string_reference(child, context.string_tables) {
                            if let Some(trace) = event_trace.as_mut() {
                                record_captured_text_event(
                                    trace.capture,
                                    trace.source,
                                    trace.source_path,
                                    position,
                                    "string_reference",
                                    text,
                                    &adjacent_style_events,
                                );
                            }
                            let navigation_heading_position = if *heading_target_pending {
                                *heading_target_pending = false;
                                active_heading_position
                            } else {
                                None
                            };
                            output.push(ContentSegment::Text(ContentTextSegment {
                                position,
                                parent_position,
                                navigation_heading_position,
                                source_heading_level,
                                link_ranges: active_link_ranges.to_vec(),
                                link_target_symbol_id: active_link_target,
                                text: text.to_owned(),
                                style_name: style_name.clone(),
                                style_events: adjacent_style_events.clone(),
                                ruby_annotations: content_ruby_annotations(
                                    value,
                                    context,
                                    text.chars().count(),
                                ),
                            }));
                        } else {
                            *unresolved = unresolved.saturating_add(1);
                        }
                    }
                    FIELD_FRAGMENT_TEXT => {
                        if is_image_text_record {
                            // `$584` is the KFX image title/alternative-text
                            // field.  It is attached to the Image node when
                            // the source resource is resolved; when identity
                            // is unresolved it must still not become fake
                            // reading-order prose (for example, `imgtitlepage`).
                        } else if let Some(text) = as_string(child) {
                            if let Some(trace) = event_trace.as_mut() {
                                record_captured_text_event(
                                    trace.capture,
                                    trace.source,
                                    trace.source_path,
                                    position,
                                    "direct_text",
                                    text,
                                    &adjacent_style_events,
                                );
                            }
                            let navigation_heading_position = if *heading_target_pending {
                                *heading_target_pending = false;
                                active_heading_position
                            } else {
                                None
                            };
                            output.push(ContentSegment::Text(ContentTextSegment {
                                position,
                                parent_position,
                                navigation_heading_position,
                                source_heading_level,
                                link_ranges: active_link_ranges.to_vec(),
                                link_target_symbol_id: active_link_target,
                                text: text.to_owned(),
                                style_name: style_name.clone(),
                                style_events: adjacent_style_events.clone(),
                                ruby_annotations: content_ruby_annotations(
                                    value,
                                    context,
                                    text.chars().count(),
                                ),
                            }));
                        }
                    }
                    _ => {
                        let child_allows_content_list_text = if *field == FIELD_TEXT_VALUES {
                            !inside_image_container
                                && node_type != Some(KFX_IMAGE_CONTAINER_NODE_TYPE)
                        } else {
                            false
                        };
                        collect_content_segments_traced(
                            child,
                            context,
                            position,
                            style_name.as_deref(),
                            single_text_position,
                            single_text_heading_level,
                            inside_image_container,
                            inline_image_context,
                            emit_scoped_image_text,
                            child_allows_content_list_text,
                            active_heading_position,
                            active_link_target,
                            active_link_ranges,
                            active_image_link_target,
                            heading_target_pending,
                            output,
                            unresolved,
                            inferred_images,
                            link_target_positions,
                            link_target_segment_indices,
                            event_trace,
                        )
                    }
                }
                if let Some(trace) = event_trace.as_mut() {
                    trace.source_path.pop();
                }
            }
            record_first_link_target_segment(
                own_position,
                output_start,
                output,
                link_target_positions,
                link_target_segment_indices,
            );
        }
        IonValue::List(items) | IonValue::SExp(items) => {
            for (index, item) in items.iter().enumerate() {
                if let Some(trace) = event_trace.as_mut() {
                    trace.source_path.push(format!("[{index}]"));
                }
                collect_content_segments_traced(
                    item,
                    context,
                    inherited_position,
                    inherited_style_name,
                    inherited_single_text_position,
                    inherited_single_text_heading_level,
                    inside_image_container,
                    inline_image_context,
                    emit_scoped_image_text,
                    allow_content_list_text,
                    inherited_heading_position,
                    inherited_link_target,
                    inherited_link_ranges,
                    inherited_image_link_target,
                    heading_target_pending,
                    output,
                    unresolved,
                    inferred_images,
                    link_target_positions,
                    link_target_segment_indices,
                    event_trace,
                );
                if let Some(trace) = event_trace.as_mut() {
                    trace.source_path.pop();
                }
            }
        }
        IonValue::Annotation { value, .. } => collect_content_segments_traced(
            value,
            context,
            inherited_position,
            inherited_style_name,
            inherited_single_text_position,
            inherited_single_text_heading_level,
            inside_image_container,
            inline_image_context,
            emit_scoped_image_text,
            allow_content_list_text,
            inherited_heading_position,
            inherited_link_target,
            inherited_link_ranges,
            inherited_image_link_target,
            heading_target_pending,
            output,
            unresolved,
            inferred_images,
            link_target_positions,
            link_target_segment_indices,
            event_trace,
        ),
        IonValue::String(text) if allow_content_list_text && !text.is_empty() => {
            if let Some(trace) = event_trace.as_mut() {
                record_captured_text_event(
                    trace.capture,
                    trace.source,
                    trace.source_path,
                    inherited_position,
                    "bare_content_list_text",
                    text,
                    &[],
                );
            }
            let navigation_heading_position = if *heading_target_pending {
                *heading_target_pending = false;
                inherited_heading_position
            } else {
                None
            };
            output.push(ContentSegment::Text(ContentTextSegment {
                position: inherited_position,
                parent_position: None,
                navigation_heading_position,
                source_heading_level: None,
                link_ranges: inherited_link_ranges.to_vec(),
                link_target_symbol_id: inherited_link_target,
                text: text.clone(),
                style_name: inherited_style_name.map(str::to_owned),
                style_events: Vec::new(),
                ruby_annotations: Vec::new(),
            }));
        }
        IonValue::String(text) if emit_scoped_image_text && !text.is_empty() => {
            if let Some(trace) = event_trace.as_mut() {
                record_captured_text_event(
                    trace.capture,
                    trace.source,
                    trace.source_path,
                    inherited_position,
                    "scoped_image_caption",
                    text,
                    &[],
                );
            }
            output.push(ContentSegment::Text(ContentTextSegment {
                position: inherited_position,
                parent_position: None,
                navigation_heading_position: None,
                source_heading_level: None,
                link_ranges: inherited_link_ranges.to_vec(),
                link_target_symbol_id: inherited_link_target,
                text: text.clone(),
                style_name: inherited_style_name.map(str::to_owned),
                style_events: Vec::new(),
                ruby_annotations: Vec::new(),
            }));
        }
        _ => {}
    }
}

fn record_first_link_target_segment(
    own_position: Option<u32>,
    output_start: usize,
    output: &[ContentSegment],
    link_target_positions: &BTreeSet<u32>,
    link_target_segment_indices: &mut BTreeMap<u32, usize>,
) {
    let Some(position) = own_position.filter(|position| link_target_positions.contains(position))
    else {
        return;
    };
    if output.len() > output_start {
        // A structural KFX position may be inherited by several nested
        // content-list items. It denotes the container, not an ambiguous set
        // of text locations, so retain the first readable descendant only.
        link_target_segment_indices
            .entry(position)
            .or_insert(output_start);
    }
}

fn record_captured_text_event(
    capture: &mut TextEventCapture,
    source: Option<&ContentTraceSource>,
    source_path: &[String],
    source_position: Option<u32>,
    source_value_kind: &'static str,
    text: &str,
    adjacent_style_events: &[KfxTextStyleEvent],
) {
    let Some(source) = source else {
        return;
    };
    let mut path = String::new();
    for part in source_path {
        if part.starts_with('$') && !path.is_empty() {
            path.push('.');
        }
        path.push_str(part);
    }
    capture.events.push(CapturedKfxTextEvent {
        section_id: source.section_id.clone(),
        story_id: source.story_id.clone(),
        source_fragment: source.source_fragment.clone(),
        source_fragment_order: source.source_fragment_order,
        eid: source.eid,
        source_path: path,
        source_position,
        text_kind: source_value_kind,
        text: text.to_owned(),
        adjacent_style_events: adjacent_style_events.to_vec(),
    });
}

/// `$157` is the content-node style SID for normal text records. The same
/// field is used as a resource SID by image placeholders, so only compact
/// Kindle style symbols are accepted here; resource paths and other symbols
/// are deliberately left unresolved.
fn content_style_name(value: &IonValue, context: &ContentDecodeContext<'_>) -> Option<String> {
    let IonValue::Symbol(symbol) = struct_get(value, FIELD_CONTENT_RESOURCE_SYMBOL)? else {
        return None;
    };
    let name = context.style_names?.get(symbol)?;
    let mut characters = name.chars();
    if characters.next()? != 's' || !characters.all(|character| character.is_ascii_alphanumeric()) {
        return None;
    }
    Some(name.clone())
}

fn content_image_alt(value: &IonValue) -> String {
    struct_get(value, FIELD_FRAGMENT_TEXT)
        .and_then(as_string)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn first_content_string<'a>(
    value: &'a IonValue,
    tables: &'a BTreeMap<u32, Vec<String>>,
) -> Option<&'a str> {
    match value {
        IonValue::String(text) => Some(text.as_str()),
        IonValue::Struct(fields) => {
            for (field, child) in fields {
                if *field == FIELD_CONTENT_STRING_REFERENCE {
                    if let Some(text) =
                        as_string(child).or_else(|| resolve_string_reference(child, tables))
                    {
                        return Some(text);
                    }
                }
                if let Some(text) = first_content_string(child, tables) {
                    return Some(text);
                }
            }
            None
        }
        IonValue::List(items) | IonValue::SExp(items) => items
            .iter()
            .find_map(|item| first_content_string(item, tables)),
        IonValue::Annotation { value, .. } => first_content_string(value, tables),
        _ => None,
    }
}

/// Recover the exact semantic pair emitted by Kindle Previewer for the
/// MathML probes.  The pair is only accepted when one item is a sanitized
/// MathML document and the other is its accessible text; arbitrary strings in
/// `$683` are not promoted to visible prose.
fn kfx_math_occurrence(
    value: &IonValue,
    tables: &BTreeMap<u32, Vec<String>>,
) -> Option<KfxMathOccurrence> {
    let entries = match struct_get(value, FIELD_MATH_CONTENT) {
        Some(IonValue::List(items) | IonValue::SExp(items)) => items,
        _ => return None,
    };
    let strings = entries
        .iter()
        .filter_map(|entry| first_content_string(entry, tables))
        .take(8)
        .collect::<Vec<_>>();
    let mathml = strings
        .iter()
        .copied()
        .find(|text| text.trim_start().starts_with("<math"))?;
    let (mathml, parsed_alt, display) = sanitize_kfx_mathml(mathml)?;
    let alt = parsed_alt.or_else(|| {
        strings
            .iter()
            .copied()
            .find(|text| !text.trim_start().starts_with("<math"))
            .map(ToOwned::to_owned)
    });
    Some(KfxMathOccurrence {
        mathml,
        alt,
        display,
        provenance: KfxMathProvenance {
            content_field_id: FIELD_MATH_CONTENT,
            visual_fragment_type: None,
        },
    })
}

fn sanitize_kfx_mathml(input: &str) -> Option<(String, Option<String>, bool)> {
    if input.len() > 1_000_000
        || input.to_ascii_uppercase().contains("<!DOCTYPE")
        || input.to_ascii_uppercase().contains("<!ENTITY")
    {
        return None;
    }
    let document = roxmltree::Document::parse(input).ok()?;
    let root = document.root_element();
    if root.tag_name().name() != "math"
        || root.tag_name().namespace() != Some("http://www.w3.org/1998/Math/MathML")
    {
        return None;
    }
    let alt = root.attribute("alttext").map(ToOwned::to_owned);
    let display = root.attribute("display") == Some("block");
    let mut output = String::new();
    let mut count = 0usize;
    serialize_kfx_mathml(root, 0, &mut count, &mut output).ok()?;
    Some((output, alt, display))
}

fn serialize_kfx_mathml(
    element: roxmltree::Node<'_, '_>,
    depth: usize,
    count: &mut usize,
    output: &mut String,
) -> Result<(), ()> {
    if depth > 64 || element.tag_name().namespace() != Some("http://www.w3.org/1998/Math/MathML") {
        return Err(());
    }
    *count = count.saturating_add(1);
    if *count > 4096 {
        return Err(());
    }
    let name = element.tag_name().name();
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err(());
    }
    output.push('<');
    output.push_str(name);
    if depth == 0 {
        output.push_str(" xmlns=\"http://www.w3.org/1998/Math/MathML\"");
    }
    for attribute in element.attributes() {
        let name = attribute.name();
        if matches!(
            name,
            "alttext"
                | "display"
                | "encoding"
                | "id"
                | "mathvariant"
                | "displaystyle"
                | "scriptlevel"
                | "stretchy"
                | "fence"
                | "separator"
                | "open"
                | "close"
                | "movablelimits"
                | "accent"
                | "rowalign"
                | "columnalign"
                | "columnspacing"
                | "rowspacing"
        ) {
            output.push(' ');
            output.push_str(name);
            output.push_str("=\"");
            output.push_str(&escape_kfx_xml(attribute.value()));
            output.push('"');
        }
    }
    let child_elements = element.children().filter(|child| child.is_element());
    let has_children = child_elements.clone().next().is_some();
    let text = element
        .children()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect::<String>();
    if !has_children && text.is_empty() {
        output.push_str("/>");
        return Ok(());
    }
    output.push('>');
    if !text.is_empty() {
        output.push_str(&escape_kfx_xml(&text));
    }
    for child in child_elements {
        serialize_kfx_mathml(child, depth + 1, count, output)?;
    }
    output.push_str("</");
    output.push_str(name);
    output.push('>');
    Ok(())
}

fn escape_kfx_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn resolve_string_reference<'a>(
    value: &IonValue,
    tables: &'a BTreeMap<u32, Vec<String>>,
) -> Option<&'a str> {
    let table_id = struct_get(value, FIELD_FRAGMENT_ID).and_then(|value| match value {
        IonValue::Symbol(id) => u32::try_from(*id).ok(),
        _ => None,
    })?;
    let index = struct_get(value, FIELD_FRAGMENT_STRING_INDEX)
        .and_then(as_int)
        .and_then(|value| usize::try_from(value).ok())?;
    tables.get(&table_id)?.get(index).map(String::as_str)
}

fn ruby_content_text<'a>(
    value: &'a IonValue,
    string_tables: &'a BTreeMap<u32, Vec<String>>,
) -> Option<&'a str> {
    let content = struct_get(value, FIELD_CONTENT_STRING_REFERENCE)
        .or_else(|| struct_get(value, FIELD_FRAGMENT_TEXT))?;
    as_string(content).or_else(|| resolve_string_reference(content, string_tables))
}

fn collect_ruby_contents(model: &NativeModel) -> RubyContentMap {
    let mut output = RubyContentMap::new();
    for fragment in model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == KFX_RUBY_CONTENT_FRAGMENT_TYPE)
    {
        let Some(entries) =
            struct_get(&fragment.value, FIELD_TEXT_VALUES).and_then(|value| match value {
                IonValue::List(items) | IonValue::SExp(items) => Some(items.as_slice()),
                _ => None,
            })
        else {
            continue;
        };

        let mut values = BTreeMap::new();
        for entry in entries {
            let Some(ruby_id) = struct_get(entry, FIELD_RUBY_ID)
                .and_then(as_int)
                .and_then(|value| u64::try_from(value).ok())
            else {
                continue;
            };
            let Some(text) = ruby_content_text(entry, &model.string_tables) else {
                continue;
            };
            values.insert(ruby_id, text.to_owned());
        }
        if values.is_empty() {
            continue;
        }

        let mut keys = BTreeSet::from([u64::from(fragment.entity_id)]);
        if let Some(IonValue::Symbol(symbol)) =
            struct_get(&fragment.value, FIELD_RUBY_FRAGMENT_SYMBOL)
        {
            keys.insert(*symbol);
        }
        for key in keys {
            output.insert((fragment.container_index, key), values.clone());
        }
    }
    output
}

fn ruby_text_for_id(
    context: &ContentDecodeContext<'_>,
    symbol: u64,
    ruby_id: i64,
) -> Option<String> {
    let ruby_id = u64::try_from(ruby_id).ok()?;
    context
        .ruby_contents
        .get(&(context.container_index, symbol))?
        .get(&ruby_id)
        .cloned()
}

fn text_style_range(item: &IonValue) -> Option<(usize, usize)> {
    let start =
        usize::try_from(struct_get(item, FIELD_TEXT_STYLE_OFFSET).and_then(as_int)?).ok()?;
    let length =
        usize::try_from(struct_get(item, FIELD_TEXT_STYLE_LENGTH).and_then(as_int)?).ok()?;
    let end = start.checked_add(length)?;
    (start < end).then_some((start, end))
}

fn ruby_range(item: &IonValue, text_char_count: usize) -> Option<(usize, usize)> {
    let (start, end) = text_style_range(item)?;
    (end <= text_char_count).then_some((start, end))
}

fn content_ruby_annotations(
    parent: &IonValue,
    context: &ContentDecodeContext<'_>,
    text_char_count: usize,
) -> Vec<KfxRubyAnnotation> {
    let Some(IonValue::List(items) | IonValue::SExp(items)) =
        struct_get(parent, FIELD_TEXT_STYLE_EVENTS)
    else {
        return Vec::new();
    };

    let mut annotations = Vec::new();
    for item in items {
        let symbol = struct_get(item, FIELD_RUBY_FRAGMENT_SYMBOL).and_then(|value| match value {
            IonValue::Symbol(symbol) => Some(*symbol),
            _ => None,
        });
        let Some(symbol) = symbol else {
            continue;
        };
        let default_range = ruby_range(item, text_char_count);
        if let Some(ruby_id) = struct_get(item, FIELD_RUBY_ID).and_then(as_int) {
            if let (Some((start, end)), Some(text)) =
                (default_range, ruby_text_for_id(context, symbol, ruby_id))
            {
                annotations.push(KfxRubyAnnotation {
                    text_offset: start,
                    text_length: end - start,
                    text,
                });
            }
        }

        let Some(IonValue::List(ranges) | IonValue::SExp(ranges)) =
            struct_get(item, FIELD_RUBY_ID_LIST)
        else {
            continue;
        };
        for range in ranges {
            let ruby_id = struct_get(range, FIELD_RUBY_ID)
                .and_then(as_int)
                .or_else(|| struct_get(item, FIELD_RUBY_ID).and_then(as_int));
            let Some(ruby_id) = ruby_id else {
                continue;
            };
            let range = ruby_range(range, text_char_count).or(default_range);
            if let (Some((start, end)), Some(text)) =
                (range, ruby_text_for_id(context, symbol, ruby_id))
            {
                annotations.push(KfxRubyAnnotation {
                    text_offset: start,
                    text_length: end - start,
                    text,
                });
            }
        }
    }
    annotations.sort_by_key(|annotation| (annotation.text_offset, annotation.text_length));
    annotations.dedup();
    annotations
}

fn content_text_in_traversal_order(segments: &[ContentSegment]) -> (String, usize) {
    let mut text = String::new();
    let mut text_segment_count = 0;
    for segment in segments {
        match segment {
            ContentSegment::Text(segment) => {
                text.push_str(&segment.text);
                text_segment_count += 1;
            }
            ContentSegment::Math { occurrence, .. } => {
                if let Some(alt) = occurrence.alt.as_deref() {
                    text.push_str(alt);
                }
                text_segment_count += 1;
            }
            ContentSegment::Image { .. } => {}
        }
    }
    (text, text_segment_count)
}

fn ordered_content_fragment_indices(model: &NativeModel) -> Option<Vec<usize>> {
    fn symbol_name(value: &IonValue, container: &NativeContainerOwned) -> Option<String> {
        match value {
            IonValue::Symbol(symbol) => container.symbols.names.get(symbol).cloned(),
            IonValue::String(name) => Some(name.clone()),
            _ => None,
        }
    }

    fn collect_story_names(
        value: &IonValue,
        container: &NativeContainerOwned,
        output: &mut Vec<String>,
    ) {
        match value {
            IonValue::Struct(fields) => {
                for (field, child) in fields {
                    if *field == FIELD_CONTENT_FRAGMENT_SYMBOL {
                        if let Some(name) = symbol_name(child, container) {
                            if !output.contains(&name) {
                                output.push(name);
                            }
                        }
                    } else {
                        collect_story_names(child, container, output);
                    }
                }
            }
            IonValue::List(items) | IonValue::SExp(items) => {
                for item in items {
                    collect_story_names(item, container, output);
                }
            }
            IonValue::Annotation { value, .. } => collect_story_names(value, container, output),
            _ => {}
        }
    }

    let reading_order = model
        .fragments
        .iter()
        .find(|fragment| fragment.fragment_type == 538)
        .or_else(|| {
            model
                .fragments
                .iter()
                .find(|fragment| fragment.fragment_type == 258)
        })?;
    let reading_order_container = model.containers.get(reading_order.container_index)?;
    let reading_orders = match struct_get(&reading_order.value, FIELD_READING_ORDERS)? {
        IonValue::List(reading_orders) => reading_orders,
        _ => return None,
    };
    let mut section_names = Vec::new();
    for reading_order in reading_orders {
        let Some(IonValue::List(sections)) =
            struct_get(reading_order, FIELD_READING_ORDER_SECTIONS)
        else {
            continue;
        };
        for section in sections {
            if let Some(name) = symbol_name(section, reading_order_container) {
                if !section_names.contains(&name) {
                    section_names.push(name);
                }
            }
        }
    }

    let mut output = Vec::new();
    let mut visited_stories = BTreeSet::new();
    for section_name in section_names {
        let sections = model
            .fragments
            .iter()
            .enumerate()
            .filter(|(_, fragment)| {
                fragment.fragment_type == 260
                    && resolved_fragment_fid(model, fragment).as_deref()
                        == Some(section_name.as_str())
            })
            .collect::<Vec<_>>();
        if sections.len() != 1 {
            continue;
        }
        let (_, section) = sections[0];
        let Some(container) = model.containers.get(section.container_index) else {
            continue;
        };
        let mut story_names = Vec::new();
        collect_story_names(&section.value, container, &mut story_names);
        for story_name in story_names {
            if !visited_stories.insert(story_name.clone()) {
                continue;
            }
            let stories = model
                .fragments
                .iter()
                .enumerate()
                .filter(|(_, fragment)| {
                    fragment.fragment_type == 259
                        && resolved_fragment_fid(model, fragment).as_deref()
                            == Some(story_name.as_str())
                })
                .collect::<Vec<_>>();
            if stories.len() == 1 {
                output.push(stories[0].0);
            }
        }
    }
    Some(output)
}

fn ordered_content_trace_locations(model: &NativeModel) -> BTreeMap<usize, ContentTraceLocation> {
    fn symbol_name(value: &IonValue, container: &NativeContainerOwned) -> Option<String> {
        match value {
            IonValue::Symbol(symbol) => container.symbols.names.get(symbol).cloned(),
            IonValue::String(name) => Some(name.clone()),
            _ => None,
        }
    }

    fn collect_story_names(
        value: &IonValue,
        container: &NativeContainerOwned,
        output: &mut Vec<String>,
    ) {
        match value {
            IonValue::Struct(fields) => {
                for (field, child) in fields {
                    if *field == FIELD_CONTENT_FRAGMENT_SYMBOL {
                        if let Some(name) = symbol_name(child, container) {
                            if !output.contains(&name) {
                                output.push(name);
                            }
                        }
                    } else {
                        collect_story_names(child, container, output);
                    }
                }
            }
            IonValue::List(items) | IonValue::SExp(items) => {
                for item in items {
                    collect_story_names(item, container, output);
                }
            }
            IonValue::Annotation { value, .. } => collect_story_names(value, container, output),
            _ => {}
        }
    }

    let Some(reading_order) = model
        .fragments
        .iter()
        .find(|fragment| fragment.fragment_type == 538)
        .or_else(|| {
            model
                .fragments
                .iter()
                .find(|fragment| fragment.fragment_type == 258)
        })
    else {
        return BTreeMap::new();
    };
    let Some(reading_order_container) = model.containers.get(reading_order.container_index) else {
        return BTreeMap::new();
    };
    let Some(IonValue::List(reading_orders)) =
        struct_get(&reading_order.value, FIELD_READING_ORDERS)
    else {
        return BTreeMap::new();
    };

    let mut section_names = Vec::new();
    for reading_order in reading_orders {
        let Some(IonValue::List(sections)) =
            struct_get(reading_order, FIELD_READING_ORDER_SECTIONS)
        else {
            continue;
        };
        for section in sections {
            if let Some(name) = symbol_name(section, reading_order_container) {
                if !section_names.contains(&name) {
                    section_names.push(name);
                }
            }
        }
    }

    let mut locations = BTreeMap::new();
    let mut visited_stories = BTreeSet::new();
    let mut story_aliases = BTreeMap::<String, String>::new();
    for (section_order, section_name) in section_names.into_iter().enumerate() {
        let sections = model
            .fragments
            .iter()
            .enumerate()
            .filter(|(_, fragment)| {
                fragment.fragment_type == 260
                    && resolved_fragment_fid(model, fragment).as_deref()
                        == Some(section_name.as_str())
            })
            .collect::<Vec<_>>();
        if sections.len() != 1 {
            continue;
        }
        let (_, section) = sections[0];
        let Some(container) = model.containers.get(section.container_index) else {
            continue;
        };
        let mut names = Vec::new();
        collect_story_names(&section.value, container, &mut names);
        for story_name in names {
            if !visited_stories.insert(story_name.clone()) {
                continue;
            }
            let stories = model
                .fragments
                .iter()
                .enumerate()
                .filter(|(_, fragment)| {
                    fragment.fragment_type == 259
                        && resolved_fragment_fid(model, fragment).as_deref()
                            == Some(story_name.as_str())
                })
                .collect::<Vec<_>>();
            if stories.len() != 1 {
                continue;
            }
            let story_alias = format!("ST{:03}", story_aliases.len() + 1);
            story_aliases.insert(story_name, story_alias.clone());
            locations.insert(
                stories[0].0,
                ContentTraceLocation {
                    section_id: Some(format!("S{:03}", section_order + 1)),
                    story_id: Some(story_alias),
                },
            );
        }
    }
    locations
}

fn count_field_occurrences(value: &IonValue, field_id: u64) -> usize {
    match value {
        IonValue::Struct(fields) => fields
            .iter()
            .map(|(field, child)| {
                usize::from(*field == field_id) + count_field_occurrences(child, field_id)
            })
            .sum(),
        IonValue::List(items) | IonValue::SExp(items) => items
            .iter()
            .map(|child| count_field_occurrences(child, field_id))
            .sum(),
        IonValue::Annotation { value, .. } => count_field_occurrences(value, field_id),
        _ => 0,
    }
}

fn contains_linked_note_display(value: &IonValue) -> bool {
    match value {
        IonValue::Struct(fields) => {
            if fields.iter().any(|(field, child)| {
                *field == FIELD_TEXT_STYLE_EVENTS
                    && matches!(child, IonValue::List(_) | IonValue::SExp(_))
                    && matches!(child, IonValue::List(items) | IonValue::SExp(items) if items.iter().any(|event| {
                        struct_get(event, FIELD_TEXT_STYLE_KIND)
                            .and_then(ion_symbol_id)
                            == Some(617)
                            && struct_get(event, FIELD_TEXT_STYLE_LINK_TARGET)
                                .and_then(ion_symbol_id)
                                .is_some()
                    }))
            }) {
                return true;
            }
            fields
                .iter()
                .any(|(_, child)| contains_linked_note_display(child))
        }
        IonValue::List(items) | IonValue::SExp(items) => {
            items.iter().any(contains_linked_note_display)
        }
        IonValue::Annotation { value, .. } => contains_linked_note_display(value),
        _ => false,
    }
}

fn contains_illustrated_image_structure(value: &IonValue) -> bool {
    match value {
        IonValue::Struct(fields) => {
            let node_type = struct_get(value, FIELD_CONTENT_NODE_TYPE).and_then(ion_symbol_id);
            if node_type == Some(KFX_IMAGE_CONTAINER_NODE_TYPE)
                && contains_illustrated_caption_structure(value)
                && count_field_occurrences(value, FIELD_CONTENT_ORACLE_RESOURCE_SYMBOL) > 0
            {
                return true;
            }
            fields
                .iter()
                .any(|(_, child)| contains_illustrated_image_structure(child))
        }
        IonValue::List(items) | IonValue::SExp(items) => {
            items.iter().any(contains_illustrated_image_structure)
        }
        IonValue::Annotation { value, .. } => contains_illustrated_image_structure(value),
        _ => false,
    }
}

/// A node type pair alone is not enough to call a KFX block illustrated.  The
/// same container/caption shape is also used by image labels that are
/// deliberately kept out of the reading stream.  Require the caption branch
/// to carry visible text and a style event, matching the production decoder's
/// scoped-caption rule, before reporting an illustrated candidate.
fn contains_illustrated_caption_structure(value: &IonValue) -> bool {
    match value {
        IonValue::Struct(fields) => {
            let node_type = struct_get(value, FIELD_CONTENT_NODE_TYPE).and_then(ion_symbol_id);
            if node_type == Some(KFX_IMAGE_CAPTION_NODE_TYPE)
                && has_non_empty_text_style_events(value)
                && contains_caption_reading_text(value)
            {
                return true;
            }
            fields
                .iter()
                .any(|(_, child)| contains_illustrated_caption_structure(child))
        }
        IonValue::List(items) | IonValue::SExp(items) => {
            items.iter().any(contains_illustrated_caption_structure)
        }
        IonValue::Annotation { value, .. } => contains_illustrated_caption_structure(value),
        _ => false,
    }
}

fn has_non_empty_text_style_events(value: &IonValue) -> bool {
    matches!(
        struct_get(value, FIELD_TEXT_STYLE_EVENTS),
        Some(IonValue::List(items) | IonValue::SExp(items)) if !items.is_empty()
    )
}

fn contains_caption_reading_text(value: &IonValue) -> bool {
    match value {
        IonValue::Struct(fields) => {
            // Image-record `$584` is alternative text, not the visible
            // caption.  Do not let it satisfy the caption detector.
            if struct_get(value, FIELD_CONTENT_NODE_TYPE).and_then(ion_symbol_id)
                == Some(KFX_IMAGE_RECORD_NODE_TYPE)
            {
                return false;
            }
            fields.iter().any(|(field, child)| match *field {
                FIELD_CONTENT_STRING_REFERENCE => !matches!(child, IonValue::Null),
                FIELD_FRAGMENT_TEXT => as_string(child).is_some_and(|text| !text.trim().is_empty()),
                _ => contains_caption_reading_text(child),
            })
        }
        IonValue::List(items) | IonValue::SExp(items) => {
            items.iter().any(contains_caption_reading_text)
        }
        IonValue::Annotation { value, .. } => contains_caption_reading_text(value),
        IonValue::String(text) => !text.trim().is_empty(),
        _ => false,
    }
}

fn book_has_semantic_role(book: &Book, role: SemanticRole) -> bool {
    fn visit(nodes: &[Node], role: SemanticRole) -> bool {
        nodes
            .iter()
            .any(|node| node.role == role || visit(&node.children, role))
    }

    book.documents
        .iter()
        .any(|document| visit(&document.nodes, role))
}

fn decode_semantic(
    model: &mut NativeModel,
    mode: ParseMode,
    resource_resolution: &NativeResourcePathResolution,
    mut placement_capture: Option<&mut PlacementCapture>,
) -> Result<Book, AmazonKfxError> {
    if model.drm_detected {
        return Err(AmazonKfxError::ProtectedContent);
    }
    let (resources, loader, resource_ids_by_path) =
        materialize_ir_resources(model, resource_resolution);
    let (image_identities, _, _) =
        placement_identity_index(model, resource_resolution, &resource_ids_by_path);
    let mut book = Book::new().with_resource_loader(std::sync::Arc::new(loader));
    book.metadata = model.metadata.clone();
    book.resources = resources;
    let default_style = book.styles.intern(ComputedStyle::default());
    let mut documents = Vec::new();
    let mut anchors = Vec::new();
    let mut navigation_targets = BTreeSet::new();
    collect_navigation_targets(&model.navigation, &mut navigation_targets);
    let mut navigation_labels = BTreeMap::new();
    collect_navigation_labels(&model.navigation, &mut navigation_labels);
    let mut navigation_heading_levels = BTreeMap::new();
    collect_navigation_heading_levels(&model.navigation, &mut navigation_heading_levels);
    let navigation_section_symbols = collect_navigation_section_symbols(model, &navigation_targets);
    // Unlike navigation, hyperlinks may target an ordinary KFX content
    // position. Keep this set separate so we can establish exact locations
    // for `$266.$183.$155` without turning every source position into an EPUB
    // anchor.
    let link_target_positions = model
        .link_targets
        .values()
        .filter_map(|target| match target {
            KfxLinkTarget::Position(position) => Some(*position),
            KfxLinkTarget::Uri(_) => None,
        })
        .collect::<BTreeSet<_>>();
    let mut position_locations = BTreeMap::<u32, Option<TargetLocation>>::new();
    let mut content_symbol_locations = BTreeMap::<u64, Option<TargetLocation>>::new();
    let mut entity_locations = BTreeMap::<u32, Option<TargetLocation>>::new();
    let mut document_index = 0u32;
    let mut next_node_id = 0u32;
    let mut inferred_image_count = 0usize;
    let mut unresolved_link_target_count = 0usize;
    let mut projected_link_target_count = 0usize;
    let ruby_contents = collect_ruby_contents(model);
    let text_event_trace_enabled = placement_capture
        .as_ref()
        .is_some_and(|capture| capture.text_events.is_some());
    let trace_locations = if text_event_trace_enabled {
        ordered_content_trace_locations(model)
    } else {
        BTreeMap::new()
    };
    let content_fragment_indices = ordered_content_fragment_indices(model).unwrap_or_else(|| {
        model
            .fragments
            .iter()
            .enumerate()
            .filter_map(|(index, fragment)| (fragment.fragment_type == 259).then_some(index))
            .collect()
    });
    for (content_fragment_order, fragment_index) in content_fragment_indices.into_iter().enumerate()
    {
        let fragment = &model.fragments[fragment_index];
        // `$259` is the reflowable document/content fragment used by the
        // samples. Metadata, navigation, style, string-pool, and resource
        // fragments remain native inputs to their own decoders and must not
        // become phantom documents.
        if fragment.fragment_type != 259 {
            continue;
        }
        let content_resource_symbols = resolve_content_resource_symbols(
            model,
            resource_resolution,
            &resource_ids_by_path,
            fragment.container_index,
        );
        let image_resource_symbols =
            resolve_content_image_symbols(model, &image_identities, fragment.container_index);
        let image_record_symbol =
            model
                .containers
                .get(fragment.container_index)
                .and_then(|container| {
                    observed_whitespace_resource_record_symbol_for_imports_content(
                        &container.symbols.imports,
                    )
                });
        let style_names = model
            .containers
            .get(fragment.container_index)
            .map(|container| &container.symbols.names);
        let content_context = ContentDecodeContext {
            container_index: fragment.container_index,
            string_tables: &model.string_tables,
            ruby_contents: &ruby_contents,
            resource_symbols: &content_resource_symbols,
            image_resource_symbols: &image_resource_symbols,
            style_names,
            image_record_symbol,
            navigation_heading_levels: Some(&navigation_heading_levels),
            link_targets: &model.link_targets,
        };
        let mut unresolved_text_references = 0usize;
        let mut content_segments = Vec::new();
        let mut link_target_segment_indices = BTreeMap::new();
        let trace_location = trace_locations.get(&fragment_index);
        let trace_source = text_event_trace_enabled.then(|| ContentTraceSource {
            section_id: trace_location.and_then(|location| location.section_id.clone()),
            story_id: trace_location.and_then(|location| location.story_id.clone()),
            source_fragment: format!("F{:03}", content_fragment_order + 1),
            source_fragment_order: content_fragment_order,
            eid: fragment.entity_id,
        });
        {
            let mut source_path = Vec::new();
            let mut heading_target_pending = false;
            let event_capture = placement_capture
                .as_deref_mut()
                .and_then(|capture| capture.text_events.as_mut());
            let mut event_trace = event_capture.map(|capture| ContentTextTraceState {
                capture,
                source: trace_source.as_ref(),
                source_path: &mut source_path,
            });
            collect_content_segments_traced(
                &fragment.value,
                &content_context,
                None,
                None,
                None,
                None,
                false,
                false,
                false,
                false,
                None,
                None,
                &[],
                None,
                &mut heading_target_pending,
                &mut content_segments,
                &mut unresolved_text_references,
                &mut inferred_image_count,
                &link_target_positions,
                &mut link_target_segment_indices,
                &mut event_trace,
            );
        }
        if let Some(event_capture) = placement_capture
            .as_deref_mut()
            .and_then(|capture| capture.text_events.as_mut())
        {
            event_capture.unresolved_text_reference_count += unresolved_text_references;
        }
        if let Some(capture) = placement_capture.as_deref_mut() {
            capture
                .native
                .extend(content_segments.iter().filter_map(|segment| match segment {
                    ContentSegment::Image {
                        position, resource, ..
                    } => Some(CapturedImagePlacement {
                        position_target_id: *position,
                        resource: *resource,
                    }),
                    ContentSegment::Math { .. } => None,
                    ContentSegment::Text(_) => None,
                }));
            if let Some(text_trace) = capture.text.as_mut() {
                let (text, text_segment_count) = content_text_in_traversal_order(&content_segments);
                text_trace.source_fragments.push(CapturedTextUnit {
                    content_fragment_order,
                    document_order: None,
                    text_segment_count,
                    text,
                });
            }
        }
        let mut fragment_navigation_positions = BTreeSet::new();
        collect_navigation_positions(
            &fragment.value,
            &navigation_targets,
            &mut fragment_navigation_positions,
        );
        if unresolved_text_references > 0 {
            if let Some(text_trace) = placement_capture
                .as_deref_mut()
                .and_then(|capture| capture.text.as_mut())
            {
                text_trace.unresolved_text_reference_count += unresolved_text_references;
            }
            model
                .unknown_features
                .insert("kfx-text-reference-unresolved".to_owned());
            model.input_loss.push(format!(
                "{unresolved_text_references} KFX text references did not resolve in their declared string table."
            ));
        }
        let content_symbol = struct_get(&fragment.value, FIELD_CONTENT_FRAGMENT_SYMBOL).and_then(
            |value| match value {
                IonValue::Symbol(symbol) => Some(*symbol),
                _ => None,
            },
        );
        let has_readable_content = content_segments.iter().any(|segment| match segment {
            ContentSegment::Text(segment) => !segment.text.trim().is_empty(),
            ContentSegment::Math { .. } => true,
            ContentSegment::Image { .. } => true,
        });
        if !has_readable_content {
            let referenced_by_section =
                content_symbol.is_some_and(|symbol| navigation_section_symbols.contains(&symbol));
            if !referenced_by_section && fragment_navigation_positions.is_empty() {
                continue;
            }

            // Preserve an explicitly referenced but textless content fragment
            // as an empty section. This keeps KFX's navigation target valid
            // without inventing body text or redirecting it to a neighbor.
            let document_id = DocumentId::new(document_index);
            let document_href = format!("document-{document_index}.xhtml");
            let node_id = allocate_node_id(&mut next_node_id)?;
            let section = Node::new(node_id, NodeKind::Section, default_style, Vec::new())
                .with_semantics(
                    SemanticRole::Section,
                    Default::default(),
                    Confidence::StronglyInferred,
                );
            documents.push(Document {
                id: document_id,
                href: document_href.clone(),
                media_type: "application/xhtml+xml".to_owned(),
                title: None,
                nodes: vec![section],
            });
            if let Some(text_trace) = placement_capture
                .as_deref_mut()
                .and_then(|capture| capture.text.as_mut())
            {
                text_trace.semantic_documents.push(CapturedTextUnit {
                    content_fragment_order,
                    document_order: Some(document_index as usize),
                    text_segment_count: 0,
                    text: String::new(),
                });
            }
            // Entity IDs are scoped to a CONT container. The document index is
            // unique across the merged input and prevents duplicate IR anchors.
            let fragment_anchor = format!("fragment-{document_index}-{}", fragment.entity_id);
            append_anchor(&mut anchors, document_id, node_id, fragment_anchor.clone())?;
            let fragment_location = TargetLocation {
                document: document_id,
                node: node_id,
                href: format!("{document_href}#{fragment_anchor}"),
                anchor_name: fragment_anchor,
            };
            insert_target_location(
                &mut entity_locations,
                fragment.entity_id,
                fragment_location.clone(),
            );
            if let Some(content_symbol) = content_symbol {
                insert_target_location(
                    &mut content_symbol_locations,
                    content_symbol,
                    fragment_location.clone(),
                );
            }
            for position in fragment_navigation_positions {
                insert_target_location(
                    &mut position_locations,
                    position,
                    fragment_location.clone(),
                );
            }
            document_index = document_index.checked_add(1).ok_or_else(|| {
                AmazonKfxError::Semantic(
                    "KFX produced more documents than the IR can identify".to_owned(),
                )
            })?;
            continue;
        }

        let mut link_target_positions_by_segment = BTreeMap::<usize, Vec<u32>>::new();
        for (position, segment_index) in link_target_segment_indices {
            if segment_index < content_segments.len() {
                link_target_positions_by_segment
                    .entry(segment_index)
                    .or_default()
                    .push(position);
                projected_link_target_count = projected_link_target_count.saturating_add(1);
            } else {
                mark_target_location_ambiguous(&mut position_locations, position);
            }
        }
        let link_target_positions_anchored_by_segment = link_target_positions_by_segment
            .values()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();

        if let Some(capture) = placement_capture.as_deref_mut() {
            capture
                .semantic
                .extend(content_segments.iter().filter_map(|segment| match segment {
                    ContentSegment::Image {
                        position, resource, ..
                    } => Some(CapturedImagePlacement {
                        position_target_id: *position,
                        resource: *resource,
                    }),
                    ContentSegment::Math { .. } => None,
                    ContentSegment::Text(_) => None,
                }));
        }

        let mut position_occurrences = BTreeMap::<u32, usize>::new();
        for position in content_segments.iter().filter_map(|segment| match segment {
            ContentSegment::Text(segment) => kfx_navigation_position(segment, &navigation_targets)
                .or(segment.navigation_heading_position),
            ContentSegment::Math { position, .. } => *position,
            ContentSegment::Image { position, .. } => *position,
        }) {
            *position_occurrences.entry(position).or_default() += 1;
        }

        let document_id = DocumentId::new(document_index);
        let document_href = format!("document-{document_index}.xhtml");
        let mut nodes: Vec<Node> = Vec::new();
        let mut pending_inline_images = Vec::new();
        let mut pending_inline_math = Vec::new();
        let mut pending_inline_math_before_position = None;
        let mut last_text_group_position = None;
        let mut first_node_id = None;
        let mut document_title = None;
        let mut kfx_style_ids = BTreeMap::<String, StyleId>::new();
        let mut semantic_text = String::new();
        let mut semantic_text_segment_count = 0;
        let is_toc_document = content_segments
            .iter()
            .find_map(|segment| match segment {
                ContentSegment::Text(segment) => Some(segment.text.trim()),
                ContentSegment::Math { .. } => None,
                ContentSegment::Image { .. } => None,
            })
            .is_some_and(|text| text == "目录" || text.eq_ignore_ascii_case("table of contents"));
        let mut repeated_link_target_text_counts = BTreeMap::<u32, usize>::new();
        for position in content_segments.iter().filter_map(|segment| match segment {
            ContentSegment::Text(segment) => segment
                .position
                .filter(|position| link_target_positions.contains(position)),
            ContentSegment::Math { .. } => None,
            ContentSegment::Image { .. } => None,
        }) {
            *repeated_link_target_text_counts
                .entry(position)
                .or_default() += 1;
        }
        let mut link_target_text_offsets = BTreeMap::<u32, usize>::new();
        for (segment_index, segment) in content_segments.into_iter().enumerate() {
            let structural_link_target_positions = link_target_positions_by_segment
                .remove(&segment_index)
                .unwrap_or_default();
            let localized_link_ranges = match &segment {
                ContentSegment::Text(segment) => {
                    if let Some(position) = segment
                        .position
                        .filter(|position| link_target_positions.contains(position))
                    {
                        if repeated_link_target_text_counts.get(&position) == Some(&1) {
                            segment.link_ranges.clone()
                        } else {
                            let segment_offset = link_target_text_offsets
                                .get(&position)
                                .copied()
                                .unwrap_or_default();
                            let localized = localize_kfx_link_ranges(
                                &segment.link_ranges,
                                segment_offset,
                                segment.text.chars().count(),
                            );
                            link_target_text_offsets.insert(
                                position,
                                advance_kfx_nested_text_offset(
                                    segment_offset,
                                    segment.text.chars().count(),
                                ),
                            );
                            localized
                        }
                    } else {
                        segment.link_ranges.clone()
                    }
                }
                ContentSegment::Math { .. } => Vec::new(),
                ContentSegment::Image { .. } => Vec::new(),
            };
            match segment {
                ContentSegment::Text(segment) => {
                    semantic_text.push_str(&segment.text);
                    semantic_text_segment_count += 1;
                    let text_group_position = segment.parent_position.or(segment.position);
                    let link_target_position = segment.position.filter(|position| {
                        link_target_positions.contains(position)
                            && !link_target_positions_anchored_by_segment.contains(position)
                    });
                    let navigation_position =
                        kfx_navigation_position(&segment, &navigation_targets)
                            .or(segment.navigation_heading_position);
                    let uniquely_targeted = navigation_position
                        .is_some_and(|position| position_occurrences.get(&position) == Some(&1));
                    if let Some(position) = navigation_position.filter(|_| !uniquely_targeted) {
                        mark_target_location_ambiguous(&mut position_locations, position);
                    }
                    let heading_level = kfx_heading_level(
                        segment.style_name.as_deref(),
                        &segment.text,
                        segment.source_heading_level,
                        navigation_position
                            .and_then(|position| navigation_heading_levels.get(&position).copied()),
                        navigation_position.is_some(),
                        is_toc_document,
                        kfx_navigation_label_matches(
                            &segment.text,
                            navigation_position,
                            &navigation_labels,
                        ),
                    );
                    let style_id = kfx_style_id(
                        &mut book,
                        &mut kfx_style_ids,
                        segment.style_name.as_deref(),
                        Some(&segment.text),
                        heading_level.is_some(),
                    );
                    let (node_kind, role, confidence) = if let Some(level) = heading_level {
                        (
                            NodeKind::Heading { level },
                            SemanticRole::Heading,
                            Confidence::Heuristic,
                        )
                    } else {
                        (
                            NodeKind::Paragraph,
                            SemanticRole::Paragraph,
                            Confidence::Heuristic,
                        )
                    };
                    let node_id = allocate_node_id(&mut next_node_id)?;
                    let text = segment.text;
                    let text_children = kfx_text_children(
                        &mut book,
                        &mut next_node_id,
                        &text,
                        &segment.style_events,
                        &localized_link_ranges,
                        segment.link_target_symbol_id,
                        fragment.container_index,
                        content_context.link_targets,
                        &segment.ruby_annotations,
                        default_style,
                        &mut unresolved_link_target_count,
                    )?;
                    let mut node = Node::new(node_id, node_kind, style_id, text_children)
                        .with_semantics(role, Default::default(), confidence);
                    let pending_figure_images = !pending_inline_images.is_empty()
                        && nodes.last().is_some_and(|previous| {
                            matches!(&previous.kind, NodeKind::Heading { .. })
                        });
                    if !pending_figure_images && !pending_inline_images.is_empty() {
                        let mut leading_images = std::mem::take(&mut pending_inline_images);
                        leading_images.append(&mut node.children);
                        node.children = leading_images;
                    }
                    if pending_inline_math_before_position.is_some()
                        && pending_inline_math_before_position == text_group_position
                        && nodes.last().is_some_and(|previous| {
                            matches!(
                                &previous.kind,
                                NodeKind::Paragraph | NodeKind::Heading { .. }
                            )
                        })
                    {
                        let mut preceding_math = std::mem::take(&mut pending_inline_math);
                        pending_inline_math_before_position = None;
                        if let Some(previous) = nodes.last_mut() {
                            previous.children.append(&mut preceding_math);
                        }
                    }
                    if !pending_inline_math.is_empty() {
                        let mut leading_math = std::mem::take(&mut pending_inline_math);
                        pending_inline_math_before_position = None;
                        leading_math.append(&mut node.children);
                        node.children = leading_math;
                    }
                    if document_title.is_none() && heading_level.is_some() {
                        let title = text.trim();
                        if !title.is_empty() {
                            document_title = Some(title.to_owned());
                        }
                    }
                    first_node_id.get_or_insert(node_id);
                    let merge_into_previous = !pending_figure_images
                        && text_group_position.is_some()
                        && last_text_group_position == text_group_position
                        && nodes.last().is_some_and(|previous| {
                            matches!(
                                &previous.kind,
                                NodeKind::Paragraph | NodeKind::Heading { .. }
                            )
                        });
                    let anchor_node_id = if pending_figure_images {
                        let mut figure_children = std::mem::take(&mut pending_inline_images);
                        figure_children.push(node);
                        let figure_id = allocate_node_id(&mut next_node_id)?;
                        let figure_style = book.styles.intern(
                            ComputedStyle::default()
                                .with("display", "block")
                                .with("margin", "1em 0")
                                .with("text-align", "center")
                                .with("break-inside", "avoid")
                                .with("page-break-inside", "avoid"),
                        );
                        let figure = Node::new(
                            figure_id,
                            NodeKind::GenericBlock {
                                tag: "figure".to_owned(),
                            },
                            figure_style,
                            figure_children,
                        )
                        .with_semantics(
                            SemanticRole::Figure,
                            PresentationIntent::default()
                                .with(PresentationFeature::KeepTogether)
                                .with(PresentationFeature::AvoidBreakInside),
                            Confidence::StronglyInferred,
                        );
                        nodes.push(figure);
                        figure_id
                    } else if merge_into_previous {
                        let previous = nodes.last_mut().expect("checked by merge predicate");
                        previous.children.append(&mut node.children);
                        previous.id
                    } else {
                        nodes.push(node);
                        node_id
                    };
                    last_text_group_position = text_group_position;
                    if let Some(position) = navigation_position {
                        let anchor_name = format!("kfx-position-{position}");
                        insert_target_location(
                            &mut position_locations,
                            position,
                            TargetLocation {
                                document: document_id,
                                node: anchor_node_id,
                                href: format!("{document_href}#{anchor_name}"),
                                anchor_name,
                            },
                        );
                    }
                    if let Some(position) = link_target_position {
                        let anchor_name = format!("kfx-position-{position}");
                        insert_target_location(
                            &mut position_locations,
                            position,
                            TargetLocation {
                                document: document_id,
                                node: anchor_node_id,
                                href: format!("{document_href}#{anchor_name}"),
                                anchor_name,
                            },
                        );
                    }
                    for position in structural_link_target_positions {
                        let anchor_name = format!("kfx-position-{position}");
                        insert_target_location(
                            &mut position_locations,
                            position,
                            TargetLocation {
                                document: document_id,
                                node: anchor_node_id,
                                href: format!("{document_href}#{anchor_name}"),
                                anchor_name,
                            },
                        );
                    }
                }
                ContentSegment::Math {
                    position,
                    parent_position,
                    occurrence,
                    style_name,
                } => {
                    if let Some(alt) = occurrence.alt.as_deref() {
                        semantic_text.push_str(alt);
                    }
                    semantic_text_segment_count += 1;
                    let link_target_position = position.filter(|position| {
                        link_target_positions.contains(position)
                            && !link_target_positions_anchored_by_segment.contains(position)
                    });
                    let navigation_position =
                        position.filter(|position| navigation_targets.contains(position));
                    let uniquely_targeted = navigation_position
                        .is_some_and(|position| position_occurrences.get(&position) == Some(&1));
                    if let Some(position) = navigation_position.filter(|_| !uniquely_targeted) {
                        mark_target_location_ambiguous(&mut position_locations, position);
                    }
                    let style_id = kfx_style_id(
                        &mut book,
                        &mut kfx_style_ids,
                        style_name.as_deref(),
                        occurrence.alt.as_deref(),
                        false,
                    );
                    let node_id = allocate_node_id(&mut next_node_id)?;
                    let math_node = Node::new(
                        node_id,
                        NodeKind::Math {
                            mathml: occurrence.mathml,
                            alt: occurrence.alt,
                            display: occurrence.display,
                        },
                        style_id,
                        Vec::new(),
                    )
                    .with_semantics(
                        SemanticRole::Math,
                        Default::default(),
                        Confidence::Explicit,
                    );
                    first_node_id.get_or_insert(node_id);
                    let math_anchor_node_id = if occurrence.display {
                        nodes.append(&mut pending_inline_images);
                        nodes.append(&mut pending_inline_math);
                        pending_inline_math_before_position = None;
                        last_text_group_position = None;
                        nodes.push(math_node);
                        node_id
                    } else if let Some(previous) = nodes.last_mut().filter(|node| {
                        matches!(&node.kind, NodeKind::Paragraph | NodeKind::Heading { .. })
                            && parent_position.is_some()
                            && last_text_group_position == parent_position
                    }) {
                        previous.children.push(math_node);
                        previous.id
                    } else {
                        pending_inline_math_before_position = last_text_group_position;
                        pending_inline_math.push(math_node);
                        node_id
                    };
                    if let Some(position) = navigation_position {
                        let anchor_name = format!("kfx-position-{position}");
                        insert_target_location(
                            &mut position_locations,
                            position,
                            TargetLocation {
                                document: document_id,
                                node: math_anchor_node_id,
                                href: format!("{document_href}#{anchor_name}"),
                                anchor_name,
                            },
                        );
                    }
                    if let Some(position) = link_target_position {
                        let anchor_name = format!("kfx-position-{position}");
                        insert_target_location(
                            &mut position_locations,
                            position,
                            TargetLocation {
                                document: document_id,
                                node: math_anchor_node_id,
                                href: format!("{document_href}#{anchor_name}"),
                                anchor_name,
                            },
                        );
                    }
                    for position in structural_link_target_positions {
                        let anchor_name = format!("kfx-position-{position}");
                        insert_target_location(
                            &mut position_locations,
                            position,
                            TargetLocation {
                                document: document_id,
                                node: math_anchor_node_id,
                                href: format!("{document_href}#{anchor_name}"),
                                anchor_name,
                            },
                        );
                    }
                }
                ContentSegment::Image {
                    position,
                    resource,
                    alt,
                    style_name,
                    inline,
                    link_target_symbol_id,
                } => {
                    let link_target_position = position.filter(|position| {
                        link_target_positions.contains(position)
                            && !link_target_positions_anchored_by_segment.contains(position)
                    });
                    let navigation_position =
                        position.filter(|position| navigation_targets.contains(position));
                    let uniquely_targeted = navigation_position
                        .is_some_and(|position| position_occurrences.get(&position) == Some(&1));
                    if let Some(position) = navigation_position.filter(|_| !uniquely_targeted) {
                        mark_target_location_ambiguous(&mut position_locations, position);
                    }
                    let flow_inline = inline
                        && nodes
                            .last()
                            .is_some_and(|node| matches!(&node.kind, NodeKind::Paragraph));
                    let style_id = if flow_inline {
                        book.styles.intern(
                            kfx_computed_style(
                                style_name.as_deref().unwrap_or_default(),
                                "",
                                false,
                            )
                            .with("display", "inline")
                            .with("margin", "0")
                            .with("vertical-align", "middle"),
                        )
                    } else {
                        kfx_style_id(
                            &mut book,
                            &mut kfx_style_ids,
                            style_name.as_deref(),
                            None,
                            false,
                        )
                    };
                    let node_id = allocate_node_id(&mut next_node_id)?;
                    let image_node = Node::new(
                        node_id,
                        NodeKind::Image { resource, alt },
                        style_id,
                        Vec::new(),
                    )
                    .with_semantics(
                        SemanticRole::Image,
                        Default::default(),
                        Confidence::Heuristic,
                    );
                    let image_node = if let Some(target_symbol_id) = link_target_symbol_id {
                        if let Some(href) = kfx_link_target_href(
                            fragment.container_index,
                            target_symbol_id,
                            content_context.link_targets,
                            &mut unresolved_link_target_count,
                        ) {
                            let link_id = allocate_node_id(&mut next_node_id)?;
                            Node::new(link_id, NodeKind::Link { href }, style_id, vec![image_node])
                                .with_semantics(
                                    SemanticRole::Link,
                                    Default::default(),
                                    Confidence::Explicit,
                                )
                        } else {
                            image_node
                        }
                    } else {
                        image_node
                    };
                    first_node_id.get_or_insert(node_id);
                    if flow_inline {
                        if let Some(previous) = nodes.last_mut() {
                            if let Some(image_node) =
                                merge_adjacent_kfx_linked_image(&mut previous.children, image_node)
                            {
                                previous.children.push(image_node);
                            }
                        }
                    } else if inline {
                        if let Some(image_node) =
                            merge_adjacent_kfx_linked_image(&mut pending_inline_images, image_node)
                        {
                            pending_inline_images.push(image_node);
                        }
                    } else {
                        nodes.append(&mut pending_inline_images);
                        if let Some(image_node) =
                            merge_adjacent_kfx_linked_image(&mut nodes, image_node)
                        {
                            nodes.push(image_node);
                        }
                    }
                    if let Some(capture) = placement_capture.as_deref_mut() {
                        capture.ir.push(CapturedImagePlacement {
                            position_target_id: position,
                            resource,
                        });
                    }
                    if uniquely_targeted {
                        if let Some(position) = navigation_position {
                            let anchor_name = format!("kfx-position-{position}");
                            insert_target_location(
                                &mut position_locations,
                                position,
                                TargetLocation {
                                    document: document_id,
                                    node: node_id,
                                    href: format!("{document_href}#{anchor_name}"),
                                    anchor_name,
                                },
                            );
                        }
                    }
                    if let Some(position) = link_target_position {
                        let anchor_name = format!("kfx-position-{position}");
                        insert_target_location(
                            &mut position_locations,
                            position,
                            TargetLocation {
                                document: document_id,
                                node: node_id,
                                href: format!("{document_href}#{anchor_name}"),
                                anchor_name,
                            },
                        );
                    }
                    for position in structural_link_target_positions {
                        let anchor_name = format!("kfx-position-{position}");
                        insert_target_location(
                            &mut position_locations,
                            position,
                            TargetLocation {
                                document: document_id,
                                node: node_id,
                                href: format!("{document_href}#{anchor_name}"),
                                anchor_name,
                            },
                        );
                    }
                }
            }
        }
        nodes.append(&mut pending_inline_images);
        nodes.append(&mut pending_inline_math);
        documents.push(Document {
            id: document_id,
            href: document_href.clone(),
            media_type: "application/xhtml+xml".to_owned(),
            title: document_title,
            nodes,
        });
        if let Some(text_trace) = placement_capture
            .as_deref_mut()
            .and_then(|capture| capture.text.as_mut())
        {
            text_trace.semantic_documents.push(CapturedTextUnit {
                content_fragment_order,
                document_order: Some(document_index as usize),
                text_segment_count: semantic_text_segment_count,
                text: semantic_text,
            });
        }
        // Entity IDs are scoped to a CONT container. The document index is
        // unique across the merged input and prevents duplicate IR anchors.
        let fragment_anchor = format!("fragment-{document_index}-{}", fragment.entity_id);
        let fragment_node = first_node_id.ok_or_else(|| {
            AmazonKfxError::Semantic(
                "KFX content fragment produced readable content without an IR node".to_owned(),
            )
        })?;
        append_anchor(
            &mut anchors,
            document_id,
            fragment_node,
            fragment_anchor.clone(),
        )?;
        let fragment_location = TargetLocation {
            document: document_id,
            node: fragment_node,
            href: format!("{document_href}#{fragment_anchor}"),
            anchor_name: fragment_anchor,
        };
        insert_target_location(
            &mut entity_locations,
            fragment.entity_id,
            fragment_location.clone(),
        );
        for position in fragment_navigation_positions {
            if position_occurrences.get(&position).copied().unwrap_or(0) == 0 {
                // Some KFX positions belong to structural content nodes rather
                // than a text run. Resolve those only to the containing
                // fragment; never pretend that a precise text offset exists.
                insert_target_location(
                    &mut position_locations,
                    position,
                    fragment_location.clone(),
                );
            }
        }
        if let Some(symbol) = content_symbol {
            insert_target_location(&mut content_symbol_locations, symbol, fragment_location);
        }

        document_index = document_index.checked_add(1).ok_or_else(|| {
            AmazonKfxError::Semantic(
                "KFX produced more documents than the IR can identify".to_owned(),
            )
        })?;
    }
    // KFX hyperlinks are declared separately from the content that uses them:
    // a `$179` value names a `$266` anchor, and `$183.$155` points at a KFX
    // position. Resolve that indirection only after every content fragment
    // has established its document/node location. This is the same ordering
    // used by the Calibre KFX Input reference implementation, and avoids
    // emitting implementation-only `kfx-position:` URIs into EPUB.
    let footnote_target_nodes = collect_kfx_footnote_target_nodes(&documents, &position_locations);
    resolve_kfx_link_nodes(
        &mut documents,
        &position_locations,
        &mut unresolved_link_target_count,
    );
    mark_kfx_footnote_target_nodes(&mut documents, &footnote_target_nodes);
    let mut link_edges = Vec::new();
    collect_kfx_link_edges(&documents, &mut link_edges);
    if inferred_image_count > 0 {
        model
            .unknown_features
            .insert("kfx-image-placeholder-placement-inferred".to_owned());
        model.input_loss.push(format!(
            "{inferred_image_count} KFX raster image placements were inferred from whitespace resource records; source alternative text may be unavailable."
        ));
    }
    if unresolved_link_target_count > 0 {
        model
            .unknown_features
            .insert("kfx-link-target-unresolved".to_owned());
        model.input_loss.push(format!(
            "{unresolved_link_target_count} KFX link targets did not resolve to a declared `$266` anchor."
        ));
    }
    if projected_link_target_count > 0 {
        model
            .unknown_features
            .insert("kfx-link-target-parent-anchored-to-first-descendant".to_owned());
        model.input_loss.push(format!(
            "{projected_link_target_count} KFX link target container positions were projected to their first readable descendant because the IR has no generic container anchor node."
        ));
    }
    let mut section_locations = BTreeMap::<u32, Option<TargetLocation>>::new();
    for fragment in model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 260)
    {
        let Some(IonValue::List(entries)) = struct_get(&fragment.value, FIELD_SECTION_ENTRIES)
        else {
            continue;
        };
        for entry in entries {
            let Some(position) = struct_get(entry, FIELD_NAV_TARGET_ID)
                .and_then(as_int)
                .and_then(|value| u32::try_from(value).ok())
            else {
                continue;
            };
            let Some(IonValue::Symbol(symbol)) = struct_get(entry, FIELD_CONTENT_FRAGMENT_SYMBOL)
            else {
                continue;
            };
            match content_symbol_locations.get(symbol) {
                Some(Some(location)) => {
                    insert_target_location(&mut section_locations, position, location.clone())
                }
                Some(None) => mark_target_location_ambiguous(&mut section_locations, position),
                None => {}
            }
        }
    }
    if documents.is_empty() {
        let message = "KFX fragments were parsed but no readable text was recovered".to_owned();
        if mode == ParseMode::Strict {
            return Err(AmazonKfxError::Semantic(message));
        }
        model.input_loss.push(message);
        documents.push(Document {
            id: DocumentId::new(0),
            href: "book.xhtml".to_owned(),
            media_type: "application/xhtml+xml".to_owned(),
            title: None,
            nodes: Vec::new(),
        });
    }
    let mut anchor_names = anchors
        .iter()
        .map(|anchor| (anchor.document, anchor.name.clone()))
        .collect::<BTreeSet<_>>();
    for location in position_locations.values().flatten() {
        if anchor_names.insert((location.document, location.anchor_name.clone())) {
            append_anchor(
                &mut anchors,
                location.document,
                location.node,
                location.anchor_name.clone(),
            )?;
        }
    }
    let mut unresolved_navigation = 0usize;
    let navigation = filter_kfx_navigation(resolve_navigation(
        &model.navigation,
        &position_locations,
        &section_locations,
        &entity_locations,
        &mut unresolved_navigation,
    ));
    if unresolved_navigation > 0 {
        model
            .unknown_features
            .insert("kfx-navigation-target-unresolved".to_owned());
        model.input_loss.push(format!(
            "{unresolved_navigation} KFX navigation targets did not resolve to a readable content position."
        ));
    }
    infer_kfx_cover_resource(&mut book.resources, &documents, model);
    book.documents = documents;
    book.anchors = anchors;
    book.navigation.toc = navigation;
    book.navigation.anchor_graph.edges.extend(link_edges);
    Ok(book)
}

/// Replace the temporary href used while decoding a KFX `$179` reference
/// with the final EPUB document/fragment URI. A positional target can be
/// ambiguous or absent when the source contains an unsupported layout branch;
/// in that case flatten the link but keep its reading text rather than
/// exporting an invalid URI.
fn resolve_kfx_link_nodes(
    documents: &mut [Document],
    position_locations: &BTreeMap<u32, Option<TargetLocation>>,
    unresolved: &mut usize,
) {
    fn resolve_nodes(
        nodes: Vec<Node>,
        position_locations: &BTreeMap<u32, Option<TargetLocation>>,
        unresolved: &mut usize,
    ) -> Vec<Node> {
        nodes
            .into_iter()
            .flat_map(|mut node| {
                node.children = resolve_nodes(
                    std::mem::take(&mut node.children),
                    position_locations,
                    unresolved,
                );
                let unresolved_position = if let NodeKind::Link { href }
                | NodeKind::Footnote { href: Some(href) } =
                    &mut node.kind
                {
                    if let Some(raw_position) = href.strip_prefix("kfx-position:") {
                        let position = raw_position.parse::<u32>().ok();
                        match position
                            .and_then(|position| position_locations.get(&position))
                            .and_then(|location| location.as_ref())
                        {
                            Some(location) => {
                                *href = location.href.clone();
                                false
                            }
                            None => true,
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };
                if unresolved_position {
                    *unresolved = unresolved.saturating_add(1);
                    node.children
                } else {
                    vec![node]
                }
            })
            .collect()
    }

    for document in documents {
        document.nodes = resolve_nodes(
            std::mem::take(&mut document.nodes),
            position_locations,
            unresolved,
        );
    }
}

fn collect_kfx_footnote_target_nodes(
    documents: &[Document],
    position_locations: &BTreeMap<u32, Option<TargetLocation>>,
) -> BTreeMap<(DocumentId, NodeId), KfxNote> {
    fn visit(
        nodes: &[Node],
        position_locations: &BTreeMap<u32, Option<TargetLocation>>,
        output: &mut BTreeMap<(DocumentId, NodeId), KfxNote>,
    ) {
        for node in nodes {
            if let NodeKind::Footnote { href: Some(href) } = &node.kind {
                if let Some(position) = href
                    .strip_prefix("kfx-position:")
                    .and_then(|value| value.parse::<u32>().ok())
                {
                    if let Some(Some(location)) = position_locations.get(&position) {
                        output
                            .entry((location.document, location.node))
                            .or_insert_with(|| KfxNote {
                                kind: KfxNoteKind::Footnote,
                                target: location.clone(),
                            });
                    }
                }
            }
            visit(&node.children, position_locations, output);
        }
    }

    let mut output = BTreeMap::new();
    for document in documents {
        visit(&document.nodes, position_locations, &mut output);
    }
    output
}

/// Turn only proven footnote targets into EPUB note bodies. The target set is
/// derived from a linked `$616 = 617` marker, never from numbering or text
/// similarity, so ordinary internal links remain ordinary paragraphs.
fn mark_kfx_footnote_target_nodes(
    documents: &mut [Document],
    target_nodes: &BTreeMap<(DocumentId, NodeId), KfxNote>,
) {
    fn visit(
        nodes: &mut [Node],
        document: DocumentId,
        target_nodes: &BTreeMap<(DocumentId, NodeId), KfxNote>,
    ) {
        for node in nodes {
            if let Some(note) = target_nodes.get(&(document, node.id)) {
                node.kind = NodeKind::GenericBlock {
                    tag: "aside".to_owned(),
                };
                node.role = match note.kind {
                    KfxNoteKind::Footnote => SemanticRole::Footnote,
                    KfxNoteKind::Endnote => SemanticRole::Endnote,
                };
                node.confidence = Confidence::Explicit;
            }
            visit(&mut node.children, document, target_nodes);
        }
    }

    for document in documents {
        visit(&mut document.nodes, document.id, target_nodes);
    }
}

fn collect_kfx_link_edges(documents: &[Document], output: &mut Vec<AnchorEdge>) {
    fn visit(nodes: &[Node], output: &mut Vec<AnchorEdge>) {
        for node in nodes {
            let (href, relation) = match &node.kind {
                NodeKind::Link { href } => (Some(href), AnchorRelation::Link),
                NodeKind::Footnote { href: Some(href) } => (Some(href), AnchorRelation::Footnote),
                _ => (None, AnchorRelation::Link),
            };
            if let Some(href) = href {
                output.push(AnchorEdge {
                    source: format!("kfx-node:{}", node.id.get()),
                    target: href.clone(),
                    relation,
                });
            }
            visit(&node.children, output);
        }
    }

    for document in documents {
        visit(&document.nodes, output);
    }
}

/// KFX image-only linked blocks can contain a run of adjacent image records.
/// Keep that run under one IR link, matching the source/Calibre shape, while
/// leaving non-adjacent occurrences as separate links.
fn merge_adjacent_kfx_linked_image(nodes: &mut [Node], mut image_node: Node) -> Option<Node> {
    let image_href = match &image_node.kind {
        NodeKind::Link { href } => href.clone(),
        _ => return Some(image_node),
    };
    let Some(last) = nodes.last_mut() else {
        return Some(image_node);
    };
    let NodeKind::Link { href: last_href } = &last.kind else {
        return Some(image_node);
    };
    if *last_href != image_href {
        return Some(image_node);
    }
    let Some(image) = image_node.children.pop() else {
        return Some(image_node);
    };
    last.children.push(image);
    None
}

fn optional_ion_range(
    bytes: &[u8],
    base: usize,
    fields: &[(u64, IonValue)],
    offset_id: u64,
    length_id: u64,
    mode: ParseMode,
) -> Result<Option<IonValue>, AmazonKfxError> {
    let Some(offset) = fields
        .iter()
        .find_map(|(id, value)| (*id == offset_id).then(|| as_int(value)))
        .flatten()
    else {
        return Ok(None);
    };
    let Some(length) = fields
        .iter()
        .find_map(|(id, value)| (*id == length_id).then(|| as_int(value)))
        .flatten()
    else {
        return Ok(None);
    };
    let offset = usize::try_from(offset).map_err(|_| AmazonKfxError::Truncated(base))?;
    let length = usize::try_from(length).map_err(|_| AmazonKfxError::Truncated(base + offset))?;
    let end = checked_slice_end(base, offset, length, bytes.len())?;
    decode_one(bytes, base + offset + VERSION_MARKER.len(), end)
        .or_else(|error| recover_ion(bytes, base + offset, end, mode, error))
        .map(|(value, _)| Some(value))
}

fn recover_ion(
    bytes: &[u8],
    start: usize,
    end: usize,
    mode: ParseMode,
    error: IonError,
) -> Result<(IonValue, ion::IonSpan), AmazonKfxError> {
    if mode != ParseMode::Recovery {
        return Err(AmazonKfxError::Ion(error));
    }
    decode_one(bytes, start, end).map_err(AmazonKfxError::Ion)
}

fn recovery_actions(mode: ParseMode, model: &NativeModel) -> Vec<String> {
    if mode != ParseMode::Recovery {
        return Vec::new();
    }
    if model.input_loss.is_empty() {
        vec!["validated all bounded Ion/entity ranges without recovery".to_owned()]
    } else {
        vec!["retained readable fragments and reported unrecovered fields".to_owned()]
    }
}

fn checked_slice_end(
    base: usize,
    offset: usize,
    length: usize,
    total: usize,
) -> Result<usize, AmazonKfxError> {
    let start = base
        .checked_add(offset)
        .ok_or(AmazonKfxError::Truncated(base))?;
    let end = start
        .checked_add(length)
        .ok_or(AmazonKfxError::Truncated(start))?;
    if end > total {
        return Err(AmazonKfxError::Truncated(start));
    }
    Ok(end)
}

fn required_u64(fields: &[(u64, IonValue)], id: u64) -> Result<u64, AmazonKfxError> {
    fields
        .iter()
        .find_map(|(field, value)| {
            (*field == id).then(|| as_int(value).and_then(|value| u64::try_from(value).ok()))
        })
        .flatten()
        .ok_or_else(|| AmazonKfxError::InvalidHeader(format!("missing numeric field ${id}")))
}

fn struct_get_from_fields(fields: &[(u64, IonValue)], id: u64) -> Option<&IonValue> {
    fields
        .iter()
        .find_map(|(field, value)| (*field == id).then_some(value))
}
fn unwrap_struct(value: &IonValue) -> Option<&[(u64, IonValue)]> {
    match value {
        IonValue::Struct(fields) => Some(fields),
        IonValue::Annotation { value, .. } => unwrap_struct(value),
        _ => None,
    }
}
fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let value = bytes.get(offset..end)?;
    Some(u32::from_le_bytes(value.try_into().ok()?))
}
fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    let value = bytes.get(offset..end)?;
    Some(u64::from_le_bytes(value.try_into().ok()?))
}
fn entity_type_id(type_id: u32) -> u32 {
    type_id
}

fn raster_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
        return Some((
            u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?),
            u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?),
        ));
    }
    if (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) && bytes.len() >= 10 {
        return Some((
            u32::from(u16::from_le_bytes(bytes.get(6..8)?.try_into().ok()?)),
            u32::from(u16::from_le_bytes(bytes.get(8..10)?.try_into().ok()?)),
        ));
    }
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut offset = 2usize;
    while offset < bytes.len() {
        while bytes.get(offset) == Some(&0xff) {
            offset = offset.checked_add(1)?;
        }
        let marker = *bytes.get(offset)?;
        offset = offset.checked_add(1)?;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if matches!(marker, 0x01 | 0xd0..=0xd7) {
            continue;
        }
        let length_end = offset.checked_add(2)?;
        let segment_length = usize::from(u16::from_be_bytes(
            bytes.get(offset..length_end)?.try_into().ok()?,
        ));
        if segment_length < 2 {
            return None;
        }
        let segment_start = length_end;
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            return Some((
                u32::from(u16::from_be_bytes(
                    bytes
                        .get(segment_start + 3..segment_start + 5)?
                        .try_into()
                        .ok()?,
                )),
                u32::from(u16::from_be_bytes(
                    bytes
                        .get(segment_start + 1..segment_start + 3)?
                        .try_into()
                        .ok()?,
                )),
            ));
        }
        offset = offset.checked_add(segment_length)?;
    }
    None
}

fn media_type_for_bytes(bytes: &[u8]) -> (String, &'static str) {
    if bytes.starts_with(&[0xff, 0xd8]) {
        ("image/jpeg".to_owned(), "jpg")
    } else if bytes.starts_with(b"\x89PNG") {
        ("image/png".to_owned(), "png")
    } else if bytes.starts_with(b"GIF8") {
        ("image/gif".to_owned(), "gif")
    } else {
        ("application/octet-stream".to_owned(), "bin")
    }
}
fn resource_kind(media_type: &str) -> ResourceKind {
    match media_type {
        "image/jpeg" => ResourceKind::Jpeg,
        "image/png" => ResourceKind::Png,
        "image/gif" => ResourceKind::Gif,
        _ => ResourceKind::Unknown,
    }
}

#[cfg(all(test, feature = "maintainer-tests"))]
#[rustfmt::skip]
#[path = "../../../../tests/unit/crates/folio-kfx/src/amazon/mod.rs"]
mod tests;
