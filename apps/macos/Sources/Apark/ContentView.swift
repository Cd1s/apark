import SwiftUI

struct ContentView: View {
    @EnvironmentObject var store: AppStore
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Group {
            if store.info == nil {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if store.accounts.isEmpty {
                OnboardingView()
            } else {
                NavigationSplitView {
                    SidebarView()
                        .navigationSplitViewColumnWidth(min: 190, ideal: 220, max: 320)
                } content: {
                    MessageListView()
                        .navigationSplitViewColumnWidth(min: 300, ideal: 370, max: 560)
                } detail: {
                    MessageDetailView()
                }
            }
        }
        .frame(minWidth: 760, minHeight: 480)
        .sheet(isPresented: $store.showAddAccount) {
            AddAccountView().environmentObject(store)
        }
        .onChange(of: store.pendingCompose) { id in
            guard let id else { return }
            openWindow(id: "compose", value: id)
            store.pendingCompose = nil
        }
        .overlay(alignment: .bottom) {
            if let error = store.error, !store.accounts.isEmpty {
                ErrorBanner(message: error) { store.error = nil }
                    .padding(12)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.easeOut(duration: 0.2), value: store.error)
    }
}

struct ErrorBanner: View {
    let message: String
    let dismiss: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.yellow)
            Text(message).font(.callout).lineLimit(2)
            Spacer(minLength: 8)
            Button(action: dismiss) { Image(systemName: "xmark") }.buttonStyle(.borderless)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .shadow(color: .black.opacity(0.15), radius: 8, y: 2)
        .frame(maxWidth: 560)
    }
}
