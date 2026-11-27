// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ElysianMobileShim",
    platforms: [
        .iOS(.v15),
        .macOS(.v12),
    ],
    products: [
        .library(
            name: "ElysianMobileShim",
            targets: ["ElysianMobileShim"]
        ),
    ],
    targets: [
        .target(
            name: "ElysianMobileShim",
            path: "Sources/ElysianMobileShim"
        ),
        .testTarget(
            name: "ElysianMobileShimTests",
            dependencies: ["ElysianMobileShim"],
            path: "Tests/ElysianMobileShimTests"
        ),
    ]
)
