import Observation
import System
import SwiftUI

struct RecordingRow: Identifiable, Hashable {
    var recording: VoiceMemoRecording
    var targetPath: FilePath?
    var exported: Bool

    var id: Int64 { recording.id }
    var canExport: Bool { recording.availability.isExportable && !exported }
    var title: String { recording.title }
    var folderDisplayName: String { recording.folderName?.nilIfEmpty ?? String(localized: "folder.none") }
    var recordedDate: Date { recording.recordingDate }
    var duration: TimeInterval { recording.duration }
    var availabilityLabel: String { exported ? String(localized: "availability.exported") : recording.availability.label }
}

@MainActor
@Observable
final class RecordingsViewModel {
    static let outputDirectoryDefaultsKey = "com.ixerol.exvms.output-directory"

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
    private let userDefaults: UserDefaults
    @ObservationIgnored
    private let outputMonitor = DirectoryMonitor()
    @ObservationIgnored
    private var recordings: [VoiceMemoRecording] = []

    init(client: ExvmsClient, userDefaults: UserDefaults = .standard) {
        self.client = client
        self.userDefaults = userDefaults
        self.outputDirectory = Self.loadPersistedOutputDirectory(from: userDefaults)
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

        isLoading = false
        refresh()
    }

    func refresh() {
        isLoading = true
        errorMessage = nil
        fullDiskAccessMessage = nil
        do {
            let catalog = try client.listRecordings()
            store = catalog.store
            recordings = catalog.recordings
            rebuildRows()
        } catch {
            errorMessage = error.localizedDescription
            if needsFullDiskAccessHelp {
                fullDiskAccessMessage = String(localized: "error.voice_memos_access.generic")
            }
        }
        isLoading = false
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
        panel.prompt = String(localized: "action.choose")
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

    func exportSelection() {
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
            lastExportSummary = try client.exportRecordings(
                ids: ids,
                outputDirectory: outputDirectory,
                template: filenameRecipe.rustTemplate,
                jobs: 4
            )
            selectedIDs.removeAll()
            refresh()
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
                self?.rebuildRows()
            }
        }
    }

    private func rebuildRows() {
        rows = recordings.map { recording in
            let target = outputDirectory.flatMap { filenameRecipe.preview(for: recording, outputDirectory: $0) }
            let exported = target.map { FileManager.default.fileExists(atPath: $0.string) } ?? false
            return RecordingRow(recording: recording, targetPath: target, exported: exported)
        }
        selectedIDs = selectedIDs.intersection(Set(rows.filter(\.canExport).map(\.id)))
        if let folderFilter, !folderOptions.contains(folderFilter) {
            self.folderFilter = nil
        }
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
                    return String(localized: "error.voice_memos_access.path \(directory.path)")
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
                    return String(localized: "error.voice_memos_access.path \(databaseURL.path)")
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
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
