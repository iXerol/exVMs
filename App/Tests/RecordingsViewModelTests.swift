import Foundation
import System
import Testing
@testable import exVMsApp

@MainActor
struct RecordingsViewModelTests {
    @Test
    func exportIsDisabledUntilDirectoryAndExportableSelectionExist() {
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "Local", availability: .local),
            .fixture(id: 2, title: "Cloud", availability: .needsDownload),
        ])
        let viewModel = RecordingsViewModel(client: client)

        viewModel.refresh()
        #expect(!viewModel.canExportSelection)

        viewModel.outputDirectory = FilePath(NSTemporaryDirectory())
        #expect(!viewModel.canExportSelection)

        viewModel.selectedIDs = [2]
        #expect(!viewModel.canExportSelection)

        viewModel.selectedIDs = [1]
        #expect(viewModel.canExportSelection)
    }

    @Test
    func exportedFileDisablesSelection() throws {
        let tmp = try temporaryDirectory()
        let target = tmp.appendingPathComponent("Local.m4a")
        FileManager.default.createFile(atPath: target.path, contents: Data())

        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "Local", availability: .local),
        ])
        let viewModel = RecordingsViewModel(client: client)
        viewModel.refresh()
        viewModel.outputDirectory = FilePath(tmp.path)
        viewModel.selectedIDs = [1]

        #expect(!viewModel.canExportSelection)
        #expect(viewModel.rows.first?.exported == true)
    }

    @Test
    func successfulExportRefreshesAndClearsSelection() {
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "Local", availability: .local),
        ])
        let viewModel = RecordingsViewModel(client: client)
        viewModel.refresh()
        viewModel.outputDirectory = FilePath(NSTemporaryDirectory())
        viewModel.selectedIDs = [1]

        viewModel.exportSelection()

        #expect(viewModel.selectedIDs.isEmpty)
        #expect(viewModel.lastExportSummary == ExportSummary(succeeded: 1, skipped: 0, failed: 0))
        #expect(client.exportedIDs == [1])
    }

    @Test
    func outputDirectoryPersistsAndRestoresExistingDirectory() throws {
        let defaults = try temporaryUserDefaults()
        let tmp = try temporaryDirectory()
        let firstViewModel = RecordingsViewModel(
            client: MockExvmsClient(recordings: []),
            userDefaults: defaults
        )

        firstViewModel.outputDirectory = FilePath(tmp.path)
        let restoredViewModel = RecordingsViewModel(
            client: MockExvmsClient(recordings: []),
            userDefaults: defaults
        )

        #expect(defaults.string(forKey: RecordingsViewModel.outputDirectoryDefaultsKey) == tmp.path)
        #expect(restoredViewModel.outputDirectory == FilePath(tmp.path))
    }

    @Test
    func missingPersistedOutputDirectoryIsIgnored() throws {
        let defaults = try temporaryUserDefaults()
        let missingPath = NSTemporaryDirectory() + UUID().uuidString
        defaults.set(missingPath, forKey: RecordingsViewModel.outputDirectoryDefaultsKey)

        let viewModel = RecordingsViewModel(
            client: MockExvmsClient(recordings: []),
            userDefaults: defaults
        )

        #expect(viewModel.outputDirectory == nil)
    }

    @Test
    func folderFilterReturnsMatchingRowsOnly() {
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "A", availability: .local, folderName: "Work"),
            .fixture(id: 2, title: "B", availability: .local, folderName: "Personal"),
        ])
        let viewModel = RecordingsViewModel(client: client)

        viewModel.refresh()
        viewModel.folderFilter = "Work"

        #expect(viewModel.folderOptions == ["Personal", "Work"])
        #expect(viewModel.filteredRows.map(\.id) == [1])
    }

    @Test
    func cannotSelectNonExportableRowForExport() {
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "Cloud", availability: .needsDownload),
        ])
        let viewModel = RecordingsViewModel(client: client)

        viewModel.refresh()
        let row = viewModel.rows[0]
        viewModel.setExportSelection(row: row, isSelected: true)

        #expect(viewModel.selectedIDs.isEmpty)
    }

    @Test
    func permissionErrorsEnableFullDiskAccessGuidance() {
        let client = MockExvmsClient(
            recordings: [],
            listError: ExvmsClientError.rust("permission denied reading Voice Memos data at /Users/example/Library")
        )
        let viewModel = RecordingsViewModel(client: client)

        viewModel.refresh()

        #expect(viewModel.needsFullDiskAccessHelp)
    }

    @Test
    func sqliteOpenFailuresEnableFullDiskAccessGuidance() {
        let client = MockExvmsClient(
            recordings: [],
            listError: ExvmsClientError.rust("SQLite error: unable to open database file: /Users/example/CloudRecordings.db")
        )
        let viewModel = RecordingsViewModel(client: client)

        viewModel.refresh()

        #expect(viewModel.needsFullDiskAccessHelp)
        #expect(viewModel.fullDiskAccessMessage != nil)
    }

    @Test
    func localSourceFileEnablesPreview() throws {
        let tmp = try temporaryDirectory()
        let source = tmp.appendingPathComponent("Local.m4a")
        FileManager.default.createFile(atPath: source.path, contents: Data([1]))
        let client = MockExvmsClient(
            store: VoiceMemosStore(
                recordingsDirectory: FilePath(tmp.path),
                databasePath: FilePath(tmp.appendingPathComponent("CloudRecordings.db").path)
            ),
            recordings: [
                .fixture(
                    id: 1,
                    title: "Local",
                    availability: .local,
                    sourcePath: FilePath(source.path)
                ),
                .fixture(
                    id: 2,
                    title: "Cloud",
                    availability: .needsDownload,
                    sourcePath: FilePath(source.path)
                ),
            ]
        )
        let viewModel = RecordingsViewModel(client: client)

        viewModel.refresh()

        #expect(viewModel.rows[0].canPreview)
        #expect(!viewModel.rows[1].canPreview)
    }

    private func temporaryDirectory() throws -> URL {
        let url = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    private func temporaryUserDefaults() throws -> UserDefaults {
        let suiteName = "com.ixerol.exvms.tests.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        return defaults
    }
}

private final class MockExvmsClient: ExvmsClient {
    var exportedIDs: [Int64] = []
    private var store: VoiceMemosStore
    private var recordings: [VoiceMemoRecording]
    private var listError: Error?

    init(
        store: VoiceMemosStore = VoiceMemosStore(
            recordingsDirectory: FilePath("/tmp/Recordings"),
            databasePath: FilePath("/tmp/Recordings/CloudRecordings.db")
        ),
        recordings: [VoiceMemoRecording],
        listError: Error? = nil
    ) {
        self.store = store
        self.recordings = recordings
        self.listError = listError
    }

    func listRecordings() throws -> RecordingCatalog {
        if let listError {
            throw listError
        }
        return RecordingCatalog(
            store: store,
            recordings: recordings
        )
    }

    func exportRecordings(ids: [Int64], outputDirectory: FilePath, template: String, jobs: Int) throws -> ExportSummary {
        exportedIDs = ids
        return ExportSummary(succeeded: ids.count, skipped: 0, failed: 0)
    }
}

private extension VoiceMemoRecording {
    static func fixture(
        id: Int64,
        title: String,
        availability: RecordingAvailability,
        folderName: String? = nil,
        sourcePath: FilePath? = nil
    ) -> VoiceMemoRecording {
        VoiceMemoRecording(
            id: id,
            title: title,
            recordedAt: 1,
            duration: 2,
            localDuration: availability.isExportable ? 2 : 0,
            databasePath: FilePath("\(title).m4a"),
            sourcePath: sourcePath,
            pathKind: .relative,
            folderName: folderName,
            availability: availability,
            availabilityDetail: nil
        )
    }
}
