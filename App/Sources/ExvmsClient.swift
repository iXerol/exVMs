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
    case local = 0
    case needsDownload = 1
    case outsideRecordingsDirectory = 2

    var label: String {
        switch self {
        case .local:
            String(localized: "availability.local")
        case .needsDownload:
            String(localized: "availability.needs_download")
        case .outsideRecordingsDirectory:
            String(localized: "availability.outside_storage")
        }
    }

    var isExportable: Bool {
        self == .local
    }

    init(exvmsAvailability: ExvmsAvailability) {
        switch exvmsAvailability {
        case .local:
            self = .local
        case .needsDownload:
            self = .needsDownload
        case .outsideRecordingsDirectory:
            self = .outsideRecordingsDirectory
        default:
            self = .needsDownload
        }
    }
}

struct VoiceMemoRecording: Identifiable, Hashable {
    var id: Int64
    var title: String
    var recordedAt: TimeInterval
    var duration: TimeInterval
    var localDuration: TimeInterval
    var databasePath: FilePath
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

protocol ExvmsClient {
    func listRecordings() throws -> RecordingCatalog
    func exportRecordings(ids: [Int64], outputDirectory: FilePath, template: String, jobs: Int) throws -> ExportSummary
}

final class RustExvmsClient: ExvmsClient {
    func listRecordings() throws -> RecordingCatalog {
        let result = exvmsListRecordings()
        defer { exvmsFreeListResult(result) }

        guard result.code == .OK else {
            throw ExvmsClientError.rust(Self.string(from: result.message) ?? String(localized: "error.recordings.list_failed"))
        }

        let store = VoiceMemosStore(
            recordingsDirectory: Self.filePath(from: result.recordings_dir) ?? FilePath(""),
            databasePath: Self.filePath(from: result.db_path) ?? FilePath("")
        )
        let rows = Self.buffer(result.recordings, count: Int(result.recordings_len)).map { row in
            VoiceMemoRecording(
                id: row.id,
                title: Self.string(from: row.title) ?? "",
                recordedAt: row.recorded_at,
                duration: row.duration,
                localDuration: row.local_duration,
                databasePath: Self.filePath(from: row.db_path) ?? FilePath(""),
                folderName: Self.nonEmptyString(from: row.folder_name),
                availability: RecordingAvailability(exvmsAvailability: row.availability),
                availabilityDetail: Self.nonEmptyString(from: row.availability_detail)
            )
        }
        return RecordingCatalog(store: store, recordings: rows)
    }

    func exportRecordings(ids: [Int64], outputDirectory: FilePath, template: String, jobs: Int) throws -> ExportSummary {
        return try outputDirectory.withPlatformString { outputPointer in
            try template.withCString { templatePointer in
                var mutableIds = ids
                let result = mutableIds.withUnsafeMutableBufferPointer { buffer in
                    precondition(buffer.baseAddress != nil, "export requires at least one recording id")
                    return exvmsExportRecordings(
                        outputDirectory: outputPointer,
                        ids: buffer.baseAddress!,
                        idsCount: UInt(buffer.count),
                        jobs: UInt(jobs),
                        filenameTemplate: templatePointer
                    )
                }
                defer { exvmsFreeExportResult(result) }

                guard result.code == .OK else {
                    throw ExvmsClientError.rust(Self.string(from: result.message) ?? String(localized: "error.export.failed"))
                }

                return ExportSummary(
                    succeeded: Int(result.successes_len),
                    skipped: Int(result.skipped_len),
                    failed: Int(result.failures_len)
                )
            }
        }
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
}
