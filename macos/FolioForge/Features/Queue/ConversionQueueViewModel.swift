import AppKit
import Foundation
import SwiftUI
import UniformTypeIdentifiers

private struct PreflightFailure: Error {
    let message: String
}

@MainActor
final class ConversionQueueViewModel: ObservableObject {
    @Published private(set) var items: [BookItem] = []
    @Published var selectedIDs: Set<BookItem.ID> = [] {
        didSet {
            guard !synchronizingSelection, selectedIDs != oldValue else { return }
            synchronizingSelection = true
            if let selectedID, selectedIDs.contains(selectedID) {
                synchronizingSelection = false
                return
            }
            selectedID = items.first(where: { selectedIDs.contains($0.id) })?.id
            synchronizingSelection = false
        }
    }
    @Published var selectedID: BookItem.ID? {
        didSet {
            guard !synchronizingSelection, selectedID != oldValue else { return }
            synchronizingSelection = true
            selectedIDs = selectedID.map { Set([$0]) } ?? []
            synchronizingSelection = false
        }
    }
    @Published var target: FolioTarget = .epub
    @Published var compression: FolioCompression = .none
    @Published var degradationMode: FolioDegradationMode = .compatible
    @Published var batchMode: FolioBatchMode = .bestEffort
    @Published var linearizeComplexTables = false
    @Published var preferRasterization = false
    @Published var stripEmbeddedFonts = false
    @Published var deterministic = true
    @Published var textImportMode: FolioTextImportMode = .auto
    @Published var paragraphMode: FolioParagraphMode = .auto
    @Published var textEncodingOverride: FolioTextEncoding?
    @Published var textTitleOverride = ""
    @Published var textAuthorOverride = ""
    @Published var outputFolder: URL?
    @Published var lastError: String?
    @Published var isDropTargeted = false
    @Published var showingCapabilities = false
    @Published private(set) var isPreflighting = false
    @Published private(set) var isBatchConverting = false
    @Published private(set) var isBatchCancelling = false
    @Published private(set) var isScanningFolder = false
    @Published private(set) var capabilityProfiles: [FolioCapabilityProfile] = []
    @Published private(set) var inputFormatCapabilities: [FolioInputFormatCapability] = []
    @Published private(set) var targetFormats: [FolioTarget] = []
    @Published private(set) var capabilityError: String?

    private let bridge = FolioCoreBridge()
    private var activeBatchHandle: FolioCancellationHandle?
    private var synchronizingSelection = false
    private var outputFolderAccessURL: URL?
    private var sourceFolderAccessURLs: [URL] = []
    private var plannedBatchIDs: [BookItem.ID] = []
    private var completedBatchIDs: [BookItem.ID] = []
    private var currentBatchItemIndex = 0
    private var activeBatchMode: FolioBatchMode = .bestEffort

    private var supportedExtensions: Set<String> {
        Set(inputFormatCapabilities.flatMap(\.extensions))
    }

    var inputFormatSummary: String {
        let names = inputFormatCapabilities.map(\.format)
        return names.isEmpty ? "Registered input formats" : names.joined(separator: " · ")
    }

    var availableTargets: [FolioTarget] {
        targetFormats.isEmpty ? [target] : targetFormats
    }

    var textImportOptions: FolioTextImportOptions {
        FolioTextImportOptions(
            mode: textImportMode,
            paragraphMode: paragraphMode,
            encodingOverride: textEncodingOverride,
            titleOverride: textTitleOverride.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                ? nil : textTitleOverride,
            authorOverride: textAuthorOverride.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                ? nil : textAuthorOverride
        )
    }

    init() {
        NotificationService.requestAuthorization()
        loadCapabilities()
    }

    var coreVersion: String { bridge.version }

    var selectedItem: BookItem? {
        guard let selectedID else { return nil }
        return items.first { $0.id == selectedID }
    }

    var selectedItems: [BookItem] {
        items.filter { selectedIDs.contains($0.id) }
    }

    var selectedCount: Int { selectedItems.count }

    var convertingItems: [BookItem] {
        items.filter { $0.status == .converting }
    }

    var hasActiveConversions: Bool { isBatchConverting || !convertingItems.isEmpty }

    var conversionProgress: Double? {
        if isBatchConverting, !plannedBatchIDs.isEmpty {
            let activeProgress = convertingItems.compactMap(\.progress).first ?? 0
            return min(1, (Double(completedBatchIDs.count) + activeProgress) / Double(plannedBatchIDs.count))
        }
        return convertingItems.first?.progress
    }

    var conversionStatus: String {
        if isBatchCancelling { return "Cancelling batch…" }
        if isBatchConverting, !plannedBatchIDs.isEmpty {
            return "Converting \(min(max(currentBatchItemIndex, 1), plannedBatchIDs.count)) of \(plannedBatchIDs.count)"
        }
        return convertingItems.first?.stage?.label ?? "Converting"
    }

    var outputDestinationDescription: String {
        if let outputFolder { return outputFolder.path }
        if items.contains(where: { $0.sourceRoot != nil }) { return "FolioForge-output beside source folder" }
        return "Same folder as each source"
    }

    var degradationOptions: FolioDegradationOptions {
        FolioDegradationOptions(
            linearizeComplexTables: linearizeComplexTables,
            preferRasterization: preferRasterization,
            stripEmbeddedFonts: stripEmbeddedFonts
        )
    }

    var hasPreflightForAll: Bool {
        !items.isEmpty && items.allSatisfy(\.hasPreflightResult)
    }

    var preflightReadyCount: Int {
        items.filter { $0.analysis?.plan.blocked == false }.count
    }

    var preflightBlockedCount: Int {
        items.filter { $0.analysis?.plan.blocked == true || $0.analysisError != nil }.count
    }

    var preflightSummary: String {
        guard !items.isEmpty else { return "No files" }
        let analyzed = items.filter(\.hasPreflightResult).count
        let exact = items.filter { $0.analysis?.plan.quality == .exact }.count
        let high = items.filter { $0.analysis?.plan.quality == .high }.count
        let compatible = items.filter { $0.analysis?.plan.quality == .compatible }.count
        let reduced = items.filter { $0.analysis?.plan.quality == .reduced || $0.analysis?.plan.quality == .severeLoss }.count
        return "\(analyzed)/\(items.count) analyzed · \(exact) excellent · \(high) high fidelity · \(compatible) minor changes · \(reduced) fallback · \(preflightBlockedCount) blocked"
    }

    var selectedCanConvert: Bool {
        let selection = selectedItems
        guard !selection.isEmpty else { return false }
        guard !isPreflighting, !isBatchConverting, !isScanningFolder, !hasActiveConversions else { return false }
        guard selection.allSatisfy(\.hasPreflightResult) else { return false }
        let blocked = selection.contains { $0.analysis?.plan.blocked != false }
        if batchMode == .strict && blocked { return false }
        return selection.contains { item in
            (item.status == .ready || item.status == .failed || item.status == .cancelled || item.status == .completed)
                && item.analysis?.plan.blocked == false
        }
    }

    func add(urls: [URL]) {
        for url in urls {
            var isDirectory = ObjCBool(false)
            if FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory), isDirectory.boolValue {
                addFolder(url)
            } else {
                addFile(url)
            }
        }
        if selectedID == nil { selectedID = items.first?.id }
    }

    func importProviders(_ providers: [NSItemProvider]) {
        for provider in providers {
            provider.loadDataRepresentation(forTypeIdentifier: "public.file-url") { [weak self] data, _ in
                guard let data,
                      let url = URL(dataRepresentation: data, relativeTo: nil)
                else { return }
                Task { @MainActor [weak self] in self?.add(urls: [url]) }
            }
        }
    }

    func openFiles() {
        guard !supportedExtensions.isEmpty else {
            lastError = "Folio Core input capabilities are not available yet. Try again in a moment."
            loadCapabilities()
            return
        }
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.allowedContentTypes = supportedExtensions.sorted().compactMap {
            UTType(filenameExtension: $0)
        }
        panel.begin { [weak self] response in
            guard response == .OK else { return }
            Task { @MainActor [weak self] in self?.add(urls: panel.urls) }
        }
    }

    func openFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.begin { [weak self] response in
            guard response == .OK, let folder = panel.url else { return }
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.addFolder(folder)
            }
        }
    }

    func chooseOutputFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.begin { [weak self] response in
            guard response == .OK, let folder = panel.url else { return }
            Task { @MainActor [weak self] in
                guard let self else { return }
                if let oldFolder = self.outputFolderAccessURL {
                    oldFolder.stopAccessingSecurityScopedResource()
                }
                self.outputFolder = folder
                self.outputFolderAccessURL = folder.startAccessingSecurityScopedResource() ? folder : nil
            }
        }
    }

    func resetOutputFolder() {
        outputFolderAccessURL?.stopAccessingSecurityScopedResource()
        outputFolderAccessURL = nil
        outputFolder = nil
    }

    func preflightSelected() {
        let ids = selectedItems.map(\.id)
        if !ids.isEmpty {
            preflight(ids: ids)
        } else if let selectedID {
            preflight(ids: [selectedID])
        } else {
            preflightAll()
        }
    }

    func preflightAll() {
        preflight(ids: items.map(\.id))
    }

    func loadCapabilities() {
        let bridge = self.bridge
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            do {
                let snapshot = try bridge.capabilitySnapshot()
                Task { @MainActor [weak self] in
                    self?.capabilityProfiles = snapshot.capabilityProfiles
                    self?.inputFormatCapabilities = snapshot.inputFormats
                    self?.targetFormats = snapshot.targets
                    if let first = snapshot.targets.first,
                       !snapshot.targets.contains(where: { $0.rawValue == self?.target.rawValue }) {
                        self?.target = first
                    }
                    self?.capabilityError = nil
                }
            } catch {
                let message = error.localizedDescription
                Task { @MainActor [weak self] in
                    self?.capabilityError = message
                }
            }
        }
    }

    func invalidatePreflight() {
        guard !isPreflighting else { return }
        for index in items.indices {
            items[index].analysis = nil
            items[index].analysisError = nil
        }
    }

    func setEditPlan(_ plan: FolioBookEditPlan, for id: BookItem.ID) {
        guard !isPreflighting, !isBatchConverting else { return }
        update(id) { item in
            item.editPlan = plan
            item.analysis = nil
            item.analysisError = nil
        }
    }

    func applyBulkEdit(
        to ids: [BookItem.ID],
        typography: FolioTypographyEdit? = nil,
        stripEmbeddedFonts: Bool? = nil,
        style: FolioStyleEdit? = nil
    ) {
        guard !isPreflighting, !isBatchConverting, !hasActiveConversions else { return }
        for id in ids {
            update(id) { item in
                if let typography {
                    if let value = typography.fontFamily { item.editPlan.typography.fontFamily = value }
                    if let value = typography.bodyFontSize { item.editPlan.typography.bodyFontSize = value }
                    if let value = typography.lineHeight { item.editPlan.typography.lineHeight = value }
                    if let value = typography.letterSpacing { item.editPlan.typography.letterSpacing = value }
                }
                if let stripEmbeddedFonts {
                    item.editPlan.fonts.stripEmbeddedFonts = stripEmbeddedFonts
                }
                if let style { item.editPlan.styles.append(style) }
                item.analysis = nil
                item.analysisError = nil
            }
        }
    }

    func convertSelected() {
        let selection = selectedItems.isEmpty ? selectedItem.map { [$0] } ?? [] : selectedItems
        guard !selection.isEmpty else { return }
        startBatch(ids: selection.map(\.id), includeCompleted: true)
    }

    func retrySelected() {
        convertSelected()
    }

    func convertAll() {
        startBatch(ids: items.map(\.id))
    }

    private func startBatch(ids: [BookItem.ID], includeCompleted: Bool = false) {
        guard !isPreflighting, !isBatchConverting, !isScanningFolder, !hasActiveConversions else { return }
        let selection = items.filter { ids.contains($0.id) }
        guard !selection.isEmpty else { return }
        guard selection.allSatisfy(\.hasPreflightResult) else {
            lastError = "Run Check for every selected book before starting a batch conversion."
            return
        }

        let candidates = selection.filter {
            ($0.status == .ready || $0.status == .failed || $0.status == .cancelled || (includeCompleted && $0.status == .completed))
                && $0.analysis?.plan.blocked == false
        }
        let blocked = selection.filter { $0.analysis?.plan.blocked != false }
        if batchMode == .strict && !blocked.isEmpty {
            markPreflightBlocked(blocked)
            lastError = "Strict batch stopped during Preflight. Resolve every blocked item before converting."
            return
        }
        if candidates.isEmpty {
            lastError = "There are no preflight-approved books ready to convert."
            return
        }

        plannedBatchIDs = candidates.map(\.id)
        activeBatchMode = batchMode
        completedBatchIDs = []
        currentBatchItemIndex = 1
        isBatchConverting = true
        isBatchCancelling = false

        for item in candidates {
            update(item.id) { current in
                current.status = .ready
                current.outputURL = nil
                current.progress = nil
                current.stage = nil
                current.errorMessage = nil
                current.warnings = []
                current.report = nil
            }
        }
        if let firstID = plannedBatchIDs.first {
            update(firstID) { current in
                current.status = .converting
                current.progress = 0
                current.stage = .opening
            }
        }

        let outputRoots = candidates.map { batchOutputRoot(for: $0) }
        let inputs = candidates.map { item in
            FolioBatchInput(
                source: item.inputURL.path,
                relativePath: item.relativePath,
                outputRoot: batchOutputRoot(for: item).path,
                edit: item.editPlan
            )
        }
        let request = FolioBatchConversionRequest(
            inputs: inputs,
            outputDirectory: outputRoots.first?.path ?? candidates[0].inputURL.deletingLastPathComponent().path,
            options: FolioBatchOptions(
                target: target,
                conversion: .init(
                    deterministic: deterministic,
                    compression: compression,
                    degradationMode: degradationMode,
                    degradation: degradationOptions,
                    text: textImportOptions
                ),
                batchMode: batchMode,
                collisionPolicy: "Rename",
                preserveTree: true,
                edit: FolioBookEditPlan()
            )
        )
        let itemIDs = plannedBatchIDs
        let idsByPath = Dictionary(uniqueKeysWithValues: candidates.map {
            ($0.inputURL.standardizedFileURL.path, $0.id)
        })
        let individuallyAccessedURLs = candidates
            .filter { $0.sourceRoot == nil }
            .map(\.inputURL)
            .filter { $0.startAccessingSecurityScopedResource() }
        let bridge = self.bridge
        activeBatchHandle = bridge.startBatch(
            request: request,
            progress: { [weak self] event in
                Task { @MainActor [weak self] in
                    self?.receiveBatchProgress(event, itemIDs: itemIDs)
                }
            },
            completion: { [weak self] result in
                for url in individuallyAccessedURLs {
                    url.stopAccessingSecurityScopedResource()
                }
                Task { @MainActor [weak self] in
                    self?.finishBatch(result, itemIDs: itemIDs, idsByPath: idsByPath)
                }
            }
        )
    }

    func cancel(_ id: BookItem.ID) {
        if isBatchConverting {
            cancelConversions()
        } else {
            update(id) { item in
                item.status = .cancelled
                item.stage = nil
            }
        }
    }

    func cancelConversions() {
        guard isBatchConverting, !isBatchCancelling else { return }
        isBatchCancelling = true
        activeBatchHandle?.cancel()
    }

    func revealOutput(_ item: BookItem) {
        guard let outputURL = item.outputURL else { return }
        NSWorkspace.shared.activateFileViewerSelecting([outputURL])
    }

    func remove(_ id: BookItem.ID) {
        if isBatchConverting { cancelConversions() }
        let removed = items.first { $0.id == id }
        plannedBatchIDs.removeAll { $0 == id }
        items.removeAll { $0.id == id }
        selectedIDs.remove(id)
        if selectedID == id { selectedID = items.first?.id }
        if !isBatchConverting, let sourceRoot = removed?.sourceRoot,
           !items.contains(where: { $0.sourceRoot?.standardizedFileURL == sourceRoot.standardizedFileURL }) {
            releaseSourceFolder(sourceRoot)
        }
    }

    func removeSelected() {
        let ids = selectedItems.map(\.id)
        for id in ids { remove(id) }
    }

    func removeAll() {
        let batchWasActive = isBatchConverting
        if batchWasActive {
            cancelConversions()
        } else {
            activeBatchHandle = nil
            isBatchCancelling = false
        }
        plannedBatchIDs = []
        completedBatchIDs = []
        currentBatchItemIndex = 0
        if !batchWasActive { isBatchConverting = false }
        items.removeAll()
        selectedIDs = []
        selectedID = nil
        if !batchWasActive { releaseUnusedSourceFolders() }
    }

    private func addFile(_ url: URL, relativePath: String? = nil, sourceRoot: URL? = nil) {
        guard supportedExtensions.contains(url.pathExtension.lowercased()) else { return }
        let standardized = url.standardizedFileURL
        guard !items.contains(where: { $0.inputURL.standardizedFileURL == standardized }) else { return }
        items.append(BookItem(inputURL: url, relativePath: relativePath, sourceRoot: sourceRoot))
    }

    private func addFolder(_ folder: URL) {
        guard !isScanningFolder else {
            lastError = "Wait for the current folder scan to finish before adding another folder."
            return
        }
        let root = folder.standardizedFileURL
        let alreadyAccessed = sourceFolderAccessURLs.contains {
            $0.standardizedFileURL == root
        }
        let startedAccess = !alreadyAccessed && root.startAccessingSecurityScopedResource()
        if startedAccess { sourceFolderAccessURLs.append(root) }
        isScanningFolder = true
        let bridge = self.bridge
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let result = Result { try bridge.listDirectory(url: root) }
            Task { @MainActor [weak self] in
                guard let self else {
                    if startedAccess { root.stopAccessingSecurityScopedResource() }
                    return
                }
                self.isScanningFolder = false
                switch result {
                case .success(let books):
                    for book in books {
                        self.addFile(
                            URL(fileURLWithPath: book.source),
                            relativePath: book.relativePath,
                            sourceRoot: root
                        )
                    }
                    if self.selectedID == nil { self.selectedID = self.items.first?.id }
                case .failure(let error):
                    self.lastError = "Could not scan folder: \(error.localizedDescription)"
                    if startedAccess {
                        self.releaseSourceFolder(root)
                    }
                }
            }
        }
    }

    private func preflight(ids: [BookItem.ID]) {
        guard !isPreflighting else { return }
        guard !isScanningFolder else {
            lastError = "Wait for the current folder scan to finish before checking books."
            return
        }
        let jobs = ids.compactMap { id -> (BookItem.ID, URL, FolioBookEditPlan)? in
            guard let item = items.first(where: { $0.id == id }) else { return nil }
            return (item.id, item.inputURL, item.editPlan)
        }
        guard !jobs.isEmpty else {
            lastError = "Add at least one supported book file first."
            return
        }

        isPreflighting = true
        for id in ids {
            update(id) { item in
                item.analysis = nil
                item.analysisError = nil
            }
        }
        let target = self.target
        let mode = self.degradationMode
        let options = self.degradationOptions
        let textOptions = self.textImportOptions
        let bridge = self.bridge
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            var results: [(BookItem.ID, URL, Result<FolioAnalysisReport, PreflightFailure>)] = []
            for (id, url, editPlan) in jobs {
                let accessed = url.startAccessingSecurityScopedResource()
                do {
                    let report = try bridge.analyze(
                        url: url,
                        target: target,
                        mode: mode,
                        degradation: options,
                        edit: editPlan,
                        text: textOptions
                    )
                    results.append((id, url, .success(report)))
                } catch {
                    results.append((id, url, .failure(PreflightFailure(message: error.localizedDescription))))
                }
                if accessed { url.stopAccessingSecurityScopedResource() }
            }
            let completedResults = results
            Task { @MainActor [weak self] in
                guard let self else { return }
                for (id, url, result) in completedResults {
                    self.update(id) { item in
                        switch result {
                        case .success(let report):
                            item.analysis = report
                            item.analysisError = nil
                        case .failure(let failure):
                            item.analysis = nil
                            item.analysisError = self.userFacingAnalysisError(failure.message, url: url)
                        }
                    }
                }
                self.isPreflighting = false
            }
        }
    }

    private func receiveBatchProgress(_ progress: FolioBatchProgressEvent, itemIDs: [BookItem.ID]) {
        guard progress.currentItem > 0,
              progress.currentItem <= itemIDs.count
        else { return }
        currentBatchItemIndex = progress.currentItem
        let id = itemIDs[progress.currentItem - 1]
        update(id) { item in
            item.status = .converting
            item.stage = progress.event.stage
            item.progress = progress.event.fraction.map(Double.init)
        }
        if progress.event.stage == .finished {
            if !completedBatchIDs.contains(id) { completedBatchIDs.append(id) }
            update(id) { item in
                item.status = .completed
                item.progress = 1
                item.stage = .finished
            }
        }
    }

    private func finishBatch(
        _ result: Result<FolioBatchReport, FolioError>,
        itemIDs: [BookItem.ID],
        idsByPath: [String: BookItem.ID]
    ) {
        switch result {
        case .success(let report) where report.aborted && activeBatchMode == .strict:
            let message = report.items.compactMap(\.error).first
                ?? "Strict batch aborted; successful outputs were rolled back."
            for id in itemIDs {
                update(id) { item in
                    item.status = .failed
                    item.outputURL = nil
                    item.progress = nil
                    item.report = nil
                    item.errorMessage = message
                    item.stage = nil
                }
            }
            lastError = message
        case .success(let report):
            var reportedIDs = Set<BookItem.ID>()
            for itemReport in report.items {
                let path = URL(fileURLWithPath: itemReport.source).standardizedFileURL.path
                guard let id = idsByPath[path] else { continue }
                reportedIDs.insert(id)
                let cancelled = itemReport.error?.localizedCaseInsensitiveContains("cancel") == true
                update(id) { item in
                    item.status = itemReport.success ? .completed : (cancelled ? .cancelled : .failed)
                    item.progress = itemReport.success ? 1 : nil
                    item.stage = itemReport.success ? .finished : nil
                    item.outputURL = itemReport.output.map { URL(fileURLWithPath: $0) }
                    item.report = itemReport.report
                    item.warnings = itemReport.report?.warnings ?? []
                    item.errorMessage = itemReport.success ? nil : itemReport.error
                }
            }
            for id in itemIDs where !reportedIDs.contains(id) {
                update(id) { item in
                    item.status = .failed
                    item.errorMessage = "Batch engine did not return a result for this book."
                    item.stage = nil
                }
            }
            if report.failed > 0 {
                lastError = "\(report.failed) of \(itemIDs.count) books failed. See the queue for details."
            } else {
                lastError = nil
            }
            NotificationService.send(
                title: report.failed == 0 ? "Batch complete" : "Batch finished with errors",
                body: "\(report.succeeded) converted · \(report.failed) failed",
                identifier: UUID().uuidString
            )
        case .failure(let error):
            let cancelled = error.localizedDescription.localizedCaseInsensitiveContains("cancel")
            for id in itemIDs {
                update(id) { item in
                    item.status = cancelled ? .cancelled : .failed
                    item.errorMessage = error.localizedDescription
                    item.stage = nil
                }
            }
            lastError = cancelled ? nil : error.localizedDescription
        }

        activeBatchHandle = nil
        isBatchConverting = false
        isBatchCancelling = false
        plannedBatchIDs = []
        completedBatchIDs = []
        currentBatchItemIndex = 0
        releaseUnusedSourceFolders()
    }

    private func markPreflightBlocked(_ blocked: [BookItem]) {
        for item in blocked {
            let message = item.analysisError
                ?? "Preflight blocked this item because the selected compatibility mode does not allow its degradation plan."
            update(item.id) { current in
                current.status = .failed
                current.errorMessage = message
                current.stage = nil
            }
        }
    }

    private func batchOutputRoot(for item: BookItem) -> URL {
        if let outputFolder {
            return outputFolder
        }
        if let sourceRoot = item.sourceRoot {
            return sourceRoot.appendingPathComponent("FolioForge-output", isDirectory: true)
        }
        return item.inputURL.deletingLastPathComponent()
    }

    private func releaseUnusedSourceFolders() {
        let unused = sourceFolderAccessURLs.filter { root in
            !items.contains { $0.sourceRoot?.standardizedFileURL == root.standardizedFileURL }
        }
        for root in unused { releaseSourceFolder(root) }
    }

    private func releaseSourceFolder(_ root: URL) {
        guard let index = sourceFolderAccessURLs.firstIndex(where: {
            $0.standardizedFileURL == root.standardizedFileURL
        }) else { return }
        let scopedURL = sourceFolderAccessURLs.remove(at: index)
        scopedURL.stopAccessingSecurityScopedResource()
    }

    private func userFacingAnalysisError(_ message: String, url: URL) -> String {
        if url.pathExtension.caseInsensitiveCompare("kfx") == .orderedSame
            && message.localizedCaseInsensitiveContains("compatibility container") {
            return "This Amazon KFX input could not be imported into Folio Semantic IR. DRM-protected content is never decrypted; unsupported KFX structures are reported as input limitations."
        }
        return message
    }

    private func update(_ id: BookItem.ID, _ mutate: (inout BookItem) -> Void) {
        guard let index = items.firstIndex(where: { $0.id == id }) else { return }
        mutate(&items[index])
    }
}
