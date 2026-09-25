import AppKit
import SwiftUI
import WebKit

struct CompatibilityPreviewPane: View {
    let bundle: FolioReaderPreviewBundle?
    let isLoading: Bool
    let error: String?
    @Binding var settings: FolioPreviewSettings
    let refresh: () -> Void
    let imageCache: FolioReaderImageCache
    let imageSessionID: String

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Label(FolioL10n.string("ui.compatibility_preview", default: "Compatibility Preview"), systemImage: FolioAction.preview.symbol.name)
                    .font(.headline)
                Spacer()
                if let bundle {
                    Text(bundle.target?.label ?? FolioL10n.string("ui.semantic_ir", default: "Semantic IR"))
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(bundle.blocked ? .orange : .secondary)
                        .lineLimit(1)
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 14)
            .padding(.bottom, 12)

            HStack(spacing: 8) {
                Picker(FolioL10n.string("ui.reader", default: "Reader"), selection: $settings.device) {
                    ForEach(FolioPreviewDevice.allCases) { device in
                        Text(device.title).tag(device)
                    }
                }
                .pickerStyle(.menu)
                .labelsHidden()
                .accessibilityLabel(FolioL10n.string("ui.reader_device", default: "Reader device"))
                .fixedSize()

                Picker(FolioL10n.string("ui.orientation", default: "Orientation"), selection: $settings.orientation) {
                    ForEach(FolioPreviewOrientation.allCases) { orientation in
                        Text(orientation.title).tag(orientation)
                    }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .accessibilityLabel(FolioL10n.string("ui.preview_orientation", default: "Preview orientation"))
                .fixedSize()

                Spacer(minLength: 4)

                Button {
                    settings.fontSizePercent = max(60, settings.fontSizePercent - 5)
                } label: { Text(FolioL10n.string("ui.preview.decrease_text_size", default: "A−")) }
                .buttonStyle(.borderless)
                .help(FolioL10n.string("ui.decrease_preview_text_size", default: "Decrease preview text size"))
                Text("\(settings.fontSizePercent)%")
                    .font(.caption.monospacedDigit())
                    .frame(minWidth: 34)
                Button {
                    settings.fontSizePercent = min(200, settings.fontSizePercent + 5)
                } label: { Text(FolioL10n.string("ui.preview.increase_text_size", default: "A+")) }
                .buttonStyle(.borderless)
                .help(FolioL10n.string("ui.increase_preview_text_size", default: "Increase preview text size"))
                Button(action: refresh) {
                    Image(folioSymbol: FolioAction.refreshPreview.symbol)
                }
                .buttonStyle(.borderless)
                .disabled(isLoading)
                .help(FolioL10n.string("ui.refresh_compatibility_preview", default: "Refresh compatibility preview"))
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)

            if let bundle {
                VStack(alignment: .leading, spacing: 5) {
                    if let target = bundle.target {
                        Text(target.disclaimer)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                        Text("\(target.viewportWidth) × \(target.viewportHeight) · \(target.device) · \(target.orientation)")
                            .font(.caption2.monospaced())
                            .foregroundStyle(.tertiary)
                    }
                    if bundle.blocked {
                        Label(FolioL10n.string("ui.the_selected_mode_blocks_this_target_the_source_ir", default: "The selected mode blocks this target. The source IR is shown without target projection."), systemImage: FolioSymbol.warning.name)
                            .font(.caption)
                            .foregroundStyle(.orange)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 11)

                Group {
                    switch bundle.content {
                    case .reflowable(_, let html):
                        HTMLPreviewView(html: html)
                    case .fixedPage(let page):
                        FolioReaderFixedPagePreview(
                            page: page,
                            imageCache: imageCache,
                            sessionID: imageSessionID
                        )
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(paperColor)
                .clipShape(RoundedRectangle(cornerRadius: 10))
                .overlay {
                    RoundedRectangle(cornerRadius: 10)
                        .strokeBorder(Color.black.opacity(0.08), lineWidth: 1)
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 12)
            } else {
                VStack(spacing: 12) {
                    if isLoading {
                        ProgressView(FolioL10n.string("ui.preparing_preview", default: "Preparing preview…"))
                            .controlSize(.regular)
                    } else {
                        Image(folioSymbol: .reader)
                            .font(.system(size: 34, weight: .light))
                            .foregroundStyle(.secondary)
                        Text(error == nil
                            ? FolioL10n.string("preview.ready", default: "Preview is ready when you are")
                            : FolioL10n.string("preview.failed", default: "Preview could not be generated"))
                            .font(.headline)
                        Text(error ?? FolioL10n.string(
                            "preview.explanation",
                            default: "This reader view is generated from the source IR, book edits, target profile, and compatibility plan."
                        ))
                            .font(.caption)
                            .foregroundStyle(error == nil ? Color.secondary : Color.red)
                            .multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: 360)
                        Button { refresh() } label: { Label(FolioL10n.string("ui.generate_preview", default: "Generate Preview"), systemImage: FolioAction.preview.symbol.name) }
                            .disabled(isLoading)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(24)
                .background(paperColor, in: RoundedRectangle(cornerRadius: 10))
                .padding(12)
            }
        }
        .background(Color(nsColor: .windowBackgroundColor))
    }

    private var paperColor: Color {
        Color(red: 0.975, green: 0.967, blue: 0.936)
    }
}

private struct FolioReaderFixedPagePreview: View {
    let page: FolioReaderPageModel
    let imageCache: FolioReaderImageCache
    let sessionID: String

    var body: some View {
        GeometryReader { proxy in
            let viewportWidth = CGFloat(page.viewportSize.width)
            let viewportHeight = CGFloat(page.viewportSize.height)
            if viewportWidth > 0, viewportHeight > 0,
               proxy.size.width > 0, proxy.size.height > 0 {
                let scale = min(proxy.size.width / viewportWidth, proxy.size.height / viewportHeight)
                ZStack(alignment: .topLeading) {
                    ForEach(Array(page.placements.enumerated()), id: \.offset) { _, placement in
                        if let image = imageCache.image(
                            data: placement.resource.data,
                            sessionID: sessionID,
                            resourceID: placement.resource.resourceId
                        ) {
                            placedImage(image, placement: placement)
                        }
                    }
                }
                .frame(width: viewportWidth, height: viewportHeight, alignment: .topLeading)
                .scaleEffect(scale, anchor: .topLeading)
                .frame(width: viewportWidth * scale, height: viewportHeight * scale)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ProgressView(FolioL10n.string("ui.preparing_reader_page", default: "Preparing Reader page…"))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }

    @ViewBuilder
    private func placedImage(
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

private struct HTMLPreviewView: NSViewRepresentable {
    let html: String

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> WKWebView {
        let view = WKWebView()
        view.setValue(false, forKey: "drawsBackground")
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        guard context.coordinator.lastHTML != html else { return }
        context.coordinator.lastHTML = html
        view.loadHTMLString(html, baseURL: nil)
    }

    final class Coordinator {
        var lastHTML: String?
    }
}
