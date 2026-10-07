import SwiftUI

struct AddAccountView: View {
    @EnvironmentObject var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @State private var kind = 0
    @State private var email = ""
    @State private var password = ""
    @State private var name = ""
    @State private var imap = ""
    @State private var smtp = ""
    @State private var error: String?

    var body: some View {
        VStack(spacing: 0) {
            Form {
                Picker("类型", selection: $kind) {
                    Text("Google").tag(0)
                    Text("Microsoft").tag(1)
                    Text("其他邮箱").tag(2)
                }
                .pickerStyle(.segmented)

                if kind == 2 {
                    Section {
                        TextField("邮箱", text: $email, prompt: Text("you@example.com"))
                        SecureField("密码", text: $password, prompt: Text("密码或应用专用密码"))
                        TextField("名字", text: $name, prompt: Text("可选"))
                    }
                    Section("服务器（留空自动识别）") {
                        TextField("IMAP", text: $imap, prompt: Text("imap.example.com:993"))
                        TextField("SMTP", text: $smtp, prompt: Text("smtp.example.com:465"))
                    }
                } else {
                    Section {
                        Text(kind == 0 ? "会在浏览器中打开 Google 授权页面。" : "会在浏览器中打开 Microsoft 授权页面。")
                            .foregroundStyle(.secondary)
                        if (kind == 0 && store.info?.googleReady != true) || (kind == 1 && store.info?.microsoftReady != true) {
                            Label("还没有配置 OAuth 客户端，请先在“设置”里填写。", systemImage: "exclamationmark.triangle")
                                .foregroundStyle(.orange)
                        }
                    }
                }
                if let error = error ?? store.error {
                    Text(error).foregroundStyle(.red).font(.callout)
                }
            }
            .formStyle(.grouped)

            HStack {
                if let busy = store.busy {
                    ProgressView().controlSize(.small)
                    Text(busy).font(.callout).foregroundStyle(.secondary)
                    if let url = store.loginURL { Link("打开授权页", destination: url).font(.callout) }
                }
                Spacer()
                Button("取消") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(kind == 2 ? "添加" : "继续") { submit() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(store.busy != nil || (kind == 2 && (!email.contains("@") || password.isEmpty)))
            }
            .padding(16)
        }
        .frame(width: 460)
    }

    private func submit() {
        error = nil
        switch kind {
        case 0: store.addOAuth("google")
        case 1: store.addOAuth("microsoft")
        default:
            Task {
                do {
                    try await store.addPassword(email: email, password: password, name: name, imap: imap, smtp: smtp)
                    dismiss()
                } catch {
                    self.error = error.localizedDescription
                }
            }
        }
    }
}
