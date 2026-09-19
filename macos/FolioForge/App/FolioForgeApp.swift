import AppKit
import SwiftUI

@main
struct FolioForgeApp: App {
    @StateObject private var queue = ConversionQueueViewModel()

    init() {
        if let logo = Bundle.module.url(forResource: "folioforge-logo", withExtension: "png"),
           let image = NSImage(contentsOf: logo) {
            NSApplication.shared.applicationIconImage = image
        }
    }

    var body: some Scene {
        WindowGroup("FolioForge") {
            ContentView()
                .environmentObject(queue)
                .frame(minWidth: 1100, minHeight: 720)
        }
        .defaultSize(width: 1280, height: 800)

        Settings {
            SettingsView()
                .environmentObject(queue)
        }
    }
}
