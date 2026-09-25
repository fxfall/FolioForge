//! Format-neutral, transient reading-session foundation.
//!
//! This crate consumes Folio IR only. It does not import source formats,
//! apply edits or target compatibility, export books, or persist reading
//! state. The 0.2.1 migration gates add behavior incrementally; Core owns
//! target compatibility projection and Reader owns generic display rendering.

mod reflowable;
pub use reflowable::{render_reflowable_documents, ReflowableDocumentRenderModel};
mod preview;
pub use preview::{
    render_preview, ReaderPreviewContent, ReaderPreviewMode, ReaderPreviewRenderModel,
    ReaderPreviewRequest, ReaderPreviewTargetContext, ReaderPreviewViewport,
};

use std::{collections::HashMap, io::Cursor, ops::Range, sync::Arc};

use folio_model::{
    Book, DocumentId, LayoutMode, NavPoint, NodeId, NodeKind, PageSide, ResourceId,
    ResourceLoadError, SemanticRole, SpreadHint,
};
use image::ImageReader;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A source-format-independent location in the semantic reading model.
///
/// For fixed-layout books, `document_id` identifies the page. EPUB CFI, KFX
/// positions, DOM paths, and filesystem paths are deliberately not Reader
/// identity. Source provenance may be added separately if a caller needs it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReaderLocation {
    pub document_id: DocumentId,
    pub node_id: Option<NodeId>,
    pub text_offset: Option<u64>,
    pub semantic_context: Option<SemanticRole>,
}

impl ReaderLocation {
    pub const fn document(document_id: DocumentId) -> Self {
        Self {
            document_id,
            node_id: None,
            text_offset: None,
            semantic_context: None,
        }
    }
}

/// The transient display mode used by a Reader viewport.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderContentMode {
    Fit,
    Fill,
    ActualSize,
}

/// Generic navigation direction. The Reader never infers it from a source
/// format or reverses the source IR itself.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderDirection {
    LeftToRight,
    RightToLeft,
}

impl ReaderDirection {
    fn from_book(book: &Book) -> Self {
        match book.presentation.direction.as_deref().map(str::trim) {
            Some(value)
                if value.eq_ignore_ascii_case("rtl")
                    || value.eq_ignore_ascii_case("right-to-left") =>
            {
                Self::RightToLeft
            }
            _ => Self::LeftToRight,
        }
    }
}

/// The generic navigation collection a Reader target belongs to.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderNavigationSection {
    Toc,
    Landmark,
    PageList,
}

/// Stable, source-format-independent reference to an entry in immutable IR
/// navigation. The path is a sequence of child indices within its section;
/// it never contains a source URL, CFI, PID, or filesystem path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReaderNavigationTarget {
    pub section: ReaderNavigationSection,
    pub entry_path: Vec<usize>,
}

/// Why an IR navigation entry could not be proven to resolve to a Reader
/// location. Labels remain visible to callers; unresolved destinations are
/// never guessed from similar strings or neighboring documents.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderNavigationUnresolvedReason {
    EmptyTarget,
    ExternalTarget,
    MissingDocumentContext,
    DocumentNotFound,
    AmbiguousDocument,
    AnchorNotFound,
    AmbiguousAnchor,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReaderNavigationEntry {
    pub label: String,
    pub target: Option<ReaderNavigationTarget>,
    pub location: Option<ReaderLocation>,
    pub unresolved_reason: Option<ReaderNavigationUnresolvedReason>,
    pub children: Vec<ReaderNavigationEntry>,
}

/// Reader-owned semantic navigation tree from the immutable Folio IR.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReaderNavigationSnapshot {
    pub toc: Vec<ReaderNavigationEntry>,
    pub landmarks: Vec<ReaderNavigationEntry>,
    pub page_list: Vec<ReaderNavigationEntry>,
}

/// Display dimensions and zoom scale for one transient Reader session.
///
/// This is presentation state, not Folio IR or persistent user state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReaderViewport {
    width: f32,
    height: f32,
    scale: f32,
    content_mode: ReaderContentMode,
    pan_x: f32,
    pan_y: f32,
}

impl ReaderViewport {
    pub fn new(
        width: f32,
        height: f32,
        scale: f32,
        content_mode: ReaderContentMode,
    ) -> Result<Self, ReaderError> {
        if !width.is_finite()
            || !height.is_finite()
            || !scale.is_finite()
            || width <= 0.0
            || height <= 0.0
            || scale <= 0.0
        {
            return Err(ReaderError::new(ReaderErrorCode::InvalidViewport));
        }
        Ok(Self {
            width,
            height,
            scale,
            content_mode,
            pan_x: 0.0,
            pan_y: 0.0,
        })
    }

    pub fn with_pan(mut self, x: f32, y: f32) -> Result<Self, ReaderError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(ReaderError::new(ReaderErrorCode::InvalidViewport));
        }
        self.pan_x = x;
        self.pan_y = y;
        Ok(self)
    }

    pub const fn width(self) -> f32 {
        self.width
    }

    pub const fn height(self) -> f32 {
        self.height
    }

    pub const fn scale(self) -> f32 {
        self.scale
    }

    pub const fn content_mode(self) -> ReaderContentMode {
        self.content_mode
    }

    pub const fn pan_x(self) -> f32 {
        self.pan_x
    }

    pub const fn pan_y(self) -> f32 {
        self.pan_y
    }
}

/// A format-neutral two-dimensional size in Reader display points.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReaderSize {
    pub width: f32,
    pub height: f32,
}

/// A display rectangle measured from the top-left of its coordinate space.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReaderRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// One generic image placement in a fixed-page render model.
#[derive(Clone, Debug)]
pub struct ImagePlacementRenderModel {
    /// Encoded data is retained as shared bytes so the FFI can transport this
    /// exact resolved identity without re-reading the loader for a second
    /// geometry/resource pass.
    pub resource: ResourceView,
    pub destination: ReaderRect,
    pub clipping_rect: ReaderRect,
    pub clips_to_viewport: bool,
    pub alt_text: String,
}

/// Stable display projection for the supported generic single-image page
/// shape. It contains no source path and does not expose the IR tree to GUI.
#[derive(Clone, Debug)]
pub struct FixedPageRenderModel {
    pub location: ReaderLocation,
    /// Zero-based position in the Reader's effective reading order.
    pub page_index: usize,
    pub page_count: usize,
    /// Intrinsic image dimensions are used as the page coordinate space when
    /// the IR supplies one full-page image but no separate viewport.
    pub page_size: ReaderSize,
    pub viewport_size: ReaderSize,
    pub page_side: Option<PageSide>,
    pub spread_hint: SpreadHint,
    pub placements: Vec<ImagePlacementRenderModel>,
    pub diagnostics: Vec<ReaderDiagnostic>,
}

/// Transient Reader display choice. IR remains immutable and single-page is
/// the default until a caller explicitly opts into synthetic spreads.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderSpreadMode {
    #[default]
    SinglePage,
    Synthetic,
}

/// One page positioned in a Reader-composed two-slot viewport. Placement
/// rectangles are expressed in the full spread coordinate space.
#[derive(Clone, Debug)]
pub struct FixedSpreadPageRenderModel {
    pub location: ReaderLocation,
    pub page_index: usize,
    pub page_side: PageSide,
    pub spread_hint: SpreadHint,
    pub page_size: ReaderSize,
    pub viewport_rect: ReaderRect,
    pub placements: Vec<ImagePlacementRenderModel>,
    pub diagnostics: Vec<ReaderDiagnostic>,
}

/// Reader-owned fixed-layout spread model. Pages are returned in reading
/// sequence; `viewport_rect` carries their visual left/right placement.
#[derive(Clone, Debug)]
pub struct FixedSpreadRenderModel {
    pub current_location: ReaderLocation,
    pub page_count: usize,
    pub direction: ReaderDirection,
    pub viewport_size: ReaderSize,
    pub pages: Vec<FixedSpreadPageRenderModel>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderDiagnostic {
    IntrinsicImageSizeUsedAsPageViewport,
    ConflictingSpreadMetadata,
}

/// Stable machine-readable Reader failure category.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReaderErrorCode {
    InvalidLocation,
    InvalidNavigationTarget,
    MissingResource,
    UnsupportedLayout,
    InvalidPageGeometry,
    DecodeFailed,
    SessionClosed,
    ResourceUnavailable,
    ResourceTooLarge,
    ResourceHandleSessionMismatch,
    InvalidViewport,
    InvalidPreviewRequest,
    EmptyBook,
    Cancelled,
}

/// Structured Reader error. Callers should branch on `code`, never its text.
#[derive(Clone, Debug, Deserialize, Eq, Error, PartialEq, Serialize)]
#[error("reader operation failed: {code:?}")]
pub struct ReaderError {
    pub code: ReaderErrorCode,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub resource_id: Option<ResourceId>,
}

impl ReaderError {
    const fn new(code: ReaderErrorCode) -> Self {
        Self {
            code,
            resource_id: None,
        }
    }

    pub const fn invalid_preview_request() -> Self {
        Self::new(ReaderErrorCode::InvalidPreviewRequest)
    }

    const fn for_resource(code: ReaderErrorCode, resource_id: ResourceId) -> Self {
        Self {
            code,
            resource_id: Some(resource_id),
        }
    }

    pub const fn code(&self) -> ReaderErrorCode {
        self.code
    }
}

/// Non-persistent session over one immutable, format-neutral Folio IR book.
///
/// Session-local navigation, viewport and render caches are discarded when
/// this value is dropped. This skeleton deliberately has no importer,
/// exporter, Library, database, Comic, or GUI dependency.
pub struct ReaderSession {
    resources: ResourceResolver,
    documents_by_id: HashMap<DocumentId, usize>,
    reading_order: Vec<DocumentId>,
    reading_order_indices: HashMap<DocumentId, usize>,
    spread_groups: Vec<Range<usize>>,
    spread_group_indices: Vec<usize>,
    current_location: ReaderLocation,
    viewport: ReaderViewport,
    direction: ReaderDirection,
    spread_mode: ReaderSpreadMode,
}

const MAX_RESOLVED_RESOURCE_BYTES: u64 = 256 * 1024 * 1024;

/// Opaque, session-scoped reference to a declared IR resource.
#[derive(Clone, Debug)]
pub struct ResourceHandle {
    resource_id: ResourceId,
    session_scope: Arc<()>,
}

impl PartialEq for ResourceHandle {
    fn eq(&self, other: &Self) -> bool {
        self.resource_id == other.resource_id
            && Arc::ptr_eq(&self.session_scope, &other.session_scope)
    }
}

impl Eq for ResourceHandle {}

impl ResourceHandle {
    pub const fn resource_id(&self) -> ResourceId {
        self.resource_id
    }
}

/// Encoded resource bytes resolved through the model's existing loader.
#[derive(Clone, Debug)]
pub struct ResourceView {
    handle: ResourceHandle,
    media_type: String,
    bytes: Arc<[u8]>,
}

impl ResourceView {
    pub fn handle(&self) -> &ResourceHandle {
        &self.handle
    }

    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn shared_bytes(&self) -> Arc<[u8]> {
        Arc::clone(&self.bytes)
    }
}

/// Reader's single resource authority. Lookup and loading are delegated to
/// `Book::resource` and `Book::load_resource`; this layer adds no store.
struct ResourceResolver {
    book: Arc<Book>,
    session_scope: Arc<()>,
}

impl ResourceResolver {
    fn new(book: Arc<Book>) -> Self {
        Self {
            book,
            session_scope: Arc::new(()),
        }
    }

    fn handle(&self, resource_id: ResourceId) -> Result<ResourceHandle, ReaderError> {
        self.book
            .resource(resource_id)
            .filter(|resource| resource.id == resource_id)
            .map(|_| ResourceHandle {
                resource_id,
                session_scope: Arc::clone(&self.session_scope),
            })
            .ok_or_else(|| ReaderError::for_resource(ReaderErrorCode::MissingResource, resource_id))
    }

    fn resolve(&self, handle: ResourceHandle) -> Result<ResourceView, ReaderError> {
        if !Arc::ptr_eq(&self.session_scope, &handle.session_scope) {
            return Err(ReaderError::for_resource(
                ReaderErrorCode::ResourceHandleSessionMismatch,
                handle.resource_id,
            ));
        }
        let resource = self
            .book
            .resource(handle.resource_id)
            .filter(|resource| resource.id == handle.resource_id)
            .ok_or_else(|| {
                ReaderError::for_resource(ReaderErrorCode::MissingResource, handle.resource_id)
            })?;
        let media_type = resource.media_type.clone();
        let bytes = self
            .book
            .load_resource(handle.resource_id, Some(MAX_RESOLVED_RESOURCE_BYTES))
            .map_err(|error| {
                let code = match error {
                    ResourceLoadError::TooLarge(_) => ReaderErrorCode::ResourceTooLarge,
                    ResourceLoadError::Io(_)
                    | ResourceLoadError::NotFound(_)
                    | ResourceLoadError::Other(_) => ReaderErrorCode::ResourceUnavailable,
                };
                ReaderError::for_resource(code, handle.resource_id)
            })?;
        if bytes.len() as u64 > MAX_RESOLVED_RESOURCE_BYTES {
            return Err(ReaderError::for_resource(
                ReaderErrorCode::ResourceTooLarge,
                handle.resource_id,
            ));
        }
        Ok(ResourceView {
            handle: handle.clone(),
            media_type,
            bytes: Arc::from(bytes),
        })
    }
}

impl ReaderSession {
    pub fn open(book: Arc<Book>, viewport: ReaderViewport) -> Result<Self, ReaderError> {
        let direction = ReaderDirection::from_book(&book);
        Self::open_with_direction(book, viewport, direction)
    }

    pub fn open_with_direction(
        book: Arc<Book>,
        viewport: ReaderViewport,
        direction: ReaderDirection,
    ) -> Result<Self, ReaderError> {
        if book.documents.is_empty() {
            return Err(ReaderError::new(ReaderErrorCode::EmptyBook));
        }
        let mut documents_by_id = HashMap::with_capacity(book.documents.len());
        for (index, document) in book.documents.iter().enumerate() {
            if documents_by_id.insert(document.id, index).is_some() {
                return Err(ReaderError::new(ReaderErrorCode::InvalidLocation));
            }
        }
        let mut reading_order: Vec<_> = book.documents.iter().map(|doc| doc.id).collect();
        if direction == ReaderDirection::RightToLeft {
            reading_order.reverse();
        }
        let reading_order_indices = reading_order
            .iter()
            .enumerate()
            .map(|(index, document_id)| (*document_id, index))
            .collect();
        let first_document_id = reading_order[0];
        Ok(Self {
            resources: ResourceResolver::new(book),
            documents_by_id,
            reading_order,
            reading_order_indices,
            spread_groups: Vec::new(),
            spread_group_indices: Vec::new(),
            current_location: ReaderLocation::document(first_document_id),
            viewport,
            direction,
            spread_mode: ReaderSpreadMode::SinglePage,
        })
    }

    pub fn layout_mode(&self) -> LayoutMode {
        self.resources.book.presentation.layout
    }

    pub fn current_location(&self) -> ReaderLocation {
        self.current_location
    }

    pub fn viewport(&self) -> ReaderViewport {
        self.viewport
    }

    pub fn set_viewport(&mut self, viewport: ReaderViewport) {
        self.viewport = viewport;
    }

    pub const fn direction(&self) -> ReaderDirection {
        self.direction
    }

    pub fn set_direction(&mut self, direction: ReaderDirection) {
        if self.direction == direction {
            return;
        }
        let current_document_id = self.current_location.document_id;
        self.reading_order.reverse();
        self.reading_order_indices = self
            .reading_order
            .iter()
            .enumerate()
            .map(|(index, document_id)| (*document_id, index))
            .collect();
        self.direction = direction;
        if self.spread_mode == ReaderSpreadMode::Synthetic {
            self.rebuild_spread_groups();
        }
        if !self
            .reading_order_indices
            .contains_key(&current_document_id)
        {
            self.current_location = ReaderLocation::document(self.reading_order[0]);
        }
    }

    pub const fn spread_mode(&self) -> ReaderSpreadMode {
        self.spread_mode
    }

    pub fn set_spread_mode(&mut self, spread_mode: ReaderSpreadMode) {
        if self.spread_mode == spread_mode {
            return;
        }
        self.spread_mode = spread_mode;
        if spread_mode == ReaderSpreadMode::Synthetic {
            self.rebuild_spread_groups();
        } else {
            self.spread_groups.clear();
            self.spread_group_indices.clear();
        }
    }

    pub fn current_index(&self) -> usize {
        self.reading_order_indices[&self.current_location.document_id]
    }

    /// Zero-based index in the immutable IR document vector. This remains
    /// distinct from `current_index()`, which follows the effective reading
    /// direction and may therefore be reversed.
    pub fn current_document_index(&self) -> usize {
        self.documents_by_id[&self.current_location.document_id]
    }

    pub fn page_count(&self) -> usize {
        self.reading_order.len()
    }

    /// Return the immutable IR navigation tree with only proven destinations
    /// bound to Reader locations. Unresolved entries retain labels and an
    /// explicit reason instead of silently redirecting elsewhere.
    pub fn navigation(&self) -> ReaderNavigationSnapshot {
        let book = self.book();
        ReaderNavigationSnapshot {
            toc: project_navigation_points(
                &book.navigation.toc,
                book,
                ReaderNavigationSection::Toc,
                &[],
            ),
            landmarks: project_navigation_points(
                &book.navigation.landmarks,
                book,
                ReaderNavigationSection::Landmark,
                &[],
            ),
            page_list: project_navigation_points(
                &book.navigation.page_list,
                book,
                ReaderNavigationSection::PageList,
                &[],
            ),
        }
    }

    pub fn first(&mut self) -> ReaderLocation {
        self.current_location = ReaderLocation::document(self.reading_order[0]);
        self.current_location
    }

    pub fn last(&mut self) -> ReaderLocation {
        let document_id = self.reading_order[self.reading_order.len() - 1];
        self.current_location = ReaderLocation::document(document_id);
        self.current_location
    }

    /// Navigate by the document's stable position in the input IR vector,
    /// independent of the effective LTR/RTL traversal order.
    pub fn go_to_document_index(
        &mut self,
        document_index: usize,
    ) -> Result<ReaderLocation, ReaderError> {
        let document = self
            .resources
            .book
            .documents
            .get(document_index)
            .ok_or_else(|| ReaderError::new(ReaderErrorCode::InvalidLocation))?;
        self.current_location = ReaderLocation::document(document.id);
        Ok(self.current_location)
    }

    /// Navigate directly by the stable identity of an IR document/page.
    pub fn go_to_page(&mut self, document_id: DocumentId) -> Result<ReaderLocation, ReaderError> {
        self.go_to(ReaderLocation::document(document_id))
    }

    /// Resolve a Reader-owned navigation reference against this session's
    /// immutable IR and move to its exact semantic destination.
    pub fn go_to_navigation_target(
        &mut self,
        target: &ReaderNavigationTarget,
    ) -> Result<ReaderLocation, ReaderError> {
        let points = navigation_points(self.book(), target.section);
        let point = navigation_point_at(points, &target.entry_path)
            .ok_or_else(|| ReaderError::new(ReaderErrorCode::InvalidNavigationTarget))?;
        let location = resolve_navigation_href(self.book(), &point.href)
            .map_err(|_| ReaderError::new(ReaderErrorCode::InvalidNavigationTarget))?;
        self.go_to(location)
    }

    // This is a stateful Reader navigation command, not an iterator step.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<ReaderLocation> {
        let next_index = if self.spread_mode == ReaderSpreadMode::Synthetic {
            self.spread_range_for(self.current_index())?.end
        } else {
            self.current_index().checked_add(1)?
        };
        let document_id = *self.reading_order.get(next_index)?;
        self.current_location = ReaderLocation::document(document_id);
        Some(self.current_location)
    }

    pub fn previous(&mut self) -> Option<ReaderLocation> {
        let previous_index = if self.spread_mode == ReaderSpreadMode::Synthetic {
            let before = self
                .spread_range_for(self.current_index())?
                .start
                .checked_sub(1)?;
            self.spread_range_for(before)?.start
        } else {
            self.current_index().checked_sub(1)?
        };
        let document_id = *self.reading_order.get(previous_index)?;
        self.current_location = ReaderLocation::document(document_id);
        Some(self.current_location)
    }

    pub fn go_to(&mut self, location: ReaderLocation) -> Result<ReaderLocation, ReaderError> {
        let document = self.document(location.document_id)?;
        if location.node_id.is_some_and(|node_id| {
            !document
                .nodes
                .iter()
                .any(|node| contains_node(node, node_id))
        }) {
            return Err(ReaderError::new(ReaderErrorCode::InvalidLocation));
        }
        self.current_location = location;
        Ok(location)
    }

    /// Build a page render model only for a proven one-root-image fixed page.
    /// More complex pages require explicit generic placement data and are
    /// rejected instead of guessed from source-format conventions.
    pub fn fixed_page(
        &self,
        location: ReaderLocation,
    ) -> Result<FixedPageRenderModel, ReaderError> {
        self.fixed_page_with_cancellation(location, || false)
    }

    pub fn fixed_page_with_cancellation<F>(
        &self,
        location: ReaderLocation,
        mut is_cancelled: F,
    ) -> Result<FixedPageRenderModel, ReaderError>
    where
        F: FnMut() -> bool,
    {
        self.fixed_page_in_viewport(location, self.viewport, &mut is_cancelled)
    }

    fn fixed_page_in_viewport<F>(
        &self,
        location: ReaderLocation,
        viewport: ReaderViewport,
        is_cancelled: &mut F,
    ) -> Result<FixedPageRenderModel, ReaderError>
    where
        F: FnMut() -> bool,
    {
        if is_cancelled() {
            return Err(ReaderError::new(ReaderErrorCode::Cancelled));
        }
        if self.layout_mode() != LayoutMode::Fixed {
            return Err(ReaderError::new(ReaderErrorCode::UnsupportedLayout));
        }
        let index = *self
            .reading_order_indices
            .get(&location.document_id)
            .ok_or_else(|| ReaderError::new(ReaderErrorCode::InvalidLocation))?;
        let document = self.document(location.document_id)?;
        if location.node_id.is_some_and(|node_id| {
            !document
                .nodes
                .iter()
                .any(|node| contains_node(node, node_id))
        }) {
            return Err(ReaderError::new(ReaderErrorCode::InvalidLocation));
        }
        let [node] = document.nodes.as_slice() else {
            return Err(ReaderError::new(ReaderErrorCode::UnsupportedLayout));
        };
        if !node.children.is_empty()
            || !node.presentation.features.is_empty()
            || !node.variants.is_empty()
            || self
                .resources
                .book
                .styles
                .get(node.style)
                .is_some_and(|style| !style.properties.is_empty())
        {
            return Err(ReaderError::new(ReaderErrorCode::UnsupportedLayout));
        }
        let (resource_id, alt_text) = match &node.kind {
            NodeKind::Image { resource, alt } => (*resource, alt.clone()),
            _ => return Err(ReaderError::new(ReaderErrorCode::UnsupportedLayout)),
        };
        let view = self
            .resources
            .resolve(self.resources.handle(resource_id)?)?;
        if is_cancelled() {
            return Err(ReaderError::new(ReaderErrorCode::Cancelled));
        }
        let (image_width, image_height) = image_dimensions(view.bytes())?;
        let page_size = ReaderSize {
            width: image_width as f32,
            height: image_height as f32,
        };
        let (destination, clips_to_viewport) = fit_rect(page_size, viewport)?;
        let viewport_size = ReaderSize {
            width: viewport.width,
            height: viewport.height,
        };
        if is_cancelled() {
            return Err(ReaderError::new(ReaderErrorCode::Cancelled));
        }
        Ok(FixedPageRenderModel {
            location,
            page_index: index,
            page_count: self.reading_order.len(),
            page_size,
            viewport_size,
            page_side: self
                .book()
                .presentation
                .page_sides
                .get(&location.document_id)
                .copied(),
            spread_hint: self.spread_hint(location.document_id),
            placements: vec![ImagePlacementRenderModel {
                resource: view,
                destination,
                clipping_rect: ReaderRect {
                    x: 0.0,
                    y: 0.0,
                    width: viewport.width,
                    height: viewport.height,
                },
                clips_to_viewport,
                alt_text,
            }],
            diagnostics: vec![ReaderDiagnostic::IntrinsicImageSizeUsedAsPageViewport],
        })
    }

    /// Build the current fixed-layout spread. If synthetic mode is active,
    /// only adjacent pages are composed; no image/file-name heuristic is used.
    pub fn fixed_spread(&self) -> Result<FixedSpreadRenderModel, ReaderError> {
        if self.layout_mode() != LayoutMode::Fixed {
            return Err(ReaderError::new(ReaderErrorCode::UnsupportedLayout));
        }
        let page_indices = self
            .spread_range_for(self.current_index())
            .ok_or_else(|| ReaderError::new(ReaderErrorCode::InvalidLocation))?;
        let viewport_size = ReaderSize {
            width: self.viewport.width,
            height: self.viewport.height,
        };
        let mut pages = Vec::with_capacity(page_indices.len());
        for (slot, page_index) in page_indices.clone().enumerate() {
            let document_id = self.reading_order[page_index];
            let location = ReaderLocation::document(document_id);
            let page_side = self.effective_page_side(page_index, slot, page_indices.len());
            let centered = page_side == PageSide::Center;
            let slot_width = if centered {
                self.viewport.width
            } else {
                self.viewport.width / 2.0
            };
            let slot_x = match page_side {
                PageSide::Left => 0.0,
                PageSide::Right => self.viewport.width / 2.0,
                PageSide::Center => 0.0,
            };
            let page_viewport = ReaderViewport {
                width: slot_width,
                height: self.viewport.height,
                scale: self.viewport.scale,
                content_mode: self.viewport.content_mode,
                pan_x: self.viewport.pan_x,
                pan_y: self.viewport.pan_y,
            };
            let mut page = self.fixed_page_in_viewport(location, page_viewport, &mut || false)?;
            for placement in &mut page.placements {
                placement.destination.x += slot_x;
                placement.clipping_rect = ReaderRect {
                    x: slot_x,
                    y: 0.0,
                    width: slot_width,
                    height: self.viewport.height,
                };
            }
            let mut diagnostics = page.diagnostics;
            if self.has_conflicting_spread_metadata(document_id) {
                diagnostics.push(ReaderDiagnostic::ConflictingSpreadMetadata);
            }
            pages.push(FixedSpreadPageRenderModel {
                location,
                page_index,
                page_side,
                spread_hint: self.spread_hint(document_id),
                page_size: page.page_size,
                viewport_rect: ReaderRect {
                    x: slot_x,
                    y: 0.0,
                    width: slot_width,
                    height: self.viewport.height,
                },
                placements: page.placements,
                diagnostics,
            });
        }
        Ok(FixedSpreadRenderModel {
            current_location: self.current_location,
            page_count: self.reading_order.len(),
            direction: self.direction,
            viewport_size,
            pages,
        })
    }

    fn spread_range_for(&self, target: usize) -> Option<Range<usize>> {
        if target >= self.reading_order.len() {
            return None;
        }
        if self.spread_mode == ReaderSpreadMode::SinglePage {
            return Some(target..target + 1);
        }
        let group_index = *self.spread_group_indices.get(target)?;
        self.spread_groups.get(group_index).cloned()
    }

    fn rebuild_spread_groups(&mut self) {
        let mut groups = Vec::with_capacity(self.reading_order.len());
        let mut group_indices = vec![0; self.reading_order.len()];
        let mut start = 0;
        while start < self.reading_order.len() {
            let next = start + 1;
            let end = if start > 0
                && next < self.reading_order.len()
                && self.page_can_pair(start)
                && self.page_can_pair(next)
                && self.sides_can_pair(start, next)
            {
                next + 1
            } else {
                next
            };
            let group_index = groups.len();
            group_indices[start..end].fill(group_index);
            groups.push(start..end);
            start = end;
        }
        self.spread_groups = groups;
        self.spread_group_indices = group_indices;
    }

    fn page_can_pair(&self, reading_index: usize) -> bool {
        let document_id = self.reading_order[reading_index];
        self.spread_hint(document_id) != SpreadHint::SinglePage
            && self.book().presentation.page_sides.get(&document_id) != Some(&PageSide::Center)
    }

    fn sides_can_pair(&self, first: usize, second: usize) -> bool {
        let side = |index| {
            self.book()
                .presentation
                .page_sides
                .get(&self.reading_order[index])
                .copied()
        };
        match (side(first), side(second)) {
            (Some(PageSide::Center), _) | (_, Some(PageSide::Center)) => false,
            (Some(left), Some(right)) => left != right,
            _ => true,
        }
    }

    fn effective_page_side(&self, reading_index: usize, slot: usize, group_len: usize) -> PageSide {
        let document_id = self.reading_order[reading_index];
        if let Some(side) = self.book().presentation.page_sides.get(&document_id) {
            return *side;
        }
        if group_len == 2 {
            let sibling_index = if slot == 0 {
                reading_index + 1
            } else {
                reading_index - 1
            };
            if let Some(sibling_side) = self
                .book()
                .presentation
                .page_sides
                .get(&self.reading_order[sibling_index])
            {
                return match sibling_side {
                    PageSide::Left => PageSide::Right,
                    PageSide::Right => PageSide::Left,
                    PageSide::Center => PageSide::Center,
                };
            }
            return match (self.direction, slot) {
                (ReaderDirection::LeftToRight, 0) | (ReaderDirection::RightToLeft, 1) => {
                    PageSide::Left
                }
                _ => PageSide::Right,
            };
        }
        if reading_index == 0 {
            return match self.direction {
                ReaderDirection::LeftToRight => PageSide::Right,
                ReaderDirection::RightToLeft => PageSide::Left,
            };
        }
        PageSide::Center
    }

    fn spread_hint(&self, document_id: DocumentId) -> SpreadHint {
        self.book()
            .presentation
            .spread_hints
            .get(&document_id)
            .copied()
            .unwrap_or_default()
    }

    fn has_conflicting_spread_metadata(&self, document_id: DocumentId) -> bool {
        let side = self.book().presentation.page_sides.get(&document_id);
        let hint = self.spread_hint(document_id);
        matches!(
            (side, hint),
            (Some(PageSide::Center), SpreadHint::Pair)
                | (
                    Some(PageSide::Left | PageSide::Right),
                    SpreadHint::SinglePage
                )
        )
    }

    pub fn book(&self) -> &Book {
        &self.resources.book
    }

    fn document(&self, document_id: DocumentId) -> Result<&folio_model::Document, ReaderError> {
        let index = self
            .documents_by_id
            .get(&document_id)
            .copied()
            .ok_or_else(|| ReaderError::new(ReaderErrorCode::InvalidLocation))?;
        self.resources
            .book
            .documents
            .get(index)
            .ok_or_else(|| ReaderError::new(ReaderErrorCode::InvalidLocation))
    }

    pub fn resource_handle(&self, resource_id: ResourceId) -> Result<ResourceHandle, ReaderError> {
        self.resources.handle(resource_id)
    }

    pub fn resolve_resource(&self, handle: ResourceHandle) -> Result<ResourceView, ReaderError> {
        self.resources.resolve(handle)
    }
}

fn image_dimensions(bytes: &[u8]) -> Result<(u32, u32), ReaderError> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ReaderError::new(ReaderErrorCode::DecodeFailed))?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| ReaderError::new(ReaderErrorCode::DecodeFailed))?;
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > 32_768 || height > 32_768 || pixels > 32_000_000 {
        return Err(ReaderError::new(ReaderErrorCode::InvalidPageGeometry));
    }
    Ok((width, height))
}

fn contains_node(node: &folio_model::Node, node_id: NodeId) -> bool {
    node.id == node_id
        || node
            .children
            .iter()
            .any(|child| contains_node(child, node_id))
}

fn project_navigation_points(
    points: &[NavPoint],
    book: &Book,
    section: ReaderNavigationSection,
    parent_path: &[usize],
) -> Vec<ReaderNavigationEntry> {
    points
        .iter()
        .enumerate()
        .map(|(index, point)| {
            let mut entry_path = parent_path.to_vec();
            entry_path.push(index);
            let (location, unresolved_reason) = match resolve_navigation_href(book, &point.href) {
                Ok(location) => (Some(location), None),
                Err(reason) => (None, Some(reason)),
            };
            let children = project_navigation_points(&point.children, book, section, &entry_path);
            let target = location.as_ref().map(|_| ReaderNavigationTarget {
                section,
                entry_path,
            });
            ReaderNavigationEntry {
                label: point.label.clone(),
                target,
                location,
                unresolved_reason,
                children,
            }
        })
        .collect()
}

fn navigation_points(book: &Book, section: ReaderNavigationSection) -> &[NavPoint] {
    match section {
        ReaderNavigationSection::Toc => &book.navigation.toc,
        ReaderNavigationSection::Landmark => &book.navigation.landmarks,
        ReaderNavigationSection::PageList => &book.navigation.page_list,
    }
}

fn navigation_point_at<'a>(points: &'a [NavPoint], path: &[usize]) -> Option<&'a NavPoint> {
    let (first, rest) = path.split_first()?;
    let point = points.get(*first)?;
    if rest.is_empty() {
        Some(point)
    } else {
        navigation_point_at(&point.children, rest)
    }
}

fn resolve_navigation_href(
    book: &Book,
    href: &str,
) -> Result<ReaderLocation, ReaderNavigationUnresolvedReason> {
    let href = href.trim();
    if href.is_empty() {
        return Err(ReaderNavigationUnresolvedReason::EmptyTarget);
    }
    if is_external_reference(href) {
        return Err(ReaderNavigationUnresolvedReason::ExternalTarget);
    }
    let (document_href, fragment) = href.split_once('#').unwrap_or((href, ""));
    if document_href.is_empty() {
        return Err(ReaderNavigationUnresolvedReason::MissingDocumentContext);
    }
    let mut documents = book
        .documents
        .iter()
        .filter(|document| document.href == document_href);
    let document = documents
        .next()
        .ok_or(ReaderNavigationUnresolvedReason::DocumentNotFound)?;
    if documents.next().is_some() {
        return Err(ReaderNavigationUnresolvedReason::AmbiguousDocument);
    }
    if fragment.is_empty() {
        return Ok(ReaderLocation::document(document.id));
    }

    let mut anchors = book.anchors.iter().filter(|anchor| {
        anchor.document == document.id
            && anchor.name == fragment
            && document
                .nodes
                .iter()
                .any(|node| contains_node(node, anchor.node))
    });
    let anchor = anchors
        .next()
        .ok_or(ReaderNavigationUnresolvedReason::AnchorNotFound)?;
    if anchors.next().is_some() {
        return Err(ReaderNavigationUnresolvedReason::AmbiguousAnchor);
    }
    Ok(ReaderLocation {
        document_id: document.id,
        node_id: Some(anchor.node),
        text_offset: None,
        semantic_context: None,
    })
}

fn is_external_reference(href: &str) -> bool {
    if href.starts_with("//") {
        return true;
    }
    href.find(':').is_some_and(|colon| {
        colon > 0
            && href[..colon].chars().enumerate().all(|(index, character)| {
                if index == 0 {
                    character.is_ascii_alphabetic()
                } else {
                    character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
                }
            })
    })
}

fn fit_rect(page: ReaderSize, viewport: ReaderViewport) -> Result<(ReaderRect, bool), ReaderError> {
    let fit_scale = (viewport.width / page.width).min(viewport.height / page.height);
    let base_scale = match viewport.content_mode {
        ReaderContentMode::Fit => fit_scale,
        ReaderContentMode::Fill => (viewport.width / page.width).max(viewport.height / page.height),
        ReaderContentMode::ActualSize => 1.0,
    };
    let scale = base_scale * viewport.scale;
    let width = page.width * scale;
    let height = page.height * scale;
    let x = (viewport.width - width) / 2.0 + viewport.pan_x;
    let y = (viewport.height - height) / 2.0 + viewport.pan_y;
    if !scale.is_finite()
        || scale <= 0.0
        || !width.is_finite()
        || width <= 0.0
        || !height.is_finite()
        || height <= 0.0
        || !x.is_finite()
        || !y.is_finite()
    {
        return Err(ReaderError::new(ReaderErrorCode::InvalidPageGeometry));
    }
    let rect = ReaderRect {
        x,
        y,
        width,
        height,
    };
    let clipped = x < 0.0 || y < 0.0 || x + width > viewport.width || y + height > viewport.height;
    Ok((rect, clipped))
}
