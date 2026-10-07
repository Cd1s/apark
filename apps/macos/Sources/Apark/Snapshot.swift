import AppKit

/// CI/dev helper: with APARK_SNAPSHOT=<dir>, render the UI to PNGs and quit.
@MainActor
enum Snapshot {
    static func run(store: AppStore, dir: String) {
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 2_500_000_000)
            if let first = store.messages.first { store.selection = first.id }
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            capture(dir, "main")
            if !store.accounts.isEmpty {
                store.compose(draft: Draft(to: ["sarah@example.org"], subject: "周末计划", body: "嗨 Sarah，\n\n周六一起去看展吗？\n"))
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                capture(dir, "compose")
            }
            NSApp.terminate(nil)
        }
    }

    static func capture(_ dir: String, _ prefix: String) {
        for (i, window) in NSApp.windows.enumerated() where window.isVisible {
            guard let view = window.contentView?.superview ?? window.contentView,
                  let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { continue }
            view.cacheDisplay(in: view.bounds, to: rep)
            let url = URL(fileURLWithPath: dir).appendingPathComponent("\(prefix)-\(i).png")
            try? rep.representation(using: .png, properties: [:])?.write(to: url)
        }
    }
}
