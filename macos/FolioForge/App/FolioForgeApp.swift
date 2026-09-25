import AppKit
import SwiftUI

@main
struct FolioForgeApp: App {
    @StateObject private var queue = ConversionQueueViewModel()
    @StateObject private var comicWorkspace = ComicWorkspaceViewModel()
    @AppStorage("folioforge.ui.language") private var interfaceLanguage = "system"

    private var interfaceLocale: Locale {
        FolioL10n.locale(for: interfaceLanguage)
    }

    init() {
        if let logo = Bundle.module.url(forResource: "folioforge-logo", withExtension: "png"),
           let image = NSImage(contentsOf: logo) {
            NSApplication.shared.applicationIconImage = image
        }
    }

    var body: some Scene {
        WindowGroup("FolioForge") {
            TabView {
                ContentView()
                    .environmentObject(queue)
                    .tabItem { Label(FolioL10n.string("ui.books", default: "Books"), systemImage: "book.closed") }
                ComicWorkspaceView()
                    .environmentObject(comicWorkspace)
                    .tabItem { Label(FolioL10n.string("ui.comics", default: "Comics"), systemImage: "books.vertical") }
            }
            .environment(\.locale, interfaceLocale)
            .frame(minWidth: 1100, minHeight: 720)
        }
        .defaultSize(width: 1280, height: 800)

        Settings {
            SettingsView()
                .environmentObject(queue)
                .environment(\.locale, interfaceLocale)
        }
    }
}
