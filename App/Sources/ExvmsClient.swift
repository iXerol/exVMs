import Foundation
import System

enum ExvmsClientError: LocalizedError {
    case rust(String)

    var errorDescription: String? {
        switch self {
        case .rust(let message):
            message
        }
    }
}

struct VoiceMemosStore: Equatable {
    var recordingsDirectory: FilePath
    var databasePath: FilePath
}

enum RecordingAvailability: Int32 {
    case inRecordingsDirectory = 0
    case needsDownload = 1
    case outsideRecordingsDirectory = 2
    case corrupted = 3
    case checking = 4

    var label: String {
        switch self {
        case .inRecordingsDirectory:
            ExVMsStrings.Availability.inRecordingsDirectory
        case .needsDownload:
            ExVMsStrings.Availability.needsDownload
        case .outsideRecordingsDirectory:
            ExVMsStrings.Availability.outsideStorage
        case .corrupted:
            ExVMsStrings.Availability.corrupted
        case .checking:
            String(localized: "availability.checking")
        }
    }

    var isExportable: Bool {
        self == .inRecordingsDirectory
    }

    init(exvmsAvailability: ExvmsAvailability) {
        switch exvmsAvailability {
        case .inRecordingsDirectory:
            self = .inRecordingsDirectory
        case .needsDownload:
            self = .needsDownload
        case .outsideRecordingsDirectory:
            self = .outsideRecordingsDirectory
        case .corrupted:
            self = .corrupted
        case .checking:
            self = .checking
        default:
            self = .checking
        }
    }
}

enum RecordingPathKind: Int32 {
    case relative = 0
    case absolute = 1
    case missing = 2

    init(exvmsPathKind: ExvmsPathKind) {
        switch exvmsPathKind {
        case .relative:
            self = .relative
        case .absolute:
            self = .absolute
        case .missing:
            self = .missing
        default:
            self = .missing
        }
    }
}

struct VoiceMemoRecording: Identifiable, Hashable {
    var id: Int64
    var title: String
    var recordedAt: TimeInterval
    var duration: TimeInterval
    var localDuration: TimeInterval
    var isDeleted: Bool
    var databasePath: FilePath
    var sourcePath: FilePath?
    var pathKind: RecordingPathKind
    var folderName: String?
    var availability: RecordingAvailability
    var availabilityDetail: String?
}

struct RecordingCatalog {
    var store: VoiceMemosStore
    var recordings: [VoiceMemoRecording]
}

struct ExportSummary: Equatable {
    var succeeded: Int
    var skipped: Int
    var failed: Int
}

protocol ExvmsClient: Sendable {
    func listRecordings() async throws -> RecordingCatalog
    func exportRecordings(
        ids: [Int64],
        outputDirectory: FilePath,
        template: String,
        exportGrouping: ExvmsExportGrouping,
        jobs: Int
    ) async throws -> ExportSummary
}

struct RustExvmsClient: ExvmsClient {
    func listRecordings() async throws -> RecordingCatalog {
        try await Task.detached(priority: .userInitiated) {
            let result = exvmsListRecordings()
            defer { exvmsFreeListResult(result) }

            guard result.code == .OK else {
                throw ExvmsClientError.rust(Self.string(from: result.message) ?? ExVMsStrings.Error.Recordings.listFailed)
            }

            let store = VoiceMemosStore(
                recordingsDirectory: Self.filePath(from: result.recordings_dir) ?? FilePath(""),
                databasePath: Self.filePath(from: result.db_path) ?? FilePath("")
            )
            let rows = Self.buffer(result.recordings, count: Int(result.recordings_len)).map { row in
                let databasePath = Self.filePath(from: row.db_path) ?? FilePath("")
                let pathKind = RecordingPathKind(exvmsPathKind: row.path_kind)
                return VoiceMemoRecording(
                    id: row.id,
                    title: Self.string(from: row.title) ?? "",
                    recordedAt: row.recorded_at,
                    duration: row.duration,
                    localDuration: row.local_duration,
                    isDeleted: row.is_deleted != 0,
                    databasePath: databasePath,
                    sourcePath: Self.sourcePath(
                        recordingsDirectory: store.recordingsDirectory,
                        databasePath: databasePath,
                        pathKind: pathKind
                    ),
                    pathKind: pathKind,
                    folderName: Self.nonEmptyString(from: row.folder_name),
                    availability: RecordingAvailability(exvmsAvailability: row.availability),
                    availabilityDetail: Self.nonEmptyString(from: row.availability_detail)
                )
            }
            return RecordingCatalog(store: store, recordings: rows)
        }.value
    }

    func exportRecordings(
        ids: [Int64],
        outputDirectory: FilePath,
        template: String,
        exportGrouping: ExvmsExportGrouping,
        jobs: Int
    ) async throws -> ExportSummary {
        try await Task.detached(priority: .userInitiated) {
            try outputDirectory.withPlatformString { outputPointer in
                try template.withCString { templatePointer in
                    var mutableIds = ids
                    let result = mutableIds.withUnsafeMutableBufferPointer { buffer in
                        precondition(buffer.baseAddress != nil, "export requires at least one recording id")
                        return exvmsExportRecordings(
                            outputDirectory: outputPointer,
                            ids: buffer.baseAddress!,
                            idsCount: UInt(buffer.count),
                            jobs: UInt(jobs),
                            filenameTemplate: templatePointer,
                            exportGrouping: exportGrouping
                        )
                    }
                    defer { exvmsFreeExportResult(result) }

                    guard result.code == .OK else {
                        throw ExvmsClientError.rust(Self.string(from: result.message) ?? ExVMsStrings.Error.Export.failed)
                    }

                    return ExportSummary(
                        succeeded: Int(result.successes_len),
                        skipped: Int(result.skipped_len),
                        failed: Int(result.failures_len)
                    )
                }
            }
        }.value
    }

    private static func buffer<T>(_ pointer: UnsafeMutablePointer<T>?, count: Int) -> [T] {
        guard let pointer, count > 0 else {
            return []
        }
        return Array(UnsafeBufferPointer(start: pointer, count: count))
    }

    private static func string(from pointer: UnsafeMutablePointer<CChar>?) -> String? {
        guard let pointer else {
            return nil
        }
        return String(cString: pointer)
    }

    private static func filePath(from pointer: UnsafeMutablePointer<CChar>?) -> FilePath? {
        guard let pointer else {
            return nil
        }
        return FilePath(platformString: UnsafePointer(pointer))
    }

    private static func nonEmptyString(from pointer: UnsafeMutablePointer<CChar>?) -> String? {
        guard let value = string(from: pointer), !value.isEmpty else {
            return nil
        }
        return value
    }

    private static func sourcePath(
        recordingsDirectory: FilePath,
        databasePath: FilePath,
        pathKind: RecordingPathKind
    ) -> FilePath? {
        switch pathKind {
        case .relative:
            return FilePath(
                recordingsDirectory
                    .fileURL(isDirectory: true)
                    .appendingPathComponent(databasePath.string)
                    .path
            )
        case .absolute:
            return resolvedAbsoluteSource(
                recordingsDirectory: recordingsDirectory,
                databasePath: databasePath
            )
        case .missing:
            return nil
        }
    }

    private static func resolvedAbsoluteSource(
        recordingsDirectory: FilePath,
        databasePath: FilePath
    ) -> FilePath? {
        let recordingsURL = recordingsDirectory.fileURL(isDirectory: true).standardizedFileURL
        let originalURL = databasePath.fileURL().standardizedFileURL

        if originalURL.path.hasPrefix(recordingsURL.path + "/") || originalURL.path == recordingsURL.path {
            return FilePath(originalURL.path)
        }

        if let remapped = remappedLegacyAbsoluteSource(
            recordingsDirectoryURL: recordingsURL,
            originalURL: originalURL
        ) {
            return FilePath(remapped.path)
        }

        let candidate = recordingsURL.appendingPathComponent(originalURL.lastPathComponent)
        if FileManager.default.fileExists(atPath: candidate.path) {
            return FilePath(candidate.path)
        }

        return FilePath(originalURL.path)
    }

    private static func remappedLegacyAbsoluteSource(
        recordingsDirectoryURL: URL,
        originalURL: URL
    ) -> URL? {
        let components = originalURL.pathComponents.filter { $0 != "/" }
        for suffix in knownRecordingsSuffixes {
            guard let index = components.firstSubsequenceIndex(of: suffix) else {
                continue
            }
            let tail = Array(components.dropFirst(index + suffix.count))
            guard !tail.isEmpty, !tail.contains("..") else {
                continue
            }
            return recordingsDirectoryURL.appendingPathComponent(tail.joined(separator: "/"))
        }
        return nil
    }

    private static let knownRecordingsSuffixes = [
        ["Library", "Group Containers", "group.com.apple.VoiceMemos.shared", "Recordings"],
        ["Library", "Containers", "Voice Memos", "Data", "Library", "Application Support", "Recordings"],
        ["Library", "Application Support", "com.apple.voicememos", "Recordings"],
    ]
}

private extension Array where Element: Equatable {
    func firstSubsequenceIndex(of subsequence: [Element]) -> Int? {
        guard !subsequence.isEmpty, count >= subsequence.count else {
            return nil
        }
        return indices.dropLast(subsequence.count - 1).first { index in
            Array(self[index..<index + subsequence.count]) == subsequence
        }
    }
}
