import SwiftUI

struct ConversionControlsView: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel

    var body: some View {
        GroupBox(FolioL10n.string("ui.conversion_pipeline", default: "Conversion pipeline")) {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Text(FolioL10n.string("ui.target_format", default: "Target format"))
                    Spacer()
                    Picker("", selection: $queue.target) {
                        ForEach(queue.availableTargets) { target in
                            Text(target.displayName).tag(target)
                        }
                    }
                    .frame(width: 220)
                    .labelsHidden()
                    .accessibilityLabel(FolioL10n.string("ui.target_format", default: "Target format"))
                }

                HStack {
                    Text(FolioL10n.string("ui.compatibility_mode", default: "Compatibility mode"))
                    Spacer()
                    Picker("", selection: $queue.degradationMode) {
                        ForEach(FolioDegradationMode.allCases) { mode in
                            Text(mode.displayName).tag(mode)
                        }
                    }
                    .frame(width: 220)
                    .labelsHidden()
                    .accessibilityLabel(FolioL10n.string("ui.compatibility_mode", default: "Compatibility mode"))
                }
                Text(modeDescription)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                HStack {
                    Text(FolioL10n.string("ui.batch_policy", default: "Batch policy"))
                    Spacer()
                    Picker("", selection: $queue.batchMode) {
                        ForEach(FolioBatchMode.allCases) { mode in
                            Text(mode.displayName).tag(mode)
                        }
                    }
                    .frame(width: 220)
                    .labelsHidden()
                    .accessibilityLabel(FolioL10n.string("ui.batch_policy", default: "Batch policy"))
                }
                Text(batchDescription)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Divider()

                DisclosureGroup(FolioL10n.string("ui.advanced_compatibility_options", default: "Advanced compatibility options")) {
                    VStack(alignment: .leading, spacing: 8) {
                        Toggle(FolioL10n.string("ui.prefer_deterministic_rasterization_when_available", default: "Prefer deterministic rasterization when available"), isOn: $queue.preferRasterization)
                        Toggle(FolioL10n.string("ui.linearize_complex_tables_when_required", default: "Linearize complex tables when required"), isOn: $queue.linearizeComplexTables)
                        Toggle(FolioL10n.string("ui.strip_embedded_fonts_for_legacy_targets", default: "Strip embedded fonts for legacy targets"), isOn: $queue.stripEmbeddedFonts)
                        Toggle(FolioL10n.string("ui.deterministic_output", default: "Deterministic output"), isOn: $queue.deterministic)
                        HStack {
                            Text(FolioL10n.string("ui.compression", default: "Compression"))
                            Spacer()
                            Picker("", selection: $queue.compression) {
                                ForEach(FolioCompression.allCases) { compression in
                                    Text(compression.displayName).tag(compression)
                                }
                            }
                            .frame(width: 160)
                            .labelsHidden()
                            .accessibilityLabel(FolioL10n.string("ui.compression", default: "Compression"))
                        }
                    }
                    .padding(.top, 6)
                }

                HStack(alignment: .firstTextBaseline) {
                    Text(FolioL10n.string("ui.output", default: "Output"))
                    Spacer()
                    Text(outputDescription)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .foregroundStyle(.secondary)
                    Button(FolioL10n.string("ui.choose", default: "Choose…")) { queue.chooseOutputFolder() }
                }

                Divider()

                HStack {
                    Button {
                        queue.preflightSelected()
                    } label: {
                        Label(FolioL10n.string("ui.preflight_selected", default: "Preflight selected"), systemImage: FolioSymbol.checklist.name)
                    }
                    .disabled(queue.selectedItem == nil || queue.isPreflighting || queue.isBatchConverting)

                    Button {
                        queue.preflightAll()
                    } label: {
                        Label(FolioL10n.string("ui.preflight_all", default: "Preflight all"), systemImage: FolioSymbol.checkAll.name)
                    }
                    .disabled(queue.items.isEmpty || queue.isPreflighting || queue.isBatchConverting)
                }

                if queue.isPreflighting {
                    ProgressView(FolioL10n.string("ui.analyzing_source_files", default: "Analyzing source files…"))
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
                        Label(
                            FolioL10n.string(
                                isRetry ? "ui.retry_selected" : "ui.convert_selected",
                                default: isRetry ? "Retry selected" : "Convert selected"
                            ),
                            systemImage: (isRetry ? FolioAction.retry : FolioAction.convert).symbol.name
                        )
                    }
                    .disabled(!queue.selectedCanConvert)

                    Button {
                        queue.convertAll()
                    } label: {
                        Label(
                            queue.isBatchConverting
                                ? FolioL10n.string("ui.converting_batch", default: "Converting batch…")
                                : FolioL10n.string("ui.convert_all_approved", default: "Convert All Approved"),
                            systemImage: FolioSymbol.convertAll.name
                        )
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
            FolioL10n.string("conversion.mode.strict_description", default: "Strict allows only Exact and Equivalent plans; any L2+ fallback blocks before writing.")
        case .compatible:
            FolioL10n.string("conversion.mode.compatible_description", default: "Compatible is the default: preserve semantics and layout, with an explicit verified fallback when needed.")
        case .readable:
            FolioL10n.string("conversion.mode.readable_description", default: "Readable prioritizes a usable document; every approximation or dropped feature remains in the report.")
        }
    }

    private var batchDescription: String {
        switch queue.batchMode {
        case .bestEffort:
            FolioL10n.string("conversion.batch.best_effort_description", default: "Best effort continues other files when one source or target fails.")
        case .strict:
            FolioL10n.string("inspector.batch_strict", default: "Strict batch rolls back successful outputs if any approved item fails.")
        }
    }

    private var outputDescription: String {
        if let folder = queue.outputFolder {
            return folder.path
        }
        if queue.items.contains(where: { $0.sourceRoot != nil }) {
            return FolioL10n.string("conversion.output.folder", default: "FolioForge-output beside selected folder")
        }
        return FolioL10n.string("conversion.output.same_folder", default: "Same folder as input")
    }
}
