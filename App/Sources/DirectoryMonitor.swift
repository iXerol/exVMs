import Foundation

final class DirectoryMonitor {
    private var descriptor: CInt = -1
    private var source: DispatchSourceFileSystemObject?

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
        source.setEventHandler(handler: onChange)
        source.setCancelHandler { [descriptor] in
            close(descriptor)
        }
        self.source = source
        source.resume()
    }

    func stop() {
        source?.cancel()
        source = nil
        descriptor = -1
    }

    deinit {
        stop()
    }
}
