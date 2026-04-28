import SwiftUI

@main
struct ExVMsApp: App {
    var body: some Scene {
        WindowGroup("app.name") {
            ContentView(viewModel: RecordingsViewModel(client: RustExvmsClient()))
                .frame(minWidth: 600, minHeight: 500)
        }
        .defaultSize(width: 1180, height: 760)
    }
}
