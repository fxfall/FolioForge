import AppKit
import SwiftUI
import WebKit

struct CompatibilityPreviewPane: View {
    let bundle: FolioPreviewBundle?
    let isLoading: Bool
    let error: String?
    @Binding var settings: FolioPreviewSettings
    let refresh: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Label("Compatibility Preview", systemImage: FolioAction.preview.symbol.name)
                    .font(.headline)
                Spacer()
                if let bundle {
                    Text(bundle.target.label)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(bundle.blocked ? .orange : .secondary)
                        .lineLimit(1)
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 14)
            .padding(.bottom, 12)

            HStack(spacing: 8) {
                Picker("Reader", selection: $settings.device) {
                    ForEach(FolioPreviewDevice.allCases) { device in
                        Text(device.title).tag(device)
                    }
                }
                .pickerStyle(.menu)
                .labelsHidden()
                .accessibilityLabel("Reader device")
                .fixedSize()

                Picker("Orientation", selection: $settings.orientation) {
                    ForEach(FolioPreviewOrientation.allCases) { orientation in
                        Text(orientation.title).tag(orientation)
                    }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .accessibilityLabel("Preview orientation")
                .fixedSize()

                Spacer(minLength: 4)

                Button {
                    settings.fontSizePercent = max(60, settings.fontSizePercent - 5)
                } label: { Text("A−") }
                .buttonStyle(.borderless)
                .help("Decrease preview text size")
                Text("\(settings.fontSizePercent)%")
                    .font(.caption.monospacedDigit())
                    .frame(minWidth: 34)
                Button {
                    settings.fontSizePercent = min(200, settings.fontSizePercent + 5)
                } label: { Text("A+") }
                .buttonStyle(.borderless)
                .help("Increase preview text size")
                Button(action: refresh) {
                    Image(folioSymbol: FolioAction.refreshPreview.symbol)
                }
                .buttonStyle(.borderless)
                .disabled(isLoading)
                .help("Refresh compatibility preview")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)

            if let bundle {
                VStack(alignment: .leading, spacing: 5) {
                    Text(bundle.target.disclaimer)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Text("\(bundle.target.viewportWidth) × \(bundle.target.viewportHeight) · \(bundle.target.device) · \(bundle.target.orientation)")
                        .font(.caption2.monospaced())
                        .foregroundStyle(.tertiary)
                    if bundle.blocked {
                        Label("The selected mode blocks this target. The source IR is shown without target projection.", systemImage: FolioSymbol.warning.name)
                            .font(.caption)
                            .foregroundStyle(.orange)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 11)

                HTMLPreviewView(html: bundle.html)
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
                        ProgressView("Preparing preview…")
                            .controlSize(.regular)
                    } else {
                        Image(folioSymbol: .reader)
                            .font(.system(size: 34, weight: .light))
                            .foregroundStyle(.secondary)
                        Text(error == nil ? "Preview is ready when you are" : "Preview could not be generated")
                            .font(.headline)
                        Text(error ?? "This reader view is generated from the source IR, book edits, target profile, and compatibility plan.")
                            .font(.caption)
                            .foregroundStyle(error == nil ? Color.secondary : Color.red)
                            .multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: 360)
                        Button { refresh() } label: { Label("Generate Preview", systemImage: FolioAction.preview.symbol.name) }
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
