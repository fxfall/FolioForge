import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel
    @AppStorage("folioforge.ui.language") private var interfaceLanguage = "system"

    var body: some View {
        Form {
            Section(FolioL10n.string("ui.language", default: "Language")) {
                Picker(FolioL10n.string("ui.language", default: "Language"), selection: $interfaceLanguage) {
                    Text(FolioL10n.string("ui.system_default", default: "System default")).tag("system")
                    Text(FolioL10n.string("ui.english", default: "English")).tag("en")
                    Text(FolioL10n.string("ui.simplified_chinese", default: "Simplified Chinese")).tag("zh-Hans")
                }
                Text(FolioL10n.string(
                    "ui.language_changes_only_the_interface_not_book_content_or_core_behavior",
                    default: "This changes only the interface language, not book content or Core behavior."
                ))
                .font(.footnote)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            }

            Section(FolioL10n.string("ui.defaults", default: "Defaults")) {
                Picker(FolioL10n.string("ui.target_format", default: "Target format"), selection: $queue.target) {
                    ForEach(queue.availableTargets) { target in
                        Text(target.displayName).tag(target)
                    }
                }
                Picker(FolioL10n.string("ui.compression", default: "Compression"), selection: $queue.compression) {
                    ForEach(FolioCompression.allCases) { compression in
                        Text(compression.displayName).tag(compression)
                    }
                }
                Picker(FolioL10n.string("ui.compatibility_mode", default: "Compatibility mode"), selection: $queue.degradationMode) {
                    ForEach(FolioDegradationMode.allCases) { mode in
                        Text(mode.displayName).tag(mode)
                    }
                }
                Picker(FolioL10n.string("ui.batch_policy", default: "Batch policy"), selection: $queue.batchMode) {
                    ForEach(FolioBatchMode.allCases) { mode in
                        Text(mode.displayName).tag(mode)
                    }
                }
                Toggle(FolioL10n.string("ui.prefer_deterministic_rasterization", default: "Prefer deterministic rasterization"), isOn: $queue.preferRasterization)
                Toggle(FolioL10n.string("ui.linearize_complex_tables", default: "Linearize complex tables"), isOn: $queue.linearizeComplexTables)
                Toggle(FolioL10n.string("ui.strip_embedded_fonts", default: "Strip embedded fonts"), isOn: $queue.stripEmbeddedFonts)
                Toggle(FolioL10n.string("ui.deterministic_output", default: "Deterministic output"), isOn: $queue.deterministic)
            }

            Section(FolioL10n.string("ui.about", default: "About")) {
                Text(FolioL10n.string("ui.folioforge_0_1", default: "FolioForge 0.1"))
                Text(FolioL10n.string("ui.folioforge_uses_a_local_rust_core_through_a_small", default: "FolioForge uses a local Rust Core through a small C ABI."))
                Text(FolioL10n.string("ui.no_network_access_is_required_for_conversion", default: "No network access is required for conversion."))
                    .foregroundStyle(.secondary)
                Text(FolioL10n.format("error.core_abi", default: "Core %@ · ABI %@", queue.coreVersion, "1"))
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
