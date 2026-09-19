import SwiftUI

private enum OnlineMetadataField: String, CaseIterable, Identifiable {
    case title
    case subtitle
    case authors
    case language
    case publisher
    case date
    case series
    case seriesIndex = "series_index"
    case contributors
    case subjects
    case identifiers
    case description
    case rights

    var id: String { rawValue }

    var title: String {
        switch self {
        case .title: "Title"
        case .subtitle: "Subtitle"
        case .authors: "Authors"
        case .language: "Language"
        case .publisher: "Publisher"
        case .date: "Published"
        case .series: "Series"
        case .seriesIndex: "Series number"
        case .contributors: "Contributors"
        case .subjects: "Subjects"
        case .identifiers: "Identifiers"
        case .description: "Description"
        case .rights: "Rights"
        }
    }

    var allowsAppend: Bool { self != .seriesIndex }

    func value(in metadata: FolioOnlineMetadata) -> String? {
        let value: String?
        switch self {
        case .title: value = metadata.title
        case .subtitle: value = metadata.subtitle
        case .authors: value = metadata.authorNames.joined(separator: ", ")
        case .language: value = metadata.language
        case .publisher: value = metadata.publisher
        case .date: value = metadata.date ?? metadata.dates.first
        case .series: value = metadata.series
        case .seriesIndex: value = metadata.seriesIndex.map { String($0) }
        case .contributors: value = metadata.contributors.joined(separator: ", ")
        case .subjects: value = metadata.subjects.joined(separator: ", ")
        case .identifiers: value = metadata.identifierValues.joined(separator: ", ")
        case .description: value = metadata.description
        case .rights: value = metadata.rights
        }
        guard let value else { return nil }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}

private enum OnlineSearchResponse: Sendable {
    case success([FolioOnlineMetadataCandidate])
    case failure(String)
}

private enum OnlineMergeResponse: Sendable {
    case success(FolioMetadataEdit)
    case failure(String)
}

@MainActor
private final class OnlineMetadataSearchState: ObservableObject {
    @Published var titleQuery: String
    @Published var authorQuery: String
    @Published var isbnQuery: String
    @Published var candidates: [FolioOnlineMetadataCandidate] = []
    @Published var selectedCandidateID: String?
    @Published var fieldActions: [String: FolioMetadataMergeAction] = [:]
    @Published var isSearching = false
    @Published var isApplying = false
    @Published var errorMessage: String?
    @Published var showingApplyConfirmation = false
    @Published var completedSearch = false

    init(current: FolioOnlineMetadata) {
        titleQuery = current.title ?? ""
        authorQuery = current.authorNames.first ?? ""
        isbnQuery = current.identifierValues.first ?? ""
    }
}

@MainActor
struct OnlineMetadataSearchView: View {
    @Environment(\.dismiss) private var dismiss

    let current: FolioOnlineMetadata
    let apply: (FolioMetadataEdit) -> Void

    @StateObject private var state: OnlineMetadataSearchState

    init(current: FolioOnlineMetadata, apply: @escaping (FolioMetadataEdit) -> Void) {
        self.current = current
        self.apply = apply
        _state = StateObject(wrappedValue: OnlineMetadataSearchState(current: current))
    }

    private var selectedCandidate: FolioOnlineMetadataCandidate? {
        state.candidates.first { $0.candidateID == state.selectedCandidateID }
    }

    private var hasSelectedEdits: Bool {
        state.fieldActions.values.contains { $0 != .keep }
    }

    var body: some View {
        VStack(spacing: 0) {
            header

            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 12) {
                    TextField("Title", text: $state.titleQuery)
                        .accessibilityLabel("Search title")
                    TextField("Author", text: $state.authorQuery)
                        .accessibilityLabel("Search author")
                    TextField("ISBN", text: $state.isbnQuery)
                        .accessibilityLabel("Search ISBN")
                        .frame(maxWidth: 170)
                    Button(action: search) {
                        Label("Search Open Library", systemImage: FolioAction.findMetadata.symbol.name)
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(state.isSearching || !hasSearchTerms)
                }

                Label("Only these search terms are sent to Open Library. FolioForge does not upload the book file.", systemImage: FolioSymbol.privacy.name)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(16)

            Divider()

            if state.isSearching {
                ProgressView("Searching Open Library…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let errorMessage = state.errorMessage {
                emptyState(
                    title: "Search Unavailable",
                    message: errorMessage,
                    symbol: .alertCircle,
                    action: "Try Again",
                    actionHandler: search
                )
            } else if state.candidates.isEmpty {
                emptyState(
                    title: state.completedSearch ? "No Matching Records" : "Search Book Metadata",
                    message: state.completedSearch
                        ? "No records matched these terms. Try a shorter title or a different identifier."
                        : "Search is manual and opt-in. Results are never applied automatically.",
                    symbol: .onlineMetadata
                )
            } else {
                resultsAndDetails
            }

            Divider()

            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Apply Selected Changes…") {
                    state.showingApplyConfirmation = true
                }
                .buttonStyle(.borderedProminent)
                .disabled(selectedCandidate == nil || !hasSelectedEdits || state.isApplying)
            }
            .padding(14)
        }
        .frame(minWidth: 780, idealWidth: 900, minHeight: 560, idealHeight: 680)
        .confirmationDialog(
            "Apply selected metadata to this book’s edit plan?",
            isPresented: $state.showingApplyConfirmation,
            titleVisibility: .visible
        ) {
            Button("Apply Selected Fields") { buildAndApplyPlan() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Only fields set to Replace or Append will change. The source book remains untouched until conversion.")
        }
    }

    private var header: some View {
        HStack(spacing: 12) {
            Image(folioSymbol: .onlineMetadata)
                .font(.title2)
                .foregroundStyle(.tint)
                .frame(width: 40, height: 40)
                .background(Color.accentColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
            VStack(alignment: .leading, spacing: 3) {
                Text("Find Metadata Online")
                    .font(.title3.weight(.semibold))
                Text("Review candidates and choose exactly which fields to use.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button("Close") { dismiss() }
                .buttonStyle(.borderless)
        }
        .padding(16)
    }

    private var resultsAndDetails: some View {
        HStack(spacing: 0) {
            ScrollView {
                VStack(spacing: 8) {
                    ForEach(state.candidates) { candidate in
                        candidateRow(candidate)
                    }
                }
                .padding(12)
            }
            .frame(width: 270)

            Divider()

            if let selectedCandidate {
                candidateDetails(selectedCandidate)
            } else {
                emptyState(title: "Select a Result", message: "Choose a candidate to review its fields.", symbol: .onlineMetadata)
            }
        }
    }

    @ViewBuilder
    private func emptyState(
        title: String,
        message: String,
        symbol: FolioSymbol,
        action: String? = nil,
        actionHandler: (() -> Void)? = nil
    ) -> some View {
        VStack(spacing: 10) {
            Image(folioSymbol: symbol)
                .font(.system(size: 32))
                .foregroundStyle(.secondary)
            Text(title).font(.headline)
            Text(message)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 420)
            if let action, let actionHandler {
                Button(action, action: actionHandler)
                    .padding(.top, 4)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(24)
    }

    private func candidateRow(_ candidate: FolioOnlineMetadataCandidate) -> some View {
        let isSelected = candidate.candidateID == state.selectedCandidateID
        return Button {
            state.selectedCandidateID = candidate.candidateID
            state.fieldActions = [:]
        } label: {
            VStack(alignment: .leading, spacing: 6) {
                Text(candidate.metadata.title ?? "Untitled result")
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Text(candidate.metadata.authorNames.joined(separator: ", ").ifEmpty("Author not listed"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
                HStack {
                    Text("\(Int((candidate.confidence * 100).rounded()))% match")
                    Spacer()
                    Text(candidate.metadata.date ?? candidate.metadata.dates.first ?? "")
                }
                .font(.caption2)
                .foregroundStyle(.secondary)
            }
            .padding(10)
            .background(
                isSelected ? Color.accentColor.opacity(0.12) : Color.secondary.opacity(0.06),
                in: RoundedRectangle(cornerRadius: 9)
            )
            .overlay {
                RoundedRectangle(cornerRadius: 9)
                    .strokeBorder(isSelected ? Color.accentColor.opacity(0.55) : Color.clear, lineWidth: 1)
            }
            .contentShape(RoundedRectangle(cornerRadius: 9))
        }
        .buttonStyle(.plain)
    }

    private func candidateDetails(_ candidate: FolioOnlineMetadataCandidate) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(candidate.metadata.title ?? "Untitled result")
                            .font(.title3.weight(.semibold))
                        Text(candidate.metadata.authorNames.joined(separator: ", ").ifEmpty("Author not listed"))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    if let source = candidate.sourceURL, let url = URL(string: source) {
                        Link("Source", destination: url)
                    }
                }

                Text("Choose Keep current, Replace, or Append for each available field.")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                ForEach(OnlineMetadataField.allCases.filter { $0.value(in: candidate.metadata) != nil }) { field in
                    metadataField(field, candidate: candidate)
                }
            }
            .padding(16)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func metadataField(
        _ field: OnlineMetadataField,
        candidate: FolioOnlineMetadataCandidate
    ) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(field.title).font(.subheadline.weight(.medium))
                Spacer()
                Picker(field.title, selection: actionBinding(for: field)) {
                    ForEach(FolioMetadataMergeAction.allCases.filter { $0 != .append || field.allowsAppend }) { action in
                        Text(action.title).tag(action)
                    }
                }
                .labelsHidden()
                .controlSize(.small)
                .frame(width: 132)
            }
            Text("Current: \(field.value(in: current) ?? "—")")
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
            Text("Online: \(field.value(in: candidate.metadata) ?? "—")")
                .font(.caption)
                .lineLimit(3)
        }
        .padding(10)
        .background(Color.secondary.opacity(0.055), in: RoundedRectangle(cornerRadius: 8))
    }

    private var hasSearchTerms: Bool {
        [state.titleQuery, state.authorQuery, state.isbnQuery]
            .contains { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    private func actionBinding(for field: OnlineMetadataField) -> Binding<FolioMetadataMergeAction> {
        Binding(
            get: { state.fieldActions[field.rawValue] ?? .keep },
            set: { state.fieldActions[field.rawValue] = $0 }
        )
    }

    private func search() {
        let title = state.titleQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        let author = state.authorQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        let isbn = state.isbnQuery.trimmingCharacters(in: .whitespacesAndNewlines)
        let request = FolioOnlineMetadataSearchRequest(
            enabled: true,
            request: FolioOnlineProviderRequest(
                query: [title, author, isbn].filter { !$0.isEmpty }.joined(separator: " "),
                locale: nil,
                isbn: isbn.isEmpty ? nil : isbn,
                identifier: nil,
                title: title.isEmpty ? nil : title,
                author: author.isEmpty ? nil : author,
                maxResults: 12
            )
        )

        state.isSearching = true
        state.errorMessage = nil
        state.completedSearch = false
        state.candidates = []
        state.selectedCandidateID = nil
        state.fieldActions = [:]

        Task {
            let result = await Task.detached(priority: .userInitiated) { () -> OnlineSearchResponse in
                do {
                    return .success(try FolioCoreBridge().searchOnlineMetadata(request: request))
                } catch {
                    return .failure(error.localizedDescription)
                }
            }.value

            state.isSearching = false
            switch result {
            case .success(let values):
                state.completedSearch = true
                state.candidates = values
                state.selectedCandidateID = values.first?.candidateID
            case .failure(let message):
                state.errorMessage = message
            }
        }
    }

    private func buildAndApplyPlan() {
        guard let candidate = selectedCandidate else { return }
        let actions = Dictionary(uniqueKeysWithValues: OnlineMetadataField.allCases.map { field in
            (field.rawValue, state.fieldActions[field.rawValue] ?? .keep)
        })
        let request = FolioOnlineMetadataMergeRequest(
            current: current,
            candidate: candidate,
            fields: actions,
            confirmed: true
        )

        state.isApplying = true
        Task {
            let result = await Task.detached(priority: .userInitiated) { () -> OnlineMergeResponse in
                do {
                    return .success(try FolioCoreBridge().onlineMetadataMergePlan(request: request))
                } catch {
                    return .failure(error.localizedDescription)
                }
            }.value

            state.isApplying = false
            switch result {
            case .success(let plan):
                apply(plan)
                dismiss()
            case .failure(let message):
                state.errorMessage = message
            }
        }
    }
}

private extension String {
    func ifEmpty(_ fallback: String) -> String {
        isEmpty ? fallback : self
    }
}
