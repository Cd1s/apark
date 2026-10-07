import SwiftUI

struct AparkApp: App {
    @StateObject private var store = AppStore()

    var body: some Scene {
        WindowGroup("Apark") {
            ContentView()
                .environmentObject(store)
                .task { await store.bootstrap() }
        }
        .defaultSize(width: 1180, height: 760)
        .windowToolbarStyle(.unified)
        .commands { MailCommands(store: store) }

        WindowGroup("新邮件", id: "compose", for: UUID.self) { $id in
            if let id, let model = store.drafts[id] {
                ComposeView(model: model).environmentObject(store)
            }
        }
        .defaultSize(width: 640, height: 560)

        Settings {
            SettingsView().environmentObject(store)
        }
    }
}

struct MailCommands: Commands {
    @ObservedObject var store: AppStore

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("新邮件") { store.compose() }.keyboardShortcut("n")
        }
        CommandMenu("邮件") {
            Button("回复") { if let m = store.selected { store.reply(m, all: false) } }
                .keyboardShortcut("r")
                .disabled(store.selected == nil)
            Button("全部回复") { if let m = store.selected { store.reply(m, all: true) } }
                .keyboardShortcut("r", modifiers: [.command, .shift])
                .disabled(store.selected == nil)
            Button("转发") { if let m = store.selected { store.forward(m) } }
                .keyboardShortcut("f", modifiers: [.command, .shift])
                .disabled(store.selected == nil)
            Divider()
            Button("归档") { if let m = store.selected { store.archive(m) } }
                .keyboardShortcut("a", modifiers: [.command, .control])
                .disabled(store.selected == nil)
            Button("删除") { if let m = store.selected { store.trash(m) } }
                .keyboardShortcut(.delete, modifiers: .command)
                .disabled(store.selected == nil)
            Button(store.selected?.seen == false ? "标为已读" : "标为未读") { if let m = store.selected { store.toggleSeen(m) } }
                .keyboardShortcut("u", modifiers: [.command, .shift])
                .disabled(store.selected == nil)
            Button(store.selected?.flagged == true ? "取消星标" : "加星标") { if let m = store.selected { store.toggleFlag(m) } }
                .keyboardShortcut("l", modifiers: [.command, .shift])
                .disabled(store.selected == nil)
            Divider()
            Button("收取新邮件") { store.syncNow() }
                .keyboardShortcut("n", modifiers: [.command, .shift])
            Button("添加账号…") { store.showAddAccount = true }
        }
    }
}
