import SwiftUI

/// Account-list sync without Google: self-hosted `apark server`, WebDAV, or a synced folder.
struct SyncSetupView: View {
    @EnvironmentObject var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @State private var kind = "server"
    @State private var url = ""
    @State private var user = ""
    @State private var password = ""
    @State private var token = ""
    @State private var path = ""
    @State private var passphrase = ""
    @State private var busy = false
    @State private var error: String?

    private var valid: Bool {
        switch kind {
        case "server": return !url.isEmpty && !user.isEmpty && !password.isEmpty
        case "webdav": return !url.isEmpty && !user.isEmpty && !password.isEmpty && !passphrase.isEmpty
        default: return !path.isEmpty && !passphrase.isEmpty
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            Form {
                Picker("方式", selection: $kind) {
                    Text("自建服务器").tag("server")
                    Text("WebDAV").tag("webdav")
                    Text("同步文件夹").tag("file")
                }
                .pickerStyle(.segmented)

                switch kind {
                case "server":
                    Section {
                        TextField("服务器地址", text: $url, prompt: Text("https://sync.example.com"))
                        TextField("用户名", text: $user)
                        SecureField("密码", text: $password, prompt: Text("同时用于加密，服务器看不到明文"))
                        TextField("访问令牌", text: $token, prompt: Text("可选，服务器设置了 --token 时填写"))
                    } footer: {
                        Text("在任意服务器上运行 `apark server --token 你的令牌` 即可，建议放在 HTTPS 反向代理后面。")
                    }
                case "webdav":
                    Section {
                        TextField("文件地址", text: $url, prompt: Text("https://dav.example.com/apark/accounts.json"))
                        TextField("用户名", text: $user)
                        SecureField("WebDAV 密码", text: $password)
                        SecureField("同步密码", text: $passphrase, prompt: Text("用来加密账号列表"))
                    } footer: {
                        Text("坚果云、Nextcloud、群晖等都支持 WebDAV。")
                    }
                default:
                    Section {
                        HStack {
                            TextField("文件路径", text: $path, prompt: Text("~/Library/Mobile Documents/com~apple~CloudDocs/Apark/accounts.json"))
                            Button("选择…") { choose() }
                        }
                        SecureField("同步密码", text: $passphrase, prompt: Text("用来加密账号列表"))
                    } footer: {
                        Text("放在 iCloud Drive、Dropbox、Syncthing 等会自动同步的文件夹里。")
                    }
                }
                if let error {
                    Text(error).foregroundStyle(.red).font(.callout)
                }
            }
            .formStyle(.grouped)

            HStack {
                if busy {
                    ProgressView().controlSize(.small)
                    Text("正在同步账号列表…").font(.callout).foregroundStyle(.secondary)
                }
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("开始同步") { submit() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!valid || busy)
            }
            .padding(16)
        }
        .frame(width: 500)
    }

    private func choose() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "apark-accounts.json"
        panel.message = "选择同步文件夹里的账号列表文件位置"
        if panel.runModal() == .OK, let url = panel.url { path = url.path }
    }

    private func submit() {
        busy = true
        error = nil
        let expanded = (path as NSString).expandingTildeInPath
        let params: [String: Any] = [
            "type": kind, "url": url, "user": user, "password": password,
            "token": token, "path": expanded, "passphrase": passphrase,
        ]
        Task {
            do {
                _ = try await store.setupSync(params)
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
            busy = false
        }
    }
}
