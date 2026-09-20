import Foundation

enum FolioError: LocalizedError, @unchecked Sendable {
    case core(String)
    case invalidResponse
    case encoding(Error)
    case decoding(Error)

    var errorDescription: String? {
        switch self {
        case .core(let message): message
        case .invalidResponse: "Rust Core returned an invalid response."
        case .encoding(let error): "Could not encode the conversion request: \(error.localizedDescription)"
        case .decoding(let error): "Could not decode the Core response: \(error.localizedDescription)"
        }
    }
}
