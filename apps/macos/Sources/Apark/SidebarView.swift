import SwiftUI

struct SidebarView: View {
    @EnvironmentObject var store: AppStore
    @State private var expanded: Set<String> = []

    private func count(_ key: String) -> Int { store.unread[key] ?? 0 }

    var body: some View {
        List(selection: $store.nav) {
            Section("智能收件箱") {
                row("收件箱", "tray.fill", .inbox, store.totalUnread)
                row("个人", "person.fill", .people, count("people"))
                row("通知", "bell.fill", .notifications, count("notification"))
                row("订阅", "newspaper.fill", .newsletters, count("newsletter"))
            }
            Section("快速筛选") {
                row("未读", "envelope.badge.fill", .unread, 0)
                row("星标", "star.fill", .flagged, 0)
            }
            Section("账号") {
                ForEach(store.accounts) { account in
                    DisclosureGroup(isExpanded: Binding(
                        get: { expanded.contains(account.email) },
                        set: { if $0 { expanded.insert(account.email) } else { expanded.remove(account.email) } }
                    )) {
                        ForEach(store.folders[account.email] ?? [], id: \.self) { folder in
                            Label(folder.title, systemImage: folder.symbol)
                                .tag(Nav.folder(account: account.email, name: folder.name))
                        }
                    } label: {
                        HStack(spacing: 8) {
                            Circle().fill(addressColor(account.email)).frame(width: 8, height: 8)
                            Text(account.email).lineLimit(1).truncationMode(.middle)
                            if account.master {
                                Image(systemName: "sparkle").font(.caption2).foregroundStyle(.secondary)
                                    .help("总账号：账号列表同步在这个 Google 账号里")
                            }
                        }
                        .contextMenu {
                            Button("立即同步") { store.syncNow() }
                            Divider()
                            Button("删除账号", role: .destructive) { store.removeAccount(account.email) }
                        }
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .safeAreaInset(edge: .bottom) {
            HStack(spacing: 8) {
                if store.syncing {
                    ProgressView().controlSize(.small)
                    Text("正在同步…")
                } else if let last = store.lastSync {
                    Image(systemName: "checkmark.circle")
                    Text("已同步 \(Fmt.short(last))")
                }
                Spacer()
                Button { store.showAddAccount = true } label: { Image(systemName: "plus") }
                    .buttonStyle(.borderless)
                    .help("添加账号")
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
        }
    }

    private func row(_ title: String, _ symbol: String, _ nav: Nav, _ badge: Int) -> some View {
        Label(title, systemImage: symbol)
            .badge(badge)
            .tag(nav)
    }
}
