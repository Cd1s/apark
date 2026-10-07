import SwiftUI

struct SettingsView: View {
    @EnvironmentObject var store: AppStore
    @State private var config: Config?
    @State private var saved = false

    var body: some View {
        TabView {
            accounts.tabItem { Label("账号", systemImage: "person.2") }
            general.tabItem { Label("同步", systemImage: "arrow.triangle.2.circlepath") }
            oauth.tabItem { Label("OAuth", systemImage: "key") }
        }
        .frame(width: 540, height: 400)
        .task { config = await store.loadConfig() }
    }

    private var accounts: some View {
        VStack(alignment: .leading, spacing: 12) {
            List {
                ForEach(store.accounts) { a in
                    HStack(spacing: 10) {
                        Avatar(name: a.email, color: addressColor(a.email), size: 28)
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 4) {
                                Text(a.email)
                                if a.master { Text("总账号").font(.caption2).padding(.horizontal, 5).background(.quaternary, in: Capsule()) }
                            }
                            Text(a.provider == "imap" ? a.imap : a.provider.capitalized).font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("删除", role: .destructive) { store.removeAccount(a.email) }
                    }
                    .padding(.vertical, 2)
                }
            }
            HStack {
                Button("添加账号…") { store.showAddAccount = true }
                Spacer()
                if store.info?.sync != nil {
                    Button("立即同步账号列表") { Task { try? await Core.shared.run("cloud_sync"); await store.refreshAccounts() } }
                }
            }
        }
        .padding(20)
    }

    private var general: some View {
        Form {
            Section("账号同步") {
                if let sync = store.info?.sync {
                    LabeledContent(sync.title) { Text(sync.location).textSelection(.enabled) }
                    HStack {
                        Button("更换…") { store.showSyncSetup = true }
                        Button("停止同步", role: .destructive) { store.syncOff() }
                    }
                } else {
                    Text("未开启。开启后，在新设备上登录一次，所有邮箱都会回来。").foregroundStyle(.secondary)
                    Button("设置同步…") { store.showSyncSetup = true }
                }
            }
            if let binding = Binding($config) {
                Stepper("同步间隔：\(binding.wrappedValue.syncIntervalSecs) 秒", value: binding.syncIntervalSecs, in: 30...3600, step: 30)
                Stepper("首次同步：每个文件夹 \(binding.wrappedValue.initialLimit) 封", value: binding.initialLimit, in: 50...20000, step: 50)
                SecureField("云同步密码（可选）", text: optional(binding.syncPassphrase))
                Text("WebDAV 和同步文件夹用这个密码加密账号列表；Google 方式可选。每台设备要填相同的密码。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                saveRow
            }
            if let dir = store.info?.dataDir {
                LabeledContent("数据目录") { Text(dir).textSelection(.enabled).font(.caption) }
            }
        }
        .formStyle(.grouped)
    }

    private var oauth: some View {
        Form {
            if let binding = Binding($config) {
                Section("Google") {
                    TextField("客户端 ID", text: optional(binding.googleClientId))
                    SecureField("客户端密钥", text: optional(binding.googleClientSecret))
                }
                Section("Microsoft") {
                    TextField("应用程序（客户端）ID", text: optional(binding.microsoftClientId))
                }
                saveRow
            }
        }
        .formStyle(.grouped)
    }

    private var saveRow: some View {
        HStack {
            if saved { Label("已保存", systemImage: "checkmark").foregroundStyle(.secondary) }
            Spacer()
            Button("保存") {
                guard let c = config else { return }
                Task {
                    await store.saveConfig(c)
                    saved = true
                }
            }
            .keyboardShortcut(.defaultAction)
        }
    }

    private func optional(_ b: Binding<String?>) -> Binding<String> {
        Binding(get: { b.wrappedValue ?? "" }, set: { b.wrappedValue = $0.isEmpty ? nil : $0; saved = false })
    }
}
