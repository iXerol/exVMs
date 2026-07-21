import AVFoundation
import Observation
import CryptoKit
import System
import SwiftUI

struct RecordingRow: Identifiable, Hashable {
    var recording: VoiceMemoRecording
    var exported: Bool
    var exportedPath: FilePath?
    var sourceFileSizeBytes: Int64?

    var id: Int64 { recording.id }
    var canExport: Bool { recording.availability.isExportable && !exported }
    var canPreview: Bool {
        guard recording.availability == .inRecordingsDirectory,
              let sourcePath = recording.sourcePath else {
            return false
        }
        return FileManager.default.fileExists(atPath: sourcePath.string)
    }
    var title: String { recording.title }
    var folderDisplayName: String { recording.folderName?.nilIfEmpty ?? ExVMsStrings.Folder.none }
    var recordedDate: Date { recording.recordingDate }
    var duration: TimeInterval { recording.duration }
    var availabilityLabel: String { exported ? ExVMsStrings.Availability.exported : recording.availability.label }
    var sizeLabel: String {
        guard let sourceFileSizeBytes else {
            return "—"
        }
        return ByteCountFormatter.string(fromByteCount: sourceFileSizeBytes, countStyle: .file)
    }
    var sizeSortValue: Int64 { sourceFileSizeBytes ?? -1 }
}

private struct ExportInspection: Sendable {
    var exportedPath: FilePath?

    var exported: Bool {
        exportedPath != nil
    }
}

private struct AvailabilityQueueState {
    var generation = 0
    var pendingIDs: [Int64] = []
    var pendingIDSet = Set<Int64>()
    var tasks: [Int64: Task<Void, Never>] = [:]

    mutating func cancelAll() {
        generation += 1
        for task in tasks.values {
            task.cancel()
        }
        tasks.removeAll()
        pendingIDs.removeAll()
        pendingIDSet.removeAll()
    }
}

private struct ExportInspectionQueueState {
    var generation = 0
    var pendingIDs: [Int64] = []
    var pendingIDSet = Set<Int64>()
    var tasks: [Int64: Task<Void, Never>] = [:]
    var results: [Int64: ExportInspection] = [:]

    mutating func cancelAllAndClearResults() {
        generation += 1
        for task in tasks.values {
            task.cancel()
        }
        tasks.removeAll()
        pendingIDs.removeAll()
        pendingIDSet.removeAll()
        results.removeAll()
    }
}

@MainActor
@Observable
final class RecordingsViewModel {
    typealias AvailabilityResolver = @Sendable (VoiceMemoRecording) async -> (RecordingAvailability, String?)

    private static let availabilityConcurrencyLimit = 4
    private static let exportInspectionConcurrencyLimit = 2
    private static let prefetchCount = 16

    static let outputDirectoryDefaultsKey = "com.ixerol.exvms.output-directory"
    static let filenameRecipeDefaultsKey = "com.ixerol.exvms.filename-recipe"
    static let exportGroupingDefaultsKey = "com.ixerol.exvms.export-grouping"
    static let showDeletedDefaultsKey = "com.ixerol.exvms.show-deleted"

    private(set) var store: VoiceMemosStore?
    private(set) var rows: [RecordingRow] = []
    var selectedIDs = Set<Int64>() {
        didSet {
            let allowed = Set(rows.filter(\.canExport).map(\.id))
            let filtered = selectedIDs.intersection(allowed)
            if filtered != selectedIDs {
                selectedIDs = filtered
            }
        }
    }
    var folderFilter: String?
    var outputDirectory: FilePath? {
        didSet {
            configureOutputMonitor()
            invalidateExportInspections()
            rebuildRows()
        }
    }

    var filenameRecipe = FilenameRecipe.default {
        didSet {
            persistFilenameRecipe()
            invalidateExportInspections()
            rebuildRows()
        }
    }
    var exportGrouping = ExvmsExportGrouping.defaultTarget {
        didSet {
            userDefaults.set(Int(exportGrouping.rawValue), forKey: Self.exportGroupingDefaultsKey)
            invalidateExportInspections()
            rebuildRows()
        }
    }
    var showDeleted = false {
        didSet {
            userDefaults.set(showDeleted, forKey: Self.showDeletedDefaultsKey)
            rebuildRows()
        }
    }
    private(set) var errorMessage: String?
    private(set) var fullDiskAccessMessage: String?
    private(set) var isLoading = false
    private(set) var isExporting = false
    private(set) var lastExportSummary: ExportSummary?

    @ObservationIgnored
    private let client: ExvmsClient
    @ObservationIgnored
    private let availabilityResolver: AvailabilityResolver
    @ObservationIgnored
    private let userDefaults: UserDefaults
    @ObservationIgnored
    private let outputMonitor = DirectoryMonitor()
    @ObservationIgnored
    private let quickLookPreviewController = QuickLookPreviewController()
    @ObservationIgnored
    private let hashCache = FileHashCache()
    @ObservationIgnored
    private var recordings: [VoiceMemoRecording] = []
    @ObservationIgnored
    private var availabilityQueue = AvailabilityQueueState()
    @ObservationIgnored
    private var exportInspectionQueue = ExportInspectionQueueState()

    init(
        client: ExvmsClient,
        userDefaults: UserDefaults = .standard,
        availabilityResolver: @escaping AvailabilityResolver = RecordingsViewModel.resolveAvailability(recording:)
    ) {
        self.client = client
        self.availabilityResolver = availabilityResolver
        self.userDefaults = userDefaults
        self.outputDirectory = Self.loadPersistedOutputDirectory(from: userDefaults)
        self.filenameRecipe = Self.loadPersistedFilenameRecipe(from: userDefaults) ?? .default
        self.exportGrouping = Self.loadPersistedExportGrouping(from: userDefaults)
        self.showDeleted = userDefaults.bool(forKey: Self.showDeletedDefaultsKey)
        configureOutputMonitor()
    }

    var canExportSelection: Bool {
        outputDirectory != nil && !selectedIDs.isEmpty && rows.contains { selectedIDs.contains($0.id) && $0.canExport }
    }

    var filteredRows: [RecordingRow] {
        guard let folderFilter else {
            return rows
        }
        return rows.filter { $0.folderDisplayName == folderFilter }
    }

    var folderOptions: [String] {
        Array(Set(rows.map(\.folderDisplayName))).sorted()
    }

    var needsFullDiskAccessHelp: Bool {
        if fullDiskAccessMessage != nil {
            return true
        }
        guard let errorMessage else {
            return false
        }
        return errorMessage.localizedCaseInsensitiveContains("permission denied reading Voice Memos data")
            || errorMessage.localizedCaseInsensitiveContains("operation not permitted")
            || errorMessage.localizedCaseInsensitiveContains("SQLite error: unable to open database file")
    }

    func checkAccessAndRefresh() {
        Task {
            await checkAccessAndRefreshAsync()
        }
    }

    func refresh() {
        Task {
            await refreshAsync()
        }
    }

    func exportSelection() {
        Task {
            await exportSelectionAsync()
        }
    }

    func checkAccessAndRefreshAsync() async {
        isLoading = true
        errorMessage = nil
        fullDiskAccessMessage = nil

        if let message = Self.voiceMemosAccessDeniedMessage() {
            fullDiskAccessMessage = message
            errorMessage = message
            rows = []
            recordings = []
            isLoading = false
            return
        }

        await refreshAsync()
    }

    func refreshAsync() async {
        isLoading = true
        errorMessage = nil
        fullDiskAccessMessage = nil
        do {
            let catalog = try await client.listRecordings()
            store = catalog.store
            recordings = catalog.recordings
            cancelAvailabilityChecks()
            invalidateExportInspections()
            await rebuildRowsAsync()
            isLoading = false
            requestInitialInspections()
        } catch {
            errorMessage = error.localizedDescription
            if needsFullDiskAccessHelp {
                fullDiskAccessMessage = ExVMsStrings.Error.VoiceMemosAccess.generic
            }
            isLoading = false
        }
    }

    func openRecordingsDirectory(){
        guard let recordingsDirectory = store?.recordingsDirectory else {
            return
        }
        NSWorkspace.shared.activateFileViewerSelecting([recordingsDirectory.fileURL(isDirectory: true)])
    }

    func chooseOutputDirectory() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = true
        panel.prompt = ExVMsStrings.Action.choose
        if panel.runModal() == .OK, let url = panel.url {
            outputDirectory = FilePath(url.path)
        }
    }

    func showAppInFinder() {
        NSWorkspace.shared.activateFileViewerSelecting([Bundle.main.bundleURL])
    }

    func openFullDiskAccessSettings() {
        guard let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles") else {
            return
        }
        NSWorkspace.shared.open(url)
    }

    func setExportSelection(row: RecordingRow, isSelected: Bool) {
        if isSelected, row.canExport {
            selectedIDs.insert(row.id)
        } else {
            selectedIDs.remove(row.id)
        }
    }

    func preview(row: RecordingRow) {
        guard row.canPreview,
              let sourcePath = row.recording.sourcePath else {
            return
        }
        quickLookPreviewController.preview(url: sourcePath.fileURL())
    }

    func revealRecordingInFinder(row: RecordingRow) {
        guard let sourcePath = row.recording.sourcePath else {
            return
        }
        NSWorkspace.shared.activateFileViewerSelecting([sourcePath.fileURL()])
    }

    func revealExportedFileInFinder(row: RecordingRow) {
        guard let exportedPath = row.exportedPath else {
            return
        }
        NSWorkspace.shared.activateFileViewerSelecting([exportedPath.fileURL()])
    }

    func exportSelectionAsync() async {
        guard let outputDirectory, canExportSelection else {
            return
        }
        let ids = rows
            .filter { selectedIDs.contains($0.id) && $0.canExport }
            .map(\.id)
        guard !ids.isEmpty else {
            return
        }

        isExporting = true
        errorMessage = nil
        do {
            lastExportSummary = try await client.exportRecordings(
                ids: ids,
                outputDirectory: outputDirectory,
                template: filenameRecipe.rustTemplate,
                exportGrouping: exportGrouping,
                jobs: 4
            )
            selectedIDs.removeAll()
            invalidateExportInspections()
            await rebuildRowsAsync()
            let orderedIDs = rows.map(\.id)
            for id in ids {
                requestInspections(for: id, orderedIDs: orderedIDs)
            }
        } catch {
            errorMessage = error.localizedDescription
        }
        isExporting = false
    }

    private func configureOutputMonitor() {
        guard let outputDirectory else {
            outputMonitor.stop()
            return
        }
        userDefaults.set(outputDirectory.string, forKey: Self.outputDirectoryDefaultsKey)
        outputMonitor.start(url: outputDirectory.fileURL(isDirectory: true)) { [weak self] in
            Task { @MainActor in
                self?.handleOutputDirectoryChange()
            }
        }
    }

    private func handleOutputDirectoryChange() {
        invalidateExportInspections()
        rebuildRows()
        requestInitialInspections()
    }

    private func rebuildRows() {
        Task { @MainActor in
            await rebuildRowsAsync()
        }
    }

    private func rebuildRowsAsync() async {
        let displayedRecordings = if showDeleted {
            recordings
        } else {
            recordings.filter { !$0.isDeleted }
        }
        rows = Self.buildRows(
            recordings: displayedRecordings,
            exportInspections: exportInspectionQueue.results
        )
        selectedIDs = selectedIDs.intersection(Set(rows.filter(\.canExport).map(\.id)))
        if let folderFilter, !folderOptions.contains(folderFilter) {
            self.folderFilter = nil
        }
        requestInitialInspections()
    }

    func requestInspections(for visibleID: Int64, orderedIDs: [Int64]) {
        guard let visibleIndex = orderedIDs.firstIndex(of: visibleID) else {
            return
        }
        let requestedIDs = orderedIDs[visibleIndex...].prefix(Self.prefetchCount)
        for id in requestedIDs {
            enqueueAvailabilityCheck(id: id)
            enqueueExportInspection(id: id)
        }
        startAvailabilityChecks()
        startExportInspections()
    }

    private func requestInitialInspections() {
        guard let firstID = rows.first?.id else {
            return
        }
        requestInspections(for: firstID, orderedIDs: rows.map(\.id))
    }

    private func cancelAvailabilityChecks() {
        availabilityQueue.cancelAll()
    }

    private func enqueueAvailabilityCheck(id: Int64) {
        guard let recording = recordings.first(where: { $0.id == id }),
              recording.availability == .checking,
              availabilityQueue.tasks[id] == nil,
              availabilityQueue.pendingIDSet.insert(id).inserted else {
            return
        }
        availabilityQueue.pendingIDs.append(id)
    }

    private func startAvailabilityChecks() {
        while availabilityQueue.tasks.count < Self.availabilityConcurrencyLimit,
              let id = availabilityQueue.pendingIDs.first {
            availabilityQueue.pendingIDs.removeFirst()
            availabilityQueue.pendingIDSet.remove(id)
            guard let recording = recordings.first(where: { $0.id == id }),
                  recording.availability == .checking else {
                continue
            }
            let generation = availabilityQueue.generation
            let resolver = availabilityResolver
            availabilityQueue.tasks[id] = Task.detached(priority: .utility) { [weak self] in
                let result = await resolver(recording)
                guard !Task.isCancelled else {
                    return
                }
                await self?.completeAvailabilityCheck(
                    id: id,
                    result: result,
                    generation: generation
                )
            }
        }
    }

    private func completeAvailabilityCheck(
        id: Int64,
        result: (RecordingAvailability, String?),
        generation: Int
    ) {
        availabilityQueue.tasks.removeValue(forKey: id)
        guard generation == availabilityQueue.generation else {
            return
        }
        updateRecordingAvailability(id: id, availability: result.0, detail: result.1)
        if let rowIndex = rows.firstIndex(where: { $0.id == id }) {
            rows[rowIndex].recording.availability = result.0
            rows[rowIndex].recording.availabilityDetail = result.1
        }
        if let row = rows.first(where: { $0.id == id }), !row.canExport {
            selectedIDs.remove(id)
        }
        enqueueExportInspection(id: id)
        startAvailabilityChecks()
        startExportInspections()
    }

    private func invalidateExportInspections() {
        exportInspectionQueue.cancelAllAndClearResults()
    }

    private func enqueueExportInspection(id: Int64) {
        guard outputDirectory != nil,
              recordings.contains(where: { $0.id == id && $0.availability.isExportable }),
              exportInspectionQueue.results[id] == nil,
              exportInspectionQueue.tasks[id] == nil,
              exportInspectionQueue.pendingIDSet.insert(id).inserted else {
            return
        }
        exportInspectionQueue.pendingIDs.append(id)
    }

    private func startExportInspections() {
        while exportInspectionQueue.tasks.count < Self.exportInspectionConcurrencyLimit,
              let id = exportInspectionQueue.pendingIDs.first {
            exportInspectionQueue.pendingIDs.removeFirst()
            exportInspectionQueue.pendingIDSet.remove(id)
            guard let recording = recordings.first(where: { $0.id == id }),
                  let outputDirectory else {
                continue
            }
            let generation = exportInspectionQueue.generation
            let recipe = filenameRecipe
            let exportGrouping = exportGrouping
            let cache = hashCache
            exportInspectionQueue.tasks[id] = Task.detached(priority: .utility) { [weak self] in
                let inspection = Self.inspectExport(
                    recording: recording,
                    outputDirectory: outputDirectory,
                    filenameRecipe: recipe,
                    exportGrouping: exportGrouping,
                    hashCache: cache
                )
                guard !Task.isCancelled else {
                    return
                }
                await self?.completeExportInspection(
                    id: id,
                    inspection: inspection,
                    generation: generation
                )
            }
        }
    }

    private func completeExportInspection(id: Int64, inspection: ExportInspection, generation: Int) {
        exportInspectionQueue.tasks.removeValue(forKey: id)
        guard generation == exportInspectionQueue.generation else {
            return
        }
        exportInspectionQueue.results[id] = inspection
        if let rowIndex = rows.firstIndex(where: { $0.id == id }) {
            rows[rowIndex].exported = inspection.exported
            rows[rowIndex].exportedPath = inspection.exportedPath
            if inspection.exported {
                selectedIDs.remove(id)
            }
        }
        startExportInspections()
    }

    private func updateRecordingAvailability(id: Int64, availability: RecordingAvailability, detail: String?) {
        guard let recordingIndex = recordings.firstIndex(where: { $0.id == id }) else {
            return
        }
        recordings[recordingIndex].availability = availability
        recordings[recordingIndex].availabilityDetail = detail
    }

    nonisolated private static func buildRows(
        recordings: [VoiceMemoRecording],
        exportInspections: [Int64: ExportInspection]
    ) -> [RecordingRow] {
        recordings.map { buildRow(recording: $0, exportInspection: exportInspections[$0.id]) }
    }

    nonisolated private static func buildRow(
        recording: VoiceMemoRecording,
        exportInspection: ExportInspection?
    ) -> RecordingRow {
        let sourceSize = sourceFileSize(path: recording.sourcePath)
        return RecordingRow(
            recording: recording,
            exported: exportInspection?.exported ?? false,
            exportedPath: exportInspection?.exportedPath,
            sourceFileSizeBytes: sourceSize
        )
    }

    nonisolated private static func inspectExport(
        recording: VoiceMemoRecording,
        outputDirectory: FilePath,
        filenameRecipe: FilenameRecipe,
        exportGrouping: ExvmsExportGrouping,
        hashCache: FileHashCache
    ) -> ExportInspection {
        let sourceSize = sourceFileSize(path: recording.sourcePath)
        return ExportInspection(
            exportedPath: exportedPath(
                recording: recording,
                outputDirectory: outputDirectory,
                filenameRecipe: filenameRecipe,
                exportGrouping: exportGrouping,
                sourceSize: sourceSize,
                hashCache: hashCache
            )
        )
    }

    nonisolated private static func appendingNumericSuffix(_ path: FilePath, suffix: Int) -> FilePath {
        let url = path.fileURL()
        let fileName = url.lastPathComponent
        let ext = url.pathExtension
        let stem = (fileName as NSString).deletingPathExtension
        let candidateName = ext.isEmpty ? "\(stem) \(suffix)" : "\(stem) \(suffix).\(ext)"
        return FilePath(url.deletingLastPathComponent().appendingPathComponent(candidateName).path)
    }

    nonisolated private static func exportedPath(
        recording: VoiceMemoRecording,
        outputDirectory: FilePath?,
        filenameRecipe: FilenameRecipe,
        exportGrouping: ExvmsExportGrouping,
        sourceSize: Int64?,
        hashCache: FileHashCache
    ) -> FilePath? {
        guard let outputDirectory,
              let baseTarget = filenameRecipe.preview(
                for: recording,
                outputDirectory: outputDirectory,
                exportGrouping: exportGrouping
              ) else {
            return nil
        }
        guard let sourcePath = recording.sourcePath,
              FileManager.default.fileExists(atPath: sourcePath.string) else {
            return nil
        }

        guard let sourceHash = fileSHA256(path: sourcePath, cache: hashCache) else {
            return nil
        }

        for candidate in exportTargetCandidates(baseTarget) {
            guard FileManager.default.fileExists(atPath: candidate.string) else {
                continue
            }
            if let sourceSize,
               let targetSize = fileSize(path: candidate),
               sourceSize != targetSize {
                continue
            }
            if fileSHA256(path: candidate, cache: hashCache) == sourceHash {
                return candidate
            }
        }
        return nil
    }

    nonisolated private static func exportTargetCandidates(_ baseTarget: FilePath) -> [FilePath] {
        let maxNumericSuffix = 20
        return [baseTarget] + (2...maxNumericSuffix).map { appendingNumericSuffix(baseTarget, suffix: $0) }
    }

    private static func voiceMemosAccessDeniedMessage() -> String? {
        for directory in candidateVoiceMemosDirectories() {
            do {
                _ = try FileManager.default.contentsOfDirectory(
                    at: directory,
                    includingPropertiesForKeys: nil,
                    options: [.skipsHiddenFiles]
                )
                if let databaseDeniedMessage = databaseAccessDeniedMessage(in: directory) {
                    return databaseDeniedMessage
                }
                return nil
            } catch {
                if isPermissionDenied(error) {
                    return ExVMsStrings.Error.VoiceMemosAccess.path(directory.path)
                }
            }
        }
        return nil
    }

    private static func loadPersistedOutputDirectory(from userDefaults: UserDefaults) -> FilePath? {
        guard let value = userDefaults.string(forKey: outputDirectoryDefaultsKey),
              !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return nil
        }

        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: value, isDirectory: &isDirectory),
              isDirectory.boolValue else {
            return nil
        }
        return FilePath(value)
    }

    private static func loadPersistedFilenameRecipe(from userDefaults: UserDefaults) -> FilenameRecipe? {
        guard let data = userDefaults.data(forKey: filenameRecipeDefaultsKey) else {
            return nil
        }
        do {
            return try JSONDecoder().decode(FilenameRecipe.self, from: data)
        } catch {
            return nil
        }
    }

    private static func loadPersistedExportGrouping(from userDefaults: UserDefaults) -> ExvmsExportGrouping {
        guard userDefaults.object(forKey: exportGroupingDefaultsKey) != nil else {
            return .defaultTarget
        }
        let rawValue = UInt32(userDefaults.integer(forKey: exportGroupingDefaultsKey))
        let grouping = ExvmsExportGrouping(rawValue: rawValue).intersection([.folder, .date])
        return grouping.isEmpty ? .defaultTarget : grouping
    }

    private static func databaseAccessDeniedMessage(in directory: URL) -> String? {
        for filename in ["CloudRecordings.db", "Recordings.db"] {
            let databaseURL = directory.appendingPathComponent(filename)
            guard FileManager.default.fileExists(atPath: databaseURL.path) else {
                continue
            }
            do {
                let handle = try FileHandle(forReadingFrom: databaseURL)
                try handle.close()
            } catch {
                if isPermissionDenied(error) {
                    return ExVMsStrings.Error.VoiceMemosAccess.path(databaseURL.path)
                }
            }
        }
        return nil
    }

    private static func candidateVoiceMemosDirectories() -> [URL] {
        let home = FileManager.default.homeDirectoryForCurrentUser
        return [
            home.appendingPathComponent("Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings", isDirectory: true),
            home.appendingPathComponent("Library/Containers/Voice Memos/Data/Library/Application Support/Recordings", isDirectory: true),
            home.appendingPathComponent("Library/Application Support/com.apple.voicememos/Recordings", isDirectory: true),
        ]
    }

    private static func isPermissionDenied(_ error: Error) -> Bool {
        let nsError = error as NSError
        if nsError.domain == NSPOSIXErrorDomain,
           nsError.code == Int(EACCES) || nsError.code == Int(EPERM) {
            return true
        }
        if nsError.domain == NSCocoaErrorDomain,
           nsError.code == NSFileReadNoPermissionError || nsError.code == NSFileReadNoSuchFileError {
            if let underlying = nsError.userInfo[NSUnderlyingErrorKey] as? NSError {
                return underlying.domain == NSPOSIXErrorDomain
                    && (underlying.code == Int(EACCES) || underlying.code == Int(EPERM))
            }
            return nsError.code == NSFileReadNoPermissionError
        }
        return false
    }

    nonisolated private static func sourceFileSize(path: FilePath?) -> Int64? {
        guard let path else {
            return nil
        }
        do {
            let attributes = try FileManager.default.attributesOfItem(atPath: path.string)
            guard let value = attributes[.size] as? NSNumber else {
                return nil
            }
            return value.int64Value
        } catch {
            return nil
        }
    }

    private func persistFilenameRecipe() {
        do {
            let data = try JSONEncoder().encode(filenameRecipe)
            userDefaults.set(data, forKey: Self.filenameRecipeDefaultsKey)
        } catch {
            userDefaults.removeObject(forKey: Self.filenameRecipeDefaultsKey)
        }
    }

    nonisolated private static func fileSize(path: FilePath) -> Int64? {
        do {
            let attributes = try FileManager.default.attributesOfItem(atPath: path.string)
            guard let value = attributes[.size] as? NSNumber else {
                return nil
            }
            return value.int64Value
        } catch {
            return nil
        }
    }

    nonisolated private static func fileSHA256(path: FilePath, cache: FileHashCache) -> SHA256.Digest? {
        guard let identity = FileHashIdentity(path: path) else {
            return nil
        }
        if let cached = cache.digest(for: identity) {
            return cached
        }
        guard let stream = InputStream(fileAtPath: path.string) else {
            return nil
        }
        stream.open()
        defer { stream.close() }

        var hasher = SHA256()
        let chunkSize = 64 * 1024
        let buffer = UnsafeMutablePointer<UInt8>.allocate(capacity: chunkSize)
        defer { buffer.deallocate() }

        while stream.hasBytesAvailable {
            if Task.isCancelled {
                return nil
            }
            let count = stream.read(buffer, maxLength: chunkSize)
            if count < 0 {
                return nil
            }
            if count == 0 {
                break
            }
            hasher.update(data: Data(bytes: buffer, count: count))
        }
        let digest = hasher.finalize()
        cache.setDigest(digest, for: identity)
        return digest
    }

    nonisolated private static func resolveAvailability(recording: VoiceMemoRecording) async -> (RecordingAvailability, String?) {
        guard recording.pathKind != .missing else {
            return (.needsDownload, "database path is empty; open/download it in Voice Memos.app first")
        }
        guard recording.localDuration > 0 else {
            return (.needsDownload, "local duration is 0; open/download it in Voice Memos.app first")
        }
        guard let sourcePath = recording.sourcePath else {
            return (.needsDownload, "local audio file is missing; open/download it in Voice Memos.app first")
        }
        do {
            let attributes = try FileManager.default.attributesOfItem(atPath: sourcePath.string)
            guard let fileSize = (attributes[.size] as? NSNumber)?.int64Value else {
                return (.corrupted, "failed to read file size")
            }
            if fileSize <= 0 {
                return (.needsDownload, "local audio file is 0 bytes; open/download it in Voice Memos.app first")
            }
        } catch {
            return (.needsDownload, "local audio file is missing; open/download it in Voice Memos.app first")
        }

        let duration: Double
        do {
            duration = try await assetDurationSeconds(path: sourcePath)
        } catch let failure {
            switch failure {
            case .timedOut:
                return (.corrupted, "timed out while reading audio container duration")
            case .cancelled:
                return (.checking, nil)
            case .invalidDuration, .loadingFailed:
                return (.corrupted, "failed to read audio container duration")
            }
        }
        let tolerance = 5.0
        if abs(duration - recording.localDuration) > tolerance {
            return (
                .corrupted,
                "audio duration mismatch: database=\(String(format: "%.3f", recording.localDuration))s, file=\(String(format: "%.3f", duration))s"
            )
        }
        return (.inRecordingsDirectory, nil)
    }

    nonisolated private static func assetDurationSeconds(
        path: FilePath
    ) async throws(AssetDurationError) -> Double {
        let asset = AVURLAsset(
            url: path.fileURL(),
            options: [AVURLAssetPreferPreciseDurationAndTimingKey: true]
        )
        do {
            return try await withTaskCancellationHandler(operation: {
                try await withThrowingTaskGroup(of: Double.self) { group in
                    group.addTask {
                        do {
                            let duration = try await asset.load(.duration)
                            let seconds = duration.seconds
                            guard seconds.isFinite, seconds > 0 else {
                                throw AssetDurationError.invalidDuration
                            }
                            return seconds
                        } catch is CancellationError {
                            throw AssetDurationError.cancelled
                        } catch let error as AssetDurationError {
                            throw error
                        } catch {
                            throw AssetDurationError.loadingFailed
                        }
                    }
                    group.addTask {
                        do {
                            try await Task.sleep(for: .seconds(15))
                        } catch is CancellationError {
                            throw AssetDurationError.cancelled
                        } catch {
                            throw AssetDurationError.loadingFailed
                        }
                        throw AssetDurationError.timedOut
                    }
                    defer {
                        group.cancelAll()
                        asset.cancelLoading()
                    }
                    guard let duration = try await group.next() else {
                        throw AssetDurationError.loadingFailed
                    }
                    return duration
                }
            }, onCancel: {
                asset.cancelLoading()
            })
        } catch let error as AssetDurationError {
            throw error
        } catch is CancellationError {
            throw .cancelled
        } catch {
            throw .loadingFailed
        }
    }

}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}

private enum AssetDurationError: Error, Sendable {
    case timedOut
    case cancelled
    case invalidDuration
    case loadingFailed
}

private struct FileHashIdentity: Hashable {
    var path: String
    var size: Int64
    var modificationTime: TimeInterval

    init?(path: FilePath) {
        do {
            let attributes = try FileManager.default.attributesOfItem(atPath: path.string)
            guard let size = (attributes[.size] as? NSNumber)?.int64Value,
                  let modifiedAt = attributes[.modificationDate] as? Date else {
                return nil
            }
            self.path = path.string
            self.size = size
            self.modificationTime = modifiedAt.timeIntervalSinceReferenceDate
        } catch {
            return nil
        }
    }
}

private final class FileHashCache: @unchecked Sendable {
    private let lock = NSLock()
    private var values: [FileHashIdentity: SHA256.Digest] = [:]

    func digest(for identity: FileHashIdentity) -> SHA256.Digest? {
        lock.withLock {
            values[identity]
        }
    }

    func setDigest(_ digest: SHA256.Digest, for identity: FileHashIdentity) {
        lock.withLock {
            values[identity] = digest
        }
    }
}
