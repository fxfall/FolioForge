import Foundation

enum FolioError: LocalizedError, @unchecked Sendable {
    case core(String)
    case invalidResponse
    case encoding(Error)
    case decoding(Error)

    var errorDescription: String? {
        switch self {
        case .core(let message): FolioL10n.format("error.core", default: "Core error: %@", message)
        case .invalidResponse: FolioL10n.string("error.invalid_response", default: "Rust Core returned an invalid response.")
        case .encoding(let error): FolioL10n.format("error.encode_request", default: "Could not encode the conversion request: %@", error.localizedDescription)
        case .decoding(let error): FolioL10n.format("error.decode_response", default: "Could not decode the Core response: %@", error.localizedDescription)
        }
    }
}

struct FolioReaderBridgeError: LocalizedError, Sendable {
    let code: String
    let message: String

    var errorDescription: String? {
        FolioL10n.format("error.reader", default: "Reader error: %@", message)
    }
}
