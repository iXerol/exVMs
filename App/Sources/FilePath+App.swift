import Foundation
import System

extension FilePath {
    func fileURL(isDirectory: Bool = false) -> URL {
        URL(fileURLWithPath: string, isDirectory: isDirectory)
    }
}
