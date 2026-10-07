import AppKit
import Foundation

/// Entry point. With arguments, behave as the `apark` CLI bundled next to this
/// binary (as apark-cli: the volume is usually case-insensitive), so the
/// desktop app accepts every CLI command too.
@main
enum Entry {
    static func main() {
        let args = CommandLine.arguments
        if args.count > 1, let first = args.dropFirst().first, !first.hasPrefix("-psn_"), !first.hasPrefix("-NS"), !first.hasPrefix("-Apple") {
            let cli = Bundle.main.executableURL!.deletingLastPathComponent().appendingPathComponent("apark-cli").path
            if FileManager.default.isExecutableFile(atPath: cli) {
                var cargs: [UnsafeMutablePointer<CChar>?] = ([cli] + args.dropFirst()).map { strdup($0) }
                cargs.append(nil)
                execv(cli, &cargs)
            }
            FileHandle.standardError.write("apark CLI not found next to the app binary\n".data(using: .utf8)!)
            exit(1)
        }
        AparkApp.main()
    }
}
