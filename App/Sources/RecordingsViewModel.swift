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

@MainActor
@Observable
final class RecordingsViewModel {
    typealias AvailabilityResolver = @Sendable (VoiceMemoRecording) -> (RecordingAvailability, String?)

    static let outputDirectoryDefaultsKey = "com.ixerol.exvms.output-directory"
    static let filenameRecipeDefaultsKey = "com.ixerol.exvms.filename-recipe"

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
            rebuildRows()
        }
    }

    var filenameRecipe = FilenameRecipe.default {
        didSet {
            persistFilenameRecipe()
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
    private var rowBuildGeneration: Int = 0
    @ObservationIgnored
    private var availabilityGeneration: Int = 0

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
            availabilityGeneration += 1
            let availabilityGeneration = availabilityGeneration
            await rebuildRowsAsync()
            isLoading = false
            await resolveAvailabilityProgressively(generation: availabilityGeneration)
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
                jobs: 4
            )
            selectedIDs.removeAll()
            await rebuildRowsAsync()
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
                await self?.rebuildRowsAsync()
            }
        }
    }

    private func rebuildRows() {
        Task { @MainActor in
            await rebuildRowsAsync()
        }
    }

    private func rebuildRowsAsync() async {
        rowBuildGeneration += 1
        let generation = rowBuildGeneration
        let snapshotRecordings = recordings
        let snapshotOutput = outputDirectory
        let snapshotRecipe = filenameRecipe
        let snapshotHashCache = hashCache
        let builtRows = await Task.detached(priority: .userInitiated) {
            await Self.buildRows(
                recordings: snapshotRecordings,
                outputDirectory: snapshotOutput,
                filenameRecipe: snapshotRecipe,
                hashCache: snapshotHashCache
            )
        }.value
        guard generation == rowBuildGeneration else {
            return
        }
        rows = builtRows
        selectedIDs = selectedIDs.intersection(Set(rows.filter(\.canExport).map(\.id)))
        if let folderFilter, !folderOptions.contains(folderFilter) {
            self.folderFilter = nil
        }
    }

    private func resolveAvailabilityProgressively(generation: Int) async {
        let snapshot = rows
        let availabilityResolver = availabilityResolver
        await withTaskGroup(of: (Int64, RecordingAvailability, String?).self) { group in
            for row in snapshot where row.recording.availability == .checking {
                let recording = row.recording
                group.addTask(priority: .utility) {
                    let result = availabilityResolver(recording)
                    return (recording.id, result.0, result.1)
                }
            }

            for await (id, availability, detail) in group {
                guard generation == availabilityGeneration else { continue }
                updateRecordingAvailability(id: id, availability: availability, detail: detail)
                let rowGeneration = rowBuildGeneration
                if let rowIndex = rows.firstIndex(where: { $0.id == id }) {
                    rows[rowIndex].recording.availability = availability
                    rows[rowIndex].recording.availabilityDetail = detail
                }
                await rebuildRow(id: id, generation: rowGeneration)
                if let row = rows.first(where: { $0.id == id }), !row.canExport {
                    selectedIDs.remove(id)
                }
            }
        }
    }

    private func updateRecordingAvailability(id: Int64, availability: RecordingAvailability, detail: String?) {
        guard let recordingIndex = recordings.firstIndex(where: { $0.id == id }) else {
            return
        }
        recordings[recordingIndex].availability = availability
        recordings[recordingIndex].availabilityDetail = detail
    }

    private func rebuildRow(id: Int64, generation: Int) async {
        guard let recording = recordings.first(where: { $0.id == id }) else {
            return
        }
        let snapshotOutput = outputDirectory
        let snapshotRecipe = filenameRecipe
        let snapshotHashCache = hashCache
        let rebuiltRow = await Task.detached(priority: .userInitiated) {
            Self.buildRow(
                recording: recording,
                outputDirectory: snapshotOutput,
                filenameRecipe: snapshotRecipe,
                hashCache: snapshotHashCache
            )
        }.value
        guard generation == rowBuildGeneration,
              let rowIndex = rows.firstIndex(where: { $0.id == id }) else {
            return
        }
        rows[rowIndex] = rebuiltRow
    }

    nonisolated private static func buildRows(
        recordings: [VoiceMemoRecording],
        outputDirectory: FilePath?,
        filenameRecipe: FilenameRecipe,
        hashCache: FileHashCache
    ) async -> [RecordingRow] {
        return await withTaskGroup(of: (Int, RecordingRow).self) { group in
            for (index, recording) in recordings.enumerated() {
                group.addTask(priority: .userInitiated) {
                    let row = buildRow(
                        recording: recording,
                        outputDirectory: outputDirectory,
                        filenameRecipe: filenameRecipe,
                        hashCache: hashCache
                    )
                    return (index, row)
                }
            }

            var ordered = Array<RecordingRow?>(repeating: nil, count: recordings.count)
            for await (index, row) in group {
                ordered[index] = row
            }
            return ordered.compactMap { $0 }
        }
    }

    nonisolated private static func buildRow(
        recording: VoiceMemoRecording,
        outputDirectory: FilePath?,
        filenameRecipe: FilenameRecipe,
        hashCache: FileHashCache
    ) -> RecordingRow {
        let sourceSize = sourceFileSize(path: recording.sourcePath)
        let exportedPath = exportedPath(
            recording: recording,
            outputDirectory: outputDirectory,
            filenameRecipe: filenameRecipe,
            sourceSize: sourceSize,
            hashCache: hashCache
        )
        return RecordingRow(
            recording: recording,
            exported: exportedPath != nil,
            exportedPath: exportedPath,
            sourceFileSizeBytes: sourceSize
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
        sourceSize: Int64?,
        hashCache: FileHashCache
    ) -> FilePath? {
        guard let outputDirectory,
              let baseTarget = filenameRecipe.preview(for: recording, outputDirectory: outputDirectory) else {
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

    nonisolated private static func resolveAvailability(recording: VoiceMemoRecording) -> (RecordingAvailability, String?) {
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

        guard let duration = afinfoDurationSeconds(path: sourcePath) else {
            return (.corrupted, "failed to read audio container duration")
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

    nonisolated private static func afinfoDurationSeconds(path: FilePath) -> Double? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/afinfo")
        process.arguments = [path.string]
        let stdout = Pipe()
        process.standardOutput = stdout
        process.standardError = Pipe()
        do {
            try process.run()
            process.waitUntilExit()
        } catch {
            return nil
        }
        guard process.terminationStatus == 0 else {
            return nil
        }
        let data = stdout.fileHandleForReading.readDataToEndOfFile()
        guard let output = String(data: data, encoding: .utf8) else {
            return nil
        }
        for line in output.split(separator: "\n") {
            let text = line.trimmingCharacters(in: .whitespacesAndNewlines)
            guard text.hasPrefix("estimated duration:") else { continue }
            let raw = text.replacingOccurrences(of: "estimated duration:", with: "")
                .replacingOccurrences(of: "sec", with: "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if let value = Double(raw), value.isFinite, value > 0 {
                return value
            }
        }
        return nil
    }

}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
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
