import SwiftUI

struct CapabilityMatrixView: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel
    @Environment(\.dismiss) private var dismiss

    private var features: [String] {
        Array(Set(queue.capabilityProfiles.flatMap { $0.levels.keys })).sorted()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Target capability matrix")
                        .font(.title2.weight(.semibold))
                    Text("Loaded from folio-capabilities through the Rust FFI; this is the same matrix used by Preflight.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Done") { dismiss() }
            }

            if let error = queue.capabilityError {
                Label(error, systemImage: FolioSymbol.error.name)
                    .foregroundStyle(.red)
            } else if queue.capabilityProfiles.isEmpty {
                ProgressView("Loading capability profiles…")
                    .task { queue.loadCapabilities() }
            } else {
                ScrollView([.vertical, .horizontal]) {
                    VStack(alignment: .leading, spacing: 10) {
                        if !queue.inputFormatCapabilities.isEmpty {
                            Text("Registered input formats")
                                .font(.headline)
                            Text(queue.inputFormatCapabilities.map { capability in
                                "\(capability.format) (.\(capability.extensions.joined(separator: ", .")))"
                            }.joined(separator: "  ·  "))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 7) {
                        GridRow {
                            Text("Feature")
                                .font(.caption.weight(.semibold))
                                .frame(width: 150, alignment: .leading)
                            ForEach(queue.capabilityProfiles) { profile in
                                Text(displayFormat(profile.format))
                                    .font(.caption.weight(.semibold))
                                    .frame(width: 115, alignment: .leading)
                            }
                        }
                        Divider()
                            .gridCellColumns(queue.capabilityProfiles.count + 1)
                        ForEach(features, id: \.self) { feature in
                            GridRow {
                                Text(displayFeature(feature))
                                    .font(.caption)
                                    .frame(width: 150, alignment: .leading)
                                ForEach(queue.capabilityProfiles) { profile in
                                    let level = profile.levels[feature] ?? "Unsupported"
                                    Text(displayLevel(level))
                                        .font(.caption2.weight(.medium))
                                        .foregroundStyle(levelColor(level))
                                        .frame(width: 115, alignment: .leading)
                                }
                            }
                        }
                        }
                    }
                    .padding(12)
                }
                .background(Color.secondary.opacity(0.08))
                .clipShape(RoundedRectangle(cornerRadius: 10))
            }

            Text("Native = exact target primitive · Compatible = semantic equivalent · Approximate = visual approximation · Flattenable = readable structural fallback · Unsupported = no direct representation")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(20)
        .frame(width: 720, height: 560)
    }

    private func displayFormat(_ format: String) -> String {
        switch format {
        case "Epub3": "EPUB3"
        case "Kf7": "KF7"
        case "Kf8": "KF8"
        case "Kfx": "KFX"
        default: format
        }
    }

    private func displayFeature(_ feature: String) -> String {
        switch feature {
        case "EmbeddedFont": "Embedded font"
        case "VerticalWriting": "Vertical writing"
        case "ComplexTable": "Complex table"
        case "FixedLayout": "Fixed layout"
        case "SemanticStructure": "Semantic structure"
        default: feature
        }
    }

    private func displayLevel(_ level: String) -> String {
        switch level {
        case "Native": "Native"
        case "Compatible": "Compatible"
        case "Approximate": "Approximate"
        case "Flattenable": "Flattenable"
        default: "Unsupported"
        }
    }

    private func levelColor(_ level: String) -> Color {
        switch level {
        case "Native": .green
        case "Compatible": .blue
        case "Approximate": .orange
        case "Flattenable": .purple
        default: .red
        }
    }
}
