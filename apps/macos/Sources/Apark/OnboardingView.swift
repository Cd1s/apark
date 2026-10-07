import SwiftUI

struct OnboardingView: View {
    @EnvironmentObject var store: AppStore
    @State private var clientId = ""
    @State private var clientSecret = ""

    var body: some View {
        VStack(spacing: 16) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 112, height: 112)
                .shadow(color: .black.opacity(0.12), radius: 10, y: 4)
            Text("欢迎使用 Apark").font(.system(size: 30, weight: .bold))
            Text("用一个 Google 账号登录，所有邮箱都会回来。")
                .font(.title3)
                .foregroundStyle(.secondary)

            if store.info?.googleReady == false {
                GroupBox {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("首次使用：填入你的 Google OAuth 客户端（桌面应用类型，见 README）")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                        TextField("客户端 ID", text: $clientId)
                        SecureField("客户端密钥", text: $clientSecret)
                        HStack {
                            Spacer()
                            Button("保存") { saveClient() }
                                .disabled(clientId.trimmingCharacters(in: .whitespaces).isEmpty)
                        }
                    }
                    .textFieldStyle(.roundedBorder)
                    .padding(6)
                }
                .frame(width: 420)
                .padding(.top, 8)
            }

            Button { store.loginMaster() } label: {
                Text("使用 Google 账号登录").frame(width: 240)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(store.info?.googleReady != true || store.busy != nil)
            .padding(.top, 8)

            Button("添加其他邮箱…") { store.showAddAccount = true }
                .buttonStyle(.link)

            if let busy = store.busy {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(busy).foregroundStyle(.secondary)
                }
                if let url = store.loginURL {
                    Link("浏览器没有打开？点这里", destination: url).font(.callout)
                }
            }
            if let error = store.error {
                Text(error)
                    .font(.callout)
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 460)
            }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func saveClient() {
        Task {
            guard var c = await store.loadConfig() else { return }
            c.googleClientId = clientId.trimmingCharacters(in: .whitespaces)
            c.googleClientSecret = clientSecret.isEmpty ? nil : clientSecret
            await store.saveConfig(c)
        }
    }
}
