use std::collections::{btree_map::Entry, BTreeMap};

use super::super::*;
use super::classify::classify_resource_shape;
use super::diagnostics::KfxResourceDiagnostic;
use crate::amazon::fragment::{FragmentGraph, FragmentKey};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UnresolvedReason {
    MissingLocation,
    MissingRawMedia,
    DuplicateRawMedia,
    UnsupportedCompositeResource,
    UnsupportedResourceKind,
    InvalidReference,
}

impl UnresolvedReason {
    pub(crate) fn diagnostic_code(self) -> &'static str {
        match self {
            Self::MissingLocation | Self::InvalidReference => "KFX-R001",
            Self::MissingRawMedia => "KFX-R002",
            Self::DuplicateRawMedia => "KFX-R003",
            Self::UnsupportedCompositeResource => "KFX-R004",
            Self::UnsupportedResourceKind => "KFX-R005",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UnresolvedResource {
    pub(crate) external_resource_id: u32,
    pub(crate) location: Option<String>,
    pub(crate) reason: UnresolvedReason,
    pub(crate) body_reference_count: usize,
    pub(crate) visible_placement_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ResourceBinding {
    Exact(usize),
    Unresolved(UnresolvedResource),
}

impl ResourceBinding {
    pub(crate) fn is_exact(&self) -> bool {
        matches!(self, Self::Exact(_))
    }

    fn unresolved_reason(&self) -> Option<UnresolvedReason> {
        match self {
            Self::Exact(_) => None,
            Self::Unresolved(unresolved) => Some(unresolved.reason),
        }
    }

    pub(crate) fn unresolved_diagnostic_code(&self) -> Option<&'static str> {
        match self {
            Self::Exact(_) => None,
            Self::Unresolved(unresolved) => Some(unresolved.reason.diagnostic_code()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResourceBindingRecord {
    pub(crate) container_origin: usize,
    pub(crate) external_resource_entity_id: u32,
    pub(crate) location: Option<String>,
    pub(crate) binding: ResourceBinding,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct NativeResourcePathResolution {
    pub(crate) by_path: BTreeMap<String, Option<usize>>,
    pub(crate) by_location: BTreeMap<String, Option<usize>>,
    pub(crate) metadata_records: usize,
    pub(crate) resolved_records: usize,
    pub(crate) unresolved_records: usize,
    pub(crate) bindings: Vec<ResourceBindingRecord>,
    pub(crate) diagnostics: Vec<KfxResourceDiagnostic>,
}

pub(crate) fn normalize_kfx_resource_path(path: &str) -> Option<String> {
    if path.is_empty()
        || path.contains('\\')
        || path.starts_with('/')
        || path
            .chars()
            .any(|character| character.is_control() || matches!(character, ':' | '?' | '#'))
    {
        return None;
    }
    let components = path.split('/').collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| component.is_empty() || matches!(*component, "." | ".."))
    {
        return None;
    }
    Some(path.to_owned())
}

pub(crate) fn validate_resource_resolution(
    mode: ParseMode,
    diagnostics: &[KfxResourceDiagnostic],
) -> Result<(), AmazonKfxError> {
    if mode != ParseMode::Strict {
        return Ok(());
    }
    let mut placements_by_location = BTreeMap::<String, usize>::new();
    for diagnostic in diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.visible_placement_count > 0)
    {
        let key = diagnostic
            .normalized_location
            .as_deref()
            .or(diagnostic.location.as_deref())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "missing:{}:{}",
                    diagnostic.container_origin, diagnostic.external_resource_entity_id
                )
            });
        placements_by_location
            .entry(key)
            .and_modify(|count| *count = (*count).max(diagnostic.visible_placement_count))
            .or_insert(diagnostic.visible_placement_count);
    }
    let count = placements_by_location.values().sum::<usize>();
    if count > 0 {
        return Err(AmazonKfxError::UnresolvedVisibleResources(count));
    }
    Ok(())
}

pub(crate) fn resolve_native_resource_paths(model: &NativeModel) -> NativeResourcePathResolution {
    let fragment_graph = FragmentGraph::from_native_model(model);

    let reference_counts = count_resource_references(model);
    let placement_counts = count_observed_resource_placements(model);
    let mut resolution = NativeResourcePathResolution::default();
    for fragment in model
        .fragments
        .iter()
        .filter(|fragment| fragment.fragment_type == 164)
    {
        resolution.metadata_records += 1;
        let (location, location_symbol_id, location_source) = resource_location(model, fragment);
        let normalized_path = location.as_deref().and_then(normalize_kfx_resource_path);
        let matches = location.as_ref().map(|fid| {
            fragment_graph.find_exact(&FragmentKey {
                fragment_type: 417,
                fid: fid.clone(),
            })
        });
        let resource_shape = classify_resource_shape(
            audit_string_or_symbol(
                struct_get(&fragment.value, FIELD_RESOURCE_FORMAT),
                model.containers.get(fragment.container_index),
            )
            .as_deref(),
            audit_string_or_symbol(
                struct_get(&fragment.value, FIELD_RESOURCE_MIME),
                model.containers.get(fragment.container_index),
            )
            .as_deref(),
            struct_get(&fragment.value, FIELD_RESOURCE_TILE_GRID).is_some(),
            struct_get(&fragment.value, FIELD_RESOURCE_OVERLAPPED_TILES).is_some(),
        );
        let composite = struct_get(&fragment.value, FIELD_RESOURCE_TILE_GRID).is_some()
            || struct_get(&fragment.value, FIELD_RESOURCE_OVERLAPPED_TILES).is_some();
        let reference_count = location
            .as_ref()
            .and_then(|location| reference_counts.get(location))
            .copied()
            .unwrap_or_default();
        let visible_placement_count = location
            .as_ref()
            .and_then(|location| placement_counts.get(location))
            .copied()
            .unwrap_or_default();

        let unresolved_reason = if location.is_none() {
            Some(UnresolvedReason::MissingLocation)
        } else if normalized_path.is_none() {
            Some(UnresolvedReason::InvalidReference)
        } else if composite {
            Some(UnresolvedReason::UnsupportedCompositeResource)
        } else if matches.is_none_or(|matches| matches.is_empty()) {
            Some(UnresolvedReason::MissingRawMedia)
        } else if matches.is_some_and(|matches| matches.len() > 1) {
            Some(UnresolvedReason::DuplicateRawMedia)
        } else {
            let index = matches.and_then(|matches| matches.first()).copied();
            let native = index.and_then(|index| model.resources.get(index));
            let unsupported_kind = matches!(resource_shape.as_str(), "PDF" | "Vector")
                || (visible_placement_count > 0
                    && native.is_some_and(|resource| {
                        !matches!(
                            resource.media_type.as_str(),
                            "image/jpeg" | "image/png" | "image/gif"
                        )
                    }));
            unsupported_kind.then_some(UnresolvedReason::UnsupportedResourceKind)
        };
        let candidate = if unresolved_reason.is_none() {
            matches.and_then(|matches| matches.first()).copied()
        } else {
            None
        };
        let binding = if let Some(index) = candidate {
            ResourceBinding::Exact(index)
        } else {
            ResourceBinding::Unresolved(UnresolvedResource {
                external_resource_id: fragment.entity_id,
                location: location.clone(),
                reason: unresolved_reason.unwrap_or(UnresolvedReason::InvalidReference),
                body_reference_count: reference_count,
                visible_placement_count,
            })
        };
        resolution.bindings.push(ResourceBindingRecord {
            container_origin: fragment.container_index,
            external_resource_entity_id: fragment.entity_id,
            location: location.clone(),
            binding: binding.clone(),
        });

        if let Some(location) = location.as_ref() {
            match resolution.by_location.entry(location.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(candidate);
                }
                Entry::Occupied(mut entry) => {
                    if entry.get().is_none() || candidate.is_none() || *entry.get() != candidate {
                        entry.insert(None);
                    }
                }
            }
            if let Some(path) = normalized_path.as_ref() {
                match resolution.by_path.entry(path.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(candidate);
                    }
                    Entry::Occupied(mut entry) => {
                        if entry.get().is_none() || candidate.is_none() || *entry.get() != candidate
                        {
                            entry.insert(None);
                        }
                    }
                }
            } else {
                resolution.by_location.insert(location.clone(), None);
            }
        }

        if binding.is_exact() {
            resolution.resolved_records += 1;
        } else {
            resolution.unresolved_records += 1;
        }

        if let Some(reason) = binding.unresolved_reason() {
            let code = reason.diagnostic_code();
            let container = model.containers.get(fragment.container_index);
            let message = match reason {
                UnresolvedReason::MissingLocation => format!(
                    "External resource entity {} in container {} has no `$165` location; body references: {reference_count}; visible placements: {visible_placement_count}.",
                    fragment.entity_id, fragment.container_index
                ),
                UnresolvedReason::InvalidReference => format!(
                    "External resource entity {} has invalid `$165` location {:?}; body references: {reference_count}; visible placements: {visible_placement_count}.",
                    fragment.entity_id, location
                ),
                UnresolvedReason::MissingRawMedia => format!(
                    "External resource has location {:?}, but no exact `$417` raw media exists in the aggregated FileSet; body references: {reference_count}; visible placements: {visible_placement_count}.",
                    location
                ),
                UnresolvedReason::DuplicateRawMedia => format!(
                    "External resource has location {:?}, but multiple exact `$417` raw media fragments share that identity; refusing to select one; body references: {reference_count}; visible placements: {visible_placement_count}.",
                    location
                ),
                UnresolvedReason::UnsupportedCompositeResource => format!(
                    "External resource at {:?} contains `$636`/`$797` composite tile data; no evidence-backed assembly rule is enabled; body references: {reference_count}; visible placements: {visible_placement_count}.",
                    location
                ),
                UnresolvedReason::UnsupportedResourceKind => format!(
                    "External resource at {:?} has unsupported kind `{resource_shape}` for semantic placement; body references: {reference_count}; visible placements: {visible_placement_count}.",
                    location
                ),
            };
            resolution.diagnostics.push(KfxResourceDiagnostic {
                code: code.to_owned(),
                message,
                location: location.clone(),
                location_symbol_id,
                location_source,
                normalized_location: normalized_path,
                location_field_id: FIELD_RESOURCE_METADATA_PATH,
                input_index: container
                    .map(|container| container.input_index)
                    .unwrap_or_default(),
                container_origin: fragment.container_index,
                external_resource_entity_id: fragment.entity_id,
                body_reference_count: reference_count,
                visible_placement_count,
            });
        }
    }
    resolution
}
