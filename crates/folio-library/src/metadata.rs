use folio_core::{BookEditPlan, InspectionReport, MetadataEdit};

use crate::model::{BookMetadata, FileMetadata};

pub const INSPECTION_CONTRACT_VERSION: &str = "folio-core-inspection-v1";

pub fn fingerprint(metadata: &BookMetadata) -> String {
    let authors = metadata.authors.clone();
    let mut tags = metadata.tags.clone();
    let mut identifiers = metadata.identifiers.clone();
    tags.sort();
    identifiers.sort_by(|left, right| {
        left.scheme
            .cmp(&right.scheme)
            .then_with(|| left.value.cmp(&right.value))
    });
    let canonical = serde_json::json!({
        "title": metadata.title,
        "subtitle": metadata.subtitle,
        "language": metadata.language,
        "publisher": metadata.publisher,
        "published_date": metadata.published_date,
        "description": metadata.description,
        "authors": authors,
        "series": metadata.series,
        "series_index": metadata.series_index,
        "tags": tags,
        "identifiers": identifiers,
    });
    blake3::hash(
        serde_json::to_string(&canonical)
            .unwrap_or_default()
            .as_bytes(),
    )
    .to_hex()
    .to_string()
}

pub fn file_metadata_from_inspection(
    inspection: &InspectionReport,
    inspected_at: i64,
) -> FileMetadata {
    let metadata = BookMetadata::from_core(&inspection.metadata);
    FileMetadata {
        title: metadata.title.clone(),
        subtitle: metadata.subtitle.clone(),
        language: metadata.language.clone(),
        publisher: metadata.publisher.clone(),
        published_date: metadata.published_date.clone(),
        metadata_fingerprint: fingerprint(&metadata),
        inspected_at,
        inspection_contract_version: INSPECTION_CONTRACT_VERSION.to_owned(),
    }
}

#[derive(Clone, Debug, Default)]
pub struct MetadataDiff {
    pub edit: BookEditPlan,
    pub changed_fields: Vec<String>,
    pub canonical_fingerprint: String,
    pub observed_fingerprint: String,
}

impl MetadataDiff {
    pub fn is_empty(&self) -> bool {
        self.changed_fields.is_empty()
    }
}

pub fn metadata_diff(canonical: &BookMetadata, observed: &BookMetadata) -> MetadataDiff {
    let mut edit = BookEditPlan::default();
    let mut changed_fields = Vec::new();

    optional_field(
        &mut edit.metadata,
        &mut changed_fields,
        "title",
        canonical.title.as_ref(),
        observed.title.as_ref(),
        |value, target| target.title = Some(value.to_owned()),
    );
    optional_field(
        &mut edit.metadata,
        &mut changed_fields,
        "subtitle",
        canonical.subtitle.as_ref(),
        observed.subtitle.as_ref(),
        |value, target| target.subtitle = Some(value.to_owned()),
    );
    optional_field(
        &mut edit.metadata,
        &mut changed_fields,
        "language",
        canonical.language.as_ref(),
        observed.language.as_ref(),
        |value, target| target.language = Some(value.to_owned()),
    );
    optional_field(
        &mut edit.metadata,
        &mut changed_fields,
        "publisher",
        canonical.publisher.as_ref(),
        observed.publisher.as_ref(),
        |value, target| target.publisher = Some(value.to_owned()),
    );
    optional_field(
        &mut edit.metadata,
        &mut changed_fields,
        "date",
        canonical.published_date.as_ref(),
        observed.published_date.as_ref(),
        |value, target| target.date = Some(value.to_owned()),
    );
    optional_field(
        &mut edit.metadata,
        &mut changed_fields,
        "description",
        canonical.description.as_ref(),
        observed.description.as_ref(),
        |value, target| target.description = Some(value.to_owned()),
    );

    if canonical.authors != observed.authors {
        edit.metadata.authors = Some(canonical.authors.clone());
        changed_fields.push("authors".to_owned());
    }
    if canonical.series != observed.series {
        match &canonical.series {
            Some(value) => edit.metadata.series = Some(value.clone()),
            None => {
                edit.metadata.clear_fields.insert("series".to_owned());
            }
        }
        changed_fields.push("series".to_owned());
    }
    if canonical.series_index != observed.series_index {
        match canonical.series_index {
            Some(value) => edit.metadata.series_index = Some(value),
            None => {
                edit.metadata.clear_fields.insert("series_index".to_owned());
            }
        }
        changed_fields.push("series_index".to_owned());
    }
    if canonical.tags != observed.tags {
        edit.metadata.subjects = Some(canonical.tags.clone());
        changed_fields.push("tags".to_owned());
    }
    if canonical.identifiers != observed.identifiers {
        edit.metadata.identifiers = Some(
            canonical
                .identifiers
                .iter()
                .map(|value| value.value.clone())
                .collect(),
        );
        changed_fields.push("identifiers".to_owned());
    }

    MetadataDiff {
        edit,
        changed_fields,
        canonical_fingerprint: fingerprint(canonical),
        observed_fingerprint: fingerprint(observed),
    }
}

fn optional_field<F>(
    edit: &mut MetadataEdit,
    changed_fields: &mut Vec<String>,
    name: &str,
    canonical: Option<&String>,
    observed: Option<&String>,
    set: F,
) where
    F: FnOnce(&str, &mut MetadataEdit),
{
    if canonical == observed {
        return;
    }
    if let Some(value) = canonical {
        set(value, edit);
    } else {
        edit.clear_fields.insert(name.to_owned());
    }
    changed_fields.push(name.to_owned());
}
