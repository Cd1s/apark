// swift-tools-version:5.9
// Native macOS UI. Links the Rust core (crates/ffi) as a static library from ./lib
// (CI puts a universal libapark_ffi.a there; see .github/workflows/build.yml).
import PackageDescription

let libDir = Context.environment["APARK_LIB_DIR"] ?? (Context.packageDirectory + "/lib")

let package = Package(
    name: "Apark",
    platforms: [.macOS(.v13)],
    targets: [
        .systemLibrary(name: "CApark", path: "Sources/CApark"),
        .executableTarget(
            name: "Apark",
            dependencies: ["CApark"],
            path: "Sources/Apark",
            linkerSettings: [
                .unsafeFlags(["-L", libDir]),
                .linkedLibrary("apark_ffi"),
                .linkedFramework("AppKit"),
                .linkedFramework("WebKit"),
                .linkedFramework("Security"),
                .linkedFramework("SystemConfiguration"),
                .linkedFramework("CoreFoundation"),
                .linkedFramework("CoreServices"),
                .linkedLibrary("iconv"),
            ]
        ),
    ]
)
