import AppKit
import Foundation
import SwiftUI
import UniformTypeIdentifiers

private final class ComicSourceAccessLease: @unchecked Sendable {
    private let lock = NSLock()
    private let url: URL
    private let startedAccess: Bool
    private var references = 1
    private var stopped = false

    init(url: URL) {
        self.url = url
        self.startedAccess = url.startAccessingSecurityScopedResource()
    }

    func retain() {
        lock.lock()
        defer { lock.unlock() }
        guard !stopped else { return }
        references += 1
    }

    func release() {
        lock.lock()
        guard !stopped else {
            lock.unlock()
            return
        }
        references = max(0, references - 1)
        let shouldStop = references == 0 && startedAccess
        if references == 0 { stopped = true }
        lock.unlock()
        if shouldStop { url.stopAccessingSecurityScopedResource() }
    }

    deinit {
        lock.lock()
        let shouldStop = !stopped && startedAccess
        stopped = true
        lock.unlock()
        if shouldStop { url.stopAccessingSecurityScopedResource() }
    }
}

@MainActor
final class ComicWorkspaceViewModel: ObservableObject {
    @Published private(set) var summary: FolioComicSummary?
    @Published private(set) var pages: [FolioComicPage] = []
    @Published private(set) var options: FolioComicConversionOptions?
    @Published var selectedPageID: String?
    @Published private(set) var selectedPageInfo: FolioComicPage?
    @Published var selectedTargetID = ""
    @Published private(set) var readerPage: FolioReaderPageModel?
    @Published private(set) var readerSpread: FolioReaderSpreadModel?
    @Published private(set) var readerSessionSummary: FolioReaderSessionSummary?
    @Published private(set) var legacyPreviewData: Data?
    @Published private(set) var readerFallbackReason: String?
    @Published private(set) var readerViewport = FolioReaderViewport(
        width: 800,
        height: 600,
        scale: 1,
        contentMode: .fit
    )
    @Published private(set) var previewLoading = false
    @Published private(set) var isOpening = false
    @Published private(set) var isConverting = false
    @Published private(set) var progress: FolioComicProgress?
    @Published private(set) var report: FolioComicConversionReport?
    @Published private(set) var errorMessage: String?
    @Published private(set) var sourceURL: URL?
    @Published var isDropTargeted = false
    @Published private(set) var thumbnailRevision = 0

    private let bridge = FolioComicCoreBridge()
    private let thumbnailCache = NSCache<NSString, NSData>()
    private let readerImageCache = FolioReaderImageCache()
    private var sessionID: String?
    private var readerSessionID: String?
    private var sourceLease: ComicSourceAccessLease?
    private var pendingOpenLease: ComicSourceAccessLease?
    private var openRequestID = UUID()
    private var openCancellation: FolioCancellationHandle?
    private var operationCancellations: [UUID: FolioCancellationHandle] = [:]
    private var thumbnailTasks: [String: Task<Data?, Never>] = [:]
    private var selectedPreviewCancellation: FolioCancellationHandle?
    private var conversionCancellation: FolioCancellationHandle?
    private var viewportUpdateTask: Task<Void, Never>?
    private var viewportUpdateID = UUID()

    init() {
        thumbnailCache.countLimit = 128
        thumbnailCache.totalCostLimit = 32 * 1024 * 1024
    }

    func readerImage(for placement: FolioReaderPlacement) -> NSImage? {
        guard let readerSessionID else { return nil }
        return readerImageCache.image(
            data: placement.resource.data,
            sessionID: readerSessionID,
            resourceID: placement.resource.resourceId
        )
    }

    var selectedPage: FolioComicPage? {
        guard let selectedPageID else { return nil }
        return pages.first { $0.pageId == selectedPageID }
    }

    func cachedThumbnail(for pageID: String) -> Data? {
        guard let sessionID else { return nil }
        return thumbnailCache.object(forKey: (sessionID + ":" + pageID) as NSString) as Data?
    }

    var canConvert: Bool {
        sessionID != nil
            && options?.targets.contains(where: { $0.id == selectedTargetID }) == true
            && !isConverting
    }

    var canControlReader: Bool { readerSessionID != nil && readerFallbackReason == nil }
    var canNavigateReader: Bool { readerSessionID != nil }

    func openPanel() {
        let panel = NSOpenPanel()
        panel.title = FolioL10n.string("ui.open_comic_source", default: "Open a comic source")
        panel.prompt = FolioL10n.string("ui.open", default: "Open")
        panel.canChooseFiles = true
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [
            .zip,
            UTType(filenameExtension: "cbz") ?? .zip,
        ]
        panel.begin { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            Task { @MainActor [weak self] in self?.open(url) }
        }
    }

    func acceptDroppedURLData(_ data: Data?) {
        guard let data, let url = URL(dataRepresentation: data, relativeTo: nil) else { return }
        open(url)
    }

    func open(_ url: URL) {
        closeCurrentSession()
        openCancellation?.cancel()
        pendingOpenLease?.release()

        let requestID = UUID()
        openRequestID = requestID
        let lease = ComicSourceAccessLease(url: url)
        pendingOpenLease = lease
        lease.retain()
        let cancellation = FolioCancellationHandle.make()
        openCancellation = cancellation
        isOpening = true
        errorMessage = nil
        report = nil
        sourceURL = url
        readerFallbackReason = nil

        let bridge = self.bridge
        Task.detached(priority: .userInitiated) { [weak self] in
            defer {
                lease.release()
                cancellation.finish()
            }
            let result: Result<(
                FolioComicSummary,
                [FolioComicPage],
                FolioComicConversionOptions,
                FolioReaderSessionSummary?,
                String?
            ), Error>
            do {
                let summary = try bridge.open(path: url, cancellation: cancellation)
                let pages = try bridge.pages(sessionID: summary.sessionId)
                let options = try bridge.conversionOptions()
                let readerResult: Result<FolioReaderSessionSummary, Error>
                do {
                    readerResult = .success(try bridge.openReader(
                        comicSessionID: summary.sessionId,
                        viewport: FolioReaderViewport(
                            width: 800,
                            height: 600,
                            scale: 1,
                            contentMode: .fit
                        )
                    ))
                } catch {
                    readerResult = .failure(error)
                }
                switch readerResult {
                case .success(let reader):
                    result = .success((summary, pages, options, reader, nil))
                case .failure(let error):
                    result = .success((
                        summary,
                        pages,
                        options,
                        nil,
                        FolioL10n.format(
                            "status.reader_fallback",
                            default: "Reader session unavailable; showing the existing Core preview. %@",
                            error.localizedDescription
                        )
                    ))
                }
            } catch {
                result = .failure(error)
            }
            await MainActor.run {
                guard let self else {
                    if case .success(let payload) = result {
                        if let reader = payload.3 { try? bridge.closeReader(sessionID: reader.sessionId) }
                        try? bridge.close(sessionID: payload.0.sessionId)
                    }
                    lease.release()
                    return
                }
                guard self.openRequestID == requestID else {
                    if case .success(let payload) = result {
                        if let reader = payload.3 { try? bridge.closeReader(sessionID: reader.sessionId) }
                        try? bridge.close(sessionID: payload.0.sessionId)
                    }
                    lease.release()
                    return
                }
                self.openCancellation = nil
                self.pendingOpenLease = nil
                self.isOpening = false
                switch result {
                case .success(let payload):
                    self.sessionID = payload.0.sessionId
                    self.summary = payload.0
                    self.pages = payload.1
                    self.options = payload.2
                    self.selectedTargetID = payload.2.targets.first?.id ?? ""
                    self.readerSessionID = payload.3?.sessionId
                    self.readerSessionSummary = payload.3
                    if let reader = payload.3 {
                        self.readerViewport = reader.viewport
                        self.selectedPageID = payload.1.first {
                            $0.displayIndex == reader.currentDocumentIndex + 1
                        }?.pageId ?? payload.1.first?.pageId
                    } else {
                        self.selectedPageID = payload.1.first?.pageId
                    }
                    self.readerFallbackReason = payload.4
                    self.sourceLease = lease
                    if payload.1.isEmpty {
                    self.errorMessage = FolioL10n.string("status.no_supported_pages", default: "Core imported the source but found no supported pages.")
                    }
                case .failure(let error):
                    lease.release()
                    self.sourceURL = nil
                    self.errorMessage = error.localizedDescription
                }
            }
        }
    }

    func loadThumbnail(for page: FolioComicPage) async -> Data? {
        guard let sessionID, let sourceLease else { return nil }
        let key = sessionID + ":" + page.pageId
        if let cached = thumbnailCache.object(forKey: key as NSString) { return cached as Data }
        if let task = thumbnailTasks[key] { return await task.value }

        let cancellation = FolioCancellationHandle.make()
        let operationID = UUID()
        operationCancellations[operationID] = cancellation
        sourceLease.retain()
        let bridge = self.bridge
        let task = Task.detached(priority: .utility) {
            defer {
                sourceLease.release()
                cancellation.finish()
            }
            return try? bridge.thumbnail(
                sessionID: sessionID,
                pageID: page.pageId,
                cancellation: cancellation
            ).data
        }
        thumbnailTasks[key] = task
        let data = await task.value
        thumbnailTasks[key] = nil
        operationCancellations[operationID] = nil
        if let data, self.sessionID == sessionID {
            thumbnailCache.setObject(data as NSData, forKey: key as NSString, cost: data.count)
            thumbnailRevision &+= 1
            return data
        }
        return nil
    }

    func loadSelectedPage() async {
        guard let sessionID, let page = selectedPage, let sourceLease else {
            selectedPageInfo = nil
            readerPage = nil
            readerSpread = nil
            legacyPreviewData = nil
            return
        }
        let readerDocumentIndex = page.displayIndex - 1
        if readerPage?.documentIndex == readerDocumentIndex,
           readerSessionSummary?.currentDocumentIndex == readerDocumentIndex {
            selectedPageInfo = page
            return
        }
        if legacyPreviewData != nil,
           readerFallbackReason != nil,
           readerSessionSummary?.currentDocumentIndex == readerDocumentIndex {
            selectedPageInfo = page
            return
        }
        selectedPreviewCancellation?.cancel()
        let cancellation = FolioCancellationHandle.make()
        selectedPreviewCancellation = cancellation
        let operationID = UUID()
        operationCancellations[operationID] = cancellation
        sourceLease.retain()
        let bridge = self.bridge
        let readerSessionID = self.readerSessionID
        let existingFallbackReason = readerFallbackReason
        previewLoading = true
        readerPage = nil
        readerSpread = nil
        legacyPreviewData = nil

        let request = Task.detached(priority: .userInitiated) { () throws -> (
            FolioComicPage,
            FolioReaderPageModel?,
            FolioReaderSpreadModel?,
            Data?,
            FolioReaderSessionSummary?,
            String?
        ) in
            defer {
                sourceLease.release()
                cancellation.finish()
            }
            let info = try bridge.pageInfo(sessionID: sessionID, pageID: page.pageId)
            if let readerSessionID {
                do {
                    try bridge.readerGoToDocumentIndex(
                        sessionID: readerSessionID,
                        documentIndex: readerDocumentIndex
                    )
                    let state = try bridge.readerSummary(sessionID: readerSessionID)
                    if state.spreadMode == .synthetic {
                        let spread = try bridge.currentReaderSpread(sessionID: readerSessionID)
                        return (info, nil, spread, nil, state, nil)
                    }
                    let model = try bridge.currentReaderPage(
                        sessionID: readerSessionID,
                        cancellation: cancellation
                    )
                    return (info, model, nil, nil, state, nil)
                } catch let error as FolioReaderBridgeError
                    where ["unsupported_layout", "resource_too_large"].contains(error.code) {
                    let preview = try bridge.preview(
                        sessionID: sessionID,
                        pageID: page.pageId,
                        cancellation: cancellation
                    )
                    guard let previewData = preview.data else { throw FolioError.invalidResponse }
                    let state = try? bridge.readerSummary(sessionID: readerSessionID)
                    return (
                        info,
                        Optional<FolioReaderPageModel>.none,
                        Optional<FolioReaderSpreadModel>.none,
                        Optional(previewData),
                        state,
                        Optional("Reader reports \(error.code); showing the existing Core preview.")
                    )
                }
            }

            let preview = try bridge.preview(
                sessionID: sessionID,
                pageID: page.pageId,
                cancellation: cancellation
            )
            guard let previewData = preview.data else { throw FolioError.invalidResponse }
            return (
                info,
                Optional<FolioReaderPageModel>.none,
                Optional<FolioReaderSpreadModel>.none,
                Optional(previewData),
                Optional<FolioReaderSessionSummary>.none,
                existingFallbackReason
                    ?? "Reader is unavailable; showing the existing Core preview."
            )
        }

        do {
            let (info, pageModel, spreadModel, fallbackData, readerState, fallbackReason) = try await withTaskCancellationHandler {
                try await request.value
            } onCancel: {
                cancellation.cancel()
            }
            if self.sessionID == sessionID, selectedPageID == page.pageId {
                selectedPageInfo = info
                readerPage = pageModel
                readerSpread = spreadModel
                legacyPreviewData = fallbackData
                if let readerState {
                    readerSessionSummary = readerState
                    readerViewport = readerState.viewport
                }
                readerFallbackReason = fallbackReason
                errorMessage = nil
            }
        } catch {
            if self.sessionID == sessionID, selectedPageID == page.pageId {
                selectedPageInfo = nil
                readerPage = nil
                readerSpread = nil
                legacyPreviewData = nil
                let wasCancelled = isCancellationError(error)
                if !(error is CancellationError), !wasCancelled {
                    errorMessage = error.localizedDescription
                }
            }
        }
        operationCancellations[operationID] = nil
        if selectedPreviewCancellation === cancellation {
            selectedPreviewCancellation = nil
            previewLoading = false
        }
    }

    func navigateReader(forward: Bool) {
        guard let sessionID = readerSessionID, let sourceLease else { return }
        selectedPreviewCancellation?.cancel()
        let cancellation = FolioCancellationHandle.make()
        selectedPreviewCancellation = cancellation
        let operationID = UUID()
        operationCancellations[operationID] = cancellation
        sourceLease.retain()
        previewLoading = true
        let bridge = self.bridge
        let pages = self.pages
        Task.detached(priority: .userInitiated) { [weak self] in
            defer {
                sourceLease.release()
                cancellation.finish()
            }
            let result: Result<(
                FolioReaderSessionSummary,
                FolioReaderPageModel?,
                FolioReaderSpreadModel?,
                Data?,
                String?,
                FolioComicPage
            ), Error>
            do {
                if forward {
                    _ = try bridge.readerNext(sessionID: sessionID)
                } else {
                    _ = try bridge.readerPrevious(sessionID: sessionID)
                }
                let state = try bridge.readerSummary(sessionID: sessionID)
                guard let comicPage = pages.first(where: {
                    $0.displayIndex == state.currentDocumentIndex + 1
                }) else {
                    throw FolioError.invalidResponse
                }
                do {
                    if state.spreadMode == .synthetic {
                        let spread = try bridge.currentReaderSpread(sessionID: sessionID)
                        result = .success((state, nil, spread, nil, nil, comicPage))
                    } else {
                        let page = try bridge.currentReaderPage(
                            sessionID: sessionID,
                            cancellation: cancellation
                        )
                        result = .success((state, page, nil, nil, nil, comicPage))
                    }
                } catch let error as FolioReaderBridgeError
                    where ["unsupported_layout", "resource_too_large"].contains(error.code) {
                    let preview = try bridge.preview(
                        sessionID: sessionID,
                        pageID: comicPage.pageId,
                        cancellation: cancellation
                    )
                    guard let previewData = preview.data else { throw FolioError.invalidResponse }
                    result = .success((
                        state,
                        nil,
                        nil,
                        previewData,
                        "Reader reports \(error.code); showing the existing Core preview.",
                        comicPage
                    ))
                }
            } catch {
                result = .failure(error)
            }
            await MainActor.run {
                guard let self, self.readerSessionID == sessionID else { return }
                self.operationCancellations[operationID] = nil
                self.previewLoading = false
                self.selectedPreviewCancellation = nil
                switch result {
                case .success(let payload):
                    self.readerSessionSummary = payload.0
                    self.readerViewport = payload.0.viewport
                    self.readerPage = payload.1
                    self.readerSpread = payload.2
                    self.legacyPreviewData = payload.3
                    self.readerFallbackReason = payload.4
                    self.selectedPageID = payload.5.pageId
                    self.selectedPageInfo = payload.5
                case .failure(let error):
                    if !self.isCancellationError(error) {
                        self.errorMessage = error.localizedDescription
                    }
                }
            }
        }
    }

    func setReaderSpreadMode(_ spreadMode: FolioReaderSpreadMode) {
        guard let sessionID = readerSessionID,
              let sourceLease,
              readerSessionSummary?.spreadMode != spreadMode
        else { return }
        selectedPreviewCancellation?.cancel()
        let cancellation = FolioCancellationHandle.make()
        selectedPreviewCancellation = cancellation
        let operationID = UUID()
        operationCancellations[operationID] = cancellation
        sourceLease.retain()
        previewLoading = true
        let bridge = self.bridge
        let pages = self.pages
        Task.detached(priority: .userInitiated) { [weak self] in
            defer {
                sourceLease.release()
                cancellation.finish()
            }
            let result: Result<(
                FolioReaderSessionSummary,
                FolioReaderPageModel?,
                FolioReaderSpreadModel?,
                Data?,
                String?,
                FolioComicPage
            ), Error>
            do {
                let state = try bridge.readerSetSpreadMode(
                    sessionID: sessionID,
                    spreadMode: spreadMode
                )
                guard let comicPage = pages.first(where: {
                    $0.displayIndex == state.currentDocumentIndex + 1
                }) else {
                    throw FolioError.invalidResponse
                }
                do {
                    if spreadMode == .synthetic {
                        let spread = try bridge.currentReaderSpread(sessionID: sessionID)
                        result = .success((state, nil, spread, nil, nil, comicPage))
                    } else {
                        let page = try bridge.currentReaderPage(
                            sessionID: sessionID,
                            cancellation: cancellation
                        )
                        result = .success((state, page, nil, nil, nil, comicPage))
                    }
                } catch let error as FolioReaderBridgeError
                    where ["unsupported_layout", "resource_too_large"].contains(error.code) {
                    let preview = try bridge.preview(
                        sessionID: sessionID,
                        pageID: comicPage.pageId,
                        cancellation: cancellation
                    )
                    guard let data = preview.data else { throw FolioError.invalidResponse }
                    result = .success((
                        state,
                        nil,
                        nil,
                        data,
                        "Reader reports \(error.code); showing the existing Core preview.",
                        comicPage
                    ))
                }
            } catch {
                result = .failure(error)
            }
            await MainActor.run {
                guard let self, self.readerSessionID == sessionID else { return }
                self.operationCancellations[operationID] = nil
                self.previewLoading = false
                self.selectedPreviewCancellation = nil
                switch result {
                case .success(let payload):
                    self.readerSessionSummary = payload.0
                    self.readerViewport = payload.0.viewport
                    self.readerPage = payload.1
                    self.readerSpread = payload.2
                    self.legacyPreviewData = payload.3
                    self.readerFallbackReason = payload.4
                    self.selectedPageID = payload.5.pageId
                    self.selectedPageInfo = payload.5
                case .failure(let error):
                    if !self.isCancellationError(error) {
                        self.errorMessage = error.localizedDescription
                    }
                }
            }
        }
    }

    func updateReaderViewport(width: CGFloat, height: CGFloat) {
        guard width.isFinite, height.isFinite, width > 0, height > 0,
              readerSessionID != nil
        else { return }
        let width = Float(width)
        let height = Float(height)
        guard abs(readerViewport.width - width) > 1 || abs(readerViewport.height - height) > 1 else {
            return
        }
        var viewport = readerViewport
        viewport.width = width
        viewport.height = height
        readerViewport = viewport
        scheduleViewportUpdate()
    }

    func setReaderContentMode(_ mode: FolioReaderContentMode) {
        guard readerSessionID != nil, readerViewport.contentMode != mode else { return }
        var viewport = readerViewport
        viewport.contentMode = mode
        viewport.scale = 1
        viewport.panX = 0
        viewport.panY = 0
        readerViewport = viewport
        scheduleViewportUpdate()
    }

    func adjustReaderZoom(by delta: Float) {
        guard readerSessionID != nil else { return }
        var viewport = readerViewport
        viewport.scale = min(8, max(0.25, viewport.scale + delta))
        readerViewport = viewport
        scheduleViewportUpdate()
    }

    func panReader(byX x: CGFloat, y: CGFloat) {
        guard readerSessionID != nil, x.isFinite, y.isFinite else { return }
        var viewport = readerViewport
        viewport.panX += Float(x)
        viewport.panY += Float(y)
        readerViewport = viewport
        scheduleViewportUpdate()
    }

    func setReaderDirection(_ direction: FolioReaderDirection) {
        guard let sessionID = readerSessionID else { return }
        selectedPreviewCancellation?.cancel()
        let cancellation = FolioCancellationHandle.make()
        selectedPreviewCancellation = cancellation
        let operationID = UUID()
        operationCancellations[operationID] = cancellation
        sourceLease?.retain()
        previewLoading = true
        let bridge = self.bridge
        let sourceLease = self.sourceLease
        Task.detached(priority: .userInitiated) { [weak self] in
            defer {
                sourceLease?.release()
                cancellation.finish()
            }
            do {
                let state = try bridge.readerSetDirection(sessionID: sessionID, direction: direction)
                let page = state.spreadMode == .synthetic
                    ? nil
                    : try bridge.currentReaderPage(sessionID: sessionID, cancellation: cancellation)
                let spread = state.spreadMode == .synthetic
                    ? try bridge.currentReaderSpread(sessionID: sessionID)
                    : nil
                await MainActor.run {
                    guard let self, self.readerSessionID == sessionID else { return }
                    self.operationCancellations[operationID] = nil
                    self.readerSessionSummary = state
                    self.readerViewport = state.viewport
                    self.readerPage = page
                    self.readerSpread = spread
                    self.legacyPreviewData = nil
                    self.readerFallbackReason = nil
                    self.selectedPageID = self.pages.first {
                        $0.displayIndex == state.currentDocumentIndex + 1
                    }?.pageId
                    self.selectedPageInfo = self.selectedPage
                    self.previewLoading = false
                    self.selectedPreviewCancellation = nil
                }
            } catch {
                await MainActor.run {
                    guard let self, self.readerSessionID == sessionID else { return }
                    self.operationCancellations[operationID] = nil
                    self.previewLoading = false
                    self.selectedPreviewCancellation = nil
                    if !self.isCancellationError(error) {
                        self.errorMessage = error.localizedDescription
                    }
                }
            }
        }
    }

    private func scheduleViewportUpdate() {
        guard let sessionID = readerSessionID, let sourceLease else { return }
        viewportUpdateTask?.cancel()
        let updateID = UUID()
        viewportUpdateID = updateID
        let viewport = readerViewport
        let bridge = self.bridge
        viewportUpdateTask = Task.detached(priority: .userInitiated) { [weak self] in
            do {
                try await Task.sleep(nanoseconds: 80_000_000)
            } catch {
                return
            }
            guard !Task.isCancelled else { return }
            sourceLease.retain()
            defer { sourceLease.release() }
            do {
                let state = try bridge.readerSetViewport(sessionID: sessionID, viewport: viewport)
                let page = state.spreadMode == .synthetic
                    ? nil
                    : try bridge.currentReaderPage(sessionID: sessionID, cancellation: nil)
                let spread = state.spreadMode == .synthetic
                    ? try bridge.currentReaderSpread(sessionID: sessionID)
                    : nil
                await MainActor.run {
                    guard let self,
                          self.readerSessionID == sessionID,
                          self.viewportUpdateID == updateID
                    else { return }
                    self.readerSessionSummary = state
                    self.readerViewport = state.viewport
                    self.readerPage = page
                    self.readerSpread = spread
                    self.viewportUpdateTask = nil
                }
            } catch {
                await MainActor.run {
                    guard let self,
                          self.readerSessionID == sessionID,
                          self.viewportUpdateID == updateID
                    else { return }
                    self.viewportUpdateTask = nil
                    if !self.isCancellationError(error) {
                        self.errorMessage = error.localizedDescription
                    }
                }
            }
        }
    }

    func chooseOutputAndConvert() {
        guard canConvert,
              let target = options?.targets.first(where: { $0.id == selectedTargetID })
        else { return }
        let panel = NSSavePanel()
        panel.title = FolioL10n.string("ui.convert_comic_dialog", default: "Convert Comic")
        panel.prompt = FolioL10n.string("ui.convert", default: "Convert")
        panel.nameFieldStringValue = "\((summary?.title.isEmpty == false ? summary?.title : nil) ?? FolioL10n.string("comic.untitled", default: "Untitled comic")).\(target.fileExtension)"
        panel.allowedContentTypes = [UTType(filenameExtension: target.fileExtension) ?? .zip]
        panel.canCreateDirectories = true
        panel.begin { [weak self] response in
            guard response == .OK, let output = panel.url else { return }
            Task { @MainActor [weak self] in self?.convert(to: output, target: target) }
        }
    }

    func cancelConversion() {
        conversionCancellation?.cancel()
    }

    func revealOutput() {
        guard let report else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: report.outputPath)])
    }

    func clearError() {
        errorMessage = nil
    }

    private func convert(to output: URL, target: FolioComicTarget) {
        guard let sessionID, let sourceLease else { return }
        conversionCancellation?.cancel()
        let cancellation = FolioCancellationHandle.make()
        let operationID = UUID()
        operationCancellations[operationID] = cancellation
        conversionCancellation = cancellation
        sourceLease.retain()
        let outputLease = ComicSourceAccessLease(url: output)
        outputLease.retain()
        isConverting = true
        progress = nil
        report = nil
        errorMessage = nil
        let bridge = self.bridge

        Task.detached(priority: .userInitiated) { [weak self] in
            defer {
                sourceLease.release()
                outputLease.release()
                outputLease.release()
                cancellation.finish()
            }
            let result: Result<FolioComicConversionReport, Error>
            do {
                result = .success(try bridge.convert(
                    sessionID: sessionID,
                    target: target.id,
                    output: output,
                    cancellation: cancellation,
                    progress: { [weak self] event in
                        Task { @MainActor [weak self] in
                            guard self?.sessionID == sessionID else { return }
                            self?.progress = event
                        }
                    }
                ))
            } catch {
                result = .failure(error)
            }
            await MainActor.run {
                guard let self else { return }
                self.operationCancellations[operationID] = nil
                if self.conversionCancellation === cancellation { self.conversionCancellation = nil }
                guard self.sessionID == sessionID else { return }
                self.isConverting = false
                switch result {
                case .success(let report):
                    self.report = report
                    self.progress = nil
                case .failure(let error):
                    self.progress = nil
                    if self.cancellationWasRequested(error: error) {
                        self.errorMessage = FolioL10n.string("status.conversion_cancelled_no_output", default: "Conversion cancelled. No partial output was committed.")
                    } else {
                        self.errorMessage = error.localizedDescription
                    }
                }
            }
        }
    }

    private func closeCurrentSession() {
        openRequestID = UUID()
        isOpening = false
        openCancellation?.cancel()
        openCancellation = nil
        pendingOpenLease?.release()
        pendingOpenLease = nil
        selectedPreviewCancellation?.cancel()
        viewportUpdateID = UUID()
        viewportUpdateTask?.cancel()
        viewportUpdateTask = nil
        conversionCancellation?.cancel()
        operationCancellations.values.forEach { $0.cancel() }
        operationCancellations.removeAll()
        thumbnailTasks.values.forEach { $0.cancel() }
        thumbnailTasks.removeAll()

        if let readerSessionID { try? bridge.closeReader(sessionID: readerSessionID) }
        readerSessionID = nil
        readerImageCache.removeAll()
        readerSessionSummary = nil
        readerPage = nil
        readerSpread = nil
        if let sessionID { try? bridge.close(sessionID: sessionID) }
        sessionID = nil
        sourceLease?.release()
        sourceLease = nil
        summary = nil
        pages = []
        options = nil
        selectedTargetID = ""
        selectedPageID = nil
        selectedPageInfo = nil
        legacyPreviewData = nil
        readerFallbackReason = nil
        readerViewport = FolioReaderViewport(width: 800, height: 600, scale: 1, contentMode: .fit)
        previewLoading = false
        isConverting = false
        progress = nil
        report = nil
        sourceURL = nil
        thumbnailCache.removeAllObjects()
    }

    private func cancellationWasRequested(error: Error) -> Bool {
        if case FolioError.core(let message) = error, message.localizedCaseInsensitiveContains("cancel") {
            return true
        }
        return false
    }

    private func isCancellationError(_ error: Error) -> Bool {
        if let readerError = error as? FolioReaderBridgeError {
            return readerError.code == "cancelled"
        }
        if let folioError = error as? FolioError,
           case .core(let message) = folioError {
            return message.localizedCaseInsensitiveContains("cancel")
        }
        return false
    }
}
