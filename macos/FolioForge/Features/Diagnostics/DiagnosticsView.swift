import SwiftUI

struct DiagnosticsView: View {
    let item: BookItem?

    var body: some View {
        GroupBox(FolioL10n.string("ui.preflight_diagnostics", default: "Preflight & diagnostics")) {
            if let item {
                ScrollView {
                    VStack(alignment: .leading, spacing: 10) {
                        HStack(alignment: .firstTextBaseline) {
                            Label(item.displayPath, systemImage: FolioSymbol.documentSearch.name)
                                .lineLimit(1)
                            Spacer()
                            Text(item.status.label)
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(statusColor(item.status))
                        }

                        if let analysis = item.analysis {
                            AnalysisSummaryView(analysis: analysis)
                        } else if let error = item.analysisError {
                            Label(error, systemImage: FolioSymbol.error.name)
                                .foregroundStyle(.red)
                                .fixedSize(horizontal: false, vertical: true)
                        } else {
                            Label(FolioL10n.string("ui.preflight_is_required_before_conversion", default: "Preflight is required before conversion."), systemImage: FolioSymbol.checklist.name)
                                .foregroundStyle(.secondary)
                        }

                        if let report = item.report {
                            ConversionResultView(report: report)
                        }

                        if let error = item.errorMessage, item.analysisError == nil {
                            Label(error, systemImage: FolioSymbol.error.name)
                                .foregroundStyle(.red)
                                .fixedSize(horizontal: false, vertical: true)
                        }

                        let warnings = item.warnings
                        if !warnings.isEmpty {
                            Divider()
                            Text(FolioL10n.string("ui.diagnostics", default: "Diagnostics"))
                                .font(.headline)
                            ForEach(Array(warnings.enumerated()), id: \.offset) { _, diagnostic in
                                DiagnosticRow(diagnostic: diagnostic)
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 360)
            } else {
                Text(FolioL10n.string("ui.select_a_book_to_see_its_capability_plan_and", default: "Select a book to see its capability plan and diagnostics."))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private func statusColor(_ status: QueueItemStatus) -> Color {
        switch status {
        case .completed: .green
        case .failed: .red
        case .converting: .accentColor
        case .cancelled: .orange
        case .ready: .secondary
        }
    }
}

private struct AnalysisSummaryView: View {
    let analysis: FolioAnalysisReport

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(FolioL10n.string("ui.compatibility_plan", default: "Compatibility plan"))
                    .font(.headline)
                Spacer()
                Text(analysis.plan.quality.displayName)
                    .font(.headline)
                    .foregroundStyle(qualityColor(analysis.plan.quality))
                Text(analysis.plan.blocked
                    ? FolioL10n.string("status.blocked", default: "Blocked")
                    : FolioL10n.string("status.ready", default: "Ready"))
                    .font(.caption.weight(.semibold))
                    .padding(.horizontal, 7)
                    .padding(.vertical, 3)
                    .background(analysis.plan.blocked ? Color.red.opacity(0.12) : Color.green.opacity(0.12))
                    .foregroundStyle(analysis.plan.blocked ? .red : .green)
                    .clipShape(Capsule())
            }
            Text("\(analysis.sourceFormat) → \(analysis.targetFormat) · \(analysis.plan.mode.displayName)")
                .font(.caption)
                .foregroundStyle(.secondary)

            HStack(spacing: 6) {
                MetricPill(title: FolioL10n.string("quality.exact", default: "Exact"), value: count("Exact"), color: .green)
                MetricPill(title: FolioL10n.string("metric.equivalent", default: "Equivalent"), value: count("Equivalent"), color: .blue)
                MetricPill(title: FolioL10n.string("metric.approx", default: "Approx"), value: count("CompatibleApproximation"), color: .orange)
                MetricPill(title: FolioL10n.string("metric.fallback", default: "Fallback"), value: count("StructuralFallback"), color: .purple)
                MetricPill(title: FolioL10n.string("metric.drop", default: "Drop"), value: count("Drop"), color: .red)
            }

            if !analysis.plan.items.isEmpty {
                DisclosureGroup(FolioL10n.format(
                    "error.fallback_plan_details",
                    default: "Fallback plan details: %@",
                    String(analysis.plan.items.count)
                )) {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(analysis.plan.items) { planItem in
                            VStack(alignment: .leading, spacing: 3) {
                                HStack {
                                    Text(planItem.feature)
                                        .font(.caption.weight(.semibold))
                                    Text(planItem.quality)
                                        .font(.caption.monospaced())
                                        .foregroundStyle(.secondary)
                                    Spacer()
                                    Text(planItem.targetCapability)
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                }
                                Text("\(planItem.sourceRepresentation) → \(planItem.selectedFallback)")
                                    .font(.caption)
                                Text(planItem.reason)
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                                if !planItem.possibleAlternatives.isEmpty {
                                    Text(FolioL10n.format(
                                        "error.alternatives",
                                        default: "Alternatives: %@",
                                        planItem.possibleAlternatives.joined(separator: " · ")
                                    ))
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                        .fixedSize(horizontal: false, vertical: true)
                                }
                                Text(planItem.diagnostic.code)
                                    .font(.caption2.monospaced())
                                    .foregroundStyle(.secondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.vertical, 3)
                            Divider()
                        }
                    }
                    .padding(.top, 6)
                }
            } else {
                Text(FolioL10n.string("ui.no_degradation_required_for_this_target", default: "No degradation required for this target."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func count(_ quality: String) -> Int {
        analysis.plan.items.filter { $0.quality == quality }.count
    }

    private func qualityColor(_ quality: FolioCompatibilityQuality) -> Color {
        switch quality {
        case .exact, .high: .green
        case .compatible: .blue
        case .reduced: .orange
        case .severeLoss: .red
        }
    }
}

private struct MetricPill: View {
    let title: String
    let value: Int
    let color: Color

    var body: some View {
        VStack(spacing: 2) {
            Text("\(value)")
                .font(.headline.monospacedDigit())
                .foregroundStyle(color)
            Text(title)
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 5)
        .background(Color.secondary.opacity(0.08))
        .clipShape(RoundedRectangle(cornerRadius: 6))
    }
}

private struct ConversionResultView: View {
    let report: FolioConversionReport

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Divider()
            Text(FolioL10n.string("ui.conversion_result", default: "Conversion result"))
                .font(.headline)
            HStack {
                Text(FolioL10n.format("error.compatibility", default: "Compatibility: %@", report.compatibility.userSummary))
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(qualityColor(report.compatibility))
                Spacer()
                Text("\(report.sourceFormat) → \(report.targetFormat) · \(report.degradationMode.displayName)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Text(FolioL10n.format("error.output", default: "Output: %@", report.outputPath))
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
            HStack(spacing: 6) {
                MetricPill(title: FolioL10n.string("quality.exact", default: "Exact"), value: report.degradation.exact, color: .green)
                MetricPill(title: FolioL10n.string("metric.equivalent", default: "Equivalent"), value: report.degradation.equivalent, color: .blue)
                MetricPill(title: FolioL10n.string("metric.approx", default: "Approx"), value: report.degradation.approximation, color: .orange)
                MetricPill(title: FolioL10n.string("metric.fallback", default: "Fallback"), value: report.degradation.structuralFallback, color: .purple)
                MetricPill(title: FolioL10n.string("metric.drop", default: "Drop"), value: report.degradation.dropped, color: .red)
            }
            if !report.degradation.items.isEmpty {
                DisclosureGroup(FolioL10n.format(
                    "error.applied_fallback_details",
                    default: "Applied fallback details: %@",
                    String(report.degradation.items.count)
                )) {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(report.degradation.items) { item in
                            VStack(alignment: .leading, spacing: 3) {
                                HStack {
                                    Text(item.feature)
                                        .font(.caption.weight(.semibold))
                                    Text(item.quality)
                                        .font(.caption.monospaced())
                                        .foregroundStyle(.secondary)
                                    Spacer()
                                }
                                Text("\(item.sourceRepresentation) → \(item.selectedFallback)")
                                    .font(.caption)
                                Text(item.reason)
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                                Text(item.diagnostic.code)
                                    .font(.caption2.monospaced())
                                    .foregroundStyle(.secondary)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.vertical, 3)
                            Divider()
                        }
                    }
                    .padding(.top, 6)
                }
            }
            Label(
                report.roundTrip.passed
                    ? FolioL10n.string("status.round_trip_passed", default: "Semantic round-trip passed")
                    : FolioL10n.string("status.round_trip_loss", default: "Semantic round-trip reported a loss"),
                systemImage: (report.roundTrip.passed ? FolioSymbol.checkSealFilled : FolioSymbol.warning).name
            )
            .font(.caption)
            .foregroundStyle(report.roundTrip.passed ? .green : .orange)
            if !report.roundTrip.unexpectedLosses.isEmpty {
                Text(report.roundTrip.unexpectedLosses.joined(separator: "\n"))
                    .font(.caption2)
                    .foregroundStyle(.orange)
            }
            if !report.degradation.diagnostics.isEmpty {
                Text(FolioL10n.string("ui.planner_diagnostics", default: "Planner diagnostics"))
                    .font(.caption.weight(.semibold))
                ForEach(report.degradation.diagnostics) { diagnostic in
                    DiagnosticRow(diagnostic: diagnostic)
                }
            }
        }
    }

    private func qualityColor(_ quality: FolioCompatibilityQuality) -> Color {
        switch quality {
        case .exact, .high: .green
        case .compatible: .blue
        case .reduced: .orange
        case .severeLoss: .red
        }
    }
}

private struct DiagnosticRow: View {
    let diagnostic: FolioDiagnostic

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(folioSymbol: diagnostic.severity == .error ? .error : .warning)
                .foregroundStyle(diagnostic.severity == .error ? .red : .orange)
            VStack(alignment: .leading, spacing: 2) {
                Text(diagnostic.code)
                    .font(.caption.monospaced())
                Text(diagnostic.message)
                    .font(.caption)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}
