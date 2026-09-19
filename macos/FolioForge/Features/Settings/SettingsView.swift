import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel

    var body: some View {
        Form {
            Section("Defaults") {
                Picker("Target format", selection: $queue.target) {
                    ForEach(queue.availableTargets) { target in
                        Text(target.displayName).tag(target)
                    }
                }
                Picker("Compression", selection: $queue.compression) {
                    ForEach(FolioCompression.allCases) { compression in
                        Text(compression.displayName).tag(compression)
                    }
                }
                Picker("Compatibility mode", selection: $queue.degradationMode) {
                    ForEach(FolioDegradationMode.allCases) { mode in
                        Text(mode.displayName).tag(mode)
                    }
                }
                Picker("Batch policy", selection: $queue.batchMode) {
                    ForEach(FolioBatchMode.allCases) { mode in
                        Text(mode.displayName).tag(mode)
                    }
                }
                Toggle("Prefer deterministic rasterization", isOn: $queue.preferRasterization)
                Toggle("Linearize complex tables", isOn: $queue.linearizeComplexTables)
                Toggle("Strip embedded fonts", isOn: $queue.stripEmbeddedFonts)
                Toggle("Deterministic output", isOn: $queue.deterministic)
            }

            Section("About") {
                Text("FolioForge 0.1")
                Text("FolioForge uses a local Rust Core through a small C ABI.")
                Text("No network access is required for conversion.")
                    .foregroundStyle(.secondary)
                Text("Core \(queue.coreVersion) · ABI 1")
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
            }
        }
        .padding(20)
        .frame(width: 420)
        .onChange(of: queue.target) { _ in queue.invalidatePreflight() }
        .onChange(of: queue.degradationMode) { _ in queue.invalidatePreflight() }
        .onChange(of: queue.linearizeComplexTables) { _ in queue.invalidatePreflight() }
        .onChange(of: queue.preferRasterization) { _ in queue.invalidatePreflight() }
        .onChange(of: queue.stripEmbeddedFonts) { _ in queue.invalidatePreflight() }
    }
}
