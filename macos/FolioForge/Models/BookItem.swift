import Foundation

enum QueueItemStatus: String {
    case ready
    case converting
    case completed
    case failed
    case cancelled

    var label: String {
        switch self {
        case .ready: "Ready"
        case .converting: "Converting"
        case .completed: "Completed"
        case .failed: "Failed"
        case .cancelled: "Cancelled"
        }
    }
}

struct BookItem: Identifiable {
    let id: UUID
    let inputURL: URL
    let relativePath: String?
    let sourceRoot: URL?
    var outputURL: URL?
    var status: QueueItemStatus
    var progress: Double?
    var stage: FolioProgressStage?
    var warnings: [FolioDiagnostic]
    var errorMessage: String?
    var report: FolioConversionReport?
    var analysis: FolioAnalysisReport?
    var analysisError: String?
    var editPlan: FolioBookEditPlan

    init(inputURL: URL, relativePath: String? = nil, sourceRoot: URL? = nil) {
        self.id = UUID()
        self.inputURL = inputURL
        self.relativePath = relativePath
        self.sourceRoot = sourceRoot
        self.outputURL = nil
        self.status = .ready
        self.progress = nil
        self.stage = nil
        self.warnings = []
        self.errorMessage = nil
        self.report = nil
        self.analysis = nil
        self.analysisError = nil
        self.editPlan = FolioBookEditPlan()
    }

    var displayPath: String {
        relativePath ?? inputURL.lastPathComponent
    }

    var hasPreflightResult: Bool {
        analysis != nil || analysisError != nil
    }
}
