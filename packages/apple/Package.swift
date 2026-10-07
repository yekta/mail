// swift-tools-version:5.10
import PackageDescription

// What the Mac app and the iOS app share: the bridge to the Rust core, the state, and the views.
// The apps link the core's static library themselves.
let package = Package(
    name: "MailUI",
    platforms: [.macOS(.v14), .iOS("18.0")],
    products: [
        .library(name: "MailUI", targets: ["MailUI"])
    ],
    targets: [
        .target(name: "CMailCore", path: "Sources/CMailCore"),
        .target(name: "MailUI", dependencies: ["CMailCore"], path: "Sources/MailUI", resources: [.copy("Fonts")]),
    ],
    swiftLanguageVersions: [.v5]
)
