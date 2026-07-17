import Foundation

@MainActor
final class DirectoryMonitor {
    private var descriptor: CInt = -1
    private var source: DispatchSourceFileSystemObject?
    private var debounceTask: Task<Void, Never>?

    func start(url: URL, onChange: @escaping @Sendable () -> Void) {
        stop()
        descriptor = open(url.path, O_EVTONLY)
        guard descriptor >= 0 else {
            return
        }

        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .delete, .rename, .attrib, .extend],
            queue: DispatchQueue.main
        )
        source.setEventHandler { [weak self] in
            self?.scheduleChange(onChange)
        }
        source.setCancelHandler { [descriptor] in
            close(descriptor)
        }
        self.source = source
        source.resume()
    }

    func stop() {
        debounceTask?.cancel()
        debounceTask = nil
        source?.cancel()
        source = nil
        descriptor = -1
    }

    private func scheduleChange(_ onChange: @escaping @Sendable () -> Void) {
        debounceTask?.cancel()
        debounceTask = Task { [weak self] in
            do {
                try await Task.sleep(for: .milliseconds(300))
            } catch {
                return
            }
            guard !Task.isCancelled else {
                return
            }
            self?.debounceTask = nil
            onChange()
        }
    }
}
