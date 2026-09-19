import SwiftUI

/// Central SF Symbols registry for the native macOS client.
enum FolioSymbol: String {
    case addFiles = "doc.badge.plus"
    case addFolder = "folder.badge.plus"
    case summary = "doc.text"
    case metadata = "info.circle"
    case cover = "photo"
    case appearance = "textformat"
    case structure = "list.bullet.indent"
    case parser = "waveform.path.ecg"
    case onlineMetadata = "magnifyingglass"
    case onlineCover = "photo.badge.plus"
    case onlineFont = "textformat.alt"
    case preview = "eye"
    case check = "checkmark.circle"
    case convert = "arrow.right.circle.fill"
    case diagnostics = "exclamationmark.triangle"
    case inspector = "sidebar.right"

    case books = "books.vertical"
    case book = "book.closed"
    case bookFilled = "book.closed.fill"
    case stop = "stop.circle"
    case pause = "pause.circle"
    case sidebarLeft = "sidebar.left"
    case success = "checkmark.circle.fill"
    case progress = "arrow.triangle.2.circlepath"
    case warning = "exclamationmark.triangle.fill"
    case error = "xmark.octagon.fill"
    case privacy = "lock.shield"
    case minusText = "minus.magnifyingglass"
    case plusText = "plus.magnifyingglass"
    case typographySize = "textformat.size"
    case styles = "paintbrush.pointed"
    case refresh = "arrow.clockwise"
    case reader = "rectangle.portrait.on.rectangle.portrait"
    case drop = "arrow.down.doc"
    case checkAll = "list.bullet.clipboard"
    case convertAll = "play.circle"
    case capabilities = "square.grid.2x2"
    case more = "ellipsis.circle"
    case removeSelected = "minus.circle"
    case remove = "xmark"
    case removeCircle = "xmark.circle.fill"
    case checklist = "checklist"
    case retry = "arrow.clockwise.circle.fill"
    case keep = "arrow.uturn.backward"
    case trash = "trash"
    case lock = "lock.fill"
    case add = "plus"
    case moveUp = "chevron.up"
    case moveDown = "chevron.down"
    case checkSeal = "checkmark.seal"
    case checkSealFilled = "checkmark.seal.fill"
    case alertCircle = "exclamationmark.circle"
    case documentSearch = "doc.text.magnifyingglass"
}

extension FolioSymbol {
    var name: String { rawValue }
}

/// Shared action-to-symbol map used by toolbar and editor commands.
enum FolioAction {
    case addFiles
    case addFolder
    case check
    case checkAll
    case retry
    case convert
    case convertAll
    case inspect
    case preview
    case refreshPreview
    case findMetadata
    case findCover
    case findFont
    case addFont
    case replaceCover
    case removeCover
    case removeSelected
    case removeItem
    case removeToken
    case removeReplacement
    case addStyleRule
    case addToken
    case applyBulkEdit
    case capabilities
    case more

    var symbol: FolioSymbol {
        switch self {
        case .addFiles: .addFiles
        case .addFolder: .addFolder
        case .check: .check
        case .checkAll: .checkAll
        case .retry: .retry
        case .convert: .convert
        case .convertAll: .convertAll
        case .inspect: .inspector
        case .preview: .preview
        case .refreshPreview: .refresh
        case .findMetadata: .onlineMetadata
        case .findCover: .onlineCover
        case .findFont: .onlineFont
        case .addFont: .add
        case .replaceCover: .cover
        case .removeCover: .trash
        case .removeSelected: .removeSelected
        case .removeItem, .removeReplacement: .remove
        case .removeToken: .removeCircle
        case .addStyleRule, .addToken: .add
        case .applyBulkEdit: .check
        case .capabilities: .capabilities
        case .more: .more
        }
    }
}

extension Image {
    init(folioSymbol: FolioSymbol) {
        self.init(systemName: folioSymbol.name)
    }
}
