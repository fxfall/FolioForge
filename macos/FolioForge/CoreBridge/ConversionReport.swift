import Foundation

enum FolioProgressStage: String, Codable, Sendable {
    case opening = "Opening"
    case parsing = "Parsing"
    case normalizing = "Normalizing"
    case resolvingStyles = "ResolvingStyles"
    case processingResources = "ProcessingResources"
    case lowering = "Lowering"
    case buildingIndexes = "BuildingIndexes"
    case encoding = "Encoding"
    case writing = "Writing"
    case validating = "Validating"
    case finished = "Finished"

    var label: String {
        switch self {
        case .opening: "Opening"
        case .parsing: "Parsing"
        case .normalizing: "Normalizing"
        case .resolvingStyles: "Resolving styles"
        case .processingResources: "Processing resources"
        case .lowering: "Lowering"
        case .buildingIndexes: "Building indexes"
        case .encoding: "Encoding"
        case .writing: "Writing"
        case .validating: "Validating"
        case .finished: "Finished"
        }
    }
}

struct FolioProgressEvent: Codable, Sendable {
    let stage: FolioProgressStage
    let current: UInt64
    let total: UInt64?
    let fraction: Float?
    let message: String
}

struct FolioBatchProgressEvent: Decodable, Sendable {
    let currentItem: Int
    let totalItems: Int
    let event: FolioProgressEvent

    enum CodingKeys: String, CodingKey {
        case currentItem = "current_item"
        case totalItems = "total_items"
        case event
    }
}

struct FolioMetadata: Codable, Sendable {
    let title: String?
    let subtitle: String?
    let language: String?
    let creators: [String]
    let contributors: [String]
    let publisher: String?
    let identifier: String?
    let description: String?
    let subjects: [String]
    let dates: [String]
    let rights: String?
}

struct FolioInputReport: Codable, Sendable {
    let detectedFormat: String
    let parser: String
    let containerCount: Int
    let entityCount: Int
    let fragmentCount: Int
    let symbolCount: Int
    let resourceCount: Int
    let documentCount: Int
    let drmDetected: Bool
    let warnings: [String]
    let unknownFeatures: [String]
    let recoveryActions: [String]
    let inputLoss: [String]
    let kfxSummary: FolioKFXInputSummary?
    let text: FolioTextImportReport?

    enum CodingKeys: String, CodingKey {
        case detectedFormat = "detected_format"
        case parser
        case containerCount = "container_count"
        case entityCount = "entity_count"
        case fragmentCount = "fragment_count"
        case symbolCount = "symbol_count"
        case resourceCount = "resource_count"
        case documentCount = "document_count"
        case drmDetected = "drm_detected"
        case warnings
        case unknownFeatures = "unknown_features"
        case recoveryActions = "recovery_actions"
        case inputLoss = "input_loss"
        case kfxSummary = "kfx_summary"
        case text
    }
}

/// A read-only coverage snapshot for real Amazon KFX after it reaches the
/// shared Semantic IR. It is intentionally not a KFX-native model and does
/// not imply that unresolved source semantics were recovered.
struct FolioKFXInputSummary: Codable, Sendable {
    let tocEntryCount: Int
    let landmarkEntryCount: Int
    let pageListEntryCount: Int
    let hasStartLocation: Bool
    let featureCounts: [String: Int]
    let imageCount: Int
    let imageAltPresentCount: Int
    let imageAltEmptyCount: Int
    let linkCount: Int
    let anchorCount: Int
    let anchorGraphEdgeCount: Int
    let styleCount: Int
    let nonDefaultStyleNodeCount: Int
    let fontFaceCount: Int

    enum CodingKeys: String, CodingKey {
        case tocEntryCount = "toc_entry_count"
        case landmarkEntryCount = "landmark_entry_count"
        case pageListEntryCount = "page_list_entry_count"
        case hasStartLocation = "has_start_location"
        case featureCounts = "feature_counts"
        case imageCount = "image_count"
        case imageAltPresentCount = "image_alt_present_count"
        case imageAltEmptyCount = "image_alt_empty_count"
        case linkCount = "link_count"
        case anchorCount = "anchor_count"
        case anchorGraphEdgeCount = "anchor_graph_edge_count"
        case styleCount = "style_count"
        case nonDefaultStyleNodeCount = "non_default_style_node_count"
        case fontFaceCount = "font_face_count"
    }
}

struct FolioTextImportReport: Codable, Sendable {
    let encoding: FolioEncodingDetection
    let normalization: FolioTextNormalization
    let mode: String
    let paragraphAnalysis: FolioParagraphAnalysis?
    let structure: FolioBookStructure
    let metadataGuess: FolioTextMetadataGuess
    let markdownLike: Bool
    let diagnostics: [String]
    let inputLoss: [String]

    enum CodingKeys: String, CodingKey {
        case encoding, normalization, mode, structure, diagnostics
        case paragraphAnalysis = "paragraph_analysis"
        case metadataGuess = "metadata_guess"
        case markdownLike = "markdown_like"
        case inputLoss = "input_loss"
    }
}

struct FolioEncodingDetection: Codable, Sendable {
    let selected: String?
    let confidence: String
    let candidates: [FolioEncodingCandidate]
    let evidence: [String]
}

struct FolioEncodingCandidate: Codable, Identifiable, Sendable {
    let encoding: String
    let score: Int
    let evidence: [String]
    var id: String { encoding }
}

struct FolioTextNormalization: Codable, Sendable {
    let removedLeadingBom: Bool
    let newlineSequencesNormalized: Int

    enum CodingKeys: String, CodingKey {
        case removedLeadingBom = "removed_leading_bom"
        case newlineSequencesNormalized = "newline_sequences_normalized"
    }
}

struct FolioParagraphAnalysis: Codable, Sendable {
    let selected: String
    let confidencePercent: Int
    let protectedPreformatted: Bool
    let evidence: [String]

    enum CodingKeys: String, CodingKey {
        case selected, evidence
        case confidencePercent = "confidence_percent"
        case protectedPreformatted = "protected_preformatted"
    }
}

struct FolioBookStructure: Codable, Sendable {
    let roots: [FolioStructureNode]
    let rejected: [FolioStructureCandidate]
}

struct FolioStructureNode: Codable, Identifiable, Sendable {
    let candidate: FolioStructureCandidate
    let children: [FolioStructureNode]
    var id: Int { candidate.lineIndex }
}

struct FolioStructureCandidate: Codable, Identifiable, Sendable {
    let lineIndex: Int
    let text: String
    let kind: String
    let level: Int
    let number: Int?
    let confidencePercent: Int
    let detector: String
    let evidence: [String]
    var id: Int { lineIndex }

    enum CodingKeys: String, CodingKey {
        case text, kind, level, number, detector, evidence
        case lineIndex = "line_index"
        case confidencePercent = "confidence_percent"
    }
}

struct FolioTextMetadataGuess: Codable, Sendable {
    let title: String?
    let author: String?
    let source: String?
    let confidence: String?
    let evidence: [String]
}

struct FolioSemanticReport: Codable, Sendable {
    let valid: Bool
    let documentCount: Int
    let resourceCount: Int
    let featureCount: Int
    let warnings: [FolioDiagnostic]
    let inputLoss: [String]

    enum CodingKeys: String, CodingKey {
        case valid
        case documentCount = "document_count"
        case resourceCount = "resource_count"
        case featureCount = "feature_count"
        case warnings
        case inputLoss = "input_loss"
    }
}

struct FolioCompatibilityReport: Codable, Sendable {
    let target: String
    let quality: FolioCompatibilityQuality
    let exact: Int
    let equivalent: Int
    let approximation: Int
    let structuralFallback: Int
    let dropped: Int
    let targetLoss: [String]
    let diagnostics: [FolioDiagnostic]

    enum CodingKeys: String, CodingKey {
        case target
        case quality
        case exact
        case equivalent
        case approximation
        case structuralFallback = "structural_fallback"
        case dropped
        case targetLoss = "target_loss"
        case diagnostics
    }
}

struct FolioOutputReport: Codable, Sendable {
    let format: String
    let path: String
    let size: UInt64
    let deterministic: Bool
    let validated: Bool
}

enum FolioCompatibilityQuality: String, Codable, Equatable, Sendable {
    case exact = "Exact"
    case high = "High"
    case compatible = "Compatible"
    case reduced = "Reduced"
    case severeLoss = "SevereLoss"

    var displayName: String {
        switch self {
        case .exact: "Exact"
        case .high: "High fidelity"
        case .compatible: "Compatible"
        case .reduced: "Reduced layout"
        case .severeLoss: "Severe loss"
        }
    }

    var userSummary: String {
        switch self {
        case .exact: "Excellent"
        case .high: "High fidelity"
        case .compatible: "Minor formatting changes"
        case .reduced: "Structural fallback"
        case .severeLoss: "Significant content loss"
        }
    }
}

struct FolioDegradationItem: Codable, Identifiable, Sendable {
    let feature: String
    let sourceRepresentation: String
    let targetCapability: String
    let selectedFallback: String
    let quality: String
    let reason: String
    let possibleAlternatives: [String]
    let nodeID: UInt32?
    let diagnostic: FolioDiagnostic

    var id: String {
        "\(feature):\(nodeID.map(String.init) ?? "book"):\(selectedFallback)"
    }

    enum CodingKeys: String, CodingKey {
        case feature
        case sourceRepresentation = "source_representation"
        case targetCapability = "target_capability"
        case selectedFallback = "selected_fallback"
        case quality
        case reason
        case possibleAlternatives = "possible_alternatives"
        case nodeID = "node_id"
        case diagnostic
    }
}

struct FolioDegradationReport: Codable, Sendable {
    let exact: Int
    let equivalent: Int
    let approximation: Int
    let structuralFallback: Int
    let dropped: Int
    let quality: FolioCompatibilityQuality
    let items: [FolioDegradationItem]
    let diagnostics: [FolioDiagnostic]

    enum CodingKeys: String, CodingKey {
        case exact
        case equivalent
        case approximation
        case structuralFallback = "structural_fallback"
        case dropped
        case quality
        case items
        case diagnostics
    }
}

struct FolioRoundTripReport: Codable, Sendable {
    let checked: Bool
    let passed: Bool
    let unexpectedLosses: [String]

    enum CodingKeys: String, CodingKey {
        case checked
        case passed
        case unexpectedLosses = "unexpected_losses"
    }
}

struct FolioCapabilityProfile: Codable, Identifiable, Sendable {
    let format: String
    let levels: [String: String]

    var id: String { format }
}

struct FolioFormatSupport: Codable, Sendable {
    let detect: Bool
    let inspect: Bool
    let `import`: Bool
    let export: Bool
    let metadataRead: Bool
    let metadataWrite: Bool
    let edit: Bool
    let preview: Bool

    enum CodingKeys: String, CodingKey {
        case detect
        case `import`
        case inspect
        case export
        case metadataRead = "metadata_read"
        case metadataWrite = "metadata_write"
        case edit
        case preview
    }
}

struct FolioInputFormatCapability: Codable, Identifiable, Sendable {
    let format: String
    let extensions: [String]
    let support: FolioFormatSupport

    var id: String { format }
}

struct FolioCapabilitiesResponse: Codable, Sendable {
    let capabilityProfiles: [FolioCapabilityProfile]
    let inputFormats: [FolioInputFormatCapability]
    let targets: [FolioTarget]

    enum CodingKeys: String, CodingKey {
        case capabilityProfiles = "capability_profiles"
        case inputFormats = "input_formats"
        case targets
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        capabilityProfiles = try container.decode([FolioCapabilityProfile].self, forKey: .capabilityProfiles)
        inputFormats = try container.decode([FolioInputFormatCapability].self, forKey: .inputFormats)
        targets = try container.decodeIfPresent([FolioTarget].self, forKey: .targets) ?? []
    }
}

struct FolioDegradationPlan: Codable, Sendable {
    let target: String
    let mode: FolioDegradationMode
    let options: FolioDegradationOptions
    let capability: FolioCapabilityProfile
    let items: [FolioDegradationItem]
    let diagnostics: [FolioDiagnostic]
    let quality: FolioCompatibilityQuality
    let blocked: Bool
}

struct FolioAnalysisReport: Codable, Sendable {
    let sourceFormat: String
    let targetFormat: String
    let plan: FolioDegradationPlan
    let inputReport: FolioInputReport?
    let semanticReport: FolioSemanticReport?

    enum CodingKeys: String, CodingKey {
        case sourceFormat = "source_format"
        case targetFormat = "target_format"
        case plan
        case inputReport = "input_report"
        case semanticReport = "semantic_report"
    }
}

struct FolioConversionReport: Codable, Sendable {
    let outputPath: String
    let outputSize: UInt64
    let warnings: [FolioDiagnostic]
    let metadata: FolioMetadata
    let features: [String: Int]
    let resourceSummary: FolioResourceSummary
    let durationMs: UInt64
    let sourceFormat: String
    let targetFormat: String
    let degradationMode: FolioDegradationMode
    let compatibility: FolioCompatibilityQuality
    let degradation: FolioDegradationReport
    let roundTrip: FolioRoundTripReport
    let inputReport: FolioInputReport
    let semanticReport: FolioSemanticReport
    let compatibilityReport: FolioCompatibilityReport
    let outputReport: FolioOutputReport

    enum CodingKeys: String, CodingKey {
        case outputPath = "output_path"
        case outputSize = "output_size"
        case warnings
        case metadata
        case features
        case resourceSummary = "resource_summary"
        case durationMs = "duration_ms"
        case sourceFormat = "source_format"
        case targetFormat = "target_format"
        case degradationMode = "degradation_mode"
        case compatibility
        case degradation
        case roundTrip = "round_trip"
        case inputReport = "input_report"
        case semanticReport = "semantic_report"
        case compatibilityReport = "compatibility_report"
        case outputReport = "output_report"
    }
}

struct FolioBatchItemReport: Decodable, Sendable {
    let source: String
    let output: String?
    let relativeOutput: String?
    let success: Bool
    let report: FolioConversionReport?
    let error: String?

    enum CodingKeys: String, CodingKey {
        case source
        case output
        case relativeOutput = "relative_output"
        case success
        case report
        case error
    }
}

struct FolioBatchReport: Decodable, Sendable {
    let items: [FolioBatchItemReport]
    let succeeded: Int
    let failed: Int
    let aborted: Bool
}

struct FolioResourceSummary: Codable, Sendable {
    let total: Int
    let images: Int
    let fonts: Int
    let stylesheets: Int
    let audio: Int
    let declaredBytes: UInt64

    enum CodingKeys: String, CodingKey {
        case total
        case images
        case fonts
        case stylesheets
        case audio
        case declaredBytes = "declared_bytes"
    }
}

struct FolioInspectReport: Codable, Sendable {
    let format: String
    let semantic: AnyCodableJSON
    let diagnostics: [FolioDiagnostic]
    let inputReport: FolioInputReport
    let semanticReport: FolioSemanticReport

    enum CodingKeys: String, CodingKey {
        case format
        case semantic
        case diagnostics
        case inputReport = "input_report"
        case semanticReport = "semantic_report"
    }
}

struct FolioPreviewDocument: Codable, Identifiable, Sendable {
    let href: String
    let title: String?
    let html: String

    var id: String { href }
}

struct FolioPreviewTargetProfile: Codable, Sendable {
    let format: String
    let label: String
    let disclaimer: String
    let device: String
    let orientation: String
    let viewportWidth: Int
    let viewportHeight: Int
    let fontSizePercent: Int

    enum CodingKeys: String, CodingKey {
        case format, label, disclaimer, device, orientation
        case viewportWidth = "viewport_width"
        case viewportHeight = "viewport_height"
        case fontSizePercent = "font_size_percent"
    }
}

struct FolioPreviewBundle: Codable, Sendable {
    let target: FolioPreviewTargetProfile
    let sourceTitle: String
    let documents: [FolioPreviewDocument]
    let html: String
    let degradation: FolioDegradationReport
    let inputLoss: [String]
    let targetLoss: [String]
    let blocked: Bool

    enum CodingKeys: String, CodingKey {
        case target
        case sourceTitle = "source_title"
        case documents
        case html
        case degradation
        case inputLoss = "input_loss"
        case targetLoss = "target_loss"
        case blocked
    }
}

/// A small type-erased JSON value used only for displaying inspect results.
enum AnyCodableJSON: Codable, Sendable {
    case object([String: AnyCodableJSON])
    case array([AnyCodableJSON])
    case string(String)
    case number(Double)
    case bool(Bool)
    case null

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([AnyCodableJSON].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: AnyCodableJSON].self))
        }
    }

    func encode(to encoder: Encoder) throws {
        switch self {
        case .object(let value): try value.encode(to: encoder)
        case .array(let value): try value.encode(to: encoder)
        case .string(let value): try value.encode(to: encoder)
        case .number(let value): try value.encode(to: encoder)
        case .bool(let value): try value.encode(to: encoder)
        case .null: var container = encoder.singleValueContainer(); try container.encodeNil()
        }
    }
}
