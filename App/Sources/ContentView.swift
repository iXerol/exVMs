import SwiftUI

struct ContentView: View {
    @State private var viewModel: RecordingsViewModel
    @State private var sortOrder = [KeyPathComparator(\RecordingRow.recordedDate, order: .reverse)]

    init(viewModel: RecordingsViewModel) {
        _viewModel = State(initialValue: viewModel)
    }

    var body: some View {
        VStack(spacing: 0) {
            toolbar
            Divider()
            filenameBuilder
            Divider()
            if viewModel.needsFullDiskAccessHelp {
                fullDiskAccessPanel
            } else {
                VStack(spacing: 0) {
                    tableControls
                    recordingsTable
                }
            }
            statusBar
        }
        .task {
            viewModel.checkAccessAndRefresh()
        }
    }

    private var toolbar: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text("ui.toolbar.title")
                    .font(.headline)
                if let store = viewModel.store {
                    Text("ui.toolbar.recordings_path \(store.recordingsDirectory.string)")
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else {
                    Text("ui.toolbar.no_storage")
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                if let output = viewModel.outputDirectory?.string {
                    Text("ui.toolbar.output_directory \(output)")
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            if viewModel.store != nil {
                Button {
                    viewModel.openRecordingsDirectory()
                } label: {
                    Label("action.show_in_finder", systemImage: "magnifyingglass")
                }
            }
            Button {
                viewModel.refresh()
            } label: {
                Label("action.refresh", systemImage: "arrow.clockwise")
            }
            Button {
                viewModel.chooseOutputDirectory()
            } label: {
                Label("action.output", systemImage: "folder")
            }
            .accessibilityLabel("accessibility.output.choose_directory")
            Button {
                viewModel.exportSelection()
            } label: {
                Label("action.export", systemImage: "square.and.arrow.up")
            }
            .keyboardShortcut(.defaultAction)
            .disabled(!viewModel.canExportSelection || viewModel.isExporting)
        }
        .padding(14)
    }

    private var filenameBuilder: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("ui.filename.section_title")
                .font(.subheadline.weight(.semibold))
            FilenameEditor(recipe: $viewModel.filenameRecipe)
        }
        .padding(14)
    }

    private var tableControls: some View {
        HStack(spacing: 8) {
            Picker("ui.filter.folder.label", selection: folderFilterBinding) {
                Text("filter.folder.all").tag(Optional<String>.none)
                ForEach(viewModel.folderOptions, id: \.self) { folder in
                    Text(folder).tag(Optional(folder))
                }
            }
            .frame(width: 240)
            Button {
                selectAllVisibleExportableRows()
            } label: {
                Label("action.select_all", systemImage: "checkmark.circle")
            }
            .disabled(!visibleRowsContainExportableItem)
            .accessibilityLabel("accessibility.recordings.select_all_visible")
            Button {
                clearVisibleExportSelection()
            } label: {
                Label("action.select_none", systemImage: "xmark.circle")
            }
            .disabled(!visibleRowsContainSelectedItem)
            .accessibilityLabel("accessibility.recordings.select_none_visible")
            Spacer()
            Text("status.items.count \(sortedRows.count)")
                .foregroundStyle(.secondary)
        }
        .font(.caption)
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
    }

    private var recordingsTable: some View {
        Table(sortedRows, sortOrder: $sortOrder) {
            TableColumn("table.column.export") { row in
                Toggle("accessibility.recording.export \(row.title)", isOn: exportBinding(for: row))
                    .labelsHidden()
                    .disabled(!row.canExport)
                    .accessibilityLabel("accessibility.recording.export \(row.title)")
            }
            .width(54)
            TableColumn("table.column.title", value: \.title) { row in
                Text(row.title)
                    .lineLimit(1)
            }
            TableColumn("table.column.folder", value: \.folderDisplayName) { row in
                Text(row.folderDisplayName)
                    .lineLimit(1)
            }
            TableColumn("table.column.recorded", value: \.recordedDate) { row in
                Text(row.recordedDate, format: .dateTime.year().month().day().hour().minute())
            }
            TableColumn("table.column.duration", value: \.duration) { row in
                Text(formatDuration(row.duration))
            }
            TableColumn("table.column.availability", value: \.availabilityLabel) { row in
                Text(row.availabilityLabel)
                    .foregroundStyle(row.canExport ? .primary : .secondary)
            }
            TableColumn("table.column.target") { row in
                if let targetPath = row.targetPath?.string {
                    Text(targetPath)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .foregroundStyle(.secondary)
                } else {
                    Text("prompt.choose_output_directory")
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .disabled(viewModel.isLoading)
    }

    private var sortedRows: [RecordingRow] {
        viewModel.filteredRows.sorted(using: sortOrder)
    }

    private var visibleRowsContainExportableItem: Bool {
        sortedRows.contains { $0.canExport }
    }

    private var visibleRowsContainSelectedItem: Bool {
        sortedRows.contains { viewModel.selectedIDs.contains($0.id) }
    }

    private var folderFilterBinding: Binding<String?> {
        Binding(
            get: { viewModel.folderFilter },
            set: { viewModel.folderFilter = $0 }
        )
    }

    private func exportBinding(for row: RecordingRow) -> Binding<Bool> {
        Binding(
            get: { viewModel.selectedIDs.contains(row.id) },
            set: { viewModel.setExportSelection(row: row, isSelected: $0) }
        )
    }

    private func selectAllVisibleExportableRows() {
        let visibleExportableIDs = sortedRows
            .filter(\.canExport)
            .map(\.id)
        viewModel.selectedIDs.formUnion(visibleExportableIDs)
    }

    private func clearVisibleExportSelection() {
        let visibleIDs = Set(sortedRows.map(\.id))
        viewModel.selectedIDs.subtract(visibleIDs)
    }

    private var fullDiskAccessPanel: some View {
        VStack(spacing: 16) {
            Image(systemName: "lock.shield")
                .font(.system(size: 42))
                .foregroundStyle(.orange)
                .accessibilityHidden(true)
            Text("ui.full_disk_access.title")
                .font(.title3.weight(.semibold))
            Text("ui.full_disk_access.message")
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 560)
            HStack(spacing: 10) {
                Button {
                    viewModel.showAppInFinder()
                } label: {
                    Label("action.show_app_in_finder", systemImage: "magnifyingglass")
                }
                Button {
                    viewModel.openFullDiskAccessSettings()
                } label: {
                    Label("action.open_settings", systemImage: "gearshape")
                }
                Button {
                    viewModel.checkAccessAndRefresh()
                } label: {
                    Label("action.refresh", systemImage: "arrow.clockwise")
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(24)
    }

    private var statusBar: some View {
        HStack {
            if let error = viewModel.errorMessage {
                VStack(alignment: .leading, spacing: 2) {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.red)
                    if viewModel.needsFullDiskAccessHelp, viewModel.fullDiskAccessMessage == nil {
                        HStack(spacing: 8) {
                            Text("ui.full_disk_access.status_hint")
                                .foregroundStyle(.secondary)
                            Button {
                                viewModel.openFullDiskAccessSettings()
                            } label: {
                                Label("action.open_settings", systemImage: "gearshape")
                            }
                            .buttonStyle(.link)
                        }
                    }
                }
            } else if let summary = viewModel.lastExportSummary {
                Text("status.export_summary \(summary.succeeded) \(summary.skipped) \(summary.failed)")
                    .foregroundStyle(.secondary)
            } else {
                if let outputDirectory = viewModel.outputDirectory?.string {
                    Text(outputDirectory)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .foregroundStyle(.secondary)
                } else {
                    Text("status.choose_output_before_export")
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            if viewModel.isLoading || viewModel.isExporting {
                ProgressView()
                    .controlSize(.small)
            }
        }
        .font(.caption)
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
    }

    private func formatDuration(_ seconds: TimeInterval) -> String {
        let total = max(0, Int(seconds.rounded()))
        return String(format: "%02d:%02d:%02d", total / 3600, (total / 60) % 60, total % 60)
    }
}
