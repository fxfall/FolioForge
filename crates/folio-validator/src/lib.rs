//! Input, IR, binary, and behavioral validation entry points.

use std::collections::BTreeSet;

use folio_kfx::KfxError;
use folio_kindle_common::binary::{read_u16_be, read_u32_be};
use folio_mobi::{inspect_pdb, MobiReadError};
use folio_model::{Book, Diagnostic, Node, NodeKind, Severity};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ValidationReport {
    pub diagnostics: Vec<Diagnostic>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    pub fn errors(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count()
    }
}

pub fn validate_book(book: &Book) -> ValidationReport {
    let mut report = ValidationReport::default();
    let mut resource_ids = BTreeSet::new();
    for (index, resource) in book.resources.iter().enumerate() {
        if resource.id.get() as usize != index || !resource_ids.insert(resource.id) {
            report.diagnostics.push(Diagnostic::error(
                "FF-IR-RES-0001",
                format!("resource ID {} is not unique and stable", resource.id),
            ));
        }
    }
    for font_face in &book.font_faces {
        if !book.resources.iter().any(|resource| {
            resource.id == font_face.resource && resource.kind == folio_model::ResourceKind::Font
        }) {
            report.diagnostics.push(Diagnostic::error(
                "FF-IR-FONT-0001",
                format!(
                    "font face {} refers to a missing or non-font resource {}",
                    font_face.family, font_face.resource
                ),
            ));
        }
        if font_face.family.trim().is_empty() {
            report.diagnostics.push(Diagnostic::error(
                "FF-IR-FONT-0002",
                "font face family must not be empty",
            ));
        }
    }
    let mut anchor_names = BTreeSet::new();
    for anchor in &book.anchors {
        if !anchor_names.insert((anchor.document, anchor.name.clone())) {
            report.diagnostics.push(Diagnostic::error(
                "FF-IR-ANCHOR-0001",
                format!("duplicate anchor {}", anchor.name),
            ));
        }
        if !book
            .documents
            .iter()
            .any(|document| document.id == anchor.document)
        {
            report.diagnostics.push(Diagnostic::error(
                "FF-IR-ANCHOR-0002",
                format!("anchor {} refers to a missing document", anchor.name),
            ));
        }
        if !book.documents.iter().any(|document| {
            document.id == anchor.document && contains_node_id(&document.nodes, anchor.node)
        }) {
            report.diagnostics.push(Diagnostic::error(
                "FF-IR-ANCHOR-0003",
                format!("anchor {} refers to a missing node", anchor.name),
            ));
        }
    }
    let mut node_ids = BTreeSet::new();
    for document in &book.documents {
        validate_nodes(
            book,
            document.id,
            &document.nodes,
            &mut node_ids,
            &mut report.diagnostics,
        );
    }
    validate_navigation(book, &mut report.diagnostics);
    validate_anchor_graph(book, &mut report.diagnostics);
    report
}

fn contains_node_id(nodes: &[Node], id: folio_model::NodeId) -> bool {
    nodes
        .iter()
        .any(|node| node.id == id || contains_node_id(&node.children, id))
}

fn validate_nodes(
    book: &Book,
    _document: folio_model::DocumentId,
    nodes: &[Node],
    node_ids: &mut BTreeSet<folio_model::NodeId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for node in nodes {
        if !node_ids.insert(node.id) {
            diagnostics.push(Diagnostic::error(
                "FF-IR-NODE-0001",
                format!("node ID {} is not unique", node.id),
            ));
        }
        if book.styles.get(node.style).is_none() {
            diagnostics.push(Diagnostic::error(
                "FF-IR-STYLE-0001",
                format!("node {} references missing style {}", node.id, node.style),
            ));
        }
        for variant in &node.variants {
            if book.styles.get(variant.style).is_none() {
                diagnostics.push(Diagnostic::error(
                    "FF-IR-STYLE-0002",
                    format!(
                        "node {} variant {:?} references missing style {}",
                        node.id, variant.target, variant.style
                    ),
                ));
            }
        }
        if let NodeKind::Svg {
            resource: Some(resource),
            ..
        } = &node.kind
        {
            if book.resource(*resource).is_none() {
                diagnostics.push(Diagnostic::error(
                    "FF-IR-RES-0003",
                    format!(
                        "node {} references missing SVG resource {}",
                        node.id, resource
                    ),
                ));
            }
        }
        if let NodeKind::Image { resource, .. } = &node.kind {
            if book.resource(*resource).is_none() {
                diagnostics.push(Diagnostic::error(
                    "FF-IR-RES-0002",
                    format!("node {} references missing resource {}", node.id, resource),
                ));
            }
        }
        if let NodeKind::Link { href } | NodeKind::Footnote { href: Some(href) } = &node.kind {
            if href.is_empty() {
                diagnostics.push(Diagnostic::warning(
                    "FF-IR-LINK-0001",
                    format!("node {} has an empty link target", node.id),
                ));
            }
        }
        validate_nodes(book, _document, &node.children, node_ids, diagnostics);
    }
}

fn validate_navigation(book: &Book, diagnostics: &mut Vec<Diagnostic>) {
    let document_hrefs: BTreeSet<&str> = book
        .documents
        .iter()
        .map(|document| document.href.as_str())
        .collect();
    for point in book
        .navigation
        .toc
        .iter()
        .chain(book.navigation.landmarks.iter())
        .chain(book.navigation.page_list.iter())
    {
        validate_nav_point(point, &document_hrefs, diagnostics);
    }
    if let Some(start_location) = &book.navigation.start_location {
        validate_href(
            start_location,
            &document_hrefs,
            diagnostics,
            "start location",
        );
    }
}

fn validate_anchor_graph(book: &Book, diagnostics: &mut Vec<Diagnostic>) {
    let document_hrefs: BTreeSet<&str> = book
        .documents
        .iter()
        .map(|document| document.href.as_str())
        .collect();
    let anchor_names: BTreeSet<&str> = book
        .anchors
        .iter()
        .map(|anchor| anchor.name.as_str())
        .collect();
    for edge in &book.navigation.anchor_graph.edges {
        validate_href(
            &edge.target,
            &document_hrefs,
            diagnostics,
            "anchor graph edge",
        );
        if let Some((_, fragment)) = edge.target.split_once('#') {
            if !fragment.is_empty() && !anchor_names.contains(fragment) {
                diagnostics.push(Diagnostic::warning(
                    "FF-IR-EDGE-0002",
                    format!(
                        "anchor graph edge targets an unknown fragment: {}",
                        edge.target
                    ),
                ));
            }
        }
    }
}

fn validate_href(
    href: &str,
    document_hrefs: &BTreeSet<&str>,
    diagnostics: &mut Vec<Diagnostic>,
    description: &str,
) {
    let path = href.split('#').next().unwrap_or(href);
    if path.is_empty()
        || path.contains("://")
        || path.starts_with("mailto:")
        || path.starts_with("data:")
    {
        return;
    }
    if !document_hrefs.contains(path) {
        diagnostics.push(Diagnostic::warning(
            "FF-IR-EDGE-0001",
            format!("{description} does not match a spine document: {href}"),
        ));
    }
}

fn validate_nav_point(
    point: &folio_model::NavPoint,
    document_hrefs: &BTreeSet<&str>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let href = point.href.split('#').next().unwrap_or(&point.href);
    if !href.is_empty() && !document_hrefs.contains(href) {
        diagnostics.push(Diagnostic::warning(
            "FF-IR-NAV-0001",
            format!(
                "navigation target does not match a spine document: {}",
                point.href
            ),
        ));
    }
    for child in &point.children {
        validate_nav_point(child, document_hrefs, diagnostics);
    }
}

pub fn validate_mobi(bytes: &[u8]) -> ValidationReport {
    let mut report = ValidationReport::default();
    let inspection = match inspect_pdb(bytes) {
        Ok(value) => value,
        Err(error) => {
            let code = match error {
                MobiReadError::Truncated => "FF-MOBI-BIN-0001",
                MobiReadError::InvalidRecordTable => "FF-MOBI-BIN-0004",
                MobiReadError::OffsetOutOfBounds => "FF-MOBI-BIN-0006",
            };
            report
                .diagnostics
                .push(Diagnostic::error(code, error.to_string()));
            return report;
        }
    };
    let Some(record_zero) = inspection.records.first() else {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0007",
            "PDB has no record zero.",
        ));
        return report;
    };
    if record_zero.len() < 20 || &record_zero[16..20] != b"MOBI" {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0007",
            "record zero does not contain a MOBI header.",
        ));
        return report;
    }
    if record_zero.len() < 248 {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0008",
            "record zero is shorter than the PalmDOC and minimum MOBI headers.",
        ));
        return report;
    }

    let compression = read_u16_be(record_zero, 0);
    let text_length = read_u32_be(record_zero, 4);
    let text_record_count = read_u16_be(record_zero, 8);
    let max_record_size = read_u16_be(record_zero, 10);
    let encryption = read_u16_be(record_zero, 12);
    let mobi_header_length = read_u32_be(record_zero, 20);
    let full_name_offset = read_u32_be(record_zero, 44);
    let full_name_length = read_u32_be(record_zero, 48);
    let first_image_index = read_u32_be(record_zero, 68);
    let exth_flags = read_u32_be(record_zero, 88);
    let values = match (
        compression,
        text_length,
        text_record_count,
        max_record_size,
        encryption,
        mobi_header_length,
        full_name_offset,
        full_name_length,
        first_image_index,
        exth_flags,
    ) {
        (
            Ok(compression),
            Ok(text_length),
            Ok(text_record_count),
            Ok(max_record_size),
            Ok(encryption),
            Ok(mobi_header_length),
            Ok(full_name_offset),
            Ok(full_name_length),
            Ok(first_image_index),
            Ok(exth_flags),
        ) => (
            compression,
            text_length,
            text_record_count,
            max_record_size,
            encryption,
            mobi_header_length,
            full_name_offset,
            full_name_length,
            first_image_index,
            exth_flags,
        ),
        _ => {
            report.diagnostics.push(Diagnostic::error(
                "FF-MOBI-BIN-0008",
                "record zero could not be read as a PalmDOC/MOBI header.",
            ));
            return report;
        }
    };
    let (
        compression,
        _text_length,
        text_record_count,
        max_record_size,
        encryption,
        mobi_header_length,
        full_name_offset,
        full_name_length,
        first_image_index,
        exth_flags,
    ) = values;
    if !matches!(compression, 1 | 2 | 17_480) {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0009",
            format!("unsupported PalmDOC compression code {compression}"),
        ));
    }
    if max_record_size == 0 {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0010",
            "PalmDOC max record size is zero.",
        ));
    }
    let required_records = usize::from(text_record_count).saturating_add(1);
    if required_records > inspection.records.len() {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0011",
            format!(
                "PalmDOC declares {text_record_count} text records but PDB has only {} records.",
                inspection.records.len()
            ),
        ));
    }
    let mobi_header_length = match usize::try_from(mobi_header_length) {
        Ok(value) if value >= 232 => value,
        _ => {
            report.diagnostics.push(Diagnostic::error(
                "FF-MOBI-BIN-0012",
                "MOBI header length is smaller than the minimum header.",
            ));
            return report;
        }
    };
    let mobi_end = match 16usize.checked_add(mobi_header_length) {
        Some(value) if value <= record_zero.len() => value,
        _ => {
            report.diagnostics.push(Diagnostic::error(
                "FF-MOBI-BIN-0012",
                "MOBI header extends beyond record zero.",
            ));
            return report;
        }
    };
    let name_start =
        match 16usize.checked_add(usize::try_from(full_name_offset).unwrap_or(usize::MAX)) {
            Some(value) => value,
            None => usize::MAX,
        };
    let name_end = name_start.checked_add(usize::try_from(full_name_length).unwrap_or(usize::MAX));
    if name_start > record_zero.len() || name_end.is_none_or(|end| end > record_zero.len()) {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0013",
            "MOBI full name points outside record zero.",
        ));
    }
    if first_image_index != u32::MAX
        && usize::try_from(first_image_index)
            .map(|index| index >= inspection.records.len())
            .unwrap_or(true)
    {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0014",
            "MOBI first image record index is outside the PDB record table.",
        ));
    }
    if encryption != 0 {
        report.diagnostics.push(Diagnostic::warning(
            "FF-MOBI-BIN-0015",
            "MOBI encryption/DRM marker is set; FolioForge does not decrypt it.",
        ));
    }
    if exth_flags & 0x40 != 0 {
        validate_exth(record_zero, mobi_end, &mut report);
    }
    report
}

fn validate_exth(record_zero: &[u8], start: usize, report: &mut ValidationReport) {
    let Some(header_end) = start.checked_add(12) else {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0016",
            "EXTH header offset overflowed.",
        ));
        return;
    };
    let Some(header) = record_zero.get(start..header_end) else {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0016",
            "EXTH header is truncated.",
        ));
        return;
    };
    if &header[..4] != b"EXTH" {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0016",
            "MOBI EXTH flag is set but the EXTH header is missing.",
        ));
        return;
    }
    let total_length = match read_u32_be(header, 4)
        .ok()
        .and_then(|value| usize::try_from(value).ok())
    {
        Some(value) if value >= 12 => value,
        _ => {
            report.diagnostics.push(Diagnostic::error(
                "FF-MOBI-BIN-0017",
                "EXTH total length is invalid.",
            ));
            return;
        }
    };
    let count = match read_u32_be(header, 8)
        .ok()
        .and_then(|value| usize::try_from(value).ok())
    {
        Some(value) => value,
        None => {
            report.diagnostics.push(Diagnostic::error(
                "FF-MOBI-BIN-0017",
                "EXTH record count is invalid.",
            ));
            return;
        }
    };
    let Some(end) = start.checked_add(total_length) else {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0017",
            "EXTH total length overflowed.",
        ));
        return;
    };
    if end > record_zero.len() {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0017",
            "EXTH extends beyond record zero.",
        ));
        return;
    }
    let mut cursor = header_end;
    for _ in 0..count {
        let Some(entry_header_end) = cursor.checked_add(8) else {
            report.diagnostics.push(Diagnostic::error(
                "FF-MOBI-BIN-0018",
                "EXTH entry offset overflowed.",
            ));
            return;
        };
        let Some(entry_header) = record_zero.get(cursor..entry_header_end) else {
            report.diagnostics.push(Diagnostic::error(
                "FF-MOBI-BIN-0018",
                "EXTH entry header is truncated.",
            ));
            return;
        };
        let Some(entry_length) = read_u32_be(entry_header, 4)
            .ok()
            .and_then(|value| usize::try_from(value).ok())
        else {
            report.diagnostics.push(Diagnostic::error(
                "FF-MOBI-BIN-0018",
                "EXTH entry length is invalid.",
            ));
            return;
        };
        if entry_length < 8 {
            report.diagnostics.push(Diagnostic::error(
                "FF-MOBI-BIN-0018",
                "EXTH entry length is smaller than its header.",
            ));
            return;
        }
        let Some(next) = cursor.checked_add(entry_length) else {
            report.diagnostics.push(Diagnostic::error(
                "FF-MOBI-BIN-0018",
                "EXTH entry length overflowed.",
            ));
            return;
        };
        if next > end {
            report.diagnostics.push(Diagnostic::error(
                "FF-MOBI-BIN-0018",
                "EXTH entry extends beyond the EXTH block.",
            ));
            return;
        }
        cursor = next;
    }
    if cursor != end {
        report.diagnostics.push(Diagnostic::error(
            "FF-MOBI-BIN-0019",
            "EXTH total length does not match its entry list.",
        ));
    }
}

pub fn validate_kfx(bytes: &[u8]) -> ValidationReport {
    let mut report = ValidationReport::default();
    match folio_kfx::validate_container(bytes) {
        Ok(()) => {}
        Err(KfxError::InvalidMagic) if bytes.starts_with(b"CONT") => {
            report.diagnostics.push(Diagnostic::warning("FF-KFX-REF-0001", "recognized an Amazon CONT reference container; semantic validation is read-only and DRM-aware parsing is not enabled."));
        }
        Err(error) => report
            .diagnostics
            .push(Diagnostic::error("FF-KFX-BIN-0002", error.to_string())),
    }
    report
}

pub fn validate_kf8(bytes: &[u8]) -> ValidationReport {
    let mut report = validate_mobi(bytes);
    if !report.is_valid() {
        return report;
    }
    if let Err(error) = folio_kf8::validate_bytes(bytes) {
        report
            .diagnostics
            .push(Diagnostic::error("FF-KF8-BIN-0001", error.to_string()));
    }
    report
}

pub fn validate_combo(bytes: &[u8]) -> ValidationReport {
    let mut report = validate_mobi(bytes);
    if !report.is_valid() {
        return report;
    }
    if let Err(error) = folio_mobi::inspect_combo(bytes) {
        report
            .diagnostics
            .push(Diagnostic::error("FF-KF-COMBO-BIN-0001", error.to_string()));
    }
    report
}
