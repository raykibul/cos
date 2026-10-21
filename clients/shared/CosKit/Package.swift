// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "CosKit",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [
        .library(name: "CosKit", targets: ["CosKit"]),
    ],
    targets: [
        .target(name: "CosKit"),
        .testTarget(name: "CosKitTests", dependencies: ["CosKit"]),
    ]
)
