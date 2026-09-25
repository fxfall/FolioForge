import Foundation

struct FolioReaderPreviewDocument: Codable, Identifiable, Sendable {
    let href: String
    let title: String?
    let html: String

    var id: String { href }
}

enum FolioReaderPreviewMode: String, Encodable, Decodable, Sendable {
    case sourceSemantic = "source_semantic"
    case editedSemantic = "edited_semantic"
    case target
}

struct FolioReaderPreviewRequest: Encodable, Sendable {
    let input: String
    let mode: FolioReaderPreviewMode
    let target: FolioTarget
    let degradationMode: FolioDegradationMode
    let degradation: FolioDegradationOptions
    let edit: FolioBookEditPlan
    let settings: FolioPreviewSettings
    let text: FolioTextImportOptions

    enum CodingKeys: String, CodingKey {
        case input
        case mode
        case target
        case degradationMode = "degradation_mode"
        case degradation
        case edit
        case settings
        case text
    }
}

enum FolioReaderPreviewContent: Decodable, Sendable {
    case reflowable(documents: [FolioReaderPreviewDocument], html: String)
    case fixedPage(FolioReaderPageModel)

    private enum CodingKeys: String, CodingKey {
        case kind
        case documents
        case html
        case page
    }

    private enum Kind: String, Decodable {
        case reflowable
        case fixedPage = "fixed_page"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(Kind.self, forKey: .kind) {
        case .reflowable:
            self = .reflowable(
                documents: try container.decode([FolioReaderPreviewDocument].self, forKey: .documents),
                html: try container.decode(String.self, forKey: .html)
            )
        case .fixedPage:
            self = .fixedPage(try container.decode(FolioReaderPageModel.self, forKey: .page))
        }
    }
}

struct FolioReaderPreviewBundle: Decodable, Sendable {
    let mode: FolioReaderPreviewMode
    let location: FolioReaderLocation?
    let viewport: FolioReaderSize
    let sourceTitle: String
    let content: FolioReaderPreviewContent
    let target: FolioPreviewTargetProfile?
    let degradation: FolioDegradationReport?
    let inputLoss: [String]
    let targetLoss: [String]
    let blocked: Bool
}
