import Foundation

struct FolioComicWarning: Decodable, Identifiable, Sendable {
    let code: String
    let severity: String
    let message: String

    var id: String { code + message }
}

struct FolioComicSummary: Decodable, Sendable {
    let sessionId: String
    let title: String
    let authors: [String]
    let pageCount: Int
    let sourceType: String
    let readingDirection: String?
    let dimensionsSummary: String?
    let warnings: [FolioComicWarning]
}

struct FolioComicPage: Decodable, Identifiable, Hashable, Sendable {
    let pageId: String
    let displayIndex: Int
    let name: String
    let sourceFormat: String
    let width: Int?
    let height: Int?
    let orientation: String?
    let spreadState: String?
    let colorState: String?
    let warningFlags: [String]

    var id: String { pageId }
}

struct FolioComicImage: Decodable, Sendable {
    let dataBase64: String
    let width: Int
    let height: Int
    let sourceWidth: Int
    let sourceHeight: Int
    let mimeType: String
    let cacheIdentity: String

    var data: Data? { Data(base64Encoded: dataBase64) }
}

struct FolioComicTarget: Decodable, Identifiable, Hashable, Sendable {
    let id: String
    let displayName: String
    let fileExtension: String
}

struct FolioComicConversionOptions: Decodable, Sendable {
    let targets: [FolioComicTarget]
    let deviceProfiles: [String]
    let notes: [String]
}

struct FolioComicPageRequest: Encodable, Sendable {
    let sessionId: String
    let pageId: String
}

struct FolioComicImageRequest: Encodable, Sendable {
    let sessionId: String
    let pageId: String
    let maxWidth: UInt32
    let maxHeight: UInt32
    let scale: Float
}

struct FolioComicConversionRequest: Encodable, Sendable {
    let sessionId: String
    let target: String
    let output: String
}

struct FolioComicConversionReport: Decodable, Sendable {
    let outputPath: String
    let outputSize: UInt64
    let pageCount: Int
    let target: String
    let durationMs: UInt64
    let warnings: [FolioComicWarning]
}

struct FolioComicProgress: Decodable, Sendable {
    let stage: FolioProgressStage
    let current: UInt64
    let total: UInt64?
    let fraction: Float?
    let message: String

    var label: String { stage.label }
}

enum FolioReaderDirection: String, Codable, CaseIterable, Identifiable, Sendable {
    case leftToRight = "left_to_right"
    case rightToLeft = "right_to_left"

    var id: String { rawValue }
    var label: String {
        self == .leftToRight
            ? FolioL10n.string("reader.direction.ltr", default: "Left to right")
            : FolioL10n.string("reader.direction.rtl", default: "Right to left")
    }
}

enum FolioReaderSpreadMode: String, Codable, CaseIterable, Identifiable, Sendable {
    case singlePage = "single_page"
    case synthetic

    var id: String { rawValue }
    var label: String {
        self == .singlePage
            ? FolioL10n.string("reader.spread.single", default: "Single page")
            : FolioL10n.string("reader.spread.two_page", default: "Two-page spread")
    }
}

enum FolioReaderPageSide: String, Decodable, Sendable {
    case left
    case right
    case center
}

enum FolioReaderSpreadHint: String, Decodable, Sendable {
    case automatic
    case singlePage = "single_page"
    case pair
}

enum FolioReaderContentMode: String, Codable, CaseIterable, Identifiable, Sendable {
    case fit
    case fill
    case actualSize = "actual_size"

    var id: String { rawValue }
    var label: String {
        switch self {
        case .fit: FolioL10n.string("reader.fit.fit", default: "Fit")
        case .fill: FolioL10n.string("reader.fit.fill", default: "Fill")
        case .actualSize: FolioL10n.string("reader.fit.actual_size", default: "Actual size")
        }
    }
}

struct FolioReaderViewport: Codable, Equatable, Sendable {
    var width: Float
    var height: Float
    var scale: Float
    var contentMode: FolioReaderContentMode
    var panX: Float = 0
    var panY: Float = 0
}

struct FolioReaderLocation: Decodable, Equatable, Sendable {
    let documentId: UInt32
    let nodeId: UInt32?
    let textOffset: UInt64?
    let semanticContext: String?
}

struct FolioReaderSessionSummary: Decodable, Sendable {
    let sessionId: String
    let direction: FolioReaderDirection
    let spreadMode: FolioReaderSpreadMode
    let currentLocation: FolioReaderLocation
    let currentDocumentIndex: Int
    let currentIndex: Int
    let pageCount: Int
    let viewport: FolioReaderViewport
}

struct FolioReaderRect: Decodable, Sendable {
    let x: Float
    let y: Float
    let width: Float
    let height: Float
}

struct FolioReaderSize: Decodable, Sendable {
    let width: Float
    let height: Float
}

struct FolioReaderResource: Decodable, Sendable {
    let resourceId: UInt32
    let mediaType: String
    let data: Data

    private enum CodingKeys: String, CodingKey {
        case resourceId
        case mediaType
        case dataBase64
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        resourceId = try values.decode(UInt32.self, forKey: .resourceId)
        mediaType = try values.decode(String.self, forKey: .mediaType)
        let encoded = try values.decode(String.self, forKey: .dataBase64)
        guard let bytes = Data(base64Encoded: encoded) else {
            throw DecodingError.dataCorruptedError(
                forKey: .dataBase64,
                in: values,
                debugDescription: "Reader resource is not valid base64 data."
            )
        }
        data = bytes
    }
}

struct FolioReaderPlacement: Decodable, Sendable {
    let resource: FolioReaderResource
    let destination: FolioReaderRect
    let clippingRect: FolioReaderRect
    let clipsToViewport: Bool
    let altText: String
}

struct FolioReaderPageModel: Decodable, Sendable {
    let location: FolioReaderLocation
    let pageIndex: Int
    let documentIndex: Int
    let pageCount: Int
    let pageSize: FolioReaderSize
    let viewportSize: FolioReaderSize
    let placements: [FolioReaderPlacement]
}

struct FolioReaderSpreadPage: Decodable, Sendable {
    let location: FolioReaderLocation
    let pageIndex: Int
    let pageSide: FolioReaderPageSide
    let spreadHint: FolioReaderSpreadHint
    let pageSize: FolioReaderSize
    let viewportRect: FolioReaderRect
    let placements: [FolioReaderPlacement]
}

struct FolioReaderSpreadModel: Decodable, Sendable {
    let currentLocation: FolioReaderLocation
    let pageCount: Int
    let direction: FolioReaderDirection
    let viewportSize: FolioReaderSize
    let pages: [FolioReaderSpreadPage]
}

struct FolioReaderOpenComicRequest: Encodable, Sendable {
    let comicSessionId: String
    let viewport: FolioReaderViewport
    let direction: FolioReaderDirection?
}

struct FolioReaderSessionRequest: Encodable, Sendable {
    let sessionId: String
}

struct FolioReaderDocumentIndexRequest: Encodable, Sendable {
    let sessionId: String
    let documentIndex: Int
}

struct FolioReaderViewportRequest: Encodable, Sendable {
    let sessionId: String
    let viewport: FolioReaderViewport
}

struct FolioReaderDirectionRequest: Encodable, Sendable {
    let sessionId: String
    let direction: FolioReaderDirection
}

struct FolioReaderSpreadModeRequest: Encodable, Sendable {
    let sessionId: String
    let spreadMode: FolioReaderSpreadMode
}

struct FolioReaderNavigationResult: Decodable, Sendable {
    let location: FolioReaderLocation
    let moved: Bool
}
