import SwiftUI
import WebKit

struct MessageDetailView: View {
    @EnvironmentObject var store: AppStore

    var body: some View {
        if let m = store.selected {
            VStack(alignment: .leading, spacing: 0) {
                header(m)
                Divider()
                content(m)
            }
            .background(Color(nsColor: .textBackgroundColor))
            .toolbar { toolbar(m) }
        } else {
            VStack(spacing: 12) {
                Image(systemName: "envelope.open")
                    .font(.system(size: 52, weight: .ultraLight))
                    .foregroundStyle(.tertiary)
                Text("没有选中邮件").font(.title3).foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func header(_ m: Message) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(m.displaySubject)
                .font(.system(size: 20, weight: .semibold))
                .textSelection(.enabled)
            HStack(alignment: .top, spacing: 10) {
                Avatar(name: m.sender, color: addressColor(m.fromAddr))
                VStack(alignment: .leading, spacing: 3) {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(m.sender).font(.headline)
                        Text(m.fromAddr).font(.subheadline).foregroundStyle(.secondary).textSelection(.enabled)
                    }
                    Text("收件人：\(m.to)").font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.tail)
                    if !m.cc.isEmpty {
                        Text("抄送：\(m.cc)").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                Spacer(minLength: 12)
                Text(Fmt.long(m.when)).font(.caption).foregroundStyle(.secondary)
            }
            if let atts = store.body?.attachments, !atts.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(Array(atts.enumerated()), id: \.offset) { i, a in
                            Button { store.saveAttachment(m, index: i) } label: {
                                Label("\(a.name)  \(Fmt.size(a.size))", systemImage: "paperclip")
                            }
                            .buttonStyle(.bordered)
                            .help("保存到“下载”并在访达中显示")
                        }
                    }
                }
            }
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 18)
    }

    @ViewBuilder
    private func content(_ m: Message) -> some View {
        if let body = store.body {
            if let html = body.html {
                HTMLView(html: html)
            } else {
                ScrollView {
                    Text(linkified(body.text.trimmingCharacters(in: .whitespacesAndNewlines)))
                        .font(.system(size: 14))
                        .lineSpacing(3)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(24)
                }
            }
        } else if let err = store.bodyError {
            Label(err, systemImage: "exclamationmark.triangle")
                .foregroundStyle(.secondary)
                .padding(24)
            Spacer()
        } else {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    @ToolbarContentBuilder
    private func toolbar(_ m: Message) -> some ToolbarContent {
        ToolbarItemGroup {
            Button { store.archive(m) } label: { Label("归档", systemImage: "archivebox") }.help("归档 ⌃⌘A")
            Button { store.trash(m) } label: { Label("删除", systemImage: "trash") }.help("删除 ⌘⌫")
            Button { store.toggleFlag(m) } label: {
                Label("星标", systemImage: m.flagged ? "star.fill" : "star")
            }
            .help("星标 ⇧⌘L")
        }
        ToolbarItemGroup {
            Button { store.reply(m, all: false) } label: { Label("回复", systemImage: "arrowshape.turn.up.left") }.help("回复 ⌘R")
            Button { store.reply(m, all: true) } label: { Label("全部回复", systemImage: "arrowshape.turn.up.left.2") }.help("全部回复 ⇧⌘R")
            Button { store.forward(m) } label: { Label("转发", systemImage: "arrowshape.turn.up.right") }.help("转发 ⇧⌘F")
        }
    }

    private func linkified(_ text: String) -> AttributedString {
        var out = AttributedString(text)
        guard let detector = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.link.rawValue) else { return out }
        let ns = text as NSString
        for match in detector.matches(in: text, range: NSRange(location: 0, length: ns.length)) {
            guard let url = match.url, let range = Range(match.range, in: text),
                  let lower = AttributedString.Index(range.lowerBound, within: out),
                  let upper = AttributedString.Index(range.upperBound, within: out) else { continue }
            out[lower..<upper].link = url
        }
        return out
    }
}

struct Avatar: View {
    let name: String
    let color: Color
    var size: CGFloat = 38

    var body: some View {
        Circle()
            .fill(color.gradient)
            .frame(width: size, height: size)
            .overlay(
                Text(String(name.trimmingCharacters(in: .whitespaces).prefix(1)).uppercased())
                    .font(.system(size: size * 0.42, weight: .semibold))
                    .foregroundStyle(.white)
            )
    }
}

/// HTML mail in WebKit: no JavaScript, links open in the default browser.
struct HTMLView: NSViewRepresentable {
    let html: String

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.defaultWebpagePreferences.allowsContentJavaScript = false
        let view = WKWebView(frame: .zero, configuration: config)
        view.navigationDelegate = context.coordinator
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        guard context.coordinator.loaded != html else { return }
        context.coordinator.loaded = html
        let page = """
        <!doctype html><html><head><meta charset="utf-8">
        <meta name="viewport" content="width=device-width, initial-scale=1">
        <style>
        body { font: 14px -apple-system, "PingFang SC", sans-serif; margin: 20px 24px; word-wrap: break-word; }
        img { max-width: 100%; height: auto; }
        table { max-width: 100%; }
        </style></head><body>\(html)</body></html>
        """
        view.loadHTMLString(page, baseURL: nil)
    }

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate {
        var loaded: String?

        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction) async -> WKNavigationActionPolicy {
            if action.navigationType == .linkActivated, let url = action.request.url {
                NSWorkspace.shared.open(url)
                return .cancel
            }
            return .allow
        }
    }
}
