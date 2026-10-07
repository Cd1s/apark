import SwiftUI
import UniformTypeIdentifiers

struct ComposeView: View {
    @ObservedObject var model: ComposeModel
    @EnvironmentObject var store: AppStore
    @State private var importing = false

    var body: some View {
        VStack(spacing: 0) {
            field("发件人") {
                Picker("", selection: $model.from) {
                    ForEach(store.accounts) { Text($0.email).tag($0.email) }
                }
                .labelsHidden()
                .fixedSize()
                Spacer()
            }
            Divider()
            field("收件人") {
                TextField("", text: $model.to).textFieldStyle(.plain)
                if !model.showCc {
                    Button("抄送/密送") { model.showCc = true }.buttonStyle(.link).font(.caption)
                }
            }
            if model.showCc {
                Divider()
                field("抄送") { TextField("", text: $model.cc).textFieldStyle(.plain) }
                Divider()
                field("密送") { TextField("", text: $model.bcc).textFieldStyle(.plain) }
            }
            Divider()
            field("主题") { TextField("", text: $model.subject).textFieldStyle(.plain).font(.system(size: 13, weight: .semibold)) }
            Divider()
            TextEditor(text: $model.body)
                .font(.system(size: 14))
                .scrollContentBackground(.hidden)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            if !model.attachments.isEmpty {
                Divider()
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(model.attachments, id: \.self) { url in
                            HStack(spacing: 4) {
                                Image(systemName: "paperclip")
                                Text(url.lastPathComponent).lineLimit(1)
                                Button { model.attachments.removeAll { $0 == url } } label: { Image(systemName: "xmark.circle.fill") }
                                    .buttonStyle(.borderless)
                                    .foregroundStyle(.secondary)
                            }
                            .font(.caption)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 4)
                            .background(.quaternary, in: Capsule())
                        }
                    }
                    .padding(10)
                }
            }
            if let err = model.error {
                Label(err, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(10)
            }
        }
        .background(Color(nsColor: .textBackgroundColor))
        .navigationTitle(model.subject.isEmpty ? "新邮件" : model.subject)
        .toolbar {
            ToolbarItemGroup {
                Button { importing = true } label: { Label("附件", systemImage: "paperclip") }.help("添加附件（也可以直接拖进来）")
                Button { send() } label: {
                    if model.sending { ProgressView().controlSize(.small) } else { Label("发送", systemImage: "paperplane.fill") }
                }
                .keyboardShortcut(.return, modifiers: .command)
                .disabled(!model.canSend)
                .help("发送 ⌘↩")
            }
        }
        .onDrop(of: [.fileURL], isTargeted: nil) { providers in
            for p in providers {
                _ = p.loadObject(ofClass: URL.self) { url, _ in
                    if let url { DispatchQueue.main.async { model.attachments.append(url) } }
                }
            }
            return true
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            if case let .success(urls) = result { model.attachments.append(contentsOf: urls) }
        }
        .frame(minWidth: 520, minHeight: 420)
    }

    private func field<Content: View>(_ label: String, @ViewBuilder _ content: () -> Content) -> some View {
        HStack(spacing: 8) {
            Text(label).foregroundStyle(.secondary).frame(width: 52, alignment: .trailing)
            content()
        }
        .font(.system(size: 13))
        .padding(.horizontal, 14)
        .frame(minHeight: 36)
    }

    private func send() {
        model.sending = true
        model.error = nil
        Task {
            do {
                try await store.send(model)
                NSApp.keyWindow?.close()
            } catch {
                model.error = error.localizedDescription
            }
            model.sending = false
        }
    }
}
