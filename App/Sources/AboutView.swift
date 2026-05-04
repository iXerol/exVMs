import SwiftUI

struct AboutView: View {
    private let projectURL = URL(string: "https://github.com/iXerol/exVMs")!

    var body: some View {
        VStack(spacing: 12) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 72, height: 72)
                .accessibilityHidden(true)

            Text("app.name")
                .font(.title3.weight(.semibold))

            Text(Bundle.main.shortVersionAndBuild)
                .font(.caption)
                .foregroundStyle(.secondary)

            Link("about.project_url", destination: projectURL)
                .font(.subheadline)
                .textSelection(.enabled)
                .accessibilityLabel("accessibility.about.project_url")
        }
        .padding(24)
        .frame(width: 360)
    }
}

private extension Bundle {
    var shortVersionAndBuild: String {
        let short = infoDictionary?["CFBundleShortVersionString"] as? String ?? "1.0"
        let build = infoDictionary?["CFBundleVersion"] as? String ?? "1"
        return "v\(short) (\(build))"
    }
}
