import SwiftUI
import UniformTypeIdentifiers

struct DropZoneView: View {
    @EnvironmentObject private var queue: ConversionQueueViewModel

    var body: some View {
        VStack(spacing: 8) {
            Image(folioSymbol: .drop)
                .font(.system(size: 30))
                .foregroundStyle(.tint)
            Text(FolioL10n.string("ui.drop_supported_book_files_or_a_folder_here", default: "Drop supported book files or a folder here"))
                .font(.headline)
            Text(FolioL10n.string("ui.or_use_add_files_add_folder", default: "or use Add Files / Add Folder"))
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, minHeight: 120)
        .background(queue.isDropTargeted ? Color.accentColor.opacity(0.16) : Color.secondary.opacity(0.08))
        .overlay {
            RoundedRectangle(cornerRadius: 12)
                .stroke(queue.isDropTargeted ? Color.accentColor : Color.secondary.opacity(0.3), style: StrokeStyle(lineWidth: 1, dash: [6]))
        }
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .onDrop(of: [UTType.fileURL.identifier], isTargeted: $queue.isDropTargeted) { providers in
            queue.importProviders(providers)
            return true
        }
    }
}
