import SwiftUI

struct ConversionControlsView: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel

    var body: some View {
        GroupBox("Conversion pipeline") {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Text("Target format")
                    Spacer()
                    Picker("", selection: $queue.target) {
                        ForEach(queue.availableTargets) { target in
                            Text(target.displayName).tag(target)
                        }
                    }
                    .frame(width: 220)
                    .labelsHidden()
                }

                HStack {
                    Text("Compatibility mode")
                    Spacer()
                    Picker("", selection: $queue.degradationMode) {
                        ForEach(FolioDegradationMode.allCases) { mode in
                            Text(mode.displayName).tag(mode)
                        }
                    }
                    .frame(width: 220)
                    .labelsHidden()
                }
                Text(modeDescription)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                HStack {
                    Text("Batch policy")
                    Spacer()
                    Picker("", selection: $queue.batchMode) {
                        ForEach(FolioBatchMode.allCases) { mode in
                            Text(mode.displayName).tag(mode)
                        }
                    }
                    .frame(width: 220)
                    .labelsHidden()
                }
                Text(batchDescription)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Divider()

                DisclosureGroup("Advanced compatibility options") {
                    VStack(alignment: .leading, spacing: 8) {
                        Toggle("Prefer deterministic rasterization when available", isOn: $queue.preferRasterization)
                        Toggle("Linearize complex tables when required", isOn: $queue.linearizeComplexTables)
                        Toggle("Strip embedded fonts for legacy targets", isOn: $queue.stripEmbeddedFonts)
                        Toggle("Deterministic output", isOn: $queue.deterministic)
                        HStack {
                            Text("Compression")
                            Spacer()
                            Picker("", selection: $queue.compression) {
                                ForEach(FolioCompression.allCases) { compression in
                                    Text(compression.displayName).tag(compression)
                                }
                            }
                            .frame(width: 160)
                            .labelsHidden()
                        }
                    }
                    .padding(.top, 6)
                }

                HStack(alignment: .firstTextBaseline) {
                    Text("Output")
                    Spacer()
                    Text(outputDescription)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .foregroundStyle(.secondary)
                    Button("Choose…") { queue.chooseOutputFolder() }
                }

                Divider()

                HStack {
                    Button {
                        queue.preflightSelected()
                    } label: {
                        Label("Preflight selected", systemImage: FolioSymbol.checklist.name)
                    }
                    .disabled(queue.selectedItem == nil || queue.isPreflighting || queue.isBatchConverting)

                    Button {
                        queue.preflightAll()
                    } label: {
                        Label("Preflight all", systemImage: FolioSymbol.checkAll.name)
                    }
                    .disabled(queue.items.isEmpty || queue.isPreflighting || queue.isBatchConverting)
                }

                if queue.isPreflighting {
                    ProgressView("Analyzing source files…")
                        .controlSize(.small)
                } else {
                    Text(queue.preflightSummary)
                        .font(.caption)
                        .foregroundStyle(queue.preflightBlockedCount > 0 ? .orange : .secondary)
                }

                HStack {
                    Button {
                        if queue.selectedItem?.status == .failed || queue.selectedItem?.status == .cancelled {
                            queue.retrySelected()
                        } else {
                            queue.convertSelected()
                        }
                    } label: {
                        let isRetry = queue.selectedItem?.status == .failed || queue.selectedItem?.status == .cancelled
                        Label(isRetry ? "Retry selected" : "Convert selected", systemImage: (isRetry ? FolioAction.retry : FolioAction.convert).symbol.name)
                    }
                    .disabled(!queue.selectedCanConvert)

                    Button {
                        queue.convertAll()
                    } label: {
                        Label(queue.isBatchConverting ? "Converting batch…" : "Convert approved files", systemImage: FolioSymbol.convertAll.name)
                    }
                    .disabled(queue.items.isEmpty || queue.isPreflighting || queue.isBatchConverting || !queue.hasPreflightForAll)
                }
            }
            .padding(4)
        }
        .onChange(of: queue.target) { _ in queue.invalidatePreflight() }
        .onChange(of: queue.degradationMode) { _ in queue.invalidatePreflight() }
        .onChange(of: queue.linearizeComplexTables) { _ in queue.invalidatePreflight() }
        .onChange(of: queue.preferRasterization) { _ in queue.invalidatePreflight() }
        .onChange(of: queue.stripEmbeddedFonts) { _ in queue.invalidatePreflight() }
    }

    private var modeDescription: String {
        switch queue.degradationMode {
        case .strict:
            "Strict allows only Exact and Equivalent plans; any L2+ fallback blocks before writing."
        case .compatible:
            "Compatible is the default: preserve semantics and layout, with an explicit verified fallback when needed."
        case .readable:
            "Readable prioritizes a usable document; every approximation or dropped feature remains in the report."
        }
    }

    private var batchDescription: String {
        switch queue.batchMode {
        case .bestEffort:
            "Best effort continues other files when one source or target fails."
        case .strict:
            "Strict batch rolls back successful outputs if any approved item fails."
        }
    }

    private var outputDescription: String {
        if let folder = queue.outputFolder {
            return folder.path
        }
        if queue.items.contains(where: { $0.sourceRoot != nil }) {
            return "FolioForge-output beside selected folder"
        }
        return "Same folder as input"
    }
}
