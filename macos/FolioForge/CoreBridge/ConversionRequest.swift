import Foundation

struct FolioTarget: Codable, Identifiable, Hashable, Sendable {
    let rawValue: String
    let canonicalName: String?
    let extensions: [String]

    var id: String { rawValue }

    var displayName: String {
        let label = canonicalName ?? rawValue
        guard let fileExtension = extensions.first, !fileExtension.isEmpty else {
            return label
        }
        return "\(label) (.\(fileExtension))"
    }

    init(rawValue: String, canonicalName: String? = nil, extensions: [String] = []) {
        self.rawValue = rawValue
        self.canonicalName = canonicalName
        self.extensions = extensions
    }

    static let epub = Self(rawValue: "EPUB", canonicalName: "EPUB", extensions: ["epub"])
    static let kf7 = Self(rawValue: "KF7", canonicalName: "KF7", extensions: ["mobi"])
    static let kf8 = Self(rawValue: "KF8", canonicalName: "KF8", extensions: ["azw3"])
    static let combo = Self(rawValue: "KF7KF8Combo", canonicalName: "KF7KF8Combo", extensions: ["mobi"])
    static let kfx = Self(rawValue: "KFX", canonicalName: "KFX", extensions: ["kfx"])

    init(from decoder: Decoder) throws {
        if let single = try? decoder.singleValueContainer().decode(String.self) {
            self.init(rawValue: single)
            return
        }
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            rawValue: try container.decode(String.self, forKey: .id),
            canonicalName: try container.decodeIfPresent(String.self, forKey: .canonicalName),
            extensions: try container.decodeIfPresent([String].self, forKey: .extensions) ?? []
        )
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }

    private enum CodingKeys: String, CodingKey {
        case id
        case canonicalName = "canonical_name"
        case extensions
    }
}

enum FolioDegradationMode: String, CaseIterable, Codable, Identifiable, Hashable, Sendable {
    case strict = "Strict"
    case compatible = "Compatible"
    case readable = "Readable"

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .strict: FolioL10n.string("mode.strict", default: "Strict")
        case .compatible: FolioL10n.string("mode.compatible", default: "Compatible")
        case .readable: FolioL10n.string("mode.readable", default: "Readable")
        }
    }
}

enum FolioBatchMode: String, CaseIterable, Codable, Identifiable, Hashable, Sendable {
    case bestEffort = "BestEffort"
    case strict = "Strict"

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .bestEffort: FolioL10n.string("batch.best_effort", default: "Best effort")
        case .strict: FolioL10n.string("batch.strict", default: "Strict batch")
        }
    }
}

enum FolioPreviewDevice: String, CaseIterable, Codable, Identifiable, Hashable, Sendable {
    case eReader = "e_reader"
    case phone
    case tablet

    var id: String { rawValue }

    var title: String {
        switch self {
        case .eReader: FolioL10n.string("preview.device.ereader", default: "E-reader")
        case .phone: FolioL10n.string("preview.device.phone", default: "Phone")
        case .tablet: FolioL10n.string("preview.device.tablet", default: "Tablet")
        }
    }
}

enum FolioPreviewOrientation: String, CaseIterable, Codable, Identifiable, Hashable, Sendable {
    case portrait
    case landscape

    var id: String { rawValue }
    var title: String {
        switch self {
        case .portrait: FolioL10n.string("preview.orientation.portrait", default: "Portrait")
        case .landscape: FolioL10n.string("preview.orientation.landscape", default: "Landscape")
        }
    }
}

struct FolioPreviewSettings: Codable, Hashable, Sendable {
    var device: FolioPreviewDevice = .eReader
    var orientation: FolioPreviewOrientation = .portrait
    var fontSizePercent = 100

    enum CodingKeys: String, CodingKey {
        case device
        case orientation
        case fontSizePercent = "font_size_percent"
    }
}

struct FolioDegradationOptions: Codable, Hashable, Sendable {
    var linearizeComplexTables = false
    var preferRasterization = false
    var stripEmbeddedFonts = false

    enum CodingKeys: String, CodingKey {
        case linearizeComplexTables = "linearize_complex_tables"
        case preferRasterization = "prefer_rasterization"
        case stripEmbeddedFonts = "strip_embedded_fonts"
    }
}

enum FolioTextImportMode: String, CaseIterable, Codable, Identifiable, Hashable, Sendable {
    case auto = "auto"
    case novel = "novel"
    case markdown = "markdown"
    case plain = "plain"
    var id: String { rawValue }
    var title: String {
        switch self {
        case .auto: FolioL10n.string("text.mode.auto", default: "Auto")
        case .novel: FolioL10n.string("text.mode.novel", default: "Novel")
        case .markdown: FolioL10n.string("text.mode.markdown", default: "Markdown")
        case .plain: FolioL10n.string("text.mode.plain", default: "Plain")
        }
    }
}

enum FolioParagraphMode: String, CaseIterable, Codable, Identifiable, Hashable, Sendable {
    case auto = "auto"
    case blankLine = "blank_line"
    case everyLine = "every_line"
    case indented = "indented"
    case hardWrap = "hard_wrap"
    var id: String { rawValue }
    var title: String {
        switch self {
        case .auto: FolioL10n.string("ui.automatic", default: "Automatic")
        case .blankLine: FolioL10n.string("paragraph.blank_line", default: "Blank lines")
        case .everyLine: FolioL10n.string("paragraph.every_line", default: "Every line")
        case .indented: FolioL10n.string("paragraph.indented", default: "Indented paragraphs")
        case .hardWrap: FolioL10n.string("paragraph.hard_wrap", default: "Hard-wrapped text")
        }
    }
}

enum FolioTextEncoding: String, CaseIterable, Codable, Identifiable, Hashable, Sendable {
    case utf8 = "utf8"
    case utf16Le = "utf16_le"
    case utf16Be = "utf16_be"
    case gb18030 = "gb18030"
    case big5 = "big5"
    case shiftJis = "shift_jis"
    case windows1252 = "windows_1252"
    var id: String { rawValue }
    var title: String {
        switch self {
        case .utf8: "UTF-8"
        case .utf16Le: "UTF-16LE"
        case .utf16Be: "UTF-16BE"
        case .gb18030: "GB18030"
        case .big5: "Big5"
        case .shiftJis: "Shift_JIS"
        case .windows1252: "Windows-1252"
        }
    }
}

struct FolioTextImportOptions: Codable, Hashable, Sendable {
    var mode: FolioTextImportMode = .auto
    var paragraphMode: FolioParagraphMode = .auto
    var encodingOverride: FolioTextEncoding?
    var titleOverride: String?
    var authorOverride: String?

    enum CodingKeys: String, CodingKey {
        case mode
        case paragraphMode = "paragraph_mode"
        case encodingOverride = "encoding_override"
        case titleOverride = "title_override"
        case authorOverride = "author_override"
    }
}

enum FolioCompression: String, CaseIterable, Codable, Identifiable, Sendable {
    case none = "None"
    case palmDoc = "PalmDoc"

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .none: FolioL10n.string("compression.none", default: "None")
        case .palmDoc: "PalmDOC"
        }
    }
}

struct FolioConversionOptions: Encodable, Sendable {
    let deterministic: Bool
    let compression: FolioCompression
    let degradationMode: FolioDegradationMode
    let degradation: FolioDegradationOptions
    let text: FolioTextImportOptions

    enum CodingKeys: String, CodingKey {
        case deterministic
        case compression
        case degradationMode = "degradation_mode"
        case degradation
        case text
    }
}

struct FolioBatchInput: Encodable, Sendable {
    let source: String
    let relativePath: String?
    let outputRoot: String?
    let edit: FolioBookEditPlan?

    enum CodingKeys: String, CodingKey {
        case source
        case relativePath = "relative_path"
        case outputRoot = "output_root"
        case edit
    }
}

struct FolioBatchOptions: Encodable, Sendable {
    let target: FolioTarget
    let conversion: FolioConversionOptions
    let batchMode: FolioBatchMode
    let collisionPolicy: String
    let preserveTree: Bool
    let edit: FolioBookEditPlan

    enum CodingKeys: String, CodingKey {
        case target
        case conversion
        case batchMode = "batch_mode"
        case collisionPolicy = "collision_policy"
        case preserveTree = "preserve_tree"
        case edit
    }
}

struct FolioBatchConversionRequest: Encodable, Sendable {
    let inputs: [FolioBatchInput]
    let outputDirectory: String
    let options: FolioBatchOptions

    enum CodingKeys: String, CodingKey {
        case inputs
        case outputDirectory = "output_dir"
        case options
    }
}

struct FolioDiscoveredBook: Decodable, Sendable {
    let source: String
    let relativePath: String?

    enum CodingKeys: String, CodingKey {
        case source
        case relativePath = "relative_path"
    }
}

struct FolioMetadataEdit: Codable, Hashable, Sendable {
    var title: String? = nil
    var subtitle: String? = nil
    var authors: [String]? = nil
    var contributors: [String]? = nil
    var language: String? = nil
    var publisher: String? = nil
    var date: String? = nil
    var series: String? = nil
    var seriesIndex: Double? = nil
    var description: String? = nil
    var subjects: [String]? = nil
    var identifiers: [String]? = nil
    var rights: String? = nil
    var clearFields: [String] = []

    enum CodingKeys: String, CodingKey {
        case title, subtitle, authors, contributors, language, publisher, date, series
        case seriesIndex = "series_index"
        case description, subjects, identifiers, rights
        case clearFields = "clear_fields"
    }
}

struct FolioOnlineMetadata: Codable, Hashable, Sendable {
    var title: String? = nil
    var subtitle: String? = nil
    var language: String? = nil
    var authors: [String] = []
    var creators: [String] = []
    var contributors: [String] = []
    var publisher: String? = nil
    var date: String? = nil
    var dates: [String] = []
    var series: String? = nil
    var seriesIndex: Double? = nil
    var identifier: String? = nil
    var identifiers: [String] = []
    var description: String? = nil
    var subjects: [String] = []
    var rights: String? = nil

    enum CodingKeys: String, CodingKey {
        case title, subtitle, language, authors, creators, contributors, publisher
        case date, dates, series, description, subjects, rights, identifier, identifiers
        case seriesIndex = "series_index"
    }

    var authorNames: [String] {
        Self.unique(authors + creators)
    }

    var identifierValues: [String] {
        Self.unique(identifiers + (identifier.map { [$0] } ?? []))
    }

    private static func unique(_ values: [String]) -> [String] {
        var seen = Set<String>()
        return values.filter { !$0.isEmpty && seen.insert($0).inserted }
    }
}

struct FolioOnlineMetadataCandidate: Codable, Hashable, Sendable, Identifiable {
    let candidateID: String
    let provider: String
    let confidence: Double
    let metadata: FolioOnlineMetadata
    let sourceURL: String?

    var id: String { candidateID }

    enum CodingKeys: String, CodingKey {
        case candidateID = "candidate_id"
        case provider, confidence, metadata
        case sourceURL = "source_url"
    }
}

struct FolioOnlineProviderRequest: Encodable, Sendable {
    var query: String
    var locale: String?
    var isbn: String?
    var identifier: String?
    var title: String?
    var author: String?
    var maxResults: Int?

    enum CodingKeys: String, CodingKey {
        case query, locale, isbn, identifier, title, author
        case maxResults = "max_results"
    }
}

struct FolioOnlineMetadataSearchRequest: Encodable, Sendable {
    let enabled: Bool
    let request: FolioOnlineProviderRequest
}

enum FolioMetadataMergeAction: String, Codable, CaseIterable, Identifiable, Sendable {
    case keep
    case replace
    case append

    var id: String { rawValue }

    var title: String {
        switch self {
        case .keep: FolioL10n.string("merge.keep", default: "Keep current")
        case .replace: FolioL10n.string("merge.replace", default: "Replace")
        case .append: FolioL10n.string("merge.append", default: "Append")
        }
    }
}

struct FolioOnlineMetadataMergeRequest: Encodable, Sendable {
    let current: FolioOnlineMetadata
    let candidate: FolioOnlineMetadataCandidate
    let fields: [String: FolioMetadataMergeAction]
    let confirmed: Bool
}

struct FolioTypographyEdit: Encodable, Sendable {
    var fontFamily: String? = nil
    var bodyFontSize: String? = nil
    var lineHeight: String? = nil
    var letterSpacing: String? = nil

    enum CodingKeys: String, CodingKey {
        case fontFamily = "font_family"
        case bodyFontSize = "body_font_size"
        case lineHeight = "line_height"
        case letterSpacing = "letter_spacing"
    }
}

struct FolioFontEdit: Encodable, Sendable {
    var stripEmbeddedFonts = false
    var preferredFamily: String? = nil
    var replacement: FolioFontReplacement? = nil

    enum CodingKeys: String, CodingKey {
        case stripEmbeddedFonts = "strip_embedded_fonts"
        case preferredFamily = "preferred_family"
        case replacement
    }
}

struct FolioFontReplacement: Encodable, Sendable {
    let fileName: String
    let mediaType: String
    let bytes: [UInt8]
    let family: String?

    enum CodingKeys: String, CodingKey {
        case fileName = "file_name"
        case mediaType = "media_type"
        case bytes, family
    }
}

struct FolioStyleEdit: Encodable, Sendable {
    var nodeID: UInt32? = nil
    var role: String? = nil
    var properties: [String: String] = [:]
    var css: String? = nil

    enum CodingKeys: String, CodingKey {
        case nodeID = "node_id"
        case role, properties, css
    }
}

struct FolioStructureEdit: Encodable, Sendable {
    var documentOrder: [UInt32]? = nil
    var documentTitles: [String: String] = [:]
    var tocLabels: [String: String] = [:]
    var removeNavigation = false

    enum CodingKeys: String, CodingKey {
        case documentOrder = "document_order"
        case documentTitles = "document_titles"
        case tocLabels = "toc_labels"
        case removeNavigation = "remove_navigation"
    }
}

enum FolioCoverFit: String, CaseIterable, Hashable, Identifiable, Encodable, Sendable {
    case fill = "Fill"
    case fit = "Fit"

    var id: String { rawValue }
}

enum FolioCoverEdit: Encodable, Sendable {
    case keep
    case remove
    case replace(fileName: String, mediaType: String, bytes: [UInt8], fit: FolioCoverFit)

    private enum VariantKey: String, CodingKey {
        case replace = "Replace"
    }

    private struct Replacement: Encodable {
        let fileName: String
        let mediaType: String
        let bytes: [UInt8]
        let fit: FolioCoverFit

        enum CodingKeys: String, CodingKey {
            case fileName = "file_name"
            case mediaType = "media_type"
            case bytes
            case fit
        }
    }

    func encode(to encoder: Encoder) throws {
        switch self {
        case .keep:
            var container = encoder.singleValueContainer()
            try container.encode("Keep")
        case .remove:
            var container = encoder.singleValueContainer()
            try container.encode("Remove")
        case let .replace(fileName, mediaType, bytes, fit):
            var container = encoder.container(keyedBy: VariantKey.self)
            try container.encode(
                Replacement(fileName: fileName, mediaType: mediaType, bytes: bytes, fit: fit),
                forKey: .replace
            )
        }
    }
}

struct FolioBookEditPlan: Encodable, Sendable {
    var metadata = FolioMetadataEdit()
    var cover: FolioCoverEdit = .keep
    var typography = FolioTypographyEdit()
    var fonts = FolioFontEdit()
    var styles: [FolioStyleEdit] = []
    var structure = FolioStructureEdit()

    init(title: String? = nil, css: String? = nil) {
        metadata.title = title
        if let css, !css.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            styles = [FolioStyleEdit(css: css)]
        }
    }
}

struct FolioAnalysisRequest: Encodable, Sendable {
    let input: String
    let target: FolioTarget
    let mode: FolioDegradationMode
    let degradation: FolioDegradationOptions
    let edit: FolioBookEditPlan
    let text: FolioTextImportOptions

    enum CodingKeys: String, CodingKey {
        case input
        case target
        case mode
        case degradation
        case edit
        case text
    }
}
