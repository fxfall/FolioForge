import Foundation

enum FolioSeverity: String, Codable, Sendable {
    case info = "Info"
    case warning = "Warning"
    case error = "Error"

    var label: String {
        switch self {
        case .info: FolioL10n.string("diagnostic.severity.info", default: "Info")
        case .warning: FolioL10n.string("diagnostic.severity.warning", default: "Warning")
        case .error: FolioL10n.string("diagnostic.severity.error", default: "Error")
        }
    }
}

struct FolioDiagnosticSource: Codable, Sendable {
    let path: String?
    let line: UInt64?
    let column: UInt64?
}

struct FolioDiagnostic: Codable, Identifiable, Sendable {
    let severity: FolioSeverity
    let code: String
    let message: String
    let source: FolioDiagnosticSource?
    let context: [String: String]

    var id: String { "\(code):\(message)" }
}
