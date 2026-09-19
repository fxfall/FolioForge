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
        case .output: "Output"
        case .compatibility: "Compatibility"
        case .input: "Input"
        case .kfx: "KFX"
        case .diagnostics: "Diagnostics"
        }
    }
}

struct Phase3InspectorView: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel
    let item: BookItem?
    let report: FolioInspectReport?
    let analysis: FolioAnalysisReport?
    let preview: FolioPreviewBundle?
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
            Picker("Inspector", selection: inspectorSelection) {
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
            InspectorSection(title: "Output") {
                InspectorRow(label: "Target", value: queue.target.displayName)
                InspectorRow(label: "Mode", value: queue.degradationMode.displayName)
                if let output = item?.report?.outputReport {
                    InspectorRow(label: "Last output", value: output.path)
                    InspectorRow(label: "Size", value: ByteCountFormatter.string(fromByteCount: Int64(clamping: output.size), countStyle: .file))
                    InspectorRow(label: "Validated", value: output.validated ? "Yes" : "No")
                } else {
                    InspectorRow(label: "Destination", value: queue.outputDestinationDescription)
                }
            }

            InspectorSection(title: "Destination") {
                Text(queue.outputDestinationDescription)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                HStack {
                    Button("Choose Folder…") { queue.chooseOutputFolder() }
                    if queue.outputFolder != nil {
                        Button("Reset", role: .destructive) { queue.resetOutputFolder() }
                    }
                }
            }

            InspectorSection(title: "Encoding") {
                InspectorPickerRow(title: "Compression", selection: $queue.compression) {
                    ForEach(FolioCompression.allCases) { value in Text(value.displayName).tag(value) }
                }
                Toggle("Deterministic output", isOn: $queue.deterministic)
                InspectorPickerRow(title: "Batch policy", selection: $queue.batchMode) {
                    ForEach(FolioBatchMode.allCases) { value in Text(value.displayName).tag(value) }
                }
                Text(queue.batchMode == .strict
                    ? "Strict batch rolls back successful outputs if any approved item fails."
                    : "Best effort continues with other books when one item fails.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            InspectorSection(title: "Text Import") {
                InspectorPickerRow(title: "Structure", selection: $queue.textImportMode) {
                    ForEach(FolioTextImportMode.allCases) { value in
                        Text(value.title).tag(value)
                    }
                }
                InspectorPickerRow(title: "Paragraphs", selection: $queue.paragraphMode) {
                    ForEach(FolioParagraphMode.allCases) { value in
                        Text(value.title).tag(value)
                    }
                }
                Picker("Encoding override", selection: $queue.textEncodingOverride) {
                    Text("Automatic").tag(nil as FolioTextEncoding?)
                    ForEach(FolioTextEncoding.allCases) { value in
                        Text(value.title).tag(Optional(value))
                    }
                }
                .font(.caption)
                TextField("Title override", text: $queue.textTitleOverride)
                    .textFieldStyle(.roundedBorder)
                TextField("Author override", text: $queue.textAuthorOverride)
                    .textFieldStyle(.roundedBorder)
                Text("These settings apply to TXT inputs. Ambiguous encodings stay unresolved until you select one.")
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
    let preview: FolioPreviewBundle?

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            InspectorSection(title: "Compatibility Plan") {
                if let plan = analysis?.plan {
                    InspectorRow(label: "Source", value: analysis?.sourceFormat ?? "—")
                    InspectorRow(label: "Target", value: plan.target)
                    InspectorRow(label: "Mode", value: plan.mode.displayName)
                    InspectorRow(label: "Quality", value: plan.quality.displayName)
                    InspectorRow(label: "Result", value: plan.blocked ? "Blocked" : "Ready")
                    InspectorRow(label: "Planned changes", value: "\(plan.items.count)")
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
                    InspectorRow(label: "Target", value: preview.target.format)
                    InspectorRow(label: "Quality", value: preview.degradation.quality.displayName)
                    InspectorRow(label: "Target loss", value: "\(preview.targetLoss.count)")
                    Text("Run Check to save a compatibility plan for conversion.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    Text("Run Check or Generate Preview to inspect target compatibility.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            InspectorSection(title: "Fallback Options") {
                Toggle("Linearize complex tables", isOn: $queue.linearizeComplexTables)
                Toggle("Strip legacy-incompatible embedded fonts", isOn: $queue.stripEmbeddedFonts)
                Text("Changing these options clears the existing check result.")
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
            InspectorSection(title: "Input Parser") {
                if let input {
                    InspectorRow(label: "Detected format", value: input.detectedFormat)
                    InspectorRow(label: "Parser", value: input.parser)
                    InspectorRow(label: "Containers", value: "\(input.containerCount)")
                    InspectorRow(label: "Entities", value: "\(input.entityCount)")
                    InspectorRow(label: "Fragments", value: "\(input.fragmentCount)")
                    InspectorRow(label: "Symbols", value: "\(input.symbolCount)")
                    InspectorRow(label: "Documents", value: "\(input.documentCount)")
                    InspectorRow(label: "Resources", value: "\(input.resourceCount)")
                    InspectorRow(label: "DRM", value: input.drmDetected ? "Detected" : "Not detected")
                    if let text = input.text {
                        TextImportDetails(report: text)
                    }
                } else if let detailError {
                    Text(detailError).font(.caption).foregroundStyle(.red)
                } else {
                    Text("Parser details are available when a book is selected.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            InspectorSection(title: "Semantic IR") {
                if let semantic = report?.semanticReport ?? analysis?.semanticReport {
                    InspectorRow(label: "Validation", value: semantic.valid ? "Valid" : "Invalid")
                    InspectorRow(label: "Documents", value: "\(semantic.documentCount)")
                    InspectorRow(label: "Resources", value: "\(semantic.resourceCount)")
                    InspectorRow(label: "Features", value: "\(semantic.featureCount)")
                    if !semantic.inputLoss.isEmpty {
                        InspectorList(title: "Input limitations", values: semantic.inputLoss)
                    }
                }
            }

            if let input {
                if !input.unknownFeatures.isEmpty {
                    InspectorSection(title: "Unknown Features") {
                        InspectorList(title: "", values: input.unknownFeatures)
                    }
                }
                if !input.recoveryActions.isEmpty {
                    InspectorSection(title: "Recovery Actions") {
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
            InspectorSection(title: "Amazon KFX Import") {
                if let input {
                    InspectorRow(
                        label: "Status",
                        value: input.drmDetected
                            ? "Blocked — DRM detected"
                            : (input.inputLoss.isEmpty && input.unknownFeatures.isEmpty
                                ? "Readable IR import"
                                : "Readable with explicit limitations")
                    )
                    InspectorRow(label: "Parser", value: input.parser)
                    InspectorRow(label: "Native containers", value: "\(input.containerCount)")
                    InspectorRow(label: "Native entities", value: "\(input.entityCount)")
                    InspectorRow(label: "Native fragments", value: "\(input.fragmentCount)")
                    InspectorRow(label: "Native symbols", value: "\(input.symbolCount)")
                    InspectorRow(label: "DRM", value: input.drmDetected ? "Detected; no decryption attempted" : "Not detected")
                    if let semantic {
                        InspectorRow(label: "IR validation", value: semantic.valid ? "Valid" : "Invalid")
                    }
                } else {
                    Text("KFX evidence is available after selecting a real Amazon KFX input.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            if let summary = input?.kfxSummary {
                InspectorSection(title: "Recovered Semantic IR Evidence") {
                    InspectorRow(label: "TOC entries", value: "\(summary.tocEntryCount)")
                    InspectorRow(
                        label: "Landmarks",
                        value: summary.landmarkEntryCount == 0
                            ? "0 — not present or not mapped"
                            : "\(summary.landmarkEntryCount)"
                    )
                    InspectorRow(
                        label: "Page list",
                        value: summary.pageListEntryCount == 0
                            ? "0 — not present or not mapped"
                            : "\(summary.pageListEntryCount)"
                    )
                    InspectorRow(label: "Start location", value: summary.hasStartLocation ? "Mapped" : "Not mapped")
                    InspectorRow(
                        label: "Images",
                        value: "\(summary.imageCount) IR image nodes · alt \(summary.imageAltPresentCount) present / \(summary.imageAltEmptyCount) empty"
                    )
                    InspectorRow(
                        label: "Links",
                        value: "\(summary.linkCount) IR link nodes · \(summary.anchorCount) anchors · \(summary.anchorGraphEdgeCount) graph edges"
                    )
                    InspectorRow(
                        label: "Styles",
                        value: "\(summary.styleCount) interned · \(summary.nonDefaultStyleNodeCount) nodes with properties"
                    )
                    InspectorRow(label: "Font faces", value: "\(summary.fontFaceCount)")
                    if !summary.featureCounts.isEmpty {
                        Divider()
                        Text("IR node kinds").font(.caption.weight(.medium))
                        ForEach(summary.featureCounts.sorted(by: { $0.key < $1.key }), id: \.key) { feature, count in
                            InspectorRow(label: feature.capitalized, value: "\(count)")
                        }
                    }
                }
            }

            InspectorSection(title: "Current Fidelity Boundary") {
                Text("The KFX page reports evidence that reached the shared Semantic IR. It does not claim Amazon-rendered equivalence or infer unresolved source relationships.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Text("Paragraph and inline style recovery, image roles/alt text, footnote semantics, and some navigation forms remain explicit input limitations. DRM-protected content is never decrypted.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if let input {
                if !input.inputLoss.isEmpty {
                    InspectorSection(title: "KFX Input Loss") {
                        InspectorList(title: "", values: input.inputLoss)
                    }
                }
                if !input.unknownFeatures.isEmpty {
                    InspectorSection(title: "Unmapped Native Features") {
                        InspectorList(title: "", values: input.unknownFeatures)
                    }
                }
                if !input.recoveryActions.isEmpty {
                    InspectorSection(title: "Recovery Actions") {
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
            Text("TXT Analysis").font(.subheadline.weight(.semibold))
            InspectorRow(label: "Encoding", value: report.encoding.selected ?? "Needs override")
            InspectorRow(label: "Encoding confidence", value: report.encoding.confidence.capitalized)
            if !report.encoding.candidates.isEmpty {
                InspectorList(
                    title: "Encoding candidates",
                    values: report.encoding.candidates.map { "\($0.encoding) · score \($0.score)" }
                )
            }
            if let paragraphs = report.paragraphAnalysis {
                InspectorRow(
                    label: "Paragraph mode",
                    value: "\(paragraphs.selected.replacingOccurrences(of: "_", with: " ").capitalized) · \(paragraphs.confidencePercent)%"
                )
                if paragraphs.protectedPreformatted {
                    InspectorRow(label: "Preformatted text", value: "Protected from joining")
                }
            }
            InspectorRow(label: "Detected headings", value: "\(report.structure.roots.count) roots · \(report.structure.rejected.count) rejected candidates")
            ForEach(report.structure.roots) { node in
                StructureInspectorNode(node: node, depth: 0)
            }
            if let title = report.metadataGuess.title {
                InspectorRow(label: "Guessed title", value: title)
            }
            if let author = report.metadataGuess.author {
                InspectorRow(label: "Guessed author", value: author)
            }
            if !report.diagnostics.isEmpty {
                InspectorList(title: "Parser notes", values: report.diagnostics)
            }
            if !report.inputLoss.isEmpty {
                InspectorList(title: "Known input loss", values: report.inputLoss)
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
            Text("Line \(node.candidate.lineIndex + 1) · \(node.candidate.confidencePercent)% · \(node.candidate.detector)")
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
                InspectorSection(title: "Preview") {
                    Label(previewError, systemImage: FolioSymbol.warning.name)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if let report, !report.diagnostics.isEmpty {
                InspectorSection(title: "Input Diagnostics") {
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
        InspectorSection(title: "Selected Font") {
            InspectorRow(label: "Family", value: font.family)
            InspectorRow(label: "Style", value: font.style)
            InspectorRow(label: "Format", value: font.mediaType)
            InspectorRow(label: "Size", value: font.sizeDescription)
            InspectorRow(label: "Glyph coverage", value: "Not provided by source")
            InspectorRow(label: "Used by", value: font.usedBy)
            InspectorRow(label: "License", value: "Not declared in source package")
            InspectorRow(label: "Package path", value: font.path)
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
