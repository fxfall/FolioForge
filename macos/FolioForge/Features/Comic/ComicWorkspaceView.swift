import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct ComicWorkspaceView: View {
    @EnvironmentObject private var model: ComicWorkspaceViewModel

    private let columns = [
        GridItem(.adaptive(minimum: 108, maximum: 150), spacing: 12, alignment: .top),
    ]

    var body: some View {
        VStack(spacing: 0) {
            header
            if let summary = model.summary {
                comicWorkspace(summary)
            } else {
                emptyWorkspace
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
        .task(id: model.selectedPageID) {
            await model.loadSelectedPage()
        }
        .onDrop(of: [UTType.fileURL.identifier], isTargeted: $model.isDropTargeted) { providers in
            guard let provider = providers.first else { return false }
            provider.loadDataRepresentation(forTypeIdentifier: UTType.fileURL.identifier) { data, _ in
                Task { @MainActor in model.acceptDroppedURLData(data) }
            }
            return true
        }
        .alert(FolioL10n.string("ui.comic_workspace", default: "Comic workspace"), isPresented: Binding(
            get: { model.errorMessage != nil },
            set: { if !$0 { model.clearError() } }
        )) {
            Button(FolioL10n.string("ui.ok", default: "OK")) { model.clearError() }
        } message: {
            Text(model.errorMessage ?? FolioL10n.string("error.unknown", default: "Unknown error"))
        }
    }

    private var header: some View {
        HStack(spacing: 14) {
            Image(folioSymbol: .book)
                .font(.system(size: 25, weight: .semibold))
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 2) {
                Text(FolioL10n.string("ui.comic_workspace", default: "Comic workspace"))
                    .font(.title2.weight(.semibold))
                Text(FolioL10n.string("ui.cbz_zip_image_folders_core_semantic_ir", default: "CBZ · ZIP · image folders · Core Semantic IR"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if let sourceURL = model.sourceURL {
                Text(sourceURL.lastPathComponent)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: 360, alignment: .trailing)
            }
            Button(action: model.openPanel) {
                Label(FolioL10n.string("ui.open_comic", default: "Open Comic…"), systemImage: "plus")
            }
            .keyboardShortcut("o", modifiers: [.command, .shift])
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
        .background(.bar)
    }

    private var emptyWorkspace: some View {
        VStack(spacing: 18) {
            Spacer()
            Image(systemName: model.isDropTargeted ? "arrow.down.doc.fill" : "books.vertical")
                .font(.system(size: 44, weight: .light))
                .foregroundStyle(model.isDropTargeted ? Color.accentColor : Color.secondary)
            Text(model.isOpening
                ? FolioL10n.string("status.opening_comic_source", default: "Opening comic source…")
                : FolioL10n.string("ui.open_comic_source", default: "Open a comic source"))
                .font(.title3.weight(.medium))
            Text(FolioL10n.string("ui.choose_a_cbz_zip_or_a_folder_of_images", default: "Choose a CBZ, ZIP, or a folder of images. Core determines page identity, order, preview, and supported output."))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 550)
            Button(FolioL10n.string("ui.choose_source", default: "Choose Source…"), action: model.openPanel)
                .buttonStyle(.borderedProminent)
                .disabled(model.isOpening)
            Text(FolioL10n.string("ui.you_can_also_drop_a_source_anywhere_in_this", default: "You can also drop a source anywhere in this workspace."))
                .font(.caption)
                .foregroundStyle(.tertiary)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(32)
        .background(model.isDropTargeted ? Color.accentColor.opacity(0.07) : Color.clear)
        .overlay {
            RoundedRectangle(cornerRadius: 14)
                .strokeBorder(
                    model.isDropTargeted ? Color.accentColor : Color.secondary.opacity(0.25),
                    style: StrokeStyle(lineWidth: 1, dash: [7, 5])
                )
                .padding(18)
                .allowsHitTesting(false)
        }
        .overlay {
            if model.isOpening {
                ProgressView()
                    .controlSize(.small)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
                    .padding(.bottom, 16)
            }
        }
    }

    private func comicWorkspace(_ summary: FolioComicSummary) -> some View {
        VStack(spacing: 0) {
            summaryBar(summary)
            HSplitView {
                pageGrid
                    .frame(minWidth: 330, idealWidth: 430, maxWidth: 620, maxHeight: .infinity)
                previewPane
                    .frame(minWidth: 350, maxWidth: .infinity, maxHeight: .infinity)
            }
            .padding(16)
            Divider()
            conversionBar
        }
    }

    private func summaryBar(_ summary: FolioComicSummary) -> some View {
        HStack(alignment: .center, spacing: 14) {
            Image(systemName: "rectangle.stack")
                .font(.system(size: 22))
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 3) {
                Text(summary.title.isEmpty
                    ? FolioL10n.string("comic.untitled", default: "Untitled comic")
                    : summary.title)
                    .font(.headline)
                    .lineLimit(1)
                Text([
                    summary.sourceType,
                    FolioL10n.format("comic.page_count", default: "Pages: %@", String(summary.pageCount)),
                    summary.authors.joined(separator: ", "),
                ].filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            if let direction = summary.readingDirection, !direction.isEmpty {
                Label(direction, systemImage: "text.alignleft")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Text(FolioL10n.string("ui.semantic_ir", default: "Semantic IR"))
                .font(.caption.weight(.medium))
                .padding(.horizontal, 9)
                .padding(.vertical, 5)
                .background(Color.accentColor.opacity(0.1), in: Capsule())
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
        .background(Color(nsColor: .controlBackgroundColor).opacity(0.6))
    }

    private var pageGrid: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(FolioL10n.string("ui.pages", default: "Pages"))
                    .font(.headline)
                Spacer()
                Text(FolioL10n.string("ui.order_from_core", default: "Order from Core"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            ScrollView {
                LazyVGrid(columns: columns, alignment: .leading, spacing: 14) {
                    ForEach(model.pages) { page in
                        ComicPageTile(
                            model: model,
                            page: page,
                            select: { model.selectedPageID = page.pageId }
                        )
                    }
                }
                .padding(.vertical, 3)
            }
        }
        .padding(12)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 12))
    }

    private var previewPane: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(FolioL10n.string("ui.reader_page", default: "Reader page"))
                    .font(.headline)
                Spacer()
                if let readerPage = model.readerPage {
                    Text(FolioL10n.format("error.page_of", default: "Page %@ of %@", String(readerPage.pageIndex + 1), String(readerPage.pageCount)))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else if let page = model.selectedPage {
                    Text(FolioL10n.format("error.source_page", default: "Source page %@ · %@", String(page.displayIndex), page.sourceFormat.uppercased()))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            readerControls
            ReaderCanvas(model: model)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .overlay {
                    RoundedRectangle(cornerRadius: 10)
                        .stroke(Color.secondary.opacity(0.15), lineWidth: 1)
                }
            if let reason = model.readerFallbackReason {
                Label(reason, systemImage: "info.circle")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            pageFacts
        }
        .padding(12)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 12))
    }

    private var readerControls: some View {
        VStack(spacing: 8) {
            HStack(spacing: 8) {
                Button(FolioL10n.string("ui.previous", default: "Previous")) { model.navigateReader(forward: false) }
                    .disabled(!model.canNavigateReader || model.previewLoading)
                Button(FolioL10n.string("ui.next", default: "Next")) { model.navigateReader(forward: true) }
                    .disabled(!model.canNavigateReader || model.previewLoading)
                Spacer()
                Picker(FolioL10n.string("ui.page_fit", default: "Page fit"), selection: Binding(
                    get: { model.readerViewport.contentMode },
                    set: model.setReaderContentMode
                )) {
                    ForEach(FolioReaderContentMode.allCases) { mode in
                        Text(mode.label).tag(mode)
                    }
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 280)
                .disabled(!model.canControlReader)
            }
            HStack(spacing: 8) {
                Picker(FolioL10n.string("ui.reading_direction", default: "Reading direction"), selection: Binding(
                    get: { model.readerSessionSummary?.direction ?? .leftToRight },
                    set: model.setReaderDirection
                )) {
                    ForEach(FolioReaderDirection.allCases) { direction in
                        Text(direction.label).tag(direction)
                    }
                }
                .frame(maxWidth: 230)
                .disabled(!model.canControlReader)
                Picker(FolioL10n.string("ui.page_layout", default: "Page layout"), selection: Binding(
                    get: { model.readerSessionSummary?.spreadMode ?? .singlePage },
                    set: model.setReaderSpreadMode
                )) {
                    ForEach(FolioReaderSpreadMode.allCases) { mode in
                        Text(mode.label).tag(mode)
                    }
                }
                .frame(maxWidth: 210)
                .disabled(!model.canControlReader)
                Spacer()
                Button { model.adjustReaderZoom(by: -0.25) } label: {
                    Image(systemName: "minus.magnifyingglass")
                }
                .help(FolioL10n.string("ui.zoom_out", default: "Zoom out"))
                .disabled(!model.canControlReader)
                Button { model.adjustReaderZoom(by: 0.25) } label: {
                    Image(systemName: "plus.magnifyingglass")
                }
                .help(FolioL10n.string("ui.zoom_in", default: "Zoom in"))
                .disabled(!model.canControlReader)
                Text("\(Int(model.readerViewport.scale * 100))%")
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .frame(minWidth: 42, alignment: .trailing)
            }
        }
        .buttonStyle(.bordered)
    }

    @ViewBuilder
    private var pageFacts: some View {
        if let page = model.selectedPageInfo ?? model.selectedPage {
            HStack(spacing: 16) {
                Label(page.name, systemImage: "doc")
                    .lineLimit(1)
                if let width = page.width, let height = page.height {
                    Label("\(width) × \(height)", systemImage: "aspectratio")
                }
                if let orientation = page.orientation {
                    Text(orientation.capitalized)
                }
                Spacer(minLength: 0)
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    private var conversionBar: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .center, spacing: 14) {
                if let options = model.options, !options.targets.isEmpty {
                    Picker(FolioL10n.string("ui.output", default: "Output"), selection: $model.selectedTargetID) {
                        ForEach(options.targets) { target in
                            Text("\(target.displayName) (.\(target.fileExtension))")
                                .tag(target.id)
                        }
                    }
                    .frame(width: 310)
                    .disabled(model.isConverting || options.targets.count == 1)
                }
                Spacer()
                if model.isConverting {
                    Button(FolioL10n.string("ui.cancel", default: "Cancel")) { model.cancelConversion() }
                        .disabled(model.progress?.stage == .finished)
                }
                Button(action: model.chooseOutputAndConvert) {
                    Label(
                        model.isConverting
                            ? FolioL10n.string("status.converting", default: "Converting")
                            : FolioL10n.string("ui.convert", default: "Convert"),
                        systemImage: "arrow.triangle.2.circlepath"
                    )
                }
                .buttonStyle(.borderedProminent)
                .disabled(!model.canConvert)
            }

            if model.isConverting {
                ProgressView(value: model.progress?.fraction)
                Text(model.progress?.label ?? FolioL10n.string("status.preparing_conversion", default: "Preparing Core conversion…"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else if let report = model.report {
                HStack(alignment: .top, spacing: 10) {
                    Label(FolioL10n.format(
                        "error.created_pages",
                        default: "Created %@ · Pages: %@",
                        URL(fileURLWithPath: report.outputPath).lastPathComponent,
                        String(report.pageCount)
                    ), systemImage: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                    Spacer()
                    Button(FolioL10n.string("ui.show_in_finder", default: "Show in Finder"), action: model.revealOutput)
                }
                .font(.caption)
                if !report.warnings.isEmpty {
                    warningList(report.warnings)
                }
            } else if let notes = model.options?.notes, !notes.isEmpty {
                Text(notes.joined(separator: "  "))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if let warnings = model.summary?.warnings, !warnings.isEmpty {
                warningList(warnings)
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
        .background(Color(nsColor: .controlBackgroundColor).opacity(0.55))
    }

    private func warningList(_ warnings: [FolioComicWarning]) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(warnings) { warning in
                Label(warning.message, systemImage: "exclamationmark.triangle")
                    .font(.caption)
                    .foregroundStyle(warning.severity.lowercased() == "error" ? .red : .orange)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

private struct ReaderCanvas: View {
    @ObservedObject var model: ComicWorkspaceViewModel

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .topLeading) {
                Color.black.opacity(0.035)
                if let spread = model.readerSpread {
                    ForEach(Array(spread.pages.enumerated()), id: \.offset) { _, page in
                        ForEach(Array(page.placements.enumerated()), id: \.offset) { _, placement in
                            if let image = model.readerImage(for: placement) {
                                readerImage(image, placement: placement)
                            }
                        }
                    }
                } else if let page = model.readerPage {
                    ForEach(Array(page.placements.enumerated()), id: \.offset) { _, placement in
                        if let image = model.readerImage(for: placement) {
                            readerImage(image, placement: placement)
                        }
                    }
                } else if let data = model.legacyPreviewData, let image = NSImage(data: data) {
                    Image(nsImage: image)
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .padding(10)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if model.previewLoading {
                    ProgressView(FolioL10n.string("ui.loading_reader_page", default: "Loading Reader page…"))
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ContentUnavailableView(
                        FolioL10n.string("ui.select_a_page", default: "Select a page"),
                        systemImage: "photo"
                    )
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .frame(width: proxy.size.width, height: proxy.size.height, alignment: .topLeading)
            .clipped()
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 10).onEnded { gesture in
                model.panReader(
                    byX: gesture.translation.width,
                    y: gesture.translation.height
                )
            })
            .onAppear {
                model.updateReaderViewport(width: proxy.size.width, height: proxy.size.height)
            }
            .onChange(of: proxy.size) { _, size in
                model.updateReaderViewport(width: size.width, height: size.height)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 10))
    }

    @ViewBuilder
    private func readerImage(
        _ image: NSImage,
        placement: FolioReaderPlacement
    ) -> some View {
        let destination = placement.destination
        let clipping = placement.clippingRect
        let rendered = Image(nsImage: image)
            .resizable()
            .interpolation(.high)
            .frame(width: CGFloat(destination.width), height: CGFloat(destination.height))

        if placement.clipsToViewport {
            rendered
                .mask(alignment: .topLeading) {
                    Rectangle()
                        .frame(width: CGFloat(clipping.width), height: CGFloat(clipping.height))
                        .offset(
                            x: CGFloat(clipping.x - destination.x),
                            y: CGFloat(clipping.y - destination.y)
                        )
                }
                .offset(x: CGFloat(destination.x), y: CGFloat(destination.y))
                .accessibilityLabel(placement.altText)
        } else {
            rendered
                .offset(x: CGFloat(destination.x), y: CGFloat(destination.y))
                .accessibilityLabel(placement.altText)
        }
    }
}

private struct ComicPageTile: View {
    @ObservedObject var model: ComicWorkspaceViewModel
    let page: FolioComicPage
    let select: () -> Void

    var body: some View {
        let _ = model.thumbnailRevision
        let selected = model.selectedPageID == page.pageId
        Button(action: select) {
            VStack(alignment: .leading, spacing: 6) {
                ZStack {
                    RoundedRectangle(cornerRadius: 7)
                        .fill(Color.black.opacity(0.045))
                    if let thumbnail = model.cachedThumbnail(for: page.pageId), let image = NSImage(data: thumbnail) {
                        Image(nsImage: image)
                            .resizable()
                            .aspectRatio(contentMode: .fit)
                            .padding(4)
                    } else {
                        Image(systemName: "photo")
                            .font(.title2)
                            .foregroundStyle(.tertiary)
                    }
                }
                .frame(height: 148)
                Text(String(format: "%03d  %@", page.displayIndex, page.name))
                    .font(.caption2)
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(7)
            .background(selected ? Color.accentColor.opacity(0.12) : Color.clear, in: RoundedRectangle(cornerRadius: 9))
            .overlay {
                RoundedRectangle(cornerRadius: 9)
                    .stroke(selected ? Color.accentColor : Color.secondary.opacity(0.18), lineWidth: selected ? 2 : 1)
            }
            .contentShape(RoundedRectangle(cornerRadius: 9))
        }
        .buttonStyle(.plain)
        .task(id: page.pageId) {
            _ = await model.loadThumbnail(for: page)
        }
        .accessibilityLabel(FolioL10n.format("error.page_accessibility", default: "Page %@, %@", String(page.displayIndex), page.name))
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}
