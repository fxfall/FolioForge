import AppKit
import Combine
import SwiftUI
import UniformTypeIdentifiers

@MainActor
final class ContentViewState: ObservableObject {
    @Published var inspectReport: FolioInspectReport?
    @Published var previewBundle: FolioReaderPreviewBundle?
    @Published var detailError: String?
    @Published var previewError: String?
    @Published var isLoadingDetails = false
    @Published var isLoadingPreview = false
    @Published var showInspector = false
    @Published var columnVisibility: NavigationSplitViewVisibility = .detailOnly
    @Published var previewSettings = FolioPreviewSettings()
    @Published var editorSection: EditorSection = .summary
    @Published var appearancePage: AppearancePage = .cover
    @Published var selectedFontID: String?
    @Published var inspectorSection: InspectorTab = .output
    @Published var bulkEditMessage: String?
    let previewImageCache = FolioReaderImageCache()

    var lastSelectionKey: String?
    var previewTask: Task<Void, Never>?
    var detailRequestID = UUID()
    var previewRequestID = UUID()
}

struct ContentView: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel
    @StateObject private var state = ContentViewState()
    @StateObject private var bulkDraft = BulkEditDraft()
    private let bridge = FolioCoreBridge()

    var body: some View {
        appWorkspace
            .toolbar { toolbarContent }
            .task {
                synchronizeSidebarVisibility()
                handleSelectionChange()
            }
            .onChange(of: queue.items.isEmpty) { _ in synchronizeSidebarVisibility() }
            .onChange(of: queue.selectedIDs) { _ in handleSelectionChange() }
            .onChange(of: queue.target) { _ in compatibilitySettingsChanged() }
            .onChange(of: queue.degradationMode) { _ in compatibilitySettingsChanged() }
            .onChange(of: queue.linearizeComplexTables) { _ in compatibilitySettingsChanged() }
            .onChange(of: queue.preferRasterization) { _ in compatibilitySettingsChanged() }
            .onChange(of: queue.stripEmbeddedFonts) { _ in compatibilitySettingsChanged() }
            .onChange(of: queue.textImportOptions) { _ in compatibilitySettingsChanged() }
            .onChange(of: state.previewSettings) { _ in schedulePreview() }
            .onChange(of: state.selectedFontID) { id in
                if id != nil { state.showInspector = true }
            }
            .sheet(isPresented: $queue.showingCapabilities) {
                CapabilityMatrixView().environmentObject(queue)
            }
            .alert(FolioL10n.string("ui.folioforge", default: "FolioForge"), isPresented: Binding(
                get: { queue.lastError != nil },
                set: { if !$0 { queue.lastError = nil } }
            )) {
                Button(FolioL10n.string("ui.ok", default: "OK")) { queue.lastError = nil }
            } message: {
                Text(queue.lastError ?? FolioL10n.string("error.unknown", default: "Unknown error"))
            }
    }

    @ViewBuilder
    private var appWorkspace: some View {
        if #available(macOS 14.0, *) {
            workspace
                .inspector(isPresented: $state.showInspector) {
                    inspector
                        .inspectorColumnWidth(min: 290, ideal: 340, max: 420)
                }
        } else {
            workspace
                .sheet(isPresented: $state.showInspector) {
                    inspector.frame(minWidth: 320, minHeight: 520)
                }
        }
    }

    private var workspace: some View {
        NavigationSplitView(columnVisibility: $state.columnVisibility) {
            BatchSidebarView()
                .navigationSplitViewColumnWidth(min: 230, ideal: 250, max: 290)
        } detail: {
            VStack(spacing: 0) {
                if queue.items.isEmpty {
                    EmptyWorkspaceView()
                        .padding(22)
                } else if queue.selectedCount > 1 {
                    HSplitView {
                        BulkEditPane(
                            draft: bulkDraft,
                            count: queue.selectedCount,
                            target: queue.target,
                            mode: queue.degradationMode,
                            message: state.bulkEditMessage,
                            apply: applyBulkEdit
                        )
                        .frame(minWidth: 360, idealWidth: 440, maxWidth: 520)

                        BulkSelectionSummaryPane(items: queue.selectedItems)
                            .frame(minWidth: 360, idealWidth: 520)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 12)
                    WorkspaceStatusBar()
                } else if let item = queue.selectedItem {
                    HSplitView {
                        BookEditorPane(
                            item: item,
                            edits: selectedEditPlanBinding,
                            report: state.inspectReport,
                            analysis: item.analysis,
                            section: $state.editorSection,
                            appearancePage: $state.appearancePage,
                            selectedFontID: $state.selectedFontID,
                            isLoading: state.isLoadingDetails,
                            detailError: state.detailError,
                            canEdit: canEditSelectedItem,
                            convert: queue.convertSelected
                        )
                        .frame(minWidth: 390, idealWidth: 455, maxWidth: 520)
                        .layoutPriority(0)

                        CompatibilityPreviewPane(
                            bundle: state.previewBundle,
                            isLoading: state.isLoadingPreview,
                            error: state.previewError,
                            settings: $state.previewSettings,
                            refresh: requestPreview,
                            imageCache: state.previewImageCache,
                            imageSessionID: state.previewRequestID.uuidString
                        )
                        .frame(minWidth: 420, idealWidth: 610)
                        .layoutPriority(1)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 12)
                    WorkspaceStatusBar()
                } else {
                    NoSelectionWorkspaceView()
                        .padding(22)
                    WorkspaceStatusBar()
                }
            }
            .navigationTitle(queue.selectedItem?.displayPath ?? "FolioForge")
        }
        .navigationSplitViewStyle(.balanced)
        .onDrop(of: [UTType.fileURL.identifier], isTargeted: $queue.isDropTargeted) { providers in
            queue.importProviders(providers)
            return true
        }
    }

    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
        ToolbarItemGroup(placement: .navigation) {
            Button { queue.openFiles() } label: {
                Label(FolioL10n.string("ui.files", default: "Files"), systemImage: FolioAction.addFiles.symbol.name)
            }
            .labelStyle(.titleAndIcon)
            .keyboardShortcut("o", modifiers: [.command])
            .help(FolioL10n.format(
                "queue.add_files_supported_formats",
                default: "Add files. Supported formats: %@",
                queue.inputFormatSummary
            ))

            Button { queue.openFolder() } label: {
                Label(FolioL10n.string("ui.folder", default: "Folder"), systemImage: FolioAction.addFolder.symbol.name)
            }
            .labelStyle(.titleAndIcon)
            .keyboardShortcut("o", modifiers: [.command, .shift])
        }

        ToolbarItem(placement: .principal) {
            if !queue.items.isEmpty {
                HStack(spacing: 8) {
                    Text(FolioL10n.string("ui.target", default: "Target"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Picker(FolioL10n.string("ui.target", default: "Target"), selection: $queue.target) {
                        ForEach(queue.availableTargets) { target in
                            Text(target.displayName).tag(target)
                        }
                    }
                    .pickerStyle(.menu)
                    .labelsHidden()
                    .accessibilityLabel(FolioL10n.string("ui.target_format", default: "Target format"))
                    .frame(width: 150)

                    Text(FolioL10n.string("ui.mode", default: "Mode"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Picker(FolioL10n.string("ui.mode", default: "Mode"), selection: $queue.degradationMode) {
                        ForEach(FolioDegradationMode.allCases) { mode in
                            Text(mode.displayName).tag(mode)
                        }
                    }
                    .pickerStyle(.menu)
                    .labelsHidden()
                    .accessibilityLabel(FolioL10n.string("ui.compatibility_mode", default: "Compatibility mode"))
                    .frame(width: 126)
                }
            }
        }

        ToolbarItemGroup(placement: .primaryAction) {
            if !queue.items.isEmpty {
                Button { queue.preflightSelected() } label: {
                    Label(FolioL10n.string("ui.check", default: "Check"), systemImage: FolioAction.check.symbol.name)
                }
                .labelStyle(.titleAndIcon)
                .disabled(queue.isPreflighting || queue.isBatchConverting || queue.isScanningFolder || queue.hasActiveConversions)

                Button { queue.convertSelected() } label: {
                    Label(FolioL10n.string("ui.convert", default: "Convert"), systemImage: FolioAction.convert.symbol.name)
                }
                .labelStyle(.titleAndIcon)
                .buttonStyle(.borderedProminent)
                .disabled(!queue.selectedCanConvert)

                Button { state.showInspector.toggle() } label: {
                    Label(FolioL10n.string("ui.inspector", default: "Inspector"), systemImage: FolioAction.inspect.symbol.name)
                }
                .labelStyle(.titleAndIcon)
            }

            Menu {
                if !queue.items.isEmpty {
                    Button { queue.preflightAll() } label: { Label(FolioL10n.string("ui.check_all_books", default: "Check All Books"), systemImage: FolioAction.checkAll.symbol.name) }
                        .disabled(queue.isPreflighting || queue.isBatchConverting || queue.isScanningFolder)
                    Button { queue.convertAll() } label: { Label(FolioL10n.string("ui.convert_all_approved", default: "Convert All Approved"), systemImage: FolioAction.convertAll.symbol.name) }
                        .disabled(queue.isPreflighting || queue.isBatchConverting || queue.isScanningFolder || !queue.hasPreflightForAll)
                    Divider()
                }
                Button { queue.showingCapabilities = true } label: { Label(FolioL10n.string("ui.capabilities", default: "Capabilities…"), systemImage: FolioAction.capabilities.symbol.name) }
            } label: {
                Label(FolioL10n.string("ui.more", default: "More"), systemImage: FolioAction.more.symbol.name)
            }
            .labelStyle(.titleAndIcon)
        }
    }

    @ViewBuilder
    private var inspector: some View {
        Phase3InspectorView(
            item: queue.selectedItem,
            report: state.inspectReport,
            analysis: queue.selectedItem?.analysis,
            preview: state.previewBundle,
            selectedFont: selectedFont,
            detailError: state.detailError,
            previewError: state.previewError,
            selection: $state.inspectorSection
        )
        .environmentObject(queue)
    }

    private var selectedFont: SemanticFontRow? {
        guard let selectedFontID = state.selectedFontID, let item = queue.selectedItem else { return nil }
        let snapshot = SemanticBookSnapshot(report: state.inspectReport, fallbackTitle: item.displayPath)
        return snapshot.fonts.first { $0.id == selectedFontID }
    }

    private var canEditSelectedItem: Bool {
        guard let item = queue.selectedItem else { return false }
        return !queue.isPreflighting && !queue.isBatchConverting && item.status != .converting
    }

    private var selectedEditPlanBinding: Binding<FolioBookEditPlan> {
        Binding(
            get: { queue.selectedItem?.editPlan ?? FolioBookEditPlan() },
            set: { plan in
                guard let id = queue.selectedID else { return }
                queue.setEditPlan(plan, for: id)
                state.previewBundle = nil
                schedulePreview()
            }
        )
    }

    private func handleSelectionChange() {
        let selectionKey = queue.items
            .filter { queue.selectedIDs.contains($0.id) }
            .map { $0.id.uuidString }
            .joined(separator: ",")
        guard state.lastSelectionKey != selectionKey else { return }
        state.lastSelectionKey = selectionKey
        state.editorSection = .summary
        state.appearancePage = .cover
        state.selectedFontID = nil
        state.bulkEditMessage = nil

        guard queue.selectedCount == 1, let item = queue.selectedItem else {
            state.inspectReport = nil
            state.previewBundle = nil
            state.detailError = nil
            state.previewError = nil
            state.isLoadingDetails = false
            state.isLoadingPreview = false
            state.previewRequestID = UUID()
            return
        }
        refreshDetails(for: item)
        requestPreview(for: item)
    }

    private func synchronizeSidebarVisibility() {
        state.columnVisibility = queue.items.isEmpty ? .detailOnly : .all
    }

    private func refreshDetails(for item: BookItem) {
        state.detailRequestID = UUID()
        let requestID = state.detailRequestID
        state.isLoadingDetails = true
        state.detailError = nil
        let bridge = self.bridge
        Task {
            let result = await Task.detached(priority: .userInitiated) {
                Result { try bridge.inspect(url: item.inputURL) }
            }.value
            guard state.detailRequestID == requestID, queue.selectedID == item.id, queue.selectedCount == 1 else { return }
            state.isLoadingDetails = false
            switch result {
            case .success(let report): state.inspectReport = report
            case .failure(let error): state.detailError = error.localizedDescription
            }
        }
    }

    private func requestPreview() {
        guard queue.selectedCount == 1, let item = queue.selectedItem else { return }
        requestPreview(for: item)
    }

    private func requestPreview(for item: BookItem) {
        state.previewRequestID = UUID()
        state.previewImageCache.removeAll()
        let requestID = state.previewRequestID
        let edit = item.editPlan
        let target = queue.target
        let mode = queue.degradationMode
        let options = queue.degradationOptions
        let settings = state.previewSettings
        let textOptions = queue.textImportOptions
        state.isLoadingPreview = true
        state.previewError = nil
        let bridge = self.bridge
        Task {
            let result = await Task.detached(priority: .userInitiated) {
                Result {
                    try bridge.preview(
                        url: item.inputURL,
                        target: target,
                        mode: mode,
                        degradation: options,
                        edit: edit,
                        settings: settings,
                        text: textOptions
                    )
                }
            }.value
            guard state.previewRequestID == requestID, queue.selectedID == item.id, queue.selectedCount == 1 else { return }
            state.isLoadingPreview = false
            switch result {
            case .success(let bundle): state.previewBundle = bundle
            case .failure(let error): state.previewError = error.localizedDescription
            }
        }
    }

    private func schedulePreview() {
        guard queue.selectedCount == 1 else { return }
        state.previewTask?.cancel()
        state.previewTask = Task { @MainActor in
            try? await Task.sleep(nanoseconds: 350_000_000)
            guard !Task.isCancelled else { return }
            requestPreview()
        }
    }

    private func compatibilitySettingsChanged() {
        queue.invalidatePreflight()
        state.previewBundle = nil
        schedulePreview()
    }

    private func applyBulkEdit() {
        let payload = bulkDraft.payload
        queue.applyBulkEdit(
            to: queue.selectedItems.map(\.id),
            typography: payload.typography,
            stripEmbeddedFonts: payload.stripEmbeddedFonts,
            style: payload.style
        )
        state.bulkEditMessage = "Applied to \(queue.selectedCount) books. Run Check again before converting."
    }
}

private struct BatchSidebarView: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel

    var body: some View {
        VStack(spacing: 8) {
            HStack(spacing: 9) {
                FolioForgeLogo()
                    .frame(width: 34, height: 34)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                VStack(alignment: .leading, spacing: 1) {
                    Text(FolioL10n.string("ui.folioforge", default: "FolioForge")).font(.headline)
                    Text(FolioL10n.string("ui.batch", default: "Batch")).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Text("\(queue.items.count)")
                    .font(.caption.monospacedDigit().weight(.medium))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(.quaternary, in: Capsule())
            }
            .padding(.horizontal, 14)
            .padding(.top, 12)

            QueueListView()

            if !queue.items.isEmpty {
                Divider().padding(.horizontal, 12)
                HStack(spacing: 8) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(FolioL10n.format("status.count_checked", default: "Checked: %@", String(queue.preflightReadyCount)))
                            .font(.caption.weight(.medium))
                        Text(FolioL10n.format("status.count_blocked", default: "Blocked: %@", String(queue.preflightBlockedCount)))
                            .font(.caption2)
                            .foregroundStyle(queue.preflightBlockedCount > 0 ? .orange : .secondary)
                    }
                    Spacer(minLength: 4)
                    if queue.selectedCount > 1 {
                        Button { queue.removeSelected() } label: { Label(FolioL10n.string("ui.remove_selected", default: "Remove Selected"), systemImage: FolioAction.removeSelected.symbol.name) }
                            .disabled(queue.isBatchConverting || queue.hasActiveConversions)
                            .help(FolioL10n.string("ui.remove_selected_books_from_the_batch", default: "Remove selected books from the batch"))
                    }
                    Button(FolioL10n.string("ui.clear", default: "Clear")) { queue.removeAll() }
                        .disabled(queue.isBatchConverting || queue.hasActiveConversions)
                }
                .padding(.horizontal, 14)
                .padding(.bottom, 12)
            }
        }
        .background(.regularMaterial)
    }
}

private struct QueueListView: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel

    var body: some View {
        List(selection: $queue.selectedIDs) {
            ForEach(queue.items) { item in
                QueueRow(item: item, isBatchConverting: queue.isBatchConverting) {
                    if item.status == .converting { queue.cancel(item.id) }
                }
                .tag(item.id)
                .contextMenu {
                    Button(FolioL10n.string("ui.check", default: "Check")) {
                        queue.selectedID = item.id
                        queue.preflightSelected()
                    }
                    if item.status == .converting {
                        Button(FolioL10n.string(
                            queue.isBatchConverting ? "status.cancel_batch" : "ui.cancel",
                            default: queue.isBatchConverting ? "Cancel the batch" : "Cancel"
                        )) { queue.cancel(item.id) }
                    } else if item.status == .completed {
                        Button(FolioL10n.string("ui.reveal_in_finder", default: "Reveal in Finder")) { queue.revealOutput(item) }
                    }
                    Divider()
                    Button(FolioL10n.string("ui.remove", default: "Remove"), role: .destructive) { queue.remove(item.id) }
                }
            }
        }
        .listStyle(.sidebar)
        .overlay {
            if queue.items.isEmpty {
                VStack(spacing: 7) {
                    Image(folioSymbol: .books)
                        .font(.title2)
                        .foregroundStyle(.tertiary)
                    Text(FolioL10n.string("ui.no_books_yet", default: "No books yet")).font(.subheadline.weight(.medium))
                    Text(FolioL10n.string("ui.use_files_or_folder_to_add_a_batch", default: "Use Files or Folder to add a batch."))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                }
                .padding(24)
                .allowsHitTesting(false)
            }
        }
    }
}

private struct QueueRow: View {
    let item: BookItem
    let isBatchConverting: Bool
    let cancel: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(folioSymbol: iconSymbol)
                .foregroundStyle(iconColor)
                .frame(width: 18)
                .padding(.top, 2)
            VStack(alignment: .leading, spacing: 4) {
                Text(item.displayPath)
                    .font(.body.weight(.medium))
                    .lineLimit(2)
                Text(detailLine)
                    .font(.caption)
                    .foregroundStyle(detailColor)
                    .lineLimit(2)
                if item.status == .converting {
                    ProgressView(value: item.progress)
                        .controlSize(.small)
                }
            }
            Spacer(minLength: 0)
            if item.status == .converting {
                Button(action: cancel) {
                    Image(folioSymbol: .stop)
                }
                .buttonStyle(.borderless)
                .help(isBatchConverting
                    ? FolioL10n.string("status.cancel_batch", default: "Cancel the batch")
                    : FolioL10n.string("status.cancel_conversion", default: "Cancel conversion"))
            }
        }
        .padding(.vertical, 5)
        .contentShape(Rectangle())
    }

    private var detailLine: String {
        if let analysis = item.analysis {
            return "\(analysis.sourceFormat) → \(analysis.targetFormat) · \(analysis.plan.quality.userSummary)"
        }
        if let error = item.analysisError {
            return FolioL10n.format("status.check_failed", default: "Check failed · %@", error)
        }
        if let stage = item.stage, item.status == .converting { return stage.label }
        return "\(item.inputURL.pathExtension.uppercased()) · \(item.status.label)"
    }

    private var detailColor: Color {
        if item.analysis?.plan.blocked == true { return .orange }
        if item.analysisError != nil || item.status == .failed { return .red }
        if item.status == .completed { return .green }
        return .secondary
    }

    private var iconSymbol: FolioSymbol {
        switch item.status {
        case .completed: .success
        case .failed: .error
        case .converting: .progress
        case .cancelled: .pause
        case .ready:
            if item.analysis?.plan.blocked == true { .warning }
            else if item.analysis != nil { .checkSealFilled }
            else { .book }
        }
    }

    private var iconColor: Color {
        switch item.status {
        case .completed: .green
        case .failed: .red
        case .converting: .accentColor
        case .cancelled: .orange
        case .ready: item.analysis?.plan.blocked == true ? .orange : .accentColor
        }
    }
}

private struct EmptyWorkspaceView: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel

    var body: some View {
        VStack(spacing: 18) {
            Spacer(minLength: 20)
            Image(folioSymbol: .books)
                .font(.system(size: 48, weight: .light))
                .foregroundStyle(.tint)
            VStack(spacing: 7) {
                Text(FolioL10n.string("ui.add_books_to_folioforge", default: "Add books to FolioForge"))
                    .font(.system(size: 25, weight: .semibold, design: .rounded))
                Text(queue.inputFormatSummary)
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.secondary)
            }
            HStack(spacing: 12) {
                Button { queue.openFiles() } label: {
                    Label(FolioL10n.string("ui.add_files", default: "Add Files"), systemImage: FolioAction.addFiles.symbol.name)
                }
                .buttonStyle(.borderedProminent)
                Button { queue.openFolder() } label: {
                    Label(FolioL10n.string("ui.add_folder", default: "Add Folder"), systemImage: FolioAction.addFolder.symbol.name)
                }
                .buttonStyle(.bordered)
            }
            Text(FolioL10n.string("ui.or_drop_books_anywhere_in_this_window", default: "or drop books anywhere in this window"))
                .font(.caption)
                .foregroundStyle(.secondary)
            Spacer(minLength: 20)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background {
            RoundedRectangle(cornerRadius: 18)
                .fill(Color.secondary.opacity(queue.isDropTargeted ? 0.12 : 0.045))
                .overlay {
                    RoundedRectangle(cornerRadius: 18)
                        .strokeBorder(
                            queue.isDropTargeted ? Color.accentColor : Color.secondary.opacity(0.22),
                            style: StrokeStyle(lineWidth: queue.isDropTargeted ? 2 : 1, dash: [7, 6])
                        )
                }
        }
        .onDrop(of: [UTType.fileURL.identifier], isTargeted: $queue.isDropTargeted) { providers in
            queue.importProviders(providers)
            return true
        }
    }
}

private struct NoSelectionWorkspaceView: View {
    var body: some View {
        VStack(spacing: 12) {
                    Image(folioSymbol: .sidebarLeft)
                .font(.system(size: 34))
                .foregroundStyle(.secondary)
            Text(FolioL10n.string("ui.select_books_in_the_batch", default: "Select books in the batch"))
                .font(.title3.weight(.semibold))
            Text(FolioL10n.string("ui.select_one_book_to_edit_and_preview_it_or", default: "Select one book to edit and preview it, or select several to apply bulk changes."))
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 420)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct BulkSelectionSummaryPane: View {
    let items: [BookItem]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Label(FolioL10n.string("ui.selection_summary", default: "Selection Summary"), systemImage: FolioSymbol.books.name)
                    .font(.headline)
                Spacer()
                Text(FolioL10n.format("status.count_selected", default: "Selected: %@", String(items.count)))
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
            }
            .padding(.bottom, 12)
            Divider()
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(items) { item in
                        HStack(alignment: .top, spacing: 12) {
                            Image(folioSymbol: item.status == .completed ? .success : .book)
                                .foregroundStyle(item.status == .completed ? .green : .secondary)
                                .frame(width: 20)
                                .padding(.top, 2)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(item.displayPath).font(.body.weight(.medium)).lineLimit(2)
                                Text("\(item.analysis?.sourceFormat ?? item.inputURL.pathExtension.uppercased()) · \(item.status.label)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                if let analysis = item.analysis {
                                    Text(analysis.plan.blocked
                                        ? FolioL10n.format("status.quality_summary", default: "%@ · Blocked", analysis.plan.quality.userSummary)
                                        : analysis.plan.quality.userSummary)
                                        .font(.caption)
                                        .foregroundStyle(analysis.plan.blocked ? .orange : .secondary)
                                } else if let error = item.analysisError {
                                    Text(error).font(.caption).foregroundStyle(.red).lineLimit(3)
                                }
                            }
                            Spacer(minLength: 0)
                        }
                        .padding(.vertical, 10)
                        Divider()
                    }
                }
            }
            .padding(.top, 4)
        }
        .padding(14)
        .background(Color(nsColor: .controlBackgroundColor))
    }
}

private struct WorkspaceStatusBar: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel

    var body: some View {
        HStack(spacing: 10) {
            if queue.hasActiveConversions {
                Image(folioSymbol: .progress)
                    .foregroundStyle(Color.accentColor)
                Text(queue.conversionStatus)
                    .font(.caption.weight(.semibold))
                if let item = queue.convertingItems.first {
                    Text(item.displayPath)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                ProgressView(value: queue.conversionProgress)
                    .frame(maxWidth: 150)
                Spacer(minLength: 8)
                Button(queue.isBatchCancelling
                    ? FolioL10n.string("status.cancelling_batch", default: "Cancelling batch…")
                    : FolioL10n.string("ui.cancel", default: "Cancel")) { queue.cancelConversions() }
                    .disabled(!queue.hasActiveConversions || queue.isBatchCancelling)
            } else {
                Image(folioSymbol: queue.preflightBlockedCount > 0 ? .diagnostics : .check)
                    .foregroundStyle(queue.preflightBlockedCount > 0 ? .orange : .secondary)
                Text(queue.items.isEmpty
                    ? FolioL10n.string("status.ready_for_books", default: "Ready for books")
                    : FolioL10n.format("status.batch_summary", default: "Books: %@ · Checked: %@", String(queue.items.count), String(queue.preflightReadyCount)))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                if queue.preflightBlockedCount > 0 {
                    Text(FolioL10n.format("status.count_attention", default: "Needs attention: %@", String(queue.preflightBlockedCount)))
                        .font(.caption)
                        .foregroundStyle(.orange)
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.bar)
        .overlay(alignment: .top) { Divider() }
    }
}
