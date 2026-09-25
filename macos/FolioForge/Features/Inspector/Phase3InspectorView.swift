import SwiftUI

enum InspectorTab: String, CaseIterable, Identifiable, Hashable {
    case output
    case compatibility
    case input
    case kfx
    case diagnostics

    var id: String { rawValue }
    var title: String {
        switch self {
        case .output: FolioL10n.string("ui.output", default: "Output")
        case .compatibility: FolioL10n.string("ui.compatibility", default: "Compatibility")
        case .input: FolioL10n.string("ui.input", default: "Input")
        case .kfx: FolioL10n.string("ui.kfx", default: "KFX")
        case .diagnostics: FolioL10n.string("ui.diagnostics", default: "Diagnostics")
        }
    }
}

struct Phase3InspectorView: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel
    let item: BookItem?
    let report: FolioInspectReport?
    let analysis: FolioAnalysisReport?
    let preview: FolioReaderPreviewBundle?
    let selectedFont: SemanticFontRow?
    let detailError: String?
    let previewError: String?
    @Binding var selection: InspectorTab

    private var isKFXInput: Bool {
        [
            report?.inputReport.detectedFormat,
            analysis?.sourceFormat,
            item?.report?.sourceFormat,
        ]
        .compactMap { $0?.lowercased() }
        .contains { $0.contains("kfx") }
    }

    private var availableTabs: [InspectorTab] {
        InspectorTab.allCases.filter { $0 != .kfx || isKFXInput }
    }

    private var effectiveSelection: InspectorTab {
        availableTabs.contains(selection) ? selection : .input
    }

    private var inspectorSelection: Binding<InspectorTab> {
        Binding(
            get: { availableTabs.contains(selection) ? selection : .input },
            set: { selection = $0 }
        )
    }

    var body: some View {
        VStack(spacing: 10) {
            Picker(FolioL10n.string("ui.inspector", default: "Inspector"), selection: inspectorSelection) {
                ForEach(availableTabs) { section in
                    Text(section.title).tag(section)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(.horizontal, 12)
            .padding(.top, 12)

            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if let selectedFont {
                        SelectedFontDetails(font: selectedFont)
                    }
                    switch effectiveSelection {
                    case .output:
                        OutputInspectorPage(item: item)
                    case .compatibility:
                        CompatibilityInspectorPage(analysis: analysis, preview: preview)
                    case .input:
                        InputInspectorPage(
                            report: report,
                            analysis: analysis,
                            convertedInput: item?.report?.inputReport,
                            detailError: detailError
                        )
                    case .kfx:
                        KFXInspectorPage(
                            report: report,
                            analysis: analysis,
                            convertedInput: item?.report?.inputReport
                        )
                    case .diagnostics:
                        DiagnosticsInspectorPage(item: item, report: report, previewError: previewError)
                    }
                }
                .padding(.horizontal, 14)
                .padding(.bottom, 16)
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }
}

private struct OutputInspectorPage: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel
    let item: BookItem?

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            InspectorSection(title: FolioL10n.string("ui.output", default: "Output")) {
                InspectorRow(label: FolioL10n.string("ui.target", default: "Target"), value: queue.target.displayName)
                InspectorRow(label: FolioL10n.string("ui.mode", default: "Mode"), value: queue.degradationMode.displayName)
                if let output = item?.report?.outputReport {
                    InspectorRow(label: FolioL10n.string("inspector.last_output", default: "Last output"), value: output.path)
                    InspectorRow(label: FolioL10n.string("ui.size", default: "Size"), value: ByteCountFormatter.string(fromByteCount: Int64(clamping: output.size), countStyle: .file))
                    InspectorRow(
                        label: FolioL10n.string("inspector.validated", default: "Validated"),
                        value: FolioL10n.string(output.validated ? "inspector.yes" : "inspector.no", default: output.validated ? "Yes" : "No")
                    )
                } else {
                    InspectorRow(label: FolioL10n.string("inspector.destination", default: "Destination"), value: queue.outputDestinationDescription)
                }
            }

            InspectorSection(title: FolioL10n.string("inspector.destination", default: "Destination")) {
                Text(queue.outputDestinationDescription)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                HStack {
                    Button(FolioL10n.string("ui.choose_folder", default: "Choose Folder…")) { queue.chooseOutputFolder() }
                    if queue.outputFolder != nil {
                        Button(FolioL10n.string("ui.reset", default: "Reset"), role: .destructive) { queue.resetOutputFolder() }
                    }
                }
            }

            InspectorSection(title: FolioL10n.string("ui.encoding", default: "Encoding")) {
                InspectorPickerRow(title: FolioL10n.string("ui.compression", default: "Compression"), selection: $queue.compression) {
                    ForEach(FolioCompression.allCases) { value in Text(value.displayName).tag(value) }
                }
                Toggle(FolioL10n.string("ui.deterministic_output", default: "Deterministic output"), isOn: $queue.deterministic)
                InspectorPickerRow(title: FolioL10n.string("ui.batch_policy", default: "Batch policy"), selection: $queue.batchMode) {
                    ForEach(FolioBatchMode.allCases) { value in Text(value.displayName).tag(value) }
                }
                Text(queue.batchMode == .strict
                    ? FolioL10n.string("inspector.batch_strict", default: "Strict batch rolls back successful outputs if any approved item fails.")
                    : FolioL10n.string("inspector.batch_best_effort", default: "Best effort continues with other books when one item fails."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            InspectorSection(title: FolioL10n.string("inspector.text_import", default: "Text Import")) {
                InspectorPickerRow(title: FolioL10n.string("ui.structure", default: "Structure"), selection: $queue.textImportMode) {
                    ForEach(FolioTextImportMode.allCases) { value in
                        Text(value.title).tag(value)
                    }
                }
                InspectorPickerRow(title: FolioL10n.string("inspector.paragraphs", default: "Paragraphs"), selection: $queue.paragraphMode) {
                    ForEach(FolioParagraphMode.allCases) { value in
                        Text(value.title).tag(value)
                    }
                }
                Picker(FolioL10n.string("ui.encoding_override", default: "Encoding override"), selection: $queue.textEncodingOverride) {
                    Text(FolioL10n.string("ui.automatic", default: "Automatic")).tag(nil as FolioTextEncoding?)
                    ForEach(FolioTextEncoding.allCases) { value in
                        Text(value.title).tag(Optional(value))
                    }
                }
                .font(.caption)
                TextField(FolioL10n.string("ui.title_override", default: "Title override"), text: $queue.textTitleOverride)
                    .textFieldStyle(.roundedBorder)
                TextField(FolioL10n.string("ui.author_override", default: "Author override"), text: $queue.textAuthorOverride)
                    .textFieldStyle(.roundedBorder)
                Text(FolioL10n.string("ui.these_settings_apply_to_txt_inputs_ambiguous_encodings_stay", default: "These settings apply to TXT inputs. Ambiguous encodings stay unresolved until you select one."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

private struct CompatibilityInspectorPage: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel
    let analysis: FolioAnalysisReport?
    let preview: FolioReaderPreviewBundle?

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            InspectorSection(title: FolioL10n.string("inspector.compatibility_plan", default: "Compatibility Plan")) {
                if let plan = analysis?.plan {
                    InspectorRow(label: FolioL10n.string("ui.source", default: "Source"), value: analysis?.sourceFormat ?? "—")
                    InspectorRow(label: FolioL10n.string("ui.target", default: "Target"), value: plan.target)
                    InspectorRow(label: FolioL10n.string("ui.mode", default: "Mode"), value: plan.mode.displayName)
                    InspectorRow(label: FolioL10n.string("inspector.quality", default: "Quality"), value: plan.quality.displayName)
                    InspectorRow(
                        label: FolioL10n.string("ui.result", default: "Result"),
                        value: FolioL10n.string(
                            plan.blocked ? "status.blocked" : "status.ready",
                            default: plan.blocked ? "Blocked" : "Ready"
                        )
                    )
                    InspectorRow(label: FolioL10n.string("inspector.planned_changes", default: "Planned changes"), value: "\(plan.items.count)")
                    if !plan.items.isEmpty {
                        Divider()
                        ForEach(plan.items) { change in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(change.feature).font(.caption.weight(.semibold))
                                Text("\(change.sourceRepresentation) → \(change.selectedFallback)")
                                    .font(.caption)
                                Text(change.reason)
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                            .padding(.vertical, 3)
                        }
                    }
                } else if let preview {
                    if let target = preview.target {
                        InspectorRow(label: FolioL10n.string("ui.target", default: "Target"), value: target.format)
                    }
                    if let degradation = preview.degradation {
                        InspectorRow(label: FolioL10n.string("inspector.quality", default: "Quality"), value: degradation.quality.displayName)
                    }
                    InspectorRow(label: FolioL10n.string("inspector.target_loss", default: "Target loss"), value: "\(preview.targetLoss.count)")
                    Text(FolioL10n.string("ui.run_check_to_save_a_compatibility_plan_for_conversion", default: "Run Check to save a compatibility plan for conversion."))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    Text(FolioL10n.string("ui.run_check_or_generate_preview_to_inspect_target_compatibility", default: "Run Check or Generate Preview to inspect target compatibility."))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            InspectorSection(title: FolioL10n.string("inspector.fallback_options", default: "Fallback Options")) {
                Toggle(FolioL10n.string("ui.linearize_complex_tables", default: "Linearize complex tables"), isOn: $queue.linearizeComplexTables)
                Toggle(FolioL10n.string("ui.strip_legacy_incompatible_embedded_fonts", default: "Strip legacy-incompatible embedded fonts"), isOn: $queue.stripEmbeddedFonts)
                Text(FolioL10n.string("ui.changing_these_options_clears_the_existing_check_result", default: "Changing these options clears the existing check result."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

private struct InputInspectorPage: View {
    let report: FolioInspectReport?
    let analysis: FolioAnalysisReport?
    let convertedInput: FolioInputReport?
    let detailError: String?

    private var input: FolioInputReport? {
        report?.inputReport ?? analysis?.inputReport ?? convertedInput
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            InspectorSection(title: FolioL10n.string("inspector.input_parser", default: "Input Parser")) {
                if let input {
                    InspectorRow(label: FolioL10n.string("inspector.detected_format", default: "Detected format"), value: input.detectedFormat)
                    InspectorRow(label: FolioL10n.string("inspector.parser", default: "Parser"), value: input.parser)
                    InspectorRow(label: FolioL10n.string("inspector.containers", default: "Containers"), value: "\(input.containerCount)")
                    InspectorRow(label: FolioL10n.string("inspector.entities", default: "Entities"), value: "\(input.entityCount)")
                    InspectorRow(label: FolioL10n.string("inspector.fragments", default: "Fragments"), value: "\(input.fragmentCount)")
                    InspectorRow(label: FolioL10n.string("inspector.symbols", default: "Symbols"), value: "\(input.symbolCount)")
                    InspectorRow(label: FolioL10n.string("inspector.documents", default: "Documents"), value: "\(input.documentCount)")
                    InspectorRow(label: FolioL10n.string("inspector.resources", default: "Resources"), value: "\(input.resourceCount)")
                    InspectorRow(label: "DRM", value: FolioL10n.string(input.drmDetected ? "inspector.detected" : "inspector.not_detected", default: input.drmDetected ? "Detected" : "Not detected"))
                    if let text = input.text {
                        TextImportDetails(report: text)
                    }
                } else if let detailError {
                    Text(detailError).font(.caption).foregroundStyle(.red)
                } else {
                    Text(FolioL10n.string("ui.parser_details_are_available_when_a_book_is_selected", default: "Parser details are available when a book is selected."))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            InspectorSection(title: FolioL10n.string("ui.semantic_ir", default: "Semantic IR")) {
                if let semantic = report?.semanticReport ?? analysis?.semanticReport {
                    InspectorRow(label: FolioL10n.string("inspector.validation", default: "Validation"), value: FolioL10n.string(semantic.valid ? "inspector.valid" : "inspector.invalid", default: semantic.valid ? "Valid" : "Invalid"))
                    InspectorRow(label: FolioL10n.string("inspector.documents", default: "Documents"), value: "\(semantic.documentCount)")
                    InspectorRow(label: FolioL10n.string("inspector.resources", default: "Resources"), value: "\(semantic.resourceCount)")
                    InspectorRow(label: FolioL10n.string("inspector.features", default: "Features"), value: "\(semantic.featureCount)")
                    if !semantic.inputLoss.isEmpty {
                        InspectorList(title: FolioL10n.string("inspector.input_limitations", default: "Input limitations"), values: semantic.inputLoss)
                    }
                }
            }

            if let input {
                if !input.unknownFeatures.isEmpty {
                    InspectorSection(title: FolioL10n.string("inspector.unknown_features", default: "Unknown Features")) {
                        InspectorList(title: "", values: input.unknownFeatures)
                    }
                }
                if !input.recoveryActions.isEmpty {
                    InspectorSection(title: FolioL10n.string("inspector.recovery_actions", default: "Recovery Actions")) {
                        InspectorList(title: "", values: input.recoveryActions)
                    }
                }
            }
        }
    }
}

private struct KFXInspectorPage: View {
    let report: FolioInspectReport?
    let analysis: FolioAnalysisReport?
    let convertedInput: FolioInputReport?

    private var input: FolioInputReport? {
        report?.inputReport ?? analysis?.inputReport ?? convertedInput
    }

    private var semantic: FolioSemanticReport? {
        report?.semanticReport ?? analysis?.semanticReport
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            InspectorSection(title: FolioL10n.string("inspector.amazon_kfx_import", default: "Amazon KFX Import")) {
                if let input {
                    InspectorRow(
                        label: FolioL10n.string("inspector.status", default: "Status"),
                        value: input.drmDetected
                            ? FolioL10n.string("inspector.blocked_drm", default: "Blocked — DRM detected")
                            : (input.inputLoss.isEmpty && input.unknownFeatures.isEmpty
                                ? FolioL10n.string("inspector.readable_ir_import", default: "Readable IR import")
                                : FolioL10n.string("inspector.readable_with_limitations", default: "Readable with explicit limitations"))
                    )
                    InspectorRow(label: FolioL10n.string("inspector.parser", default: "Parser"), value: input.parser)
                    InspectorRow(label: FolioL10n.string("inspector.native_containers", default: "Native containers"), value: "\(input.containerCount)")
                    InspectorRow(label: FolioL10n.string("inspector.native_entities", default: "Native entities"), value: "\(input.entityCount)")
                    InspectorRow(label: FolioL10n.string("inspector.native_fragments", default: "Native fragments"), value: "\(input.fragmentCount)")
                    InspectorRow(label: FolioL10n.string("inspector.native_symbols", default: "Native symbols"), value: "\(input.symbolCount)")
                    InspectorRow(label: "DRM", value: FolioL10n.string(input.drmDetected ? "inspector.drm_no_decryption" : "inspector.not_detected", default: input.drmDetected ? "Detected; no decryption attempted" : "Not detected"))
                    if let semantic {
                        InspectorRow(label: FolioL10n.string("inspector.ir_validation", default: "IR validation"), value: FolioL10n.string(semantic.valid ? "inspector.valid" : "inspector.invalid", default: semantic.valid ? "Valid" : "Invalid"))
                    }
                } else {
                    Text(FolioL10n.string("ui.kfx_evidence_is_available_after_selecting_a_real_amazon", default: "KFX evidence is available after selecting a real Amazon KFX input."))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            if let summary = input?.kfxSummary {
                InspectorSection(title: FolioL10n.string("inspector.recovered_ir_evidence", default: "Recovered Semantic IR Evidence")) {
                    InspectorRow(label: FolioL10n.string("inspector.toc_entries", default: "TOC entries"), value: "\(summary.tocEntryCount)")
                    InspectorRow(
                        label: FolioL10n.string("inspector.landmarks", default: "Landmarks"),
                        value: summary.landmarkEntryCount == 0
                            ? FolioL10n.string("inspector.zero_unmapped", default: "0 — not present or not mapped")
                            : "\(summary.landmarkEntryCount)"
                    )
                    InspectorRow(
                        label: FolioL10n.string("inspector.page_list", default: "Page list"),
                        value: summary.pageListEntryCount == 0
                            ? FolioL10n.string("inspector.zero_unmapped", default: "0 — not present or not mapped")
                            : "\(summary.pageListEntryCount)"
                    )
                    InspectorRow(label: FolioL10n.string("inspector.start_location", default: "Start location"), value: FolioL10n.string(summary.hasStartLocation ? "inspector.mapped" : "inspector.not_mapped", default: summary.hasStartLocation ? "Mapped" : "Not mapped"))
                    InspectorRow(
                        label: FolioL10n.string("inspector.images", default: "Images"),
                        value: FolioL10n.format("inspector.image_summary", default: "IR image nodes: %@ · alt text present: %@ · empty: %@", String(summary.imageCount), String(summary.imageAltPresentCount), String(summary.imageAltEmptyCount))
                    )
                    InspectorRow(
                        label: FolioL10n.string("inspector.links", default: "Links"),
                        value: FolioL10n.format("inspector.link_summary", default: "IR link nodes: %@ · anchors: %@ · graph edges: %@", String(summary.linkCount), String(summary.anchorCount), String(summary.anchorGraphEdgeCount))
                    )
                    InspectorRow(
                        label: FolioL10n.string("ui.styles", default: "Styles"),
                        value: FolioL10n.format("inspector.style_summary", default: "Interned styles: %@ · nodes with properties: %@", String(summary.styleCount), String(summary.nonDefaultStyleNodeCount))
                    )
                    InspectorRow(label: FolioL10n.string("inspector.font_faces", default: "Font faces"), value: "\(summary.fontFaceCount)")
                    if !summary.featureCounts.isEmpty {
                        Divider()
                        Text(FolioL10n.string("ui.ir_node_kinds", default: "IR node kinds")).font(.caption.weight(.medium))
                        ForEach(summary.featureCounts.sorted(by: { $0.key < $1.key }), id: \.key) { feature, count in
                            InspectorRow(label: feature.capitalized, value: "\(count)")
                        }
                    }
                }
            }

            InspectorSection(title: FolioL10n.string("inspector.current_fidelity_boundary", default: "Current Fidelity Boundary")) {
                Text(FolioL10n.string("ui.the_kfx_page_reports_evidence_that_reached_the_shared", default: "The KFX page reports evidence that reached the shared Semantic IR. It does not claim Amazon-rendered equivalence or infer unresolved source relationships."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Text(FolioL10n.string("ui.paragraph_and_inline_style_recovery_image_roles_alt_text", default: "Paragraph and inline style recovery, image roles/alt text, footnote semantics, and some navigation forms remain explicit input limitations. DRM-protected content is never decrypted."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if let input {
                if !input.inputLoss.isEmpty {
                    InspectorSection(title: FolioL10n.string("inspector.kfx_input_loss", default: "KFX Input Loss")) {
                        InspectorList(title: "", values: input.inputLoss)
                    }
                }
                if !input.unknownFeatures.isEmpty {
                    InspectorSection(title: FolioL10n.string("inspector.unmapped_native_features", default: "Unmapped Native Features")) {
                        InspectorList(title: "", values: input.unknownFeatures)
                    }
                }
                if !input.recoveryActions.isEmpty {
                    InspectorSection(title: FolioL10n.string("inspector.recovery_actions", default: "Recovery Actions")) {
                        InspectorList(title: "", values: input.recoveryActions)
                    }
                }
            }
        }
    }
}

private struct TextImportDetails: View {
    let report: FolioTextImportReport

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Divider()
            Text(FolioL10n.string("ui.txt_analysis", default: "TXT Analysis")).font(.subheadline.weight(.semibold))
            InspectorRow(label: FolioL10n.string("ui.encoding", default: "Encoding"), value: report.encoding.selected ?? FolioL10n.string("inspector.needs_override", default: "Needs override"))
            InspectorRow(label: FolioL10n.string("inspector.encoding_confidence", default: "Encoding confidence"), value: report.encoding.confidence.capitalized)
            if !report.encoding.candidates.isEmpty {
                InspectorList(
                    title: FolioL10n.string("inspector.encoding_candidates", default: "Encoding candidates"),
                    values: report.encoding.candidates.map { "\($0.encoding) · score \($0.score)" }
                )
            }
            if let paragraphs = report.paragraphAnalysis {
                InspectorRow(
                    label: FolioL10n.string("inspector.paragraph_mode", default: "Paragraph mode"),
                    value: "\(paragraphs.selected.replacingOccurrences(of: "_", with: " ").capitalized) · \(paragraphs.confidencePercent)%"
                )
                if paragraphs.protectedPreformatted {
                    InspectorRow(label: FolioL10n.string("inspector.preformatted_text", default: "Preformatted text"), value: FolioL10n.string("inspector.protected_from_joining", default: "Protected from joining"))
                }
            }
            InspectorRow(label: FolioL10n.string("inspector.detected_headings", default: "Detected headings"), value: FolioL10n.format("inspector.heading_candidates", default: "Root headings: %@ · rejected candidates: %@", String(report.structure.roots.count), String(report.structure.rejected.count)))
            ForEach(report.structure.roots) { node in
                StructureInspectorNode(node: node, depth: 0)
            }
            if let title = report.metadataGuess.title {
                InspectorRow(label: FolioL10n.string("inspector.guessed_title", default: "Guessed title"), value: title)
            }
            if let author = report.metadataGuess.author {
                InspectorRow(label: FolioL10n.string("inspector.guessed_author", default: "Guessed author"), value: author)
            }
            if !report.diagnostics.isEmpty {
                InspectorList(title: FolioL10n.string("inspector.parser_notes", default: "Parser notes"), values: report.diagnostics)
            }
            if !report.inputLoss.isEmpty {
                InspectorList(title: FolioL10n.string("inspector.known_input_loss", default: "Known input loss"), values: report.inputLoss)
            }
        }
    }
}

private struct StructureInspectorNode: View {
    let node: FolioStructureNode
    let depth: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text("\(node.candidate.kind.capitalized): \(node.candidate.text)")
                .font(.caption.weight(.medium))
            Text(FolioL10n.format(
                "error.line_confidence",
                default: "Line %@ · Confidence %@%% · %@",
                String(node.candidate.lineIndex + 1),
                String(node.candidate.confidencePercent),
                node.candidate.detector
            ))
                .font(.caption2)
                .foregroundStyle(.secondary)
            ForEach(node.children) { child in
                StructureInspectorNode(node: child, depth: depth + 1)
            }
        }
        .padding(.leading, CGFloat(depth) * 12)
        .fixedSize(horizontal: false, vertical: true)
    }
}

private struct DiagnosticsInspectorPage: View {
    let item: BookItem?
    let report: FolioInspectReport?
    let previewError: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let previewError {
                InspectorSection(title: FolioL10n.string("ui.preview", default: "Preview")) {
                    Label(previewError, systemImage: FolioSymbol.warning.name)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if let report, !report.diagnostics.isEmpty {
                InspectorSection(title: FolioL10n.string("inspector.input_diagnostics", default: "Input Diagnostics")) {
                    ForEach(report.diagnostics) { diagnostic in
                        DiagnosticInspectorRow(diagnostic: diagnostic)
                    }
                }
            }
            DiagnosticsView(item: item)
        }
    }
}

private struct SelectedFontDetails: View {
    let font: SemanticFontRow

    var body: some View {
        InspectorSection(title: FolioL10n.string("inspector.selected_font", default: "Selected Font")) {
            InspectorRow(label: FolioL10n.string("ui.family", default: "Family"), value: font.family)
            InspectorRow(label: FolioL10n.string("ui.style", default: "Style"), value: font.style)
            InspectorRow(label: FolioL10n.string("inspector.format", default: "Format"), value: font.mediaType)
            InspectorRow(label: FolioL10n.string("ui.size", default: "Size"), value: font.sizeDescription)
            InspectorRow(label: FolioL10n.string("inspector.glyph_coverage", default: "Glyph coverage"), value: FolioL10n.string("inspector.not_provided_by_source", default: "Not provided by source"))
            InspectorRow(label: FolioL10n.string("inspector.used_by", default: "Used by"), value: font.usedBy)
            InspectorRow(label: FolioL10n.string("inspector.license", default: "License"), value: FolioL10n.string("inspector.not_declared_in_source_package", default: "Not declared in source package"))
            InspectorRow(label: FolioL10n.string("inspector.package_path", default: "Package path"), value: font.path)
        }
    }
}

private struct InspectorSection<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            Text(title).font(.headline)
            Divider()
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct InspectorRow: View {
    let label: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.caption2).foregroundStyle(.secondary)
            Text(value)
                .font(.caption)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct InspectorPickerRow<Value: Hashable, Options: View>: View {
    let title: String
    @Binding var selection: Value
    let options: Options

    init(title: String, selection: Binding<Value>, @ViewBuilder options: () -> Options) {
        self.title = title
        self._selection = selection
        self.options = options()
    }

    var body: some View {
        HStack {
            Text(title).font(.caption).foregroundStyle(.secondary)
            Spacer(minLength: 8)
            Picker(title, selection: $selection) { options }
                .labelsHidden()
                .pickerStyle(.menu)
        }
    }
}

private struct InspectorList: View {
    let title: String
    let values: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if !title.isEmpty { Text(title).font(.caption.weight(.medium)) }
            ForEach(Array(values.enumerated()), id: \.offset) { _, value in
                Label(value, systemImage: FolioSymbol.alertCircle.name)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

private struct DiagnosticInspectorRow: View {
    let diagnostic: FolioDiagnostic

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(folioSymbol: diagnostic.severity.rawValue == "Error" ? .error : .warning)
                .foregroundStyle(diagnostic.severity.rawValue == "Error" ? .red : .orange)
            VStack(alignment: .leading, spacing: 3) {
                Text(diagnostic.code).font(.caption.monospaced().weight(.semibold))
                Text(diagnostic.message).font(.caption).fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}
