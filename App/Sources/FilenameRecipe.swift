import Foundation
import System

enum FilenameToken: String, CaseIterable, Identifiable, Codable {
    case title
    case id
    case year
    case month
    case day
    case hour
    case minute
    case second

    var id: String { rawValue }

    var label: String {
        switch self {
        case .title:
            String(localized: "token.title")
        case .id:
            String(localized: "token.id")
        case .year:
            "YYYY"
        case .month:
            "MM"
        case .day:
            "DD"
        case .hour:
            "HH"
        case .minute:
            "mm"
        case .second:
            "ss"
        }
    }

    var accessibilityName: String {
        switch self {
        case .title:
            String(localized: "token.title")
        case .id:
            String(localized: "token.id")
        case .year:
            String(localized: "token.year")
        case .month:
            String(localized: "token.month")
        case .day:
            String(localized: "token.day")
        case .hour:
            String(localized: "token.hour")
        case .minute:
            String(localized: "token.minute")
        case .second:
            String(localized: "token.second")
        }
    }

    var templateFragment: String {
        switch self {
        case .title:
            "{title}"
        case .id:
            "{id}"
        case .year:
            "{datetime:%Y}"
        case .month:
            "{datetime:%m}"
        case .day:
            "{datetime:%d}"
        case .hour:
            "{datetime:%H}"
        case .minute:
            "{datetime:%M}"
        case .second:
            "{datetime:%S}"
        }
    }
}

enum FilenamePart: Identifiable, Hashable, Codable {
    case token(UUID, FilenameToken)
    case literal(UUID, String)

    var id: UUID {
        switch self {
        case .token(let id, _), .literal(let id, _):
            id
        }
    }

    var isToken: Bool {
        if case .token = self {
            return true
        }
        return false
    }

    var templateFragment: String {
        switch self {
        case .token(_, let token):
            token.templateFragment
        case .literal(_, let value):
            value
        }
    }

    var accessibilityFragment: String {
        switch self {
        case .token(_, let token):
            String(localized: "accessibility.filename.token \(token.accessibilityName)")
        case .literal(_, let value):
            value.isEmpty ? "" : String(localized: "accessibility.filename.literal \(value)")
        }
    }
}

struct FilenameRecipe: Equatable, Codable {
    var parts: [FilenamePart]

    static let `default` = FilenameRecipe(parts: [
        .literal(UUID(), ""),
        .token(UUID(), .title),
        .literal(UUID(), ""),
    ])

    var rustTemplate: String {
        let value = parts.map(\.templateFragment).joined()
        return value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "{title}" : value
    }

    var accessibilityDescription: String {
        let value = parts
            .map(\.accessibilityFragment)
            .filter { !$0.isEmpty }
            .joined(separator: ", ")
        return value.isEmpty ? "Title token" : value
    }

    private mutating func normalizeTextSlots() {
        if parts.isEmpty {
            parts = Self.default.parts
            return
        }

        var normalized: [FilenamePart] = []
        for part in parts {
            if normalized.isEmpty, part.isToken {
                normalized.append(.literal(UUID(), ""))
            }

            if case .literal(_, let value) = part,
               case .literal(let previousID, let previousValue) = normalized.last {
                normalized[normalized.count - 1] = .literal(previousID, previousValue + value)
                continue
            }

            if normalized.last?.isToken == true, part.isToken {
                normalized.append(.literal(UUID(), ""))
            }

            normalized.append(part)
        }
        if normalized.last?.isToken == true {
            normalized.append(.literal(UUID(), ""))
        }

        let hasMeaningfulContent = normalized.contains { part in
            switch part {
            case .token:
                true
            case .literal(_, let value):
                !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            }
        }

        if hasMeaningfulContent {
            parts = normalized
        } else {
            parts = Self.default.parts
        }
    }

    func preview(for recording: VoiceMemoRecording, outputDirectory: FilePath) -> FilePath? {
        guard recording.availability.isExportable else {
            return nil
        }

        var directory = outputDirectory.fileURL(isDirectory: true)
        if let folderName = recording.folderName, !folderName.isEmpty {
            directory.appendPathComponent(Self.sanitizeFilename(folderName), isDirectory: true)
        }

        var stem = renderStem(for: recording)
        stem = Self.sanitizeStem(stem)
        let sourceExtension = recording.databasePath.fileURL().pathExtension
        if !sourceExtension.isEmpty && !stem.lowercased().hasSuffix(".\(sourceExtension.lowercased())") {
            stem += ".\(sourceExtension)"
        }
        if sourceExtension.isEmpty {
            stem = stem.trimmingCharacters(in: CharacterSet(charactersIn: ".").union(.whitespacesAndNewlines))
            if stem.isEmpty {
                stem = "Untitled"
            }
        }
        return FilePath(directory.appendingPathComponent(stem).path)
    }

    func renderStem(for recording: VoiceMemoRecording) -> String {
        parts.map { part in
            switch part {
            case .literal(_, let value):
                value
            case .token(_, let token):
                Self.render(token: token, recording: recording)
            }
        }.joined()
    }

    func sampleFilename(now: Date = Date()) -> String {
        let sample = VoiceMemoRecording(
            id: 123,
            title: "Recording Title",
            recordedAt: now.timeIntervalSince1970 - 978_307_200,
            duration: 12,
            localDuration: 12,
            databasePath: FilePath("recording.m4a"),
            folderName: nil,
            availability: .local,
            availabilityDetail: nil
        )
        var stem = Self.sanitizeStem(renderStem(for: sample))
        if !stem.lowercased().hasSuffix(".m4a") {
            stem += ".m4a"
        }
        return stem
    }

    private static func render(token: FilenameToken, recording: VoiceMemoRecording) -> String {
        switch token {
        case .title:
            recording.title
        case .id:
            String(recording.id)
        case .year:
            dateFormatter("yyyy").string(from: recording.recordingDate)
        case .month:
            dateFormatter("MM").string(from: recording.recordingDate)
        case .day:
            dateFormatter("dd").string(from: recording.recordingDate)
        case .hour:
            dateFormatter("HH").string(from: recording.recordingDate)
        case .minute:
            dateFormatter("mm").string(from: recording.recordingDate)
        case .second:
            dateFormatter("ss").string(from: recording.recordingDate)
        }
    }

    private static func dateFormatter(_ format: String) -> DateFormatter {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = format
        return formatter
    }

    static func sanitizeFilename(_ value: String) -> String {
        let sanitized = String(value.unicodeScalars.map { scalar in
            if isUnsafeFilenameScalar(scalar) {
                return "_"
            }
            return String(scalar)
        }.joined())
        let trimmed = sanitized.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "Untitled" : trimmed
    }

    private static func isUnsafeFilenameScalar(_ scalar: UnicodeScalar) -> Bool {
        switch scalar.value {
        case 0, 47, 58, 92, 0x2028, 0x2029:
            return true
        default:
            return CharacterSet.controlCharacters.contains(scalar)
        }
    }

    static func sanitizeStem(_ value: String) -> String {
        let sanitized = sanitizeFilename(value)
        let trimmed = sanitized
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingCharacters(in: CharacterSet(charactersIn: "."))
        return trimmed.isEmpty ? "Untitled" : trimmed
    }
}

extension VoiceMemoRecording {
    var recordingDate: Date {
        Date(timeIntervalSince1970: recordedAt + 978_307_200)
    }
}
