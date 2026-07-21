import Foundation
import System
import Testing
@testable import exVMs

struct FilenameRecipeTests {
    @Test
    func recipeConvertsTokensToRustTemplate() {
        let recipe = FilenameRecipe(parts: [
            .token(UUID(), .year),
            .literal(UUID(), "-"),
            .token(UUID(), .month),
            .literal(UUID(), "-"),
            .token(UUID(), .day),
            .literal(UUID(), " "),
            .token(UUID(), .hour),
            .literal(UUID(), "-"),
            .token(UUID(), .minute),
            .literal(UUID(), "-"),
            .token(UUID(), .second),
            .literal(UUID(), " "),
            .token(UUID(), .title),
        ])

        #expect(recipe.rustTemplate == "{datetime:%Y}-{datetime:%m}-{datetime:%d} {datetime:%H}-{datetime:%M}-{datetime:%S} {title}")
    }

    @Test
    func previewUsesSanitizedFolderAndSourceExtension() throws {
        let recipe = FilenameRecipe(parts: [
            .token(UUID(), .title),
            .literal(UUID(), " "),
            .token(UUID(), .id),
        ])
        let recording = VoiceMemoRecording(
            id: 42,
            title: "A/B:C",
            recordedAt: 1,
            duration: 2,
            localDuration: 2,
            isDeleted: false,
            databasePath: FilePath("folder/source.m4a"),
            sourcePath: nil,
            pathKind: .relative,
            folderName: "Notes:Drafts",
            availability: .inRecordingsDirectory,
            availabilityDetail: nil
        )

        let output = FilePath("/tmp/exvms-test-output")
        let preview = try #require(recipe.preview(for: recording, outputDirectory: output))
        let previewString: String = preview.withPlatformString { String(cString: $0) }
        #expect(previewString == "/tmp/exvms-test-output/Notes_Drafts/A_B_C 42.m4a")
    }

    @Test
    func previewUsesDateDirectoryWhenSelected() throws {
        let recording = VoiceMemoRecording(
            id: 42,
            title: "A/B:C",
            recordedAt: appleSeconds(year: 2026, month: 7, day: 1),
            duration: 2,
            localDuration: 2,
            isDeleted: false,
            databasePath: FilePath("source.m4a"),
            sourcePath: nil,
            pathKind: .relative,
            folderName: "Notes",
            availability: .inRecordingsDirectory,
            availabilityDetail: nil
        )

        let output = FilePath("/tmp/exvms-test-output")
        let preview = try #require(FilenameRecipe.default.preview(
            for: recording,
            outputDirectory: output,
            exportGrouping: .date
        ))
        let previewString: String = preview.withPlatformString { String(cString: $0) }
        #expect(previewString == "/tmp/exvms-test-output/2026-7-1/A_B_C.m4a")
    }

    @Test
    func previewUsesFolderThenDateAndNoFolderWhenBothSelected() throws {
        let output = FilePath("/tmp/exvms-test-output")
        let datedRecording = VoiceMemoRecording(
            id: 42,
            title: "Recording",
            recordedAt: appleSeconds(year: 2026, month: 7, day: 1),
            duration: 2,
            localDuration: 2,
            isDeleted: false,
            databasePath: FilePath("source.m4a"),
            sourcePath: nil,
            pathKind: .relative,
            folderName: nil,
            availability: .inRecordingsDirectory,
            availabilityDetail: nil
        )
        let folderRecording = VoiceMemoRecording(
            id: 43,
            title: "Recording",
            recordedAt: appleSeconds(year: 2026, month: 7, day: 2),
            duration: 2,
            localDuration: 2,
            isDeleted: false,
            databasePath: FilePath("source.m4a"),
            sourcePath: nil,
            pathKind: .relative,
            folderName: "Live",
            availability: .inRecordingsDirectory,
            availabilityDetail: nil
        )

        let noFolderPreview = try #require(FilenameRecipe.default.preview(
            for: datedRecording,
            outputDirectory: output,
            exportGrouping: [.folder, .date]
        ))
        let folderPreview = try #require(FilenameRecipe.default.preview(
            for: folderRecording,
            outputDirectory: output,
            exportGrouping: [.folder, .date]
        ))

        let noFolderString: String = noFolderPreview.withPlatformString { String(cString: $0) }
        let folderString: String = folderPreview.withPlatformString { String(cString: $0) }
        #expect(noFolderString == "/tmp/exvms-test-output/No Folder/2026-7-1/Recording.m4a")
        #expect(folderString == "/tmp/exvms-test-output/Live/2026-7-2/Recording.m4a")
    }

    @Test
    func unavailableRecordingHasNoPreviewTarget() {
        let recording = VoiceMemoRecording(
            id: 1,
            title: "Cloud",
            recordedAt: 1,
            duration: 2,
            localDuration: 0,
            isDeleted: false,
            databasePath: FilePath("cloud.m4a"),
            sourcePath: nil,
            pathKind: .relative,
            folderName: nil,
            availability: .needsDownload,
            availabilityDetail: nil
        )

        #expect(FilenameRecipe.default.preview(
            for: recording,
            outputDirectory: FilePath("/tmp")
        ) == nil)
    }

    @Test
    func sampleFilenameUsesCurrentTimeAndRecordingTitle() {
        let fixedNow = Date(timeIntervalSince1970: 1_704_153_600)
        let recipe = FilenameRecipe(parts: [
            .token(UUID(), .year),
            .literal(UUID(), "-"),
            .token(UUID(), .month),
            .literal(UUID(), " "),
            .token(UUID(), .title),
        ])

        #expect(recipe.sampleFilename(now: fixedNow) == "2024-01 Recording Title.m4a")
    }

    @Test
    func accessibilityDescriptionNamesTokensAndLiteralText() {
        let recipe = FilenameRecipe(parts: [
            .token(UUID(), .year),
            .literal(UUID(), "-"),
            .token(UUID(), .month),
            .literal(UUID(), " "),
            .token(UUID(), .title),
        ])

        #expect(recipe.accessibilityDescription == "Year token, text -, Month token, text  , Title token")
    }

    private func appleSeconds(year: Int, month: Int, day: Int) -> TimeInterval {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .current
        let date = calendar.date(from: DateComponents(
            timeZone: .current,
            year: year,
            month: month,
            day: day,
            hour: 12
        ))!
        return date.timeIntervalSince1970 - 978_307_200
    }
}
