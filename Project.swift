import ProjectDescription

let rustBuildScript = """
set -euo pipefail
cd "$SRCROOT"

if [ -n "${CARGO:-}" ]; then
  "$CARGO" build --release --lib --no-default-features
elif command -v cargo >/dev/null 2>&1; then
  cargo build --release --lib --no-default-features
elif [ -n "${SHELL:-}" ]; then
  "$SHELL" -lc 'cd "$SRCROOT" && cargo build --release --lib --no-default-features'
else
  echo "error: cargo was not found. Set CARGO or make cargo available through Xcode's PATH/login shell." >&2
  exit 127
fi
"""

let rustEmbedScript = """
set -euo pipefail
mkdir -p "$TARGET_BUILD_DIR/$FRAMEWORKS_FOLDER_PATH"
cp "$SRCROOT/target/release/libexvms.dylib" "$TARGET_BUILD_DIR/$FRAMEWORKS_FOLDER_PATH/libexvms.dylib"
"""

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
            bundleId: "com.ixerol.exvms",
            deploymentTargets: .macOS("14.0"),
            infoPlist: .extendingDefault(with: [
                "CFBundleDisplayName": "exVMs",
                "CFBundleDevelopmentRegion": "en",
                "CFBundleLocalizations": ["en", "zh-Hans", "zh-Hant", "ja"],
                "LSApplicationCategoryType": "public.app-category.utilities",
                "NSQuitAlwaysKeepsWindows": false,
            ]),
            sources: ["App/Sources/**"],
            resources: ["App/Resources/**"],
            scripts: [
                .pre(script: rustBuildScript, name: "Build Rust Library", basedOnDependencyAnalysis: false),
                .post(script: rustEmbedScript, name: "Embed Rust Library", basedOnDependencyAnalysis: false),
            ],
            dependencies: [],
            settings: .settings(base: [
                "CODE_SIGN_ENTITLEMENTS": "",
                "ENABLE_APP_SANDBOX": "NO",
                "LD_RUNPATH_SEARCH_PATHS": "$(inherited) @executable_path/../Frameworks",
                "LIBRARY_SEARCH_PATHS": "$(inherited) $(SRCROOT)/target/release",
                "OTHER_LDFLAGS": "$(inherited) -lexvms",
                "SWIFT_OBJC_BRIDGING_HEADER": "$(SRCROOT)/App/Support/ExVMs-Bridging-Header.h",
            ])
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
