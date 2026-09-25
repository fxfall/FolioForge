import Foundation

private struct FolioComicResultC {
    var code: Int32
    var json: UnsafeMutablePointer<CChar>?
}

private typealias FolioComicProgressCallback = @convention(c) (
    UnsafePointer<CChar>?,
    UnsafeMutableRawPointer?
) -> Void

@_silgen_name("folio_comic_open_with_cancellation")
private func ffComicOpen(
    _ path: UnsafePointer<CChar>?,
    _ cancellation: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_comic_close")
private func ffComicClose(_ sessionID: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_comic_summary")
private func ffComicSummary(_ sessionID: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_comic_pages")
private func ffComicPages(_ sessionID: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_comic_page_info")
private func ffComicPageInfo(_ requestJSON: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_comic_conversion_options")
private func ffComicConversionOptions() -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_comic_thumbnail")
private func ffComicThumbnail(
    _ requestJSON: UnsafePointer<CChar>?,
    _ cancellation: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_comic_preview")
private func ffComicPreview(
    _ requestJSON: UnsafePointer<CChar>?,
    _ cancellation: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_comic_convert_with_progress")
private func ffComicConvert(
    _ requestJSON: UnsafePointer<CChar>?,
    _ cancellation: UnsafeMutableRawPointer?,
    _ callback: FolioComicProgressCallback?,
    _ userData: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_reader_open_comic")
private func ffReaderOpenComic(_ requestJSON: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_reader_close")
private func ffReaderClose(_ sessionID: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_reader_summary")
private func ffReaderSummary(_ sessionID: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_reader_current_page")
private func ffReaderCurrentPage(
    _ sessionID: UnsafePointer<CChar>?,
    _ cancellation: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_reader_current_spread")
private func ffReaderCurrentSpread(_ sessionID: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_reader_next")
private func ffReaderNext(_ sessionID: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_reader_previous")
private func ffReaderPrevious(_ sessionID: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_reader_go_to_document_index")
private func ffReaderGoToDocumentIndex(_ requestJSON: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_reader_set_viewport")
private func ffReaderSetViewport(_ requestJSON: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_reader_set_direction")
private func ffReaderSetDirection(_ requestJSON: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

@_silgen_name("folio_reader_set_spread_mode")
private func ffReaderSetSpreadMode(_ requestJSON: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?

private final class FolioComicProgressBox: @unchecked Sendable {
    let receive: @Sendable (FolioComicProgress) -> Void

    init(receive: @escaping @Sendable (FolioComicProgress) -> Void) {
        self.receive = receive
    }

    func decode(_ pointer: UnsafePointer<CChar>?) {
        guard let pointer,
              let data = String(cString: pointer).data(using: .utf8),
              let event = try? JSONDecoder().decode(FolioComicProgress.self, from: data)
        else { return }
        receive(event)
    }
}

private let folioComicProgressCallback: FolioComicProgressCallback = { event, userData in
    guard let userData else { return }
    Unmanaged<FolioComicProgressBox>.fromOpaque(userData).takeUnretainedValue().decode(event)
}

final class FolioComicCoreBridge: @unchecked Sendable {
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()

    init() {
        encoder.keyEncodingStrategy = .convertToSnakeCase
        decoder.keyDecodingStrategy = .convertFromSnakeCase
    }

    func open(path: URL, cancellation: FolioCancellationHandle) throws -> FolioComicSummary {
        let result = path.path.withCString { ffComicOpen($0, cancellation.rawPointer()) }
        return try decode(result)
    }

    func close(sessionID: String) throws {
        let result = sessionID.withCString { ffComicClose($0) }
        let _: ClosedResponse = try decode(result)
    }

    func summary(sessionID: String) throws -> FolioComicSummary {
        try decode(sessionID.withCString { ffComicSummary($0) })
    }

    func pages(sessionID: String) throws -> [FolioComicPage] {
        try decode(sessionID.withCString { ffComicPages($0) })
    }

    func pageInfo(sessionID: String, pageID: String) throws -> FolioComicPage {
        try perform(FolioComicPageRequest(sessionId: sessionID, pageId: pageID)) {
            ffComicPageInfo($0)
        }
    }

    func conversionOptions() throws -> FolioComicConversionOptions {
        try decode(ffComicConversionOptions())
    }

    func thumbnail(
        sessionID: String,
        pageID: String,
        cancellation: FolioCancellationHandle
    ) throws -> FolioComicImage {
        let request = FolioComicImageRequest(
            sessionId: sessionID,
            pageId: pageID,
            maxWidth: 192,
            maxHeight: 256,
            scale: 1
        )
        return try perform(request) {
            ffComicThumbnail($0, cancellation.rawPointer())
        }
    }

    func preview(
        sessionID: String,
        pageID: String,
        cancellation: FolioCancellationHandle
    ) throws -> FolioComicImage {
        let request = FolioComicImageRequest(
            sessionId: sessionID,
            pageId: pageID,
            maxWidth: 1_024,
            maxHeight: 1_536,
            scale: 1
        )
        return try perform(request) {
            ffComicPreview($0, cancellation.rawPointer())
        }
    }

    func convert(
        sessionID: String,
        target: String,
        output: URL,
        cancellation: FolioCancellationHandle,
        progress: @escaping @Sendable (FolioComicProgress) -> Void
    ) throws -> FolioComicConversionReport {
        let request = FolioComicConversionRequest(
            sessionId: sessionID,
            target: target,
            output: output.path
        )
        let json = try encodedJSON(request)
        let box = FolioComicProgressBox(receive: progress)
        let retainedBox = Unmanaged.passRetained(box)
        defer { retainedBox.release() }
        let result = json.withCString {
            ffComicConvert(
                $0,
                cancellation.rawPointer(),
                folioComicProgressCallback,
                retainedBox.toOpaque()
            )
        }
        return try decode(result)
    }

    func openReader(
        comicSessionID: String,
        viewport: FolioReaderViewport
    ) throws -> FolioReaderSessionSummary {
        let request = FolioReaderOpenComicRequest(
            comicSessionId: comicSessionID,
            viewport: viewport,
            direction: nil
        )
        return try performReader(request) { ffReaderOpenComic($0) }
    }

    func closeReader(sessionID: String) throws {
        let result = sessionID.withCString { ffReaderClose($0) }
        let _: ClosedResponse = try decodeReader(result)
    }

    func readerSummary(sessionID: String) throws -> FolioReaderSessionSummary {
        try decodeReader(sessionID.withCString { ffReaderSummary($0) })
    }

    func currentReaderPage(
        sessionID: String,
        cancellation: FolioCancellationHandle?
    ) throws -> FolioReaderPageModel {
        try decodeReader(sessionID.withCString {
            ffReaderCurrentPage($0, cancellation?.rawPointer())
        })
    }

    func currentReaderSpread(sessionID: String) throws -> FolioReaderSpreadModel {
        try decodeReader(sessionID.withCString { ffReaderCurrentSpread($0) })
    }

    func readerNext(sessionID: String) throws -> FolioReaderNavigationResult {
        try decodeReader(sessionID.withCString { ffReaderNext($0) })
    }

    func readerPrevious(sessionID: String) throws -> FolioReaderNavigationResult {
        try decodeReader(sessionID.withCString { ffReaderPrevious($0) })
    }

    func readerGoToDocumentIndex(sessionID: String, documentIndex: Int) throws {
        let _: FolioReaderLocation = try performReader(FolioReaderDocumentIndexRequest(
            sessionId: sessionID,
            documentIndex: documentIndex
        )) { ffReaderGoToDocumentIndex($0) }
    }

    func readerSetViewport(
        sessionID: String,
        viewport: FolioReaderViewport
    ) throws -> FolioReaderSessionSummary {
        try performReader(FolioReaderViewportRequest(
            sessionId: sessionID,
            viewport: viewport
        )) { ffReaderSetViewport($0) }
    }

    func readerSetDirection(
        sessionID: String,
        direction: FolioReaderDirection
    ) throws -> FolioReaderSessionSummary {
        try performReader(FolioReaderDirectionRequest(
            sessionId: sessionID,
            direction: direction
        )) { ffReaderSetDirection($0) }
    }

    func readerSetSpreadMode(
        sessionID: String,
        spreadMode: FolioReaderSpreadMode
    ) throws -> FolioReaderSessionSummary {
        try performReader(FolioReaderSpreadModeRequest(
            sessionId: sessionID,
            spreadMode: spreadMode
        )) { ffReaderSetSpreadMode($0) }
    }

    private func perform<Request: Encodable, Value: Decodable>(
        _ request: Request,
        operation: (UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?
    ) throws -> Value {
        let json = try encodedJSON(request)
        return try decode(json, operation: operation)
    }

    private func encodedJSON<Request: Encodable>(_ request: Request) throws -> String {
        do {
            let data = try encoder.encode(request)
            guard let json = String(data: data, encoding: .utf8) else { throw FolioError.invalidResponse }
            return json
        } catch let error as FolioError {
            throw error
        } catch {
            throw FolioError.encoding(error)
        }
    }

    private func decode<Value: Decodable>(_ result: UnsafeMutablePointer<FolioComicResultC>?) throws -> Value {
        let response = try responseData(result)
        do {
            return try decoder.decode(Value.self, from: response.data)
        } catch {
            throw FolioError.decoding(error)
        }
    }

    private func performReader<Request: Encodable, Value: Decodable>(
        _ request: Request,
        operation: (UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?
    ) throws -> Value {
        let json = try encodedJSON(request)
        return try decodeReader(json.withCString { operation($0) })
    }

    private func decodeReader<Value: Decodable>(
        _ result: UnsafeMutablePointer<FolioComicResultC>?
    ) throws -> Value {
        guard let result else { throw FolioError.invalidResponse }
        let code = result.pointee.code
        let json = result.pointee.json.map { String(cString: $0) }
        folioCoreFreeOpaqueResult(UnsafeMutableRawPointer(result))
        guard let data = json?.data(using: .utf8) else { throw FolioError.invalidResponse }
        if code != 0 {
            let response = try? decoder.decode(FolioReaderErrorResponse.self, from: data)
            throw FolioReaderBridgeError(
                code: response?.errorCode ?? "reader_error",
                message: response?.error ?? "Folio Reader failed with code \(code)."
            )
        }
        do {
            return try decoder.decode(Value.self, from: data)
        } catch {
            throw FolioError.decoding(error)
        }
    }

    private func decode<Value: Decodable>(
        _ json: String,
        operation: (UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioComicResultC>?
    ) throws -> Value {
        try decode(json.withCString { operation($0) })
    }

    private func responseData(_ result: UnsafeMutablePointer<FolioComicResultC>?) throws -> (data: Data, code: Int32) {
        guard let result else { throw FolioError.invalidResponse }
        let code = result.pointee.code
        let json = result.pointee.json.map { String(cString: $0) }
        folioCoreFreeOpaqueResult(UnsafeMutableRawPointer(result))
        guard let data = json?.data(using: .utf8) else { throw FolioError.invalidResponse }
        if code != 0 {
            let message = (try? decoder.decode(FolioComicErrorResponse.self, from: data).error)
                ?? "Folio Core failed with code \(code)."
            throw FolioError.core(message)
        }
        return (data, code)
    }
}

private struct FolioComicErrorResponse: Decodable {
    let error: String
}

private struct ClosedResponse: Decodable {
    let closed: Bool
}

private struct FolioReaderErrorResponse: Decodable {
    let errorCode: String?
    let error: String?
}
