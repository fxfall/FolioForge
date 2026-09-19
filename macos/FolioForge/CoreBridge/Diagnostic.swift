import Foundation

enum FolioSeverity: String, Codable {
    case info = "Info"
    case warning = "Warning"
    case error = "Error"

    var label: String { rawValue }
}

struct FolioDiagnosticSource: Codable {
    let path: String?
    let line: UInt64?
    let column: UInt64?
}

struct FolioDiagnostic: Codable, Identifiable {
    let severity: FolioSeverity
    let code: String
    let message: String
    let source: FolioDiagnosticSource?
    let context: [String: String]

    var id: String { "\(code):\(message)" }
}
