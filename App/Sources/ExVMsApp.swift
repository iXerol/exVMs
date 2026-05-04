import SwiftUI

@main
struct ExVMsApp: App {
    var body: some Scene {
        WindowGroup("app.name") {
            ContentView(viewModel: RecordingsViewModel(client: RustExvmsClient()))
                .frame(minWidth: 600, minHeight: 500)
        }
        .defaultSize(width: 1180, height: 760)
        .commands {
            AppCommands()
        }

        Window("about.window.title", id: "about") {
            AboutView()
        }
        .windowResizability(.contentSize)
        .defaultPosition(.center)
    }
}

private struct AppCommands: Commands {
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(replacing: .appInfo) {
            Button("about.menu.title") {
                openWindow(id: "about")
            }
        }
    }
}
