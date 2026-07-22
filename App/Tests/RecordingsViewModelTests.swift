import Foundation
import System
import Testing
@testable import exVMs

@MainActor
struct RecordingsViewModelTests {
    @Test
    func exportIsDisabledUntilDirectoryAndExportableSelectionExist() async {
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "Local", availability: .inRecordingsDirectory),
            .fixture(id: 2, title: "Cloud", availability: .needsDownload),
        ])
        let viewModel = makeViewModel(client: client)

        await viewModel.refreshAsync()
        #expect(!viewModel.canExportSelection)

        viewModel.outputDirectory = FilePath(NSTemporaryDirectory())
        #expect(!viewModel.canExportSelection)

        viewModel.selectedIDs = [2]
        #expect(!viewModel.canExportSelection)

        viewModel.selectedIDs = [1]
        #expect(viewModel.canExportSelection)
    }

    @Test
    func exportedFileDisablesSelection() async throws {
        let tmp = try temporaryDirectory()
        let source = tmp.appendingPathComponent("Local-source.m4a")
        let payload = Data("hello-export".utf8)
        FileManager.default.createFile(atPath: source.path, contents: payload)
        let target = tmp.appendingPathComponent("Local.m4a")
        FileManager.default.createFile(atPath: target.path, contents: payload)

        let client = MockExvmsClient(recordings: [
            .fixture(
                id: 1,
                title: "Local",
                availability: .inRecordingsDirectory,
                sourcePath: FilePath(source.path)
            ),
        ])
        let viewModel = makeViewModel(client: client)
        viewModel.outputDirectory = FilePath(tmp.path)
        await viewModel.refreshAsync()
        #expect(await waitUntil { viewModel.rows.first?.exported == true })
        viewModel.selectedIDs = [1]

        #expect(!viewModel.canExportSelection)
        #expect(viewModel.rows.first?.exported == true)
        #expect(viewModel.rows.first?.exportedPath == FilePath(target.path))
    }

    @Test
    func exportedStateRequiresHashMatch() async throws {
        let tmp = try temporaryDirectory()
        let source = tmp.appendingPathComponent("Local-source.m4a")
        let target = tmp.appendingPathComponent("Local.m4a")
        FileManager.default.createFile(atPath: source.path, contents: Data("source-audio".utf8))
        FileManager.default.createFile(atPath: target.path, contents: Data("different-audio".utf8))

        let client = MockExvmsClient(recordings: [
            .fixture(
                id: 1,
                title: "Local",
                availability: .inRecordingsDirectory,
                sourcePath: FilePath(source.path)
            ),
        ])
        let viewModel = makeViewModel(client: client)
        viewModel.outputDirectory = FilePath(tmp.path)
        await viewModel.refreshAsync()

        #expect(viewModel.rows.first?.exported == false)
        #expect(viewModel.rows.first?.exportedPath == nil)
    }

    @Test
    func exportedStateRefreshesAfterAvailabilityResolves() async throws {
        let tmp = try temporaryDirectory()
        let source = tmp.appendingPathComponent("Deferred-source.m4a")
        let payload = Data("deferred-exported-audio".utf8)
        FileManager.default.createFile(atPath: source.path, contents: payload)
        let target = tmp.appendingPathComponent("Deferred.m4a")
        FileManager.default.createFile(atPath: target.path, contents: payload)

        let client = MockExvmsClient(recordings: [
            .fixture(
                id: 1,
                title: "Deferred",
                availability: .checking,
                sourcePath: FilePath(source.path)
            ),
        ])
        let viewModel = makeViewModel(client: client) { _ in
            (.inRecordingsDirectory, nil)
        }
        viewModel.outputDirectory = FilePath(tmp.path)

        await viewModel.refreshAsync()
        #expect(await waitUntil {
            viewModel.rows.first?.recording.availability == .inRecordingsDirectory
                && viewModel.rows.first?.exported == true
        })

        #expect(viewModel.rows.first?.recording.availability == .inRecordingsDirectory)
        #expect(viewModel.rows.first?.exported == true)
        #expect(viewModel.rows.first?.exportedPath == FilePath(target.path))
    }

    @Test
    func successfulExportRefreshesAndClearsSelection() async {
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "Local", availability: .inRecordingsDirectory),
        ])
        let viewModel = makeViewModel(client: client)
        await viewModel.refreshAsync()
        viewModel.outputDirectory = FilePath(NSTemporaryDirectory())
        viewModel.selectedIDs = [1]

        await viewModel.exportSelectionAsync()

        #expect(viewModel.selectedIDs.isEmpty)
        #expect(viewModel.lastExportSummary == ExportSummary(succeeded: 1, skipped: 0, failed: 0))
        #expect(client.exportedIDs == [1])
        #expect(client.exportedGrouping == .folder)
        #expect(client.listRecordingsCallCount == 1)
    }

    @Test
    func cancelExportReleasesExportingState() async throws {
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "Local", availability: .inRecordingsDirectory),
        ])
        client.exportDelay = .seconds(60)
        let viewModel = makeViewModel(client: client)
        await viewModel.refreshAsync()
        viewModel.outputDirectory = FilePath(NSTemporaryDirectory())
        viewModel.selectedIDs = [1]

        viewModel.exportSelection()
        try await Task.sleep(for: .milliseconds(50))
        #expect(viewModel.isExporting)

        viewModel.cancelExport()
        #expect(!viewModel.isExporting)
        #expect(viewModel.errorMessage == String(localized: "error.export.cancelled"))
    }

    @Test
    func directExportCancellationPropagatesToClient() async throws {
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "Local", availability: .inRecordingsDirectory),
        ])
        client.exportDelay = .seconds(60)
        let viewModel = makeViewModel(client: client)
        await viewModel.refreshAsync()
        viewModel.outputDirectory = FilePath(NSTemporaryDirectory())
        viewModel.selectedIDs = [1]

        let task = Task {
            await viewModel.exportSelectionAsync()
        }
        try await Task.sleep(for: .milliseconds(50))
        task.cancel()
        await task.value

        #expect(!viewModel.isExporting)
        #expect(client.exportWasCancelled)
    }

    @Test
    func exportUsesSelectedGrouping() async {
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "Local", availability: .inRecordingsDirectory),
        ])
        let viewModel = makeViewModel(client: client)
        await viewModel.refreshAsync()
        viewModel.outputDirectory = FilePath(NSTemporaryDirectory())
        viewModel.exportGrouping = [.folder, .date]
        viewModel.selectedIDs = [1]

        await viewModel.exportSelectionAsync()

        #expect(client.exportedGrouping == [.folder, .date])
    }

    @Test
    func outputDirectoryChangeDoesNotResetResolvedAvailability() async throws {
        let firstOutput = try temporaryDirectory()
        let secondOutput = try temporaryDirectory()
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "Pending", availability: .checking),
        ])
        let viewModel = makeViewModel(client: client)

        await viewModel.refreshAsync()
        #expect(await waitUntil { viewModel.rows.first?.recording.availability == .needsDownload })
        #expect(viewModel.rows.first?.recording.availability == .needsDownload)

        viewModel.outputDirectory = FilePath(firstOutput.path)
        try await Task.sleep(for: .milliseconds(50))
        #expect(viewModel.rows.first?.recording.availability == .needsDownload)

        viewModel.outputDirectory = FilePath(secondOutput.path)
        try await Task.sleep(for: .milliseconds(50))
        #expect(viewModel.rows.first?.recording.availability == .needsDownload)
    }

    @Test
    func initialAvailabilityInspectionOnlyPrefetchesFirstRows() async {
        let recorder = AvailabilityCallRecorder()
        let recordings = (1...20).map {
            VoiceMemoRecording.fixture(
                id: Int64($0),
                title: "Pending \($0)",
                availability: .checking
            )
        }
        let client = MockExvmsClient(recordings: recordings)
        let viewModel = makeViewModel(client: client) { recording in
            await recorder.resolve(recording.id)
        }

        await viewModel.refreshAsync()

        #expect(await waitUntilAsync { await recorder.inspectedIDs().count == 16 })
        #expect(await recorder.inspectedIDs() == Set((1...16).map(Int64.init)))
        #expect(await recorder.maximumConcurrentChecks() <= 4)
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
    func filenameRecipePersistsAndRestores() throws {
        let defaults = try temporaryUserDefaults()
        let recipe = FilenameRecipe(parts: [
            .token(UUID(), .year),
            .literal(UUID(), "-"),
            .token(UUID(), .title),
        ])
        let firstViewModel = RecordingsViewModel(
            client: MockExvmsClient(recordings: []),
            userDefaults: defaults
        )
        firstViewModel.filenameRecipe = recipe

        let restoredViewModel = RecordingsViewModel(
            client: MockExvmsClient(recordings: []),
            userDefaults: defaults
        )
        #expect(restoredViewModel.filenameRecipe == recipe)
    }

    @Test
    func exportGroupingPersistsAndRestoresRawValue() throws {
        let defaults = try temporaryUserDefaults()
        let firstViewModel = RecordingsViewModel(
            client: MockExvmsClient(recordings: []),
            userDefaults: defaults
        )
        firstViewModel.exportGrouping = .date

        let restoredViewModel = RecordingsViewModel(
            client: MockExvmsClient(recordings: []),
            userDefaults: defaults
        )

        #expect(defaults.integer(forKey: RecordingsViewModel.exportGroupingDefaultsKey) == Int(ExvmsExportGrouping.date.rawValue))
        #expect(restoredViewModel.exportGrouping == .date)
    }

    @Test
    func invalidPersistedFilenameRecipeFallsBackToDefault() throws {
        let defaults = try temporaryUserDefaults()
        defaults.set(Data("invalid-json".utf8), forKey: RecordingsViewModel.filenameRecipeDefaultsKey)

        let viewModel = RecordingsViewModel(
            client: MockExvmsClient(recordings: []),
            userDefaults: defaults
        )

        #expect(viewModel.filenameRecipe == .default)
    }

    @Test
    func folderFilterReturnsMatchingRowsOnly() async {
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "A", availability: .inRecordingsDirectory, folderName: "Work"),
            .fixture(id: 2, title: "B", availability: .inRecordingsDirectory, folderName: "Personal"),
        ])
        let viewModel = makeViewModel(client: client)

        await viewModel.refreshAsync()
        viewModel.folderFilter = "Work"

        #expect(viewModel.folderOptions == ["Personal", "Work"])
        #expect(viewModel.filteredRows.map(\.id) == [1])
    }

    @Test
    func deletedRecordingsAreHiddenUntilEnabled() async {
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "Available", availability: .inRecordingsDirectory),
            .fixture(
                id: 2,
                title: "Deleted",
                availability: .inRecordingsDirectory,
                isDeleted: true
            ),
        ])
        let viewModel = makeViewModel(client: client)

        await viewModel.refreshAsync()
        #expect(viewModel.rows.map(\.id) == [1])

        viewModel.showDeleted = true
        #expect(await waitUntil { viewModel.rows.map(\.id) == [1, 2] })
        #expect(viewModel.rows.map(\.id) == [1, 2])
        #expect(viewModel.rows.last?.recording.isDeleted == true)
    }

    @Test
    func deletedPreferencePersists() throws {
        let defaults = try temporaryUserDefaults()
        let firstViewModel = RecordingsViewModel(
            client: MockExvmsClient(recordings: []),
            userDefaults: defaults
        )
        firstViewModel.showDeleted = true

        let restoredViewModel = RecordingsViewModel(
            client: MockExvmsClient(recordings: []),
            userDefaults: defaults
        )
        #expect(restoredViewModel.showDeleted)
    }

    @Test
    func exportedStateChecksMacOSSuffixCandidates() async throws {
        let tmp = try temporaryDirectory()
        let source = tmp.appendingPathComponent("A_B-source.m4a")
        let payload = Data("exported-audio".utf8)
        FileManager.default.createFile(atPath: source.path, contents: payload)
        let target = tmp.appendingPathComponent("A_B 2.m4a")
        FileManager.default.createFile(atPath: target.path, contents: payload)

        let client = MockExvmsClient(recordings: [
            .fixture(
                id: 1,
                title: "A/B",
                availability: .inRecordingsDirectory,
                sourcePath: FilePath(source.path)
            ),
        ])
        let viewModel = makeViewModel(client: client)
        viewModel.outputDirectory = FilePath(tmp.path)
        await viewModel.refreshAsync()
        #expect(await waitUntil { viewModel.rows.first?.exported == true })

        #expect(viewModel.rows.first?.exported == true)
        #expect(viewModel.rows.first?.exportedPath == FilePath(target.path))
    }

    @Test
    func cannotSelectNonExportableRowForExport() async {
        let client = MockExvmsClient(recordings: [
            .fixture(id: 1, title: "Cloud", availability: .needsDownload),
        ])
        let viewModel = makeViewModel(client: client)

        await viewModel.refreshAsync()
        let row = viewModel.rows[0]
        viewModel.setExportSelection(row: row, isSelected: true)

        #expect(viewModel.selectedIDs.isEmpty)
    }

    @Test
    func permissionErrorsEnableFullDiskAccessGuidance() async {
        let client = MockExvmsClient(
            recordings: [],
            listError: ExvmsClientError.rust("permission denied reading Voice Memos data at /Users/example/Library")
        )
        let viewModel = makeViewModel(client: client)

        await viewModel.refreshAsync()

        #expect(viewModel.needsFullDiskAccessHelp)
    }

    @Test
    func sqliteOpenFailuresEnableFullDiskAccessGuidance() async {
        let client = MockExvmsClient(
            recordings: [],
            listError: ExvmsClientError.rust("SQLite error: unable to open database file: /Users/example/CloudRecordings.db")
        )
        let viewModel = makeViewModel(client: client)

        await viewModel.refreshAsync()

        #expect(viewModel.needsFullDiskAccessHelp)
        #expect(viewModel.fullDiskAccessMessage != nil)
    }

    @Test
    func localSourceFileEnablesPreview() async throws {
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
                    availability: .inRecordingsDirectory,
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
        let viewModel = makeViewModel(client: client)

        await viewModel.refreshAsync()

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

@MainActor
private func makeViewModel(
    client: MockExvmsClient,
    availabilityResolver: RecordingsViewModel.AvailabilityResolver? = nil
) -> RecordingsViewModel {
    let suiteName = "RecordingsViewModelTests.\(UUID().uuidString)"
    let defaults = UserDefaults(suiteName: suiteName)!
    defaults.removePersistentDomain(forName: suiteName)
    guard let availabilityResolver else {
        return RecordingsViewModel(client: client, userDefaults: defaults)
    }
    return RecordingsViewModel(
        client: client,
        userDefaults: defaults,
        availabilityResolver: availabilityResolver
    )
}

@MainActor
private func waitUntil(
    timeout: Duration = .seconds(1),
    condition: @escaping @MainActor () -> Bool
) async -> Bool {
    let clock = ContinuousClock()
    let deadline = clock.now + timeout
    while !condition() && clock.now < deadline {
        try? await Task.sleep(for: .milliseconds(10))
    }
    return condition()
}

private func waitUntilAsync(
    timeout: Duration = .seconds(1),
    condition: @escaping @Sendable () async -> Bool
) async -> Bool {
    let clock = ContinuousClock()
    let deadline = clock.now + timeout
    while !(await condition()) && clock.now < deadline {
        try? await Task.sleep(for: .milliseconds(10))
    }
    return await condition()
}

private actor AvailabilityCallRecorder {
    private var ids = Set<Int64>()
    private var activeChecks = 0
    private var maximumActiveChecks = 0

    func resolve(_ id: Int64) async -> (RecordingAvailability, String?) {
        ids.insert(id)
        activeChecks += 1
        maximumActiveChecks = max(maximumActiveChecks, activeChecks)
        try? await Task.sleep(for: .milliseconds(10))
        activeChecks -= 1
        return (.needsDownload, nil)
    }

    func inspectedIDs() -> Set<Int64> {
        ids
    }

    func maximumConcurrentChecks() -> Int {
        maximumActiveChecks
    }
}

private final class MockExvmsClient: ExvmsClient, @unchecked Sendable {
    var exportedIDs: [Int64] = []
    var exportedGrouping: ExvmsExportGrouping?
    var listRecordingsCallCount = 0
    var exportDelay: Duration?
    var exportWasCancelled = false
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

    func listRecordings() async throws -> RecordingCatalog {
        listRecordingsCallCount += 1
        if let listError {
            throw listError
        }
        return RecordingCatalog(
            store: store,
            recordings: recordings
        )
    }

    func exportRecordings(
        ids: [Int64],
        outputDirectory: FilePath,
        template: String,
        exportGrouping: ExvmsExportGrouping,
        jobs: Int,
        cancellationToken: ExvmsCancellationToken
    ) async throws -> ExportSummary {
        exportedIDs = ids
        exportedGrouping = exportGrouping
        if let exportDelay {
            do {
                try await Task.sleep(for: exportDelay)
            } catch is CancellationError {
                exportWasCancelled = true
                cancellationToken.cancel()
                throw CancellationError()
            }
        }
        return ExportSummary(succeeded: ids.count, skipped: 0, failed: 0)
    }
}

private extension VoiceMemoRecording {
    static func fixture(
        id: Int64,
        title: String,
        availability: RecordingAvailability,
        folderName: String? = nil,
        isDeleted: Bool = false,
        sourcePath: FilePath? = nil
    ) -> VoiceMemoRecording {
        VoiceMemoRecording(
            id: id,
            title: title,
            recordedAt: 1,
            duration: 2,
            localDuration: availability.isExportable ? 2 : 0,
            isDeleted: isDeleted,
            databasePath: FilePath("\(title).m4a"),
            sourcePath: sourcePath,
            pathKind: .relative,
            folderName: folderName,
            availability: availability,
            availabilityDetail: nil
        )
    }
}
