import SwiftUI

struct MessageListView: View {
    @EnvironmentObject var store: AppStore

    var body: some View {
        let showAccount = store.accounts.count > 1
        List(selection: $store.selection) {
            ForEach(store.messages) { m in
                MessageRow(m: m, showAccount: showAccount)
                    .tag(m.id)
                    .swipeActions(edge: .leading) {
                        Button { store.toggleSeen(m) } label: {
                            Label(m.seen ? "未读" : "已读", systemImage: m.seen ? "envelope.badge" : "envelope.open")
                        }
                        .tint(.blue)
                    }
                    .swipeActions(edge: .trailing) {
                        Button(role: .destructive) { store.trash(m) } label: { Label("删除", systemImage: "trash") }
                        Button { store.archive(m) } label: { Label("归档", systemImage: "archivebox") }.tint(.indigo)
                    }
                    .contextMenu {
                        Button("回复") { store.reply(m, all: false) }
                        Button("全部回复") { store.reply(m, all: true) }
                        Button("转发") { store.forward(m) }
                        Divider()
                        Button(m.seen ? "标为未读" : "标为已读") { store.toggleSeen(m) }
                        Button(m.flagged ? "取消星标" : "加星标") { store.toggleFlag(m) }
                        Divider()
                        Button("归档") { store.archive(m) }
                        Button("删除", role: .destructive) { store.trash(m) }
                    }
            }
        }
        .listStyle(.inset(alternatesRowBackgrounds: false))
        .overlay {
            if store.messages.isEmpty {
                VStack(spacing: 10) {
                    Image(systemName: store.search.isEmpty ? "tray" : "magnifyingglass")
                        .font(.system(size: 40, weight: .light))
                        .foregroundStyle(.tertiary)
                    Text(store.search.isEmpty ? (store.syncing ? "正在同步…" : "没有邮件") : "没有找到“\(store.search)”")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .searchable(text: $store.search, prompt: "搜索邮件")
        .navigationTitle(store.title)
        .navigationSubtitle(store.totalUnread > 0 ? "\(store.totalUnread) 封未读" : "")
        .toolbar {
            ToolbarItemGroup {
                Button { store.syncNow() } label: { Label("收取", systemImage: "arrow.clockwise") }
                    .disabled(store.syncing)
                    .help("收取新邮件 ⇧⌘N")
                Button { store.compose() } label: { Label("写邮件", systemImage: "square.and.pencil") }
                    .help("新邮件 ⌘N")
            }
        }
    }
}

struct MessageRow: View {
    let m: Message
    let showAccount: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Circle()
                .fill(m.seen ? Color.clear : Color.accentColor)
                .frame(width: 8, height: 8)
                .padding(.top, 5)
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(m.sender)
                        .font(.system(size: 13, weight: m.seen ? .medium : .bold))
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    if m.flagged {
                        Image(systemName: "star.fill").font(.system(size: 9)).foregroundStyle(.orange)
                    }
                    Text(Fmt.short(m.when))
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                }
                Text(m.displaySubject)
                    .font(.system(size: 12.5, weight: m.seen ? .regular : .semibold))
                    .lineLimit(1)
                if !m.snippet.isEmpty {
                    Text(m.snippet)
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                if showAccount {
                    HStack(spacing: 4) {
                        Circle().fill(accentColor(for: m.account)).frame(width: 6, height: 6)
                        Text(m.account).font(.system(size: 10.5)).foregroundStyle(.tertiary).lineLimit(1)
                    }
                    .padding(.top, 1)
                }
            }
        }
        .padding(.vertical, 5)
    }
}
