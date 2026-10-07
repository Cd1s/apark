import AppKit
import SwiftUI

/// Observable UI state. Reads hit the local cache through the core and return in
/// milliseconds; mutations update the UI first, then go to the server.
@MainActor
final class AppStore: ObservableObject {
    @Published var info: Info?
    @Published var accounts: [Account] = []
    @Published var folders: [String: [Folder]] = [:]
    @Published var nav: Nav? = .inbox {
        didSet {
            guard nav != oldValue else { return }
            selection = nil
            if case let .folder(account, name)? = nav, name.uppercased() != "INBOX" {
                Task { try? await core.run("sync_account", ["email": account, "folders": [name]]); await reload() }
            }
            Task { await reload() }
        }
    }
    @Published var messages: [Message] = []
    @Published var selection: Message.ID? {
        didSet { if selection != oldValue { Task { await loadSelected() } } }
    }
    @Published var body: MailBody?
    @Published var bodyError: String?
    @Published var search = "" {
        didSet { scheduleReload() }
    }
    @Published var unread: [String: Int] = [:]
    @Published var syncing = false
    @Published var lastSync: Date?
    @Published var error: String?
    @Published var busy: String?
    @Published var loginURL: URL?
    @Published var showAddAccount = false
    @Published var showSyncSetup = false
    @Published var pendingCompose: UUID?

    var drafts: [UUID: ComposeModel] = [:]

    private let core = Core.shared
    private var reloadTask: Task<Void, Never>?
    private var eventTask: Task<Void, Never>?
    private let env = ProcessInfo.processInfo.environment

    var selected: Message? { messages.first { $0.id == selection } }
    var totalUnread: Int { unread.values.reduce(0, +) }

    // MARK: - Lifecycle

    func bootstrap() async {
        guard info == nil else { return }
        core.listen()
        eventTask = Task { @MainActor in
            for await note in NotificationCenter.default.notifications(named: .aparkEvent) {
                guard let data = note.object as? Data,
                      let event = try? Core.shared.decoder.decode(CoreEvent.self, from: data) else { continue }
                self.handle(event)
            }
        }
        do {
            info = try await core.call("info")
        } catch {
            self.error = error.localizedDescription
            info = Info(version: "?", dataDir: "", hasMaster: false, googleReady: false, microsoftReady: false, sync: nil)
        }
        await refreshAccounts()
        await reload()
        startAutoSync()
        if let dir = env["APARK_SNAPSHOT"] { Snapshot.run(store: self, dir: dir) }
    }

    private func startAutoSync() {
        guard !accounts.isEmpty, env["APARK_NO_SYNC"] == nil else { return }
        syncing = true
        Task { try? await core.run("start_auto_sync") }
    }

    private func handle(_ event: CoreEvent) {
        switch event.type {
        case "synced":
            syncing = false
            lastSync = Date()
            let failures = (event.results ?? []).filter { !$0.ok }
            error = failures.first.map { "\($0.account)：\($0.error ?? "同步失败")" }
            Task { await refreshAccounts(); await reload() }
        case "login_url":
            loginURL = event.url.flatMap(URL.init(string:))
        default:
            break
        }
    }

    func refreshAccounts() async {
        accounts = (try? await core.call("accounts")) ?? []
        var map: [String: [Folder]] = [:]
        for a in accounts {
            map[a.email] = (try? await core.call("folders", ["email": a.email])) ?? []
        }
        folders = map
        if let i: Info = try? await core.call("info") { info = i }
    }

    // MARK: - Lists

    private func query() -> [String: Any] {
        var q: [String: Any] = ["limit": 5000]
        let s = search.trimmingCharacters(in: .whitespaces)
        if !s.isEmpty { q["search"] = s }
        switch nav ?? .inbox {
        case .inbox: break
        case .people: q["category"] = "people"
        case .notifications: q["category"] = "notification"
        case .newsletters: q["category"] = "newsletter"
        case .unread: q["unread"] = true; q["folder"] = "INBOX"
        case .flagged: q["flagged"] = true; q["folder"] = "INBOX"
        case let .folder(account, name): q["account"] = account; q["folder"] = name
        }
        return q
    }

    func reload() async {
        do {
            messages = try await core.call("list", query())
            unread = (try? await core.call("unread_counts")) ?? [:]
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func scheduleReload() {
        reloadTask?.cancel()
        reloadTask = Task {
            try? await Task.sleep(nanoseconds: 120_000_000)
            guard !Task.isCancelled else { return }
            await reload()
        }
    }

    var title: String {
        switch nav ?? .inbox {
        case .inbox: return "收件箱"
        case .people: return "个人"
        case .notifications: return "通知"
        case .newsletters: return "订阅"
        case .unread: return "未读"
        case .flagged: return "星标"
        case let .folder(account, name):
            return (folders[account]?.first { $0.name == name } ?? Folder(name: name, role: "", label: nil)).title
        }
    }

    // MARK: - Reading

    private func loadSelected() async {
        body = nil
        bodyError = nil
        guard let id = selection else { return }
        if let cached: MailBody = try? await core.callOptional("cached_body", ["id": id]) {
            if selection == id { body = cached }
        } else {
            do {
                let b: MailBody = try await core.call("body", ["id": id])
                if selection == id { body = b }
            } catch {
                if selection == id { bodyError = error.localizedDescription }
            }
        }
        if let i = messages.firstIndex(where: { $0.id == id }), !messages[i].seen {
            messages[i].seen = true
            perform("set_seen", ["id": id, "seen": true])
        }
    }

    // MARK: - Actions

    private func perform(_ method: String, _ params: [String: Any], reloadAfter: Bool = true) {
        Task {
            do {
                try await core.run(method, params)
            } catch {
                self.error = error.localizedDescription
            }
            if reloadAfter { await reload() }
        }
    }

    /// Drop a message from the list now and select its neighbour.
    private func takeOut(_ m: Message) {
        guard let i = messages.firstIndex(of: m) else { return }
        messages.remove(at: i)
        if selection == m.id {
            selection = messages.isEmpty ? nil : messages[min(i, messages.count - 1)].id
        }
    }

    func archive(_ m: Message) { takeOut(m); perform("archive", ["id": m.id]) }
    func trash(_ m: Message) { takeOut(m); perform("trash", ["id": m.id]) }

    func toggleSeen(_ m: Message) {
        guard let i = messages.firstIndex(of: m) else { return }
        messages[i].seen.toggle()
        perform("set_seen", ["id": m.id, "seen": messages[i].seen])
    }

    func toggleFlag(_ m: Message) {
        guard let i = messages.firstIndex(of: m) else { return }
        messages[i].flagged.toggle()
        perform("set_flagged", ["id": m.id, "flagged": messages[i].flagged])
    }

    func syncNow() {
        guard !syncing, !accounts.isEmpty else { return }
        syncing = true
        Task {
            do { try await core.run("sync_all") } catch { self.error = error.localizedDescription; syncing = false }
        }
    }

    func saveAttachment(_ m: Message, index: Int) {
        Task {
            do {
                let paths: [String] = try await core.call("save_attachments", ["id": m.id, "index": index])
                NSWorkspace.shared.activateFileViewerSelecting(paths.map { URL(fileURLWithPath: $0) })
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    // MARK: - Compose

    func compose(from: String? = nil, draft: Draft = Draft()) {
        let id = UUID()
        drafts[id] = ComposeModel(from: from ?? accounts.first?.email ?? "", draft: draft)
        pendingCompose = id
    }

    func reply(_ m: Message, all: Bool) {
        Task {
            do {
                let r: DraftReply = try await core.call("reply_draft", ["id": m.id, "all": all])
                compose(from: r.from, draft: r.draft)
            } catch { self.error = error.localizedDescription }
        }
    }

    func forward(_ m: Message) {
        Task {
            do {
                let r: DraftReply = try await core.call("forward_draft", ["id": m.id])
                compose(from: r.from, draft: r.draft)
            } catch { self.error = error.localizedDescription }
        }
    }

    func send(_ model: ComposeModel) async throws {
        var params = core.params(model.draft)
        params["from"] = model.from
        try await core.run("send", params)
    }

    // MARK: - Accounts

    func loginMaster() {
        runLogin("请在浏览器中完成 Google 授权…") { try await self.core.run("login_master") }
    }

    func addOAuth(_ provider: String) {
        runLogin("请在浏览器中完成授权…") { try await self.core.run("add_oauth", ["provider": provider]) }
    }

    func addPassword(email: String, password: String, name: String, imap: String, smtp: String) async throws {
        busy = "正在验证登录…"
        defer { busy = nil }
        try await core.run("add_password", ["email": email, "password": password, "name": name, "imap": imap, "smtp": smtp])
        await afterAccountsChanged()
    }

    private func runLogin(_ message: String, _ op: @escaping () async throws -> Void) {
        busy = message
        loginURL = nil
        error = nil
        Task {
            do {
                try await op()
                showAddAccount = false
                await afterAccountsChanged()
            } catch {
                self.error = error.localizedDescription
            }
            busy = nil
            loginURL = nil
        }
    }

    private func afterAccountsChanged() async {
        await refreshAccounts()
        await reload()
        startAutoSync()
        syncNow()
    }

    /// Sync the account list via a self-hosted server, WebDAV or a synced folder.
    func setupSync(_ params: [String: Any]) async throws -> Int {
        struct Joined: Decodable { var added: [String] }
        let r: Joined = try await core.call("sync_setup", params)
        await afterAccountsChanged()
        if accounts.isEmpty {
            // Nothing to restore yet: go straight to adding the first mailbox.
            Task {
                try? await Task.sleep(nanoseconds: 400_000_000)
                showAddAccount = true
            }
        }
        return r.added.count
    }

    func syncOff() {
        Task {
            do { try await core.run("sync_off") } catch { self.error = error.localizedDescription }
            await refreshAccounts()
        }
    }

    func removeAccount(_ email: String) {
        Task {
            do {
                try await core.run("remove_account", ["email": email])
            } catch { self.error = error.localizedDescription }
            await refreshAccounts()
            await reload()
        }
    }

    func loadConfig() async -> Config? { try? await core.call("config") }

    func saveConfig(_ c: Config) async {
        do {
            try await core.run("set_config", core.params(c))
            info = try await core.call("info")
        } catch { self.error = error.localizedDescription }
    }
}

/// State of one compose window.
@MainActor
final class ComposeModel: ObservableObject {
    @Published var from: String
    @Published var to: String
    @Published var cc: String
    @Published var bcc: String
    @Published var subject: String
    @Published var body: String
    @Published var attachments: [URL]
    @Published var showCc: Bool
    @Published var sending = false
    @Published var error: String?
    let inReplyTo: String?
    let references: String?

    init(from: String, draft: Draft) {
        self.from = from
        to = draft.to.joined(separator: ", ")
        cc = draft.cc.joined(separator: ", ")
        bcc = ""
        subject = draft.subject
        body = draft.body
        attachments = draft.attachments.map { URL(fileURLWithPath: $0) }
        showCc = !draft.cc.isEmpty
        inReplyTo = draft.inReplyTo
        references = draft.references
    }

    private func split(_ s: String) -> [String] {
        s.split(whereSeparator: { $0 == "," || $0 == ";" }).map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    }

    var draft: Draft {
        Draft(to: split(to), cc: split(cc), bcc: split(bcc), subject: subject, body: body, html: nil,
              inReplyTo: inReplyTo, references: references, attachments: attachments.map(\.path))
    }

    var canSend: Bool { !sending && !from.isEmpty && !split(to + "," + cc + "," + bcc).isEmpty }
}
