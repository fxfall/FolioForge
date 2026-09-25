import Foundation

struct SemanticDocumentRow: Identifiable, Hashable {
    let id: UInt32
    let href: String
    let title: String
}

struct SemanticFontRow: Identifiable, Hashable {
    let id: String
    let family: String
    let style: String
    let mediaType: String
    let path: String
    let byteCount: UInt64?
    let usedBy: String

    var sizeDescription: String {
        guard let byteCount else { return "—" }
        return ByteCountFormatter.string(fromByteCount: Int64(clamping: byteCount), countStyle: .file)
    }
}

struct SemanticNavigationRow: Identifiable, Hashable {
    let id: String
    let href: String
    let label: String
    let depth: Int
}

struct SemanticBookSnapshot {
    let title: String
    let subtitle: String
    let authors: [String]
    let contributors: [String]
    let language: String
    let publisher: String
    let date: String
    let series: String
    let seriesIndex: String
    let description: String
    let subjects: [String]
    let identifiers: [String]
    let rights: String
    let documents: [SemanticDocumentRow]
    let fonts: [SemanticFontRow]
    let navigation: [SemanticNavigationRow]

    init(report: FolioInspectReport?, fallbackTitle: String) {
        let root = Self.object(report?.semantic)
        let metadata = Self.object(root?["metadata"])

        title = Self.string(metadata?["title"]) ?? fallbackTitle
        subtitle = Self.string(metadata?["subtitle"]) ?? ""
        authors = Self.unique(Self.strings(metadata?["authors"]) + Self.strings(metadata?["creators"]))
        contributors = Self.strings(metadata?["contributors"])
        language = Self.string(metadata?["language"]) ?? ""
        publisher = Self.string(metadata?["publisher"]) ?? ""
        date = Self.string(metadata?["date"]) ?? Self.strings(metadata?["dates"]).first ?? ""
        series = Self.string(metadata?["series"]) ?? ""
        seriesIndex = Self.string(metadata?["series_index"]) ?? ""
        description = Self.string(metadata?["description"]) ?? ""
        subjects = Self.strings(metadata?["subjects"])
        identifiers = Self.unique(Self.strings(metadata?["identifiers"]) + (Self.string(metadata?["identifier"]).map { [$0] } ?? []))
        rights = Self.string(metadata?["rights"]) ?? ""
        documents = Self.documentRows(root?["documents"])
        fonts = Self.fontRows(root)
        navigation = Self.navigationRows(root?["navigation"])
    }

    private static func object(_ value: AnyCodableJSON?) -> [String: AnyCodableJSON]? {
        guard case .object(let object) = value else { return nil }
        return object
    }

    private static func string(_ value: AnyCodableJSON?) -> String? {
        guard let value else { return nil }
        switch value {
        case .string(let string): return string
        case .number(let number): return number.rounded() == number ? String(Int64(number)) : String(number)
        case .bool(let boolean): return boolean ? "true" : "false"
        case .array, .object, .null: return nil
        }
    }

    private static func strings(_ value: AnyCodableJSON?) -> [String] {
        guard case .array(let values) = value else { return [] }
        return values.compactMap(string).filter { !$0.isEmpty }
    }

    private static func unique(_ values: [String]) -> [String] {
        var seen = Set<String>()
        return values.filter { seen.insert($0).inserted }
    }

    private static func documentRows(_ value: AnyCodableJSON?) -> [SemanticDocumentRow] {
        guard case .array(let values) = value else { return [] }
        return values.compactMap { value in
            guard let item = object(value),
                  let id = string(item["id"]).flatMap({ UInt32($0) }),
                  let href = string(item["href"])
            else { return nil }
            return SemanticDocumentRow(
                id: id,
                href: href,
                title: string(item["title"]) ?? URL(fileURLWithPath: href).deletingPathExtension().lastPathComponent
            )
        }
    }

    private static func fontRows(_ root: [String: AnyCodableJSON]?) -> [SemanticFontRow] {
        guard let root, case .array(let resources)? = root["resources"] else { return [] }
        let fontFaces: [[String: AnyCodableJSON]] = {
            guard case .array(let faces)? = root["font_faces"] else { return [] }
            return faces.compactMap(object)
        }()

        return resources.compactMap { value in
            guard let resource = object(value) else { return nil }
            let mediaType = string(resource["media_type"]) ?? ""
            let kind = string(resource["kind"]) ?? ""
            guard kind.caseInsensitiveCompare("font") == .orderedSame || mediaType.lowercased().contains("font") else {
                return nil
            }
            let id = string(resource["id"]) ?? string(resource["path"]) ?? mediaType
            let path = string(resource["path"]) ?? id
            let face = fontFaces.first { string($0["resource"]) == id }
            let family = string(face?["family"]) ?? URL(fileURLWithPath: path).deletingPathExtension().lastPathComponent
            let byteCount = string(resource["size"]).flatMap(UInt64.init)
            return SemanticFontRow(
                id: id,
                family: family.isEmpty ? FolioL10n.string("inspector.unknown_family", default: "Unknown family") : family,
                style: FolioL10n.string("inspector.not_declared", default: "Not declared"),
                mediaType: mediaType.isEmpty ? FolioL10n.string("inspector.unknown_format", default: "Unknown format") : mediaType,
                path: path,
                byteCount: byteCount,
                usedBy: face == nil
                    ? FolioL10n.string("inspector.not_mapped_in_source_ir", default: "Not mapped in source IR")
                    : FolioL10n.string("inspector.referenced_by_source_font_face", default: "Referenced by source font face")
            )
        }
    }

    private static func navigationRows(_ value: AnyCodableJSON?) -> [SemanticNavigationRow] {
        guard let navigation = object(value), case .array(let points)? = navigation["toc"] else { return [] }
        var rows: [SemanticNavigationRow] = []
        func append(_ values: [AnyCodableJSON], depth: Int) {
            guard depth <= 12 else { return }
            for pointValue in values {
                guard let point = object(pointValue),
                      let href = string(point["href"]),
                      let label = string(point["label"])
                else { continue }
                rows.append(SemanticNavigationRow(id: "\(href)#\(rows.count)", href: href, label: label, depth: depth))
                if case .array(let children)? = point["children"] { append(children, depth: depth + 1) }
            }
        }
        append(points, depth: 0)
        return rows
    }
}
