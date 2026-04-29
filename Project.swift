import ProjectDescription

let rustBuildScript = """
/bin/sh "$SRCROOT/scripts/xcode-build-rust-library.sh"
"""

let displayName = "exVMs"
let version = "1.0.0"
let bundleVersion = "260429"
let utilityCategory = "public.app-category.utilities"

let project = Project(
    name: "exVMs",
    options: .options(automaticSchemesOptions: .enabled()),
    settings: .settings(base: [
        "ENABLE_USER_SCRIPT_SANDBOXING": "NO",
        "SWIFT_VERSION": "6.0",
        "SWIFT_STRICT_CONCURRENCY": "complete",
    ]),
    targets: [
        .target(
            name: "exVMsApp",
            destinations: .macOS,
            product: .app,
            productName: "exVMs",
            bundleId: "com.ixerol.exvms",
            deploymentTargets: .macOS("14.0"),
            infoPlist: .extendingDefault(with: [
                "CFBundleDisplayName": .string(displayName),
                "CFBundleDevelopmentRegion": "en",
                "CFBundleLocalizations": ["en", "zh-Hans", "zh-Hant", "ja"],
                "CFBundleShortVersionString": .string(version),
                "CFBundleVersion": .string(bundleVersion),
                "LSApplicationCategoryType": .string(utilityCategory),
                "NSQuitAlwaysKeepsWindows": false,
            ]),
            sources: ["App/Sources/**"],
            resources: ["App/Resources/**"],
            scripts: [
                .pre(script: rustBuildScript, name: "Build Rust Library", basedOnDependencyAnalysis: false),
            ],
            dependencies: [],
            settings: .settings(
                base: [
                "INFOPLIST_KEY_CFBundleDisplayName": .string(displayName),
                "INFOPLIST_KEY_LSApplicationCategoryType": .string(utilityCategory),
                "MARKETING_VERSION": .string(version),
                "CURRENT_PROJECT_VERSION": .string(bundleVersion),
                "DEVELOPMENT_TEAM": "52892NMS89",
                "ENABLE_HARDENED_RUNTIME": "YES",
                "ENABLE_APP_SANDBOX": "NO",
                "LD_RUNPATH_SEARCH_PATHS": "$(inherited) @executable_path/../Frameworks",
                "LIBRARY_SEARCH_PATHS": "$(inherited) $(SRCROOT)/target/xcode",
                "OTHER_LDFLAGS": "$(inherited) $(SRCROOT)/target/xcode/libexvms.a",
                "SWIFT_OBJC_BRIDGING_HEADER": "$(SRCROOT)/App/Support/ExVMs-Bridging-Header.h",
                ],
                configurations: [
                    .debug(name: "Debug", settings: [
                        "CODE_SIGN_STYLE": "Automatic",
                        "CODE_SIGN_IDENTITY": "Apple Development",
                    ]),
                    .release(name: "Release", settings: [
                        "CODE_SIGN_STYLE": "Manual",
                        "CODE_SIGN_IDENTITY": "Developer ID Application",
                    ]),
                ]
            )
        ),
        .target(
            name: "exVMsAppTests",
            destinations: .macOS,
            product: .unitTests,
            bundleId: "com.ixerol.exvms.tests",
            deploymentTargets: .macOS("14.0"),
            infoPlist: .default,
            sources: ["App/Tests/**"],
            dependencies: [.target(name: "exVMsApp")]
        ),
    ]
)
