import AppKit
import SwiftUI
import UniformTypeIdentifiers

enum EditorSection: String, CaseIterable, Identifiable, Hashable {
    case summary
    case metadata
    case appearance
    case structure

    var id: String { rawValue }
    var title: String {
        switch self {
        case .summary: FolioL10n.string("editor.section.summary", default: "Summary")
        case .metadata: FolioL10n.string("ui.metadata", default: "Metadata")
        case .appearance: FolioL10n.string("editor.section.appearance", default: "Appearance")
        case .structure: FolioL10n.string("ui.structure", default: "Structure")
        }
    }
    var symbol: FolioSymbol {
        switch self {
        case .summary: .summary
        case .metadata: .metadata
        case .appearance: .appearance
        case .structure: .structure
        }
    }
}

enum AppearancePage: String, CaseIterable, Identifiable, Hashable {
    case cover
    case typography
    case fonts
    case styles

    var id: String { rawValue }
    var title: String {
        switch self {
        case .cover: FolioL10n.string("ui.cover", default: "Cover")
        case .typography: FolioL10n.string("ui.typography", default: "Typography")
        case .fonts: FolioL10n.string("ui.fonts", default: "Fonts")
        case .styles: FolioL10n.string("ui.styles", default: "Styles")
        }
    }
    var symbol: FolioSymbol {
        switch self {
        case .cover: .cover
        case .typography: .typographySize
        case .fonts: .onlineFont
        case .styles: .styles
        }
    }
}

enum BulkEditSection: String, CaseIterable, Identifiable, Hashable {
    case output
    case typography
    case fonts
    case styles

    var id: String { rawValue }
    var title: String {
        switch self {
        case .output: FolioL10n.string("ui.output", default: "Output")
        case .typography: FolioL10n.string("ui.typography", default: "Typography")
        case .fonts: FolioL10n.string("ui.fonts", default: "Fonts")
        case .styles: FolioL10n.string("ui.styles", default: "Styles")
        }
    }
}

@MainActor
final class BulkEditDraft: ObservableObject {
    @Published var section: BulkEditSection = .typography
    @Published var applyFontFamily = false
    @Published var fontFamily = "Georgia"
    @Published var applyBodySize = false
    @Published var bodySize = "16px"
    @Published var applyLineHeight = false
    @Published var lineHeight = "1.5"
    @Published var applyFontPolicy = false
    @Published var stripEmbeddedFonts = false
    @Published var applyHeadingStyle = false
    @Published var headingStyle = "center"

    var hasChanges: Bool {
        (applyFontFamily && !fontFamily.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            || (applyBodySize && !bodySize.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            || (applyLineHeight && !lineHeight.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            || applyFontPolicy
            || applyHeadingStyle
    }

    var payload: BulkEditPayload {
        var typography = FolioTypographyEdit()
        if applyFontFamily { typography.fontFamily = fontFamily.isEmpty ? nil : fontFamily }
        if applyBodySize { typography.bodyFontSize = bodySize.isEmpty ? nil : bodySize }
        if applyLineHeight { typography.lineHeight = lineHeight.isEmpty ? nil : lineHeight }
        let hasTypography = typography.fontFamily != nil || typography.bodyFontSize != nil || typography.lineHeight != nil
        let style = applyHeadingStyle
            ? FolioStyleEdit(role: "Heading", properties: ["text-align": headingStyle])
            : nil
        return BulkEditPayload(
            typography: hasTypography ? typography : nil,
            stripEmbeddedFonts: applyFontPolicy ? stripEmbeddedFonts : nil,
            style: style
        )
    }
}

struct BulkEditPayload {
    let typography: FolioTypographyEdit?
    let stripEmbeddedFonts: Bool?
    let style: FolioStyleEdit?
}

@MainActor
private final class BookEditorLocalState: ObservableObject {
    @Published var showMetadataSearch = false
    @Published var showCoverImporter = false
    @Published var showFontImporter = false
    @Published var coverError: String?
    @Published var fontError: String?
    @Published var coverMode: CoverMode = .preserve
    @Published var styleRuleRole = "Heading"
    @Published var styleRuleKind = StyleRuleKind.center
    @Published var styleRuleOpen = false
}

@MainActor
private final class StructureEditorLocalState: ObservableObject {
    @Published var selectedDocumentID: UInt32?
}

@MainActor
private final class TokenChipEditorLocalState: ObservableObject {
    @Published var entry = ""
}

struct BookEditorPane: View {
    let item: BookItem
    @Binding var edits: FolioBookEditPlan
    let report: FolioInspectReport?
    let analysis: FolioAnalysisReport?
    @Binding var section: EditorSection
    @Binding var appearancePage: AppearancePage
    @Binding var selectedFontID: String?
    let isLoading: Bool
    let detailError: String?
    let canEdit: Bool
    let convert: () -> Void

    @EnvironmentObject private var queue: ConversionQueueViewModel
    @StateObject private var localState = BookEditorLocalState()
    @Environment(\.openURL) private var openURL

    private let maxAssetBytes = 32 * 1024 * 1024

    var body: some View {
        VStack(spacing: 0) {
            editorHeader
                .padding(.horizontal, 18)
                .padding(.top, 14)
                .padding(.bottom, 12)

            Picker(FolioL10n.string("ui.editor_section", default: "Editor Section"), selection: $section) {
                ForEach(EditorSection.allCases) { value in
                    Text(value.title).tag(value)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .controlSize(.small)
            .padding(.horizontal, 16)
            .padding(.bottom, 12)

            Divider()

            Group {
                switch section {
                case .summary:
                    SummaryEditorPage(
                        item: item,
                        snapshot: snapshot,
                        report: report,
                        analysis: analysis,
                        target: queue.target,
                        mode: queue.degradationMode,
                        outputPath: outputPath,
                        isLoading: isLoading,
                        detailError: detailError,
                        canConvert: queue.selectedCanConvert,
                        convert: convert
                    )
                case .metadata:
                    MetadataEditorPage(
                        edits: $edits,
                        snapshot: snapshot,
                        canEdit: canEdit,
                        searchOnline: { localState.showMetadataSearch = true }
                    )
                case .appearance:
                    appearanceEditor
                case .structure:
                    StructureEditorPage(edits: $edits, snapshot: snapshot, canEdit: canEdit)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .background(Color(nsColor: .windowBackgroundColor))
        .disabled(!canEdit)
        .fileImporter(
            isPresented: $localState.showCoverImporter,
            allowedContentTypes: [.image],
            allowsMultipleSelection: false,
            onCompletion: loadCover
        )
        .fileImporter(
            isPresented: $localState.showFontImporter,
            allowedContentTypes: [.font],
            allowsMultipleSelection: false,
            onCompletion: loadFont
        )
        .sheet(isPresented: $localState.showMetadataSearch) {
            OnlineMetadataSearchView(current: currentOnlineMetadata, apply: applyOnlineMetadata)
        }
        .onAppear(perform: synchronizeCoverMode)
        .onChange(of: item.id) { _ in
            synchronizeCoverMode()
            localState.coverError = nil
            localState.fontError = nil
        }
    }

    private var snapshot: SemanticBookSnapshot {
        SemanticBookSnapshot(report: report, fallbackTitle: item.inputURL.deletingPathExtension().lastPathComponent)
    }

    private var outputPath: String {
        item.outputURL?.deletingLastPathComponent().path ?? queue.outputDestinationDescription
    }

    private var currentOnlineMetadata: FolioOnlineMetadata {
        let metadata = edits.metadata
        func scalar(_ edited: String?, _ source: String, _ field: String) -> String? {
            if metadata.clearFields.contains(field) { return nil }
            let value = edited ?? source
            return value.isEmpty ? nil : value
        }
        func list(_ edited: [String]?, _ source: [String], _ field: String) -> [String] {
            metadata.clearFields.contains(field) ? [] : (edited ?? source)
        }
        return FolioOnlineMetadata(
            title: scalar(metadata.title, snapshot.title, "title"),
            subtitle: scalar(metadata.subtitle, snapshot.subtitle, "subtitle"),
            language: scalar(metadata.language, snapshot.language, "language"),
            authors: list(metadata.authors, snapshot.authors, "authors"),
            contributors: list(metadata.contributors, snapshot.contributors, "contributors"),
            publisher: scalar(metadata.publisher, snapshot.publisher, "publisher"),
            date: scalar(metadata.date, snapshot.date, "date"),
            series: scalar(metadata.series, snapshot.series, "series"),
            seriesIndex: metadata.clearFields.contains("series_index")
                ? nil
                : (metadata.seriesIndex ?? Double(snapshot.seriesIndex)),
            identifiers: list(metadata.identifiers, snapshot.identifiers, "identifiers"),
            description: scalar(metadata.description, snapshot.description, "description"),
            subjects: list(metadata.subjects, snapshot.subjects, "subjects"),
            rights: scalar(metadata.rights, snapshot.rights, "rights")
        )
    }

    private func applyOnlineMetadata(_ incoming: FolioMetadataEdit) {
        var metadata = edits.metadata
        if let value = incoming.title { metadata.title = value }
        if let value = incoming.subtitle { metadata.subtitle = value }
        if let value = incoming.authors { metadata.authors = value }
        if let value = incoming.contributors { metadata.contributors = value }
        if let value = incoming.language { metadata.language = value }
        if let value = incoming.publisher { metadata.publisher = value }
        if let value = incoming.date { metadata.date = value }
        if let value = incoming.series { metadata.series = value }
        if let value = incoming.seriesIndex { metadata.seriesIndex = value }
        if let value = incoming.description { metadata.description = value }
        if let value = incoming.subjects { metadata.subjects = value }
        if let value = incoming.identifiers { metadata.identifiers = value }
        if let value = incoming.rights { metadata.rights = value }
        metadata.clearFields = Array(Set(metadata.clearFields + incoming.clearFields)).sorted()
        edits.metadata = metadata
    }

    private var editorHeader: some View {
        HStack(alignment: .center, spacing: 12) {
            Image(folioSymbol: .bookFilled)
                .font(.title2)
                .foregroundStyle(.secondary)
                .frame(width: 40, height: 44)
                .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 9))
            VStack(alignment: .leading, spacing: 3) {
                Text(snapshot.title)
                    .font(.title3.weight(.semibold))
                    .lineLimit(1)
                    .help(snapshot.title)
                Text(item.displayPath)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 8)
            StatusBadge(title: item.status.label, color: statusColor(item.status))
        }
    }

    @ViewBuilder
    private var appearanceEditor: some View {
        VStack(spacing: 0) {
            Picker(FolioL10n.string("ui.appearance_page", default: "Appearance Page"), selection: $appearancePage) {
                ForEach(AppearancePage.allCases) { value in
                    Text(value.title).tag(value)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .controlSize(.small)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)

            Divider()

            Group {
                switch appearancePage {
                case .cover:
                    CoverEditorPage(
                        edits: $edits,
                        mode: $localState.coverMode,
                        error: localState.coverError,
                        chooseCover: { localState.showCoverImporter = true },
                        searchOnline: searchOnlineCover
                    )
                case .typography:
                    TypographyEditorPage(edits: $edits, canEdit: canEdit)
                case .fonts:
                    FontsEditorPage(
                        edits: $edits,
                        fonts: snapshot.fonts,
                        selection: $selectedFontID,
                        error: localState.fontError,
                        chooseFont: { localState.showFontImporter = true },
                        searchOnline: searchOnlineFont
                    )
                case .styles:
                    StylesEditorPage(
                        edits: $edits,
                        canEdit: canEdit,
                        role: $localState.styleRuleRole,
                        kind: $localState.styleRuleKind,
                        ruleOpen: $localState.styleRuleOpen
                    )
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    private func statusColor(_ status: QueueItemStatus) -> Color {
        switch status {
        case .ready: .secondary
        case .converting: .accentColor
        case .completed: .green
        case .failed: .red
        case .cancelled: .orange
        }
    }

    private func synchronizeCoverMode() {
        if case .replace(_, _, _, let fit) = edits.cover {
            localState.coverMode = fit == .fit ? .fit : .fill
        } else {
            localState.coverMode = .preserve
        }
    }

    private func loadCover(_ result: Result<[URL], Error>) {
        switch result {
        case .failure(let error): localState.coverError = error.localizedDescription
        case .success(let urls):
            guard let url = urls.first else { return }
            let accessed = url.startAccessingSecurityScopedResource()
            defer { if accessed { url.stopAccessingSecurityScopedResource() } }
            do {
                let data = try Data(contentsOf: url)
                guard !data.isEmpty, data.count <= maxAssetBytes else {
                    localState.coverError = "Cover image must be non-empty and at most 32 MiB."
                    return
                }
                guard let mediaType = UTType(filenameExtension: url.pathExtension)?.preferredMIMEType,
                      mediaType.hasPrefix("image/") else {
                    localState.coverError = "Choose a supported image file."
                    return
                }
                localState.coverMode = .fit
                edits.cover = .replace(
                    fileName: url.lastPathComponent,
                    mediaType: mediaType,
                    bytes: Array(data),
                    fit: .fit
                )
                localState.coverError = nil
            } catch {
                localState.coverError = error.localizedDescription
            }
        }
    }

    private func loadFont(_ result: Result<[URL], Error>) {
        switch result {
        case .failure(let error): localState.fontError = error.localizedDescription
        case .success(let urls):
            guard let url = urls.first else { return }
            let accessed = url.startAccessingSecurityScopedResource()
            defer { if accessed { url.stopAccessingSecurityScopedResource() } }
            do {
                let data = try Data(contentsOf: url)
                guard !data.isEmpty, data.count <= maxAssetBytes else {
                    localState.fontError = "Replacement font must be non-empty and at most 32 MiB."
                    return
                }
                guard let mediaType = detectedFontMediaType(data, extension: url.pathExtension) else {
                    localState.fontError = "Choose a valid TTF, OTF, WOFF, WOFF2, or TTC font file."
                    return
                }
                edits.fonts.replacement = FolioFontReplacement(
                    fileName: url.lastPathComponent,
                    mediaType: mediaType,
                    bytes: Array(data),
                    family: edits.fonts.preferredFamily
                )
                localState.fontError = nil
            } catch {
                localState.fontError = error.localizedDescription
            }
        }
    }

    private func detectedFontMediaType(_ data: Data, extension fileExtension: String) -> String? {
        if data.starts(with: Array("wOF2".utf8)) { return "font/woff2" }
        if data.starts(with: Array("wOFF".utf8)) { return "font/woff" }
        if data.starts(with: Array("OTTO".utf8)) { return "font/otf" }
        if data.starts(with: Array("ttcf".utf8)) { return "font/collection" }
        if data.count >= 4, data[0] == 0, data[1] == 1, data[2] == 0, data[3] == 0 { return "font/ttf" }
        switch fileExtension.lowercased() {
        case "woff2": return "font/woff2"
        case "woff": return "font/woff"
        case "otf": return "font/otf"
        case "ttc": return "font/collection"
        case "ttf": return "font/ttf"
        default: return nil
        }
    }

    private func searchOnlineCover() {
        let query = ([snapshot.title] + Array(snapshot.authors.prefix(1))).filter { !$0.isEmpty }.joined(separator: " ")
        openSearch("https://www.google.com/search", items: [URLQueryItem(name: "tbm", value: "isch"), URLQueryItem(name: "q", value: query)])
    }

    private func searchOnlineFont() {
        let family = edits.fonts.preferredFamily ?? "serif"
        openSearch("https://fonts.google.com/", items: [URLQueryItem(name: "query", value: family)])
    }

    private func openSearch(_ base: String, items: [URLQueryItem]) {
        guard var components = URLComponents(string: base) else { return }
        components.queryItems = items
        if let url = components.url { openURL(url) }
    }
}

private enum CoverMode: String, CaseIterable, Identifiable, Hashable {
    case preserve
    case fit
    case fill
    var id: String { rawValue }
    var title: String {
        switch self {
        case .preserve: FolioL10n.string("ui.preserve_source", default: "Preserve Source")
        case .fit: FolioL10n.string("cover.fit", default: "Fit")
        case .fill: FolioL10n.string("cover.fill_crop", default: "Fill / Crop")
        }
    }
}

private struct SummaryEditorPage: View {
    let item: BookItem
    let snapshot: SemanticBookSnapshot
    let report: FolioInspectReport?
    let analysis: FolioAnalysisReport?
    let target: FolioTarget
    let mode: FolioDegradationMode
    let outputPath: String
    let isLoading: Bool
    let detailError: String?
    let canConvert: Bool
    let convert: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 26) {
                HStack(alignment: .center, spacing: 20) {
                    BookCoverPlaceholder(title: snapshot.title, width: 108, height: 154)
                    VStack(alignment: .leading, spacing: 9) {
                        Text(snapshot.title)
                            .font(.title2.weight(.semibold))
                            .fixedSize(horizontal: false, vertical: true)
                        if !snapshot.authors.isEmpty {
                            Text(snapshot.authors.joined(separator: ", "))
                                .font(.callout)
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                        }
                        Text([item.inputURL.pathExtension.uppercased(), snapshot.language, fileSize].filter { !$0.isEmpty }.joined(separator: " · "))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        StatusBadge(title: item.status.label, color: statusColor)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.top, 8)

                VStack(alignment: .leading, spacing: 12) {
                    HStack {
                        Text(FolioL10n.string("ui.compatibility", default: "Compatibility"))
                            .font(.headline)
                        Spacer()
                        StatusBadge(
                            title: analysis?.plan.quality.displayName ?? FolioL10n.string("status.target_not_checked", default: "Not checked"),
                            color: analysis.map { $0.plan.blocked ? .orange : qualityColor($0.plan.quality) } ?? .secondary
                        )
                    }
                    if let analysis {
                        Text(analysis.plan.blocked
                            ? FolioL10n.format(
                                "status.target_blocked",
                                default: "This target is blocked in %@ mode. Review the plan in Inspector or change the mode.",
                                mode.displayName
                            )
                            : analysis.plan.quality.userSummary)
                            .font(.callout)
                            .foregroundStyle(analysis.plan.blocked ? .orange : .secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    } else {
                        Text(FolioL10n.string(
                            isLoading ? "editor.reading_validating_source" : "editor.check_compatibility_prompt",
                            default: isLoading ? "Reading and validating the source…" : "Check compatibility to see the target-specific plan."
                        ))
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    HStack(spacing: 14) {
                        PipelineStatus(title: FolioL10n.string("inspector.parser", default: "Parser"), isReady: report != nil, isLoading: isLoading)
                        PipelineStatus(title: FolioL10n.string("ui.semantic_ir", default: "Semantic IR"), isReady: report?.semanticReport.valid == true, isLoading: isLoading)
                        if let analysis, !analysis.plan.items.isEmpty {
                        Label(FolioL10n.format("error.adjustment_count", default: "Adjustments: %@", String(analysis.plan.items.count)), systemImage: FolioSymbol.warning.name)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .padding(.top, 2)
                }

                VStack(alignment: .leading, spacing: 12) {
                    Text(FolioL10n.string("ui.output", default: "Output"))
                        .font(.headline)
                    SettingsRow(label: FolioL10n.string("inspector.format", default: "Format")) { Text(target.displayName) }
                    SettingsRow(label: FolioL10n.string("ui.mode", default: "Mode")) { Text(mode.displayName) }
                    SettingsRow(label: FolioL10n.string("inspector.destination", default: "Destination")) {
                        Text(outputPath)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .foregroundStyle(.secondary)
                    }
                    if let output = item.report?.outputReport {
                        SettingsRow(label: FolioL10n.string("inspector.last_output", default: "Last output")) {
                            Text("\(output.format) · \(ByteCountFormatter.string(fromByteCount: Int64(clamping: output.size), countStyle: .file))")
                                .foregroundStyle(.secondary)
                        }
                    }
                    if let detailError {
                        Label(detailError, systemImage: FolioSymbol.warning.name)
                            .font(.caption)
                            .foregroundStyle(.orange)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    HStack {
                        Spacer()
                        Button(action: convert) {
                            Label(FolioL10n.string("ui.convert", default: "Convert"), systemImage: FolioAction.convert.symbol.name)
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(!canConvert)
                    }
                    .padding(.top, 4)
                }
            }
            .frame(maxWidth: 520, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(24)
        }
    }

    private var fileSize: String {
        guard let values = try? item.inputURL.resourceValues(forKeys: [.fileSizeKey]),
              let size = values.fileSize else { return "" }
        return ByteCountFormatter.string(fromByteCount: Int64(size), countStyle: .file)
    }

    private var statusColor: Color {
        switch item.status {
        case .ready: .secondary
        case .converting: .accentColor
        case .completed: .green
        case .failed: .red
        case .cancelled: .orange
        }
    }

    private func qualityColor(_ quality: FolioCompatibilityQuality) -> Color {
        switch quality {
        case .exact, .high: .green
        case .compatible: .blue
        case .reduced: .orange
        case .severeLoss: .red
        }
    }
}

private struct PipelineStatus: View {
    let title: String
    let isReady: Bool
    let isLoading: Bool

    var body: some View {
        Label(title, systemImage: (isReady ? FolioSymbol.success : isLoading ? FolioSymbol.progress : FolioSymbol.alertCircle).name)
            .font(.caption)
            .foregroundStyle(isReady ? Color.green : Color.secondary)
    }
}

private struct MetadataEditorPage: View {
    @Binding var edits: FolioBookEditPlan
    let snapshot: SemanticBookSnapshot
    let canEdit: Bool
    let searchOnline: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                Text(FolioL10n.string("ui.metadata", default: "Metadata"))
                    .font(.title2.weight(.semibold))
                EditorSectionBlock(title: FolioL10n.string("editor.general", default: "General")) {
                    SettingsRow(label: FolioL10n.string("ui.title", default: "Title")) {
                        TextField("", text: scalarBinding(\.title, source: snapshot.title, field: "title"))
                            .accessibilityLabel(FolioL10n.string("ui.title", default: "Title"))
                    }
                    SettingsRow(label: FolioL10n.string("ui.subtitle", default: "Subtitle")) {
                        TextField("", text: scalarBinding(\.subtitle, source: snapshot.subtitle, field: "subtitle"))
                            .accessibilityLabel(FolioL10n.string("ui.subtitle", default: "Subtitle"))
                    }
                    SettingsRow(label: FolioL10n.string("online.field.authors", default: "Authors")) {
                        TokenChipEditor(tokens: listBinding(\.authors, source: snapshot.authors, field: "authors"), placeholder: FolioL10n.string("editor.add_author", default: "Add author"))
                    }
                    SettingsRow(label: FolioL10n.string("ui.language", default: "Language")) {
                        TextField("", text: scalarBinding(\.language, source: snapshot.language, field: "language"))
                            .accessibilityLabel(FolioL10n.string("ui.language", default: "Language"))
                    }
                }

                EditorSectionBlock(title: FolioL10n.string("editor.publishing", default: "Publishing")) {
                    SettingsRow(label: FolioL10n.string("ui.publisher", default: "Publisher")) {
                        TextField("", text: scalarBinding(\.publisher, source: snapshot.publisher, field: "publisher"))
                            .accessibilityLabel(FolioL10n.string("ui.publisher", default: "Publisher"))
                    }
                    SettingsRow(label: FolioL10n.string("ui.published", default: "Published")) {
                        TextField("", text: scalarBinding(\.date, source: snapshot.date, field: "date"))
                            .accessibilityLabel(FolioL10n.string("ui.published", default: "Published"))
                    }
                    SettingsRow(label: FolioL10n.string("ui.series", default: "Series")) {
                        TextField("", text: scalarBinding(\.series, source: snapshot.series, field: "series"))
                            .accessibilityLabel(FolioL10n.string("ui.series", default: "Series"))
                    }
                    SettingsRow(label: FolioL10n.string("ui.series_number", default: "Series number")) {
                        TextField("", text: seriesIndexBinding)
                            .accessibilityLabel(FolioL10n.string("ui.series_number", default: "Series number"))
                            .frame(maxWidth: 120)
                    }
                    SettingsRow(label: FolioL10n.string("ui.rights", default: "Rights")) {
                        TextField("", text: scalarBinding(\.rights, source: snapshot.rights, field: "rights"))
                            .accessibilityLabel(FolioL10n.string("ui.rights", default: "Rights"))
                    }
                    SettingsRow(label: FolioL10n.string("online.field.contributors", default: "Contributors")) {
                        TokenChipEditor(tokens: listBinding(\.contributors, source: snapshot.contributors, field: "contributors"), placeholder: FolioL10n.string("editor.add_contributor", default: "Add contributor"))
                    }
                }

                EditorSectionBlock(title: FolioL10n.string("editor.identifiers_subjects", default: "Identifiers & Subjects")) {
                    SettingsRow(label: FolioL10n.string("online.field.subjects", default: "Subjects")) {
                        TokenChipEditor(tokens: listBinding(\.subjects, source: snapshot.subjects, field: "subjects"), placeholder: FolioL10n.string("editor.add_subject", default: "Add subject"))
                    }
                    SettingsRow(label: FolioL10n.string("online.field.identifiers", default: "Identifiers")) {
                        TokenChipEditor(tokens: listBinding(\.identifiers, source: snapshot.identifiers, field: "identifiers"), placeholder: FolioL10n.string("editor.add_identifier", default: "Add ISBN or identifier"))
                    }
                }

                EditorSectionBlock(title: FolioL10n.string("online.field.description", default: "Description")) {
                    TextEditor(text: scalarBinding(\.description, source: snapshot.description, field: "description"))
                        .font(.body)
                        .frame(minHeight: 112)
                        .scrollContentBackground(.hidden)
                        .padding(8)
                        .background(Color.secondary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
                }

                HStack {
                    Spacer()
                    Button(action: searchOnline) {
                        Label(FolioL10n.string("ui.find_metadata_online", default: "Find Metadata Online"), systemImage: FolioAction.findMetadata.symbol.name)
                    }
                    .disabled(snapshot.title.isEmpty && snapshot.authors.isEmpty && snapshot.identifiers.isEmpty)
                }
                Text(FolioL10n.string("ui.search_open_library_in_folioforge_only_the_search_terms", default: "Search Open Library in FolioForge. Only the search terms are sent; book files are never uploaded."))
                    .font(.caption)
                    .foregroundStyle(.secondary)

                DisclosureGroup(FolioL10n.string("ui.advanced_metadata", default: "Advanced metadata")) {
                    TokenChipEditor(tokens: clearFieldsBinding, placeholder: FolioL10n.string("editor.add_source_field_to_clear", default: "Add a source field to clear"))
                        .padding(.top, 8)
                    Text(FolioL10n.string("ui.use_field_names_such_as_title_authors_description_or", default: "Use field names such as title, authors, description, or rights to remove source values."))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.top, 4)
                }
                .font(.subheadline.weight(.medium))
            }
            .frame(maxWidth: 560, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(20)
        }
        .disabled(!canEdit)
    }

    private func scalarBinding(
        _ keyPath: WritableKeyPath<FolioMetadataEdit, String?>,
        source: String,
        field: String
    ) -> Binding<String> {
        Binding(
            get: {
                if edits.metadata.clearFields.contains(field) { return "" }
                return edits.metadata[keyPath: keyPath] ?? source
            },
            set: { value in
                edits.metadata.clearFields.removeAll { $0 == field }
                edits.metadata[keyPath: keyPath] = value.isEmpty ? nil : value
            }
        )
    }

    private func listBinding(
        _ keyPath: WritableKeyPath<FolioMetadataEdit, [String]?>,
        source: [String],
        field: String
    ) -> Binding<[String]> {
        Binding(
            get: {
                if edits.metadata.clearFields.contains(field) { return [] }
                return edits.metadata[keyPath: keyPath] ?? source
            },
            set: { values in
                edits.metadata.clearFields.removeAll { $0 == field }
                edits.metadata[keyPath: keyPath] = values
            }
        )
    }

    private var seriesIndexBinding: Binding<String> {
        Binding(
            get: {
                if edits.metadata.clearFields.contains("series_index") { return "" }
                return edits.metadata.seriesIndex.map { String($0) } ?? snapshot.seriesIndex
            },
            set: { value in
                edits.metadata.clearFields.removeAll { $0 == "series_index" }
                edits.metadata.seriesIndex = Double(value)
            }
        )
    }

    private var clearFieldsBinding: Binding<[String]> {
        Binding(
            get: { edits.metadata.clearFields },
            set: { edits.metadata.clearFields = Array(Set($0)).sorted() }
        )
    }
}

private struct CoverEditorPage: View {
    @Binding var edits: FolioBookEditPlan
    @Binding var mode: CoverMode
    let error: String?
    let chooseCover: () -> Void
    let searchOnline: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Text(FolioL10n.string("ui.cover", default: "Cover"))
                    .font(.title2.weight(.semibold))
                HStack(alignment: .center, spacing: 20) {
                    previewImage
                    VStack(alignment: .leading, spacing: 10) {
                        Text(coverTitle)
                            .font(.headline)
                        Text(coverDetail)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        HStack(spacing: 8) {
                            Button { chooseCover() } label: { Label(FolioL10n.string("ui.replace", default: "Replace…"), systemImage: FolioAction.replaceCover.symbol.name) }
                            Button { searchOnline() } label: { Label(FolioL10n.string("ui.find_online", default: "Find Online"), systemImage: FolioAction.findCover.symbol.name) }
                        }
                        .buttonStyle(.bordered)
                    }
                    Spacer(minLength: 0)
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                EditorSectionBlock(title: FolioL10n.string("ui.image_fitting", default: "Image fitting")) {
                    Picker(FolioL10n.string("ui.image_fitting", default: "Image fitting"), selection: $mode) {
                        ForEach(CoverMode.allCases) { value in
                            Text(value.title).tag(value)
                        }
                    }
                    .pickerStyle(.segmented)
                    Text(FolioL10n.string("ui.fit_and_fill_crop_apply_to_a_replacement_image", default: "Fit and Fill/Crop apply to a replacement image. Preserve Source keeps the original cover unchanged."))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                if let error {
                    Label(error, systemImage: FolioSymbol.warning.name)
                        .font(.caption)
                        .foregroundStyle(.red)
                }

                HStack {
                    Spacer()
                    Button(role: .destructive) {
                        edits.cover = .remove
                    } label: {
                        Label(FolioL10n.string("ui.remove_cover", default: "Remove Cover"), systemImage: FolioAction.removeCover.symbol.name)
                    }
                }
                Text(FolioL10n.string("ui.online_cover_search_opens_image_results_in_your_browser", default: "Online cover search opens image results in your browser. Review usage rights before importing a cover."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: 560, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(20)
        }
        .onChange(of: mode) { newMode in
            guard case .replace(let name, let mediaType, let bytes, _) = edits.cover else {
                if newMode == .preserve { edits.cover = .keep }
                return
            }
            if newMode == .preserve {
                edits.cover = .keep
            } else {
                edits.cover = .replace(fileName: name, mediaType: mediaType, bytes: bytes, fit: newMode == .fit ? .fit : .fill)
            }
        }
    }

    @ViewBuilder
    private var previewImage: some View {
        if case .replace(_, _, let bytes, _) = edits.cover,
           let image = NSImage(data: Data(bytes)) {
            Image(nsImage: image)
                .resizable()
                .scaledToFit()
                .frame(width: 150, height: 210)
                .background(Color.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
                .clipShape(RoundedRectangle(cornerRadius: 10))
        } else {
            BookCoverPlaceholder(title: coverTitle, width: 150, height: 210)
        }
    }

    private var coverTitle: String {
        switch edits.cover {
        case .keep: FolioL10n.string("cover.source", default: "Source cover")
        case .remove: FolioL10n.string("cover.none", default: "No cover")
        case .replace(let name, _, _, _): name
        }
    }

    private var coverDetail: String {
        switch edits.cover {
        case .keep: return FolioL10n.string("cover.preserved", default: "Original cover preserved")
        case .remove: return FolioL10n.string("cover.removed", default: "Cover will be removed from the output")
        case .replace(let name, let mediaType, let bytes, _):
            let dimensions = NSImage(data: Data(bytes))?.size
            let resolution = dimensions.map { "\(Int($0.width)) × \(Int($0.height))" } ?? "Image"
            let size = ByteCountFormatter.string(fromByteCount: Int64(bytes.count), countStyle: .file)
            return "\(resolution) · \(mediaType) · \(size) · \(name)"
        }
    }
}

private struct TypographyEditorPage: View {
    @Binding var edits: FolioBookEditPlan
    let canEdit: Bool

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                Text(FolioL10n.string("ui.typography", default: "Typography"))
                    .font(.title2.weight(.semibold))
                Text(FolioL10n.string("ui.source_typography_is_preserved_by_default_choose_auto_or", default: "Source typography is preserved by default. Choose Auto or Custom only when you want to override it."))
                    .font(.callout)
                    .foregroundStyle(.secondary)

                EditorSectionBlock(title: FolioL10n.string("editor.body_text", default: "Body Text")) {
                    TypographyValueRow(
                        title: FolioL10n.string("editor.font_family", default: "Font family"),
                        value: $edits.typography.fontFamily,
                        autoLabel: nil
                    )
                    TypographyValueRow(
                        title: FolioL10n.string("editor.body_size", default: "Body size"),
                        value: $edits.typography.bodyFontSize,
                        autoLabel: "Auto"
                    )
                    TypographyValueRow(
                        title: FolioL10n.string("editor.line_height", default: "Line height"),
                        value: $edits.typography.lineHeight,
                        autoLabel: "Auto"
                    )
                    TypographyValueRow(
                        title: FolioL10n.string("editor.letter_spacing", default: "Letter spacing"),
                        value: $edits.typography.letterSpacing,
                        autoLabel: "Auto"
                    )
                }

                if !canEdit {
                    Label(FolioL10n.string("ui.typography_controls_are_unavailable_during_conversion_or_compatibility_c", default: "Typography controls are unavailable during conversion or compatibility checking."), systemImage: FolioSymbol.lock.name)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: 560, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(20)
        }
        .disabled(!canEdit)
    }
}

private struct TypographyValueRow: View {
    let title: String
    @Binding var value: String?
    let autoLabel: String?

    private var choice: Binding<String> {
        Binding(
            get: {
                guard let value else { return "preserve" }
                return value == autoValue ? "auto" : "custom"
            },
            set: { choice in
                switch choice {
                case "preserve": value = nil
                case "auto": value = autoValue
                case "custom": if value == nil || value == autoValue { value = defaultValue }
                default: break
                }
            }
        )
    }

    private var autoValue: String? {
        switch title {
        case "Body size": return "100%"
        case "Line height", "Letter spacing": return "normal"
        default: return nil
        }
    }

    private var defaultValue: String {
        if title == "Font family" { return "serif" }
        if title == "Body size" { return "16px" }
        if title == "Line height" { return "1.5" }
        return "0.02em"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            SettingsRow(label: title) {
                Picker(title, selection: choice) {
                    Text(FolioL10n.string("ui.preserve_source", default: "Preserve Source")).tag("preserve")
                    if autoLabel != nil { Text(autoLabel!).tag("auto") }
                    Text(FolioL10n.string("ui.custom", default: "Custom")).tag("custom")
                }
                .labelsHidden()
                .pickerStyle(.menu)
            }
            if choice.wrappedValue == "custom" {
                LabeledContent(title) {
                    TextField("", text: Binding(
                        get: { value ?? defaultValue },
                        set: { value = $0.isEmpty ? nil : $0 }
                    ))
                    .accessibilityLabel(title)
                    .frame(maxWidth: 260)
                }
            }
        }
    }
}

private struct FontsEditorPage: View {
    @Binding var edits: FolioBookEditPlan
    let fonts: [SemanticFontRow]
    @Binding var selection: String?
    let error: String?
    let chooseFont: () -> Void
    let searchOnline: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Text(FolioL10n.string("ui.fonts", default: "Fonts"))
                    .font(.title2.weight(.semibold))
                if fonts.isEmpty {
                    VStack(spacing: 8) {
                        Image(folioSymbol: .onlineFont).font(.title2).foregroundStyle(.secondary)
                        Text(FolioL10n.string("ui.no_embedded_fonts", default: "No Embedded Fonts")).font(.headline)
                        Text(FolioL10n.string("ui.this_source_does_not_declare_any_embedded_font_resources", default: "This source does not declare any embedded font resources."))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                    }
                    .frame(maxWidth: .infinity, minHeight: 170)
                } else {
                    Table(fonts, selection: $selection) {
                        TableColumn(FolioL10n.string("ui.family", default: "Family")) { font in Text(font.family).lineLimit(1) }
                        TableColumn(FolioL10n.string("ui.style", default: "Style")) { font in Text(font.style).foregroundStyle(.secondary) }
                        TableColumn(FolioL10n.string("ui.size", default: "Size")) { font in Text(font.sizeDescription).monospacedDigit() }
                    }
                    .frame(minHeight: min(CGFloat(fonts.count) * 30 + 36, 250))
                }

                HStack(spacing: 8) {
                    Button { chooseFont() } label: { Label(FolioL10n.string("ui.add_local_font", default: "Add Local Font…"), systemImage: FolioAction.addFont.symbol.name) }
                    Button { searchOnline() } label: { Label(FolioL10n.string("ui.find_font_online", default: "Find Font Online"), systemImage: FolioAction.findFont.symbol.name) }
                }
                .buttonStyle(.bordered)

                EditorSectionBlock(title: FolioL10n.string("editor.font_policy", default: "Font Policy")) {
                    SettingsRow(label: FolioL10n.string("editor.preferred_family", default: "Preferred family")) {
                        TextField(FolioL10n.string("ui.reader_font_family", default: "Reader font family"), text: optionalBinding(\.preferredFamily, in: $edits.fonts))
                    }
                    Toggle(FolioL10n.string("ui.strip_embedded_fonts", default: "Strip embedded fonts"), isOn: $edits.fonts.stripEmbeddedFonts)
                    if let replacement = edits.fonts.replacement {
                        HStack {
                            Label(replacement.fileName, systemImage: FolioSymbol.onlineFont.name)
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Spacer()
                            Button { edits.fonts.replacement = nil } label: { Label(FolioL10n.string("ui.remove_replacement", default: "Remove Replacement"), systemImage: FolioAction.removeReplacement.symbol.name) }
                                .buttonStyle(.borderless)
                        }
                    }
                }

                if let error {
                    Label(error, systemImage: FolioSymbol.warning.name)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
                Text(FolioL10n.string("ui.online_font_search_opens_google_fonts_in_your_browser", default: "Online font search opens Google Fonts in your browser. Review its license before use. Source font details are shown in Inspector when a row is selected."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: 560, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(20)
        }
    }

    private func optionalBinding<Value>(_ keyPath: WritableKeyPath<Value, String?>, in value: Binding<Value>) -> Binding<String> {
        Binding(
            get: { value.wrappedValue[keyPath: keyPath] ?? "" },
            set: { value.wrappedValue[keyPath: keyPath] = $0.isEmpty ? nil : $0 }
        )
    }
}

private struct StylesEditorPage: View {
    @Binding var edits: FolioBookEditPlan
    let canEdit: Bool
    @Binding var role: String
    @Binding var kind: StyleRuleKind
    @Binding var ruleOpen: Bool

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                Text(FolioL10n.string("ui.styles", default: "Styles"))
                    .font(.title2.weight(.semibold))
                Label(FolioL10n.string("ui.source_styles_are_preserved_unless_you_add_an_override", default: "Source styles are preserved unless you add an override."), systemImage: FolioSymbol.check.name)
                    .font(.callout)
                    .foregroundStyle(.secondary)

                EditorSectionBlock(title: FolioL10n.string("ui.styles", default: "Styles")) {
                    if edits.styles.filter({ $0.css == nil }).isEmpty {
                        Text(FolioL10n.string("ui.no_style_overrides_added", default: "No style overrides added."))
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    } else {
                        ForEach(Array(edits.styles.enumerated()), id: \.offset) { index, style in
                            if style.css == nil {
                                HStack(alignment: .top) {
                                    VStack(alignment: .leading, spacing: 3) {
                                        Text(style.role ?? "All content").font(.body.weight(.medium))
                                        Text(style.properties.sorted { $0.key < $1.key }.map { "\($0.key): \($0.value)" }.joined(separator: " · "))
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                    Spacer()
                                    Button {
                                        edits.styles.remove(at: index)
                                    } label: {
                                        Label(FolioL10n.string("ui.remove", default: "Remove"), systemImage: FolioAction.removeItem.symbol.name)
                                    }
                                    .labelStyle(.iconOnly)
                                    .buttonStyle(.borderless)
                                    .help(FolioL10n.string("ui.remove_this_style_rule", default: "Remove this style rule"))
                                }
                                .padding(.vertical, 3)
                            }
                        }
                    }

                    Button { ruleOpen.toggle() } label: { Label(FolioL10n.string("ui.add_rule", default: "Add Rule"), systemImage: FolioAction.addStyleRule.symbol.name) }
                        .buttonStyle(.bordered)

                    if ruleOpen {
                        VStack(alignment: .leading, spacing: 10) {
                            Picker(FolioL10n.string("ui.apply_to", default: "Apply to"), selection: $role) {
                                Text(FolioL10n.string("ui.heading", default: "Heading")).tag("Heading")
                                Text(FolioL10n.string("ui.paragraph", default: "Paragraph")).tag("Paragraph")
                                Text(FolioL10n.string("ui.chapter", default: "Chapter")).tag("Chapter")
                                Text(FolioL10n.string("ui.quote", default: "Quote")).tag("Quote")
                            }
                            .frame(maxWidth: 220)
                            Picker(FolioL10n.string("ui.rule", default: "Rule"), selection: $kind) {
                                ForEach(StyleRuleKind.allCases) { value in Text(value.title).tag(value) }
                            }
                            .frame(maxWidth: 260)
                            HStack {
                                Spacer()
                                Button(FolioL10n.string("ui.add_style_rule", default: "Add Style Rule")) {
                                    edits.styles.append(FolioStyleEdit(role: role, properties: [kind.property: kind.value]))
                                    ruleOpen = false
                                }
                                .buttonStyle(.borderedProminent)
                            }
                        }
                        .padding(12)
                        .background(Color.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 9))
                    }
                }

                DisclosureGroup(FolioL10n.string("ui.advanced_css", default: "Advanced CSS")) {
                    TextEditor(text: cssBinding)
                        .font(.system(.body, design: .monospaced))
                        .frame(minHeight: 130)
                        .scrollContentBackground(.hidden)
                        .padding(8)
                        .background(Color.secondary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
                        .padding(.top, 8)
                }
                .font(.subheadline.weight(.medium))
                Text(FolioL10n.string("ui.custom_css_is_applied_as_an_explicit_style_override", default: "Custom CSS is applied as an explicit style override during the IR edit pass."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: 560, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(20)
        }
        .disabled(!canEdit)
    }

    private var cssBinding: Binding<String> {
        Binding(
            get: { edits.styles.compactMap(\.css).joined(separator: "\n\n") },
            set: { source in
                let css = source.trimmingCharacters(in: .whitespacesAndNewlines)
                let rules = edits.styles.filter { $0.css == nil }
                edits.styles = rules + (css.isEmpty ? [] : [FolioStyleEdit(css: css)])
            }
        )
    }
}

private enum StyleRuleKind: String, CaseIterable, Identifiable, Hashable {
    case center
    case bold
    case indent
    var id: String { rawValue }
    var title: String {
        switch self {
        case .center: FolioL10n.string("editor.style.center", default: "Center text")
        case .bold: FolioL10n.string("editor.style.bold", default: "Bold")
        case .indent: FolioL10n.string("editor.style.indent", default: "First-line indent")
        }
    }
    var property: String {
        switch self {
        case .center: "text-align"
        case .bold: "font-weight"
        case .indent: "text-indent"
        }
    }
    var value: String {
        switch self {
        case .center: "center"
        case .bold: "bold"
        case .indent: "2em"
        }
    }
}

private struct StructureEditorPage: View {
    @Binding var edits: FolioBookEditPlan
    let snapshot: SemanticBookSnapshot
    let canEdit: Bool
    @StateObject private var localState = StructureEditorLocalState()

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                Text(FolioL10n.string("ui.structure", default: "Structure"))
                    .font(.title2.weight(.semibold))
                Text(FolioL10n.string("ui.review_the_reading_order_document_titles_and_navigation_labels", default: "Review the reading order, document titles, and navigation labels before export."))
                    .font(.callout)
                    .foregroundStyle(.secondary)

                EditorSectionBlock(title: FolioL10n.string("editor.reading_order", default: "Reading Order")) {
                    if snapshot.documents.isEmpty {
                        Text(FolioL10n.string("ui.no_document_structure_is_available_in_the_source_report", default: "No document structure is available in the source report."))
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    } else {
                        Table(orderedDocuments, selection: $localState.selectedDocumentID) {
                            TableColumn(FolioL10n.string("ui.document", default: "Document")) { document in
                                TextField(document.title, text: documentTitleBinding(document))
                                    .textFieldStyle(.plain)
                            }
                            TableColumn(FolioL10n.string("ui.source", default: "Source")) { document in
                                Text(document.href).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }
                            TableColumn(FolioL10n.string("ui.order", default: "Order")) { document in
                                HStack(spacing: 4) {
                                    Button { moveDocument(document.id, by: -1) } label: {
                                        Image(folioSymbol: .moveUp)
                                    }
                                    .help(FolioL10n.string("ui.move_earlier", default: "Move earlier"))
                                    .disabled(orderIndex(for: document.id) == 0)
                                    Button { moveDocument(document.id, by: 1) } label: {
                                        Image(folioSymbol: .moveDown)
                                    }
                                    .help(FolioL10n.string("ui.move_later", default: "Move later"))
                                    .disabled(orderIndex(for: document.id) == snapshot.documents.count - 1)
                                }
                                .buttonStyle(.borderless)
                            }
                        }
                        .frame(minHeight: min(CGFloat(snapshot.documents.count) * 34 + 38, 300))
                    }
                }

                EditorSectionBlock(title: FolioL10n.string("editor.table_of_contents", default: "Table of Contents")) {
                    if snapshot.navigation.isEmpty {
                        Text(FolioL10n.string("ui.no_table_of_contents_was_detected", default: "No table of contents was detected."))
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    } else {
                        ForEach(snapshot.navigation) { entry in
                            SettingsRow(label: String(repeating: "  ", count: entry.depth) + entry.label) {
                                TextField(FolioL10n.string("ui.navigation_label", default: "Navigation label"), text: navigationBinding(entry))
                            }
                        }
                    }
                }

                Toggle(FolioL10n.string("ui.remove_navigation_from_output", default: "Remove navigation from output"), isOn: $edits.structure.removeNavigation)
                Text(FolioL10n.string("ui.structure_changes_are_applied_to_the_semantic_ir_before", default: "Structure changes are applied to the semantic IR before target compatibility planning."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: 560, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(20)
        }
        .disabled(!canEdit)
    }

    private var order: [UInt32] {
        edits.structure.documentOrder ?? snapshot.documents.map(\.id)
    }

    private var orderedDocuments: [SemanticDocumentRow] {
        snapshot.documents.sorted { orderIndex(for: $0.id) < orderIndex(for: $1.id) }
    }

    private func orderIndex(for id: UInt32) -> Int { order.firstIndex(of: id) ?? 0 }

    private func moveDocument(_ id: UInt32, by offset: Int) {
        var values = order
        guard let index = values.firstIndex(of: id) else { return }
        let next = index + offset
        guard values.indices.contains(next) else { return }
        values.swapAt(index, next)
        edits.structure.documentOrder = values
    }

    private func documentTitleBinding(_ document: SemanticDocumentRow) -> Binding<String> {
        let key = String(document.id)
        return Binding(
            get: { edits.structure.documentTitles[key] ?? document.title },
            set: { value in
                if value == document.title { edits.structure.documentTitles.removeValue(forKey: key) }
                else { edits.structure.documentTitles[key] = value }
            }
        )
    }

    private func navigationBinding(_ entry: SemanticNavigationRow) -> Binding<String> {
        Binding(
            get: { edits.structure.tocLabels[entry.href] ?? entry.label },
            set: { value in
                if value == entry.label { edits.structure.tocLabels.removeValue(forKey: entry.href) }
                else { edits.structure.tocLabels[entry.href] = value }
            }
        )
    }
}

struct BulkEditPane: View {
    @ObservedObject var draft: BulkEditDraft
    let count: Int
    let target: FolioTarget
    let mode: FolioDegradationMode
    let message: String?
    let apply: () -> Void

    @EnvironmentObject private var queue: ConversionQueueViewModel

    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 14) {
                VStack(alignment: .leading, spacing: 5) {
                    Text(FolioL10n.string("ui.bulk_edit", default: "Bulk Edit"))
                        .font(.title2.weight(.semibold))
                    Text(FolioL10n.format("error.books_selected", default: "Selected books: %@", String(count)))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                Picker(FolioL10n.string("ui.bulk_edit_section", default: "Bulk Edit Section"), selection: $draft.section) {
                    ForEach(BulkEditSection.allCases) { section in
                        Text(section.title).tag(section)
                    }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
            }
            .padding(.horizontal, 20)
            .padding(.top, 20)
            .padding(.bottom, 14)

            Divider()

            ScrollView {
                Group {
                    switch draft.section {
                    case .output:
                        EditorSectionBlock(title: FolioL10n.string("ui.output", default: "Output")) {
                            SettingsRow(label: FolioL10n.string("inspector.format", default: "Format")) { Text(target.displayName) }
                            SettingsRow(label: FolioL10n.string("ui.mode", default: "Mode")) { Text(mode.displayName) }
                            SettingsRow(label: FolioL10n.string("inspector.destination", default: "Destination")) { Text(queue.outputDestinationDescription).lineLimit(2) }
                            Text(FolioL10n.string("ui.target_and_compatibility_settings_apply_to_the_whole_batch", default: "Target and compatibility settings apply to the whole batch from the toolbar."))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    case .typography:
                        EditorSectionBlock(title: FolioL10n.string("ui.typography", default: "Typography")) {
                            Toggle(FolioL10n.string("ui.apply_font_family", default: "Apply font family"), isOn: $draft.applyFontFamily)
                            if draft.applyFontFamily {
                            LabeledContent(FolioL10n.string("editor.font_family", default: "Font family")) {
                                TextField("", text: $draft.fontFamily)
                                        .frame(maxWidth: 240)
                                }
                            }
                            Toggle(FolioL10n.string("ui.apply_body_size", default: "Apply body size"), isOn: $draft.applyBodySize)
                            if draft.applyBodySize {
                            LabeledContent(FolioL10n.string("editor.body_size", default: "Body size")) {
                                    TextField("", text: $draft.bodySize)
                                        .frame(maxWidth: 160)
                                }
                            }
                            Toggle(FolioL10n.string("ui.apply_line_height", default: "Apply line height"), isOn: $draft.applyLineHeight)
                            if draft.applyLineHeight {
                            LabeledContent(FolioL10n.string("editor.line_height", default: "Line height")) {
                                    TextField("", text: $draft.lineHeight)
                                        .frame(maxWidth: 160)
                                }
                            }
                        }
                    case .fonts:
                        EditorSectionBlock(title: FolioL10n.string("ui.fonts", default: "Fonts")) {
                            Toggle(FolioL10n.string("ui.set_embedded_font_policy", default: "Set embedded-font policy"), isOn: $draft.applyFontPolicy)
                            if draft.applyFontPolicy {
                                Picker(FolioL10n.string("ui.policy", default: "Policy"), selection: $draft.stripEmbeddedFonts) {
                                    Text(FolioL10n.string("ui.keep_embedded_fonts", default: "Keep embedded fonts")).tag(false)
                                    Text(FolioL10n.string("ui.remove_embedded_fonts", default: "Remove embedded fonts")).tag(true)
                                }
                                .pickerStyle(.segmented)
                            }
                        }
                    case .styles:
                        EditorSectionBlock(title: FolioL10n.string("ui.styles", default: "Styles")) {
                            Toggle(FolioL10n.string("ui.apply_a_heading_rule", default: "Apply a heading rule"), isOn: $draft.applyHeadingStyle)
                            if draft.applyHeadingStyle {
                                Picker(FolioL10n.string("ui.heading_alignment", default: "Heading alignment"), selection: $draft.headingStyle) {
                                    Text(FolioL10n.string("ui.center", default: "Center")).tag("center")
                                    Text(FolioL10n.string("ui.left", default: "Left")).tag("left")
                                    Text(FolioL10n.string("ui.right", default: "Right")).tag("right")
                                }
                                .frame(maxWidth: 240)
                            }
                        }
                    }
                }
                .frame(maxWidth: 520, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .padding(22)
            }

            Divider()

            HStack(spacing: 12) {
                if let message {
                    Label(message, systemImage: FolioSymbol.success.name)
                        .font(.caption)
                        .foregroundStyle(.green)
                        .lineLimit(2)
                } else {
                    Text(FolioL10n.string("ui.only_enabled_changes_are_applied_each_updated_book_must", default: "Only enabled changes are applied. Each updated book must be checked again."))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                Spacer(minLength: 8)
                Button { apply() } label: {
                    Label(FolioL10n.format("error.apply_to_books", default: "Apply to %@ books", String(count)), systemImage: FolioAction.applyBulkEdit.symbol.name)
                }
                    .buttonStyle(.borderedProminent)
                    .disabled(!draft.hasChanges || queue.isPreflighting || queue.isBatchConverting || queue.hasActiveConversions)
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 14)
        }
    }
}

private struct EditorSectionBlock<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title)
                .font(.headline)
            Divider()
            content
        }
    }
}

private struct SettingsRow<Content: View>: View {
    let label: String
    let content: Content

    init(label: String, @ViewBuilder content: () -> Content) {
        self.label = label
        self.content = content()
    }

    var body: some View {
        LabeledContent {
            content.frame(maxWidth: .infinity, alignment: .leading)
        } label: {
            Text(label)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct TokenChipEditor: View {
    @Binding var tokens: [String]
    let placeholder: String
    @StateObject private var localState = TokenChipEditorLocalState()

    private let columns = [GridItem(.adaptive(minimum: 110, maximum: 210), alignment: .leading)]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !tokens.isEmpty {
                LazyVGrid(columns: columns, alignment: .leading, spacing: 6) {
                    ForEach(tokens.indices, id: \.self) { index in
                        HStack(spacing: 5) {
                            Text(tokens[index])
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Button {
                                tokens.remove(at: index)
                            } label: {
                                Image(folioSymbol: FolioAction.removeToken.symbol)
                                    .foregroundStyle(.tertiary)
                            }
                            .buttonStyle(.plain)
                            .help(FolioL10n.format("error.remove_token", default: "Remove %@", tokens[index]))
                        }
                        .font(.caption)
                        .padding(.horizontal, 9)
                        .padding(.vertical, 6)
                        .background(Color.secondary.opacity(0.09), in: Capsule())
                    }
                }
            }
            HStack(spacing: 6) {
                TextField(placeholder, text: $localState.entry)
                    .textFieldStyle(.plain)
                    .onSubmit(addToken)
                Button(action: addToken) {
                    Label(FolioL10n.string("ui.add", default: "Add"), systemImage: FolioAction.addToken.symbol.name)
                }
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
                .disabled(localState.entry.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .help(FolioL10n.string("ui.add_value", default: "Add value"))
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 7)
            .background(Color.secondary.opacity(0.055), in: RoundedRectangle(cornerRadius: 7))
        }
    }

    private func addToken() {
        let value = localState.entry.trimmingCharacters(in: .whitespacesAndNewlines)
        if !value.isEmpty, !tokens.contains(value) { tokens.append(value) }
        localState.entry = ""
    }
}

private struct BookCoverPlaceholder: View {
    let title: String
    let width: CGFloat
    let height: CGFloat

    var body: some View {
        VStack(spacing: 9) {
            Image(folioSymbol: .bookFilled)
                .font(.system(size: min(width * 0.25, 36), weight: .light))
                .foregroundStyle(.secondary)
            Text(title)
                .font(.caption.weight(.medium))
                .lineLimit(3)
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 9)
        }
        .frame(width: width, height: height)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 10))
        .overlay {
            RoundedRectangle(cornerRadius: 10)
                .strokeBorder(Color.secondary.opacity(0.12), lineWidth: 1)
        }
    }
}

private struct StatusBadge: View {
    let title: String
    let color: Color

    var body: some View {
        Text(title)
            .font(.caption.weight(.semibold))
            .foregroundStyle(color)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(color.opacity(0.10), in: Capsule())
            .fixedSize()
    }
}
