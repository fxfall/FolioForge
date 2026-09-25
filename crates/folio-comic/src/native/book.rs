use std::collections::{BTreeMap, BTreeSet};

use blake3::Hasher;
use folio_model::Metadata;

use crate::{
    ordering::natural_order_cmp, ComicModelError, ComicSourcePage, ComicSourcePageDraft,
    SourceLocation, SourcePageId,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceBookId([u8; 32]);

impl SourceBookId {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComicSourceKind {
    Directory,
    Archive,
    Pdf,
    FixedLayoutEpub,
    Other,
}

impl ComicSourceKind {
    fn stable_tag(self) -> &'static [u8] {
        match self {
            Self::Directory => b"directory",
            Self::Archive => b"archive",
            Self::Pdf => b"pdf",
            Self::FixedLayoutEpub => b"fixed-layout-epub",
            Self::Other => b"other",
        }
    }
}

/// Opaque identity of one source container. `stable_key` must be reproducible
/// for the same source after reopening and must not be a transient temp path.
/// Only its versioned digest is retained in the model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComicSourceIdentity {
    kind: ComicSourceKind,
    book_id: SourceBookId,
}

impl ComicSourceIdentity {
    pub fn new(kind: ComicSourceKind, stable_key: &str) -> Result<Self, ComicModelError> {
        if stable_key.trim().is_empty() || stable_key.chars().any(char::is_control) {
            return Err(ComicModelError::InvalidSourceIdentity);
        }
        let mut hasher = Hasher::new();
        hasher.update(b"FolioForge Comic Source Book v1\0");
        super::hash_part(&mut hasher, kind.stable_tag());
        super::hash_part(&mut hasher, stable_key.as_bytes());
        Ok(Self {
            kind,
            book_id: SourceBookId(*hasher.finalize().as_bytes()),
        })
    }

    pub const fn kind(self) -> ComicSourceKind {
        self.kind
    }

    pub const fn book_id(self) -> SourceBookId {
        self.book_id
    }

    pub fn page_id(
        self,
        location: &SourceLocation,
        discriminator: &str,
    ) -> Result<SourcePageId, ComicModelError> {
        if discriminator.chars().any(char::is_control) {
            return Err(ComicModelError::InvalidIdentityDiscriminator);
        }
        Ok(SourcePageId::derive(
            self.book_id.as_bytes(),
            location,
            discriminator,
        ))
    }
}

#[derive(Clone, Debug)]
pub enum PageOrdering {
    /// Sort member paths using `natural_order_cmp`; independent of discovery
    /// or archive enumeration order.
    Natural,
    /// Preserve a parser-proven source sequence (for example an EPUB spine).
    Explicit(Vec<SourcePageId>),
}

/// Immutable, target-independent source facts for a comic book.
#[derive(Clone, Debug)]
pub struct ComicNativeBook {
    id: SourceBookId,
    source_kind: ComicSourceKind,
    metadata: Metadata,
    pages: BTreeMap<SourcePageId, ComicSourcePage>,
    source_order: Vec<SourcePageId>,
    source_properties: BTreeMap<String, String>,
}

impl ComicNativeBook {
    pub fn new(
        identity: ComicSourceIdentity,
        metadata: Metadata,
        source_pages: Vec<ComicSourcePageDraft>,
        source_properties: BTreeMap<String, String>,
        ordering: PageOrdering,
    ) -> Result<Self, ComicModelError> {
        if source_pages.is_empty() {
            return Err(ComicModelError::EmptyBook);
        }
        if source_properties
            .keys()
            .any(|key| key.trim().is_empty() || key.chars().any(char::is_control))
        {
            return Err(ComicModelError::InvalidMetadataKey);
        }

        let mut pages = BTreeMap::new();
        let mut source_keys = BTreeSet::new();
        for draft in source_pages {
            let source_key = (draft.location.clone(), draft.discriminator.clone());
            if !source_keys.insert(source_key) {
                return Err(ComicModelError::DuplicateSourcePage(
                    draft.location.identity_key(),
                ));
            }
            let id = identity.page_id(&draft.location, &draft.discriminator)?;
            let page = ComicSourcePage {
                id: id.clone(),
                location: draft.location,
                identity_discriminator: draft.discriminator,
                encoded_size: draft.encoded_size,
                original_dimensions: draft.original_dimensions,
                source_format: draft.source_format,
                source_name: draft.source_name,
                explicit_metadata: draft.explicit_metadata,
            };
            if pages.insert(id, page).is_some() {
                return Err(ComicModelError::DuplicatePageIdentity);
            }
        }

        let source_order = match ordering {
            PageOrdering::Natural => {
                let mut ordered = pages.values().collect::<Vec<_>>();
                ordered.sort_by(|left, right| {
                    natural_order_cmp(
                        &left.location.ordering_key(),
                        &right.location.ordering_key(),
                    )
                    .then_with(|| {
                        natural_order_cmp(
                            &left.identity_discriminator,
                            &right.identity_discriminator,
                        )
                    })
                    .then_with(|| left.id.cmp(&right.id))
                });
                ordered.into_iter().map(|page| page.id.clone()).collect()
            }
            PageOrdering::Explicit(ids) => validate_explicit_order(&pages, ids)?,
        };

        Ok(Self {
            id: identity.book_id(),
            source_kind: identity.kind(),
            metadata,
            pages,
            source_order,
            source_properties,
        })
    }

    pub const fn id(&self) -> SourceBookId {
        self.id
    }

    pub const fn source_kind(&self) -> ComicSourceKind {
        self.source_kind
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn source_pages(&self) -> impl ExactSizeIterator<Item = &ComicSourcePage> {
        self.source_order.iter().map(|id| {
            self.pages
                .get(id)
                .expect("source order is validated at construction")
        })
    }

    pub fn source_page(&self, id: &SourcePageId) -> Option<&ComicSourcePage> {
        self.pages.get(id)
    }

    pub fn source_page_ids(&self) -> &[SourcePageId] {
        &self.source_order
    }

    pub fn source_properties(&self) -> &BTreeMap<String, String> {
        &self.source_properties
    }
}

fn validate_explicit_order(
    pages: &BTreeMap<SourcePageId, ComicSourcePage>,
    order: Vec<SourcePageId>,
) -> Result<Vec<SourcePageId>, ComicModelError> {
    if order.len() != pages.len() {
        return Err(ComicModelError::InvalidOrderLength {
            expected: pages.len(),
            actual: order.len(),
        });
    }
    let mut seen = BTreeSet::new();
    for id in &order {
        if !pages.contains_key(id) {
            return Err(ComicModelError::UnknownPageInOrder(id.clone()));
        }
        if !seen.insert(id.clone()) {
            return Err(ComicModelError::DuplicatePageInOrder(id.clone()));
        }
    }
    Ok(order)
}
