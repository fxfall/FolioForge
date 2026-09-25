import Foundation

private struct FolioResultC {
    var code: Int32
    var json: UnsafeMutablePointer<CChar>?
}

private typealias FolioProgressCallback = @convention(c) (
    UnsafePointer<CChar>?,
    UnsafeMutableRawPointer?
) -> Void

@_silgen_name("folio_version")
private func ffVersion() -> UnsafePointer<CChar>?

@_silgen_name("folio_capabilities")
private func ffCapabilities() -> UnsafeMutablePointer<CChar>?

@_silgen_name("folio_analyze")
private func ffAnalyze(_ requestJSON: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioResultC>?

@_silgen_name("folio_reader_preview")
private func ffReaderPreview(_ requestJSON: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioResultC>?

@_silgen_name("folio_online_metadata_search")
private func ffOnlineMetadataSearch(_ requestJSON: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioResultC>?

@_silgen_name("folio_online_metadata_merge_plan")
private func ffOnlineMetadataMergePlan(_ requestJSON: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioResultC>?

@_silgen_name("folio_batch_list_directory")
private func ffBatchListDirectory(_ path: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioResultC>?

@_silgen_name("folio_batch_convert_with_progress")
private func ffBatchConvertWithProgress(
    _ requestJSON: UnsafePointer<CChar>?,
    _ cancellation: UnsafeMutableRawPointer?,
    _ callback: FolioProgressCallback,
    _ userData: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<FolioResultC>?

@_silgen_name("folio_inspect")
private func ffInspect(_ path: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioResultC>?

@_silgen_name("folio_validate")
private func ffValidate(_ path: UnsafePointer<CChar>?) -> UnsafeMutablePointer<FolioResultC>?

@_silgen_name("folio_cancellation_new")
private func ffCancellationNew() -> UnsafeMutableRawPointer?

@_silgen_name("folio_cancellation_cancel")
private func ffCancellationCancel(_ cancellation: UnsafeMutableRawPointer?)

@_silgen_name("folio_cancellation_free")
private func ffCancellationFree(_ cancellation: UnsafeMutableRawPointer?)

@_silgen_name("folio_result_free")
private func ffResultFree(_ result: UnsafeMutablePointer<FolioResultC>?)

@_silgen_name("folio_string_free")
private func ffStringFree(_ value: UnsafeMutablePointer<CChar>?)

private final class BatchProgressBox {
    let handler: (FolioBatchProgressEvent) -> Void

    init(handler: @escaping (FolioBatchProgressEvent) -> Void) {
        self.handler = handler
    }

    func receive(_ pointer: UnsafePointer<CChar>?) {
        guard let pointer else { return }
        let json = String(cString: pointer)
        guard let data = json.data(using: .utf8),
              let event = try? JSONDecoder().decode(FolioBatchProgressEvent.self, from: data)
        else { return }
        handler(event)
    }
}

private let folioBatchProgressCallback: FolioProgressCallback = { event, userData in
    guard let userData else { return }
    let box = Unmanaged<BatchProgressBox>.fromOpaque(userData).takeUnretainedValue()
    box.receive(event)
}

private final class BatchCompletionBox: @unchecked Sendable {
    let handler: (Result<FolioBatchReport, FolioError>) -> Void

    init(handler: @escaping (Result<FolioBatchReport, FolioError>) -> Void) {
        self.handler = handler
    }

    func complete(_ result: Result<FolioBatchReport, FolioError>) {
        handler(result)
    }
}

final class FolioCancellationHandle: @unchecked Sendable {
    private let lock = NSLock()
    private var pointer: UnsafeMutableRawPointer?

    init(pointer: UnsafeMutableRawPointer?) {
        self.pointer = pointer
    }

    static func make() -> FolioCancellationHandle {
        FolioCancellationHandle(pointer: ffCancellationNew())
    }

    func cancel() {
        lock.lock()
        defer { lock.unlock() }
        ffCancellationCancel(pointer)
    }

    func finish() {
        lock.lock()
        defer { lock.unlock() }
        guard let pointer else { return }
        self.pointer = nil
        ffCancellationFree(pointer)
    }

    func rawPointer() -> UnsafeMutableRawPointer? {
        lock.lock()
        defer { lock.unlock() }
        return pointer
    }

    deinit {
        finish()
    }
}

private final class BatchConversionWork: @unchecked Sendable {
    let requestJSON: String
    let cancellation: FolioCancellationHandle
    let retainedProgress: Unmanaged<BatchProgressBox>
    let completion: BatchCompletionBox

    init(
        requestJSON: String,
        cancellation: FolioCancellationHandle,
        retainedProgress: Unmanaged<BatchProgressBox>,
        completion: BatchCompletionBox
    ) {
        self.requestJSON = requestJSON
        self.cancellation = cancellation
        self.retainedProgress = retainedProgress
        self.completion = completion
    }

    func run() {
        let result = requestJSON.withCString { pointer in
            ffBatchConvertWithProgress(
                pointer,
                cancellation.rawPointer(),
                folioBatchProgressCallback,
                retainedProgress.toOpaque()
            )
        }
        let response = decodeBatchConversionResult(result)
        cancellation.finish()
        retainedProgress.release()
        DispatchQueue.main.async {
            self.completion.complete(response)
        }
    }
}

final class FolioCoreBridge: @unchecked Sendable {
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()

    var version: String {
        guard let pointer = ffVersion() else { return "unknown" }
        return String(cString: pointer)
    }

    func capabilities() throws -> [String: AnyCodableJSON] {
        guard let pointer = ffCapabilities() else { throw FolioError.invalidResponse }
        defer { ffStringFree(pointer) }
        let json = String(cString: pointer)
        guard let data = json.data(using: .utf8) else { throw FolioError.invalidResponse }
        do {
            return try decoder.decode([String: AnyCodableJSON].self, from: data)
        } catch {
            throw FolioError.decoding(error)
        }
    }

    func listDirectory(url: URL) throws -> [FolioDiscoveredBook] {
        let result = url.path.withCString { ffBatchListDirectory($0) }
        let (code, data) = responseData(result)
        guard let data else { throw FolioError.invalidResponse }
        if code != 0 {
            let message = (try? JSONDecoder().decode(CoreErrorResponse.self, from: data))?.error
                ?? "Folio Core failed with code \(code)."
            throw FolioError.core(message)
        }
        do {
            return try JSONDecoder().decode([FolioDiscoveredBook].self, from: data)
        } catch {
            throw FolioError.decoding(error)
        }
    }

    func capabilityProfiles() throws -> [FolioCapabilityProfile] {
        try capabilitySnapshot().capabilityProfiles
    }

    func inputFormatCapabilities() throws -> [FolioInputFormatCapability] {
        try capabilitySnapshot().inputFormats
    }

    func capabilitySnapshot() throws -> FolioCapabilitiesResponse {
        guard let pointer = ffCapabilities() else { throw FolioError.invalidResponse }
        defer { ffStringFree(pointer) }
        let json = String(cString: pointer)
        guard let data = json.data(using: .utf8) else { throw FolioError.invalidResponse }
        do {
            return try decoder.decode(FolioCapabilitiesResponse.self, from: data)
        } catch {
            throw FolioError.decoding(error)
        }
    }

    func analyze(
        url: URL,
        target: FolioTarget,
        mode: FolioDegradationMode,
        degradation: FolioDegradationOptions,
        edit: FolioBookEditPlan,
        text: FolioTextImportOptions
    ) throws -> FolioAnalysisReport {
        let request = FolioAnalysisRequest(
            input: url.path,
            target: target,
            mode: mode,
            degradation: degradation,
            edit: edit,
            text: text
        )
        let data = try encoder.encode(request)
        guard let requestJSON = String(data: data, encoding: .utf8) else { throw FolioError.invalidResponse }
        let result = requestJSON.withCString { ffAnalyze($0) }
        return try decodeAnalysisResult(result)
    }

    func searchOnlineMetadata(
        request: FolioOnlineMetadataSearchRequest
    ) throws -> [FolioOnlineMetadataCandidate] {
        let data = try encoder.encode(request)
        guard let requestJSON = String(data: data, encoding: .utf8) else { throw FolioError.invalidResponse }
        let result = requestJSON.withCString { ffOnlineMetadataSearch($0) }
        return try decodeOnlineMetadataSearchResult(result)
    }

    func onlineMetadataMergePlan(
        request: FolioOnlineMetadataMergeRequest
    ) throws -> FolioMetadataEdit {
        let data = try encoder.encode(request)
        guard let requestJSON = String(data: data, encoding: .utf8) else { throw FolioError.invalidResponse }
        let result = requestJSON.withCString { ffOnlineMetadataMergePlan($0) }
        return try decodeOnlineMetadataMergeResult(result)
    }

    func startBatch(
        request: FolioBatchConversionRequest,
        progress: @escaping (FolioBatchProgressEvent) -> Void,
        completion: @escaping (Result<FolioBatchReport, FolioError>) -> Void
    ) -> FolioCancellationHandle {
        let cancellation = FolioCancellationHandle(pointer: ffCancellationNew())
        let progressBox = BatchProgressBox(handler: progress)
        let completionBox = BatchCompletionBox(handler: completion)
        let retainedProgress = Unmanaged.passRetained(progressBox)

        let requestJSON: String
        do {
            let data = try encoder.encode(request)
            guard let value = String(data: data, encoding: .utf8) else {
                throw FolioError.invalidResponse
            }
            requestJSON = value
        } catch let error as FolioError {
            retainedProgress.release()
            cancellation.finish()
            DispatchQueue.main.async { completionBox.complete(.failure(error)) }
            return cancellation
        } catch {
            retainedProgress.release()
            cancellation.finish()
            DispatchQueue.main.async { completionBox.complete(.failure(.encoding(error))) }
            return cancellation
        }

        let work = BatchConversionWork(
            requestJSON: requestJSON,
            cancellation: cancellation,
            retainedProgress: retainedProgress,
            completion: completionBox
        )
        DispatchQueue.global(qos: .userInitiated).async {
            work.run()
        }
        return cancellation
    }

    func preview(
        url: URL,
        target: FolioTarget,
        mode: FolioDegradationMode,
        degradation: FolioDegradationOptions,
        edit: FolioBookEditPlan,
        settings: FolioPreviewSettings = FolioPreviewSettings(),
        text: FolioTextImportOptions
    ) throws -> FolioReaderPreviewBundle {
        let request = FolioReaderPreviewRequest(
            input: url.path,
            mode: .target,
            target: target,
            degradationMode: mode,
            degradation: degradation,
            edit: edit,
            settings: settings,
            text: text
        )
        let data = try encoder.encode(request)
        guard let requestJSON = String(data: data, encoding: .utf8) else { throw FolioError.invalidResponse }
        let result = requestJSON.withCString { ffReaderPreview($0) }
        return try decodeReaderPreviewResult(result)
    }

    func inspect(url: URL) throws -> FolioInspectReport {
        let result = url.path.withCString { ffInspect($0) }
        return try decodeInspectResult(result)
    }

    func validate(url: URL) throws -> [FolioDiagnostic] {
        let result = url.path.withCString { ffValidate($0) }
        let report = try decodeValidationResult(result)
        return report.diagnostics
    }

}

private struct ValidationResponse: Decodable {
    let diagnostics: [FolioDiagnostic]
}

private struct CoreErrorResponse: Decodable {
    let error: String
}

private func responseData(_ result: UnsafeMutablePointer<FolioResultC>?) -> (Int32, Data?) {
    guard let result else { return (1, nil) }
    let code = result.pointee.code
    let data = result.pointee.json.flatMap { String(cString: $0).data(using: .utf8) }
    ffResultFree(result)
    return (code, data)
}

func folioCoreFreeOpaqueResult(_ result: UnsafeMutableRawPointer?) {
    guard let result else { return }
    ffResultFree(result.assumingMemoryBound(to: FolioResultC.self))
}

private func decodeBatchConversionResult(_ result: UnsafeMutablePointer<FolioResultC>?) -> Result<FolioBatchReport, FolioError> {
    let (code, data) = responseData(result)
    guard let data else { return .failure(.invalidResponse) }
    if code != 0 {
        let message = (try? JSONDecoder().decode(CoreErrorResponse.self, from: data))?.error
            ?? "Folio Core failed with code \(code)."
        return .failure(.core(message))
    }
    do {
        return .success(try JSONDecoder().decode(FolioBatchReport.self, from: data))
    } catch {
        return .failure(.decoding(error))
    }
}

private func decodeInspectResult(_ result: UnsafeMutablePointer<FolioResultC>?) throws -> FolioInspectReport {
    let (code, data) = responseData(result)
    guard let data else { throw FolioError.invalidResponse }
    if code != 0 {
        let message = (try? JSONDecoder().decode(CoreErrorResponse.self, from: data))?.error
            ?? "Folio Core failed with code \(code)."
        throw FolioError.core(message)
    }
    do {
        return try JSONDecoder().decode(FolioInspectReport.self, from: data)
    } catch {
        throw FolioError.decoding(error)
    }
}

private func decodeAnalysisResult(_ result: UnsafeMutablePointer<FolioResultC>?) throws -> FolioAnalysisReport {
    let (code, data) = responseData(result)
    guard let data else { throw FolioError.invalidResponse }
    if code != 0 {
        let message = (try? JSONDecoder().decode(CoreErrorResponse.self, from: data))?.error
            ?? "Folio Core failed with code \(code)."
        throw FolioError.core(message)
    }
    do {
        return try JSONDecoder().decode(FolioAnalysisReport.self, from: data)
    } catch {
        throw FolioError.decoding(error)
    }
}

private func decodeReaderPreviewResult(
    _ result: UnsafeMutablePointer<FolioResultC>?
) throws -> FolioReaderPreviewBundle {
    let (code, data) = responseData(result)
    guard let data else { throw FolioError.invalidResponse }
    if code != 0 {
        let message = (try? JSONDecoder().decode(CoreErrorResponse.self, from: data))?.error
            ?? "Folio Core failed with code \(code)."
        throw FolioError.core(message)
    }
    do {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(FolioReaderPreviewBundle.self, from: data)
    } catch {
        throw FolioError.decoding(error)
    }
}

private func decodeOnlineMetadataSearchResult(
    _ result: UnsafeMutablePointer<FolioResultC>?
) throws -> [FolioOnlineMetadataCandidate] {
    let (code, data) = responseData(result)
    guard let data else { throw FolioError.invalidResponse }
    if code != 0 {
        let message = (try? JSONDecoder().decode(CoreErrorResponse.self, from: data))?.error
            ?? "Folio Core failed with code \(code)."
        throw FolioError.core(message)
    }
    do {
        return try JSONDecoder().decode([FolioOnlineMetadataCandidate].self, from: data)
    } catch {
        throw FolioError.decoding(error)
    }
}

private func decodeOnlineMetadataMergeResult(
    _ result: UnsafeMutablePointer<FolioResultC>?
) throws -> FolioMetadataEdit {
    let (code, data) = responseData(result)
    guard let data else { throw FolioError.invalidResponse }
    if code != 0 {
        let message = (try? JSONDecoder().decode(CoreErrorResponse.self, from: data))?.error
            ?? "Folio Core failed with code \(code)."
        throw FolioError.core(message)
    }
    do {
        return try JSONDecoder().decode(FolioMetadataEdit.self, from: data)
    } catch {
        throw FolioError.decoding(error)
    }
}

private func decodeValidationResult(_ result: UnsafeMutablePointer<FolioResultC>?) throws -> ValidationResponse {
    let (code, data) = responseData(result)
    guard let data else { throw FolioError.invalidResponse }
    if code != 0 {
        let message = (try? JSONDecoder().decode(CoreErrorResponse.self, from: data))?.error
            ?? "Folio Core failed with code \(code)."
        throw FolioError.core(message)
    }
    do {
        return try JSONDecoder().decode(ValidationResponse.self, from: data)
    } catch {
        throw FolioError.decoding(error)
    }
}
