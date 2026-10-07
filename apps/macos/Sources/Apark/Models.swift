import SwiftUI

struct Info: Codable {
    var version: String
    var dataDir: String
    var hasMaster: Bool
    var googleReady: Bool
    var microsoftReady: Bool
}

struct Account: Codable, Identifiable, Hashable {
    var email: String
    var name: String
    var provider: String
    var master: Bool
    var imap: String
    var smtp: String
    var id: String { email }
}

struct Folder: Codable, Hashable {
    var name: String
    var role: String

    var title: String {
        if name.uppercased() == "INBOX" { return "收件箱" }
        return name.split(whereSeparator: { $0 == "/" || $0 == "." }).last.map(String.init) ?? name
    }

    var symbol: String {
        switch role {
        case "inbox": return "tray"
        case "sent": return "paperplane"
        case "drafts": return "doc"
        case "trash": return "trash"
        case "junk": return "xmark.bin"
        case "archive", "all": return "archivebox"
        case "flagged": return "star"
        default: return "folder"
        }
    }
}

struct Message: Codable, Identifiable, Hashable {
    var id: Int64
    var account: String
    var folder: String
    var uid: UInt32
    var messageId: String
    var references: String
    var subject: String
    var fromName: String
    var fromAddr: String
    var to: String
    var cc: String
    var date: Int64
    var size: UInt32
    var seen: Bool
    var flagged: Bool
    var category: String
    var snippet: String
    var hasBody: Bool

    var sender: String { fromName.isEmpty ? fromAddr : fromName }
    var displaySubject: String { subject.isEmpty ? "（无主题）" : subject }
    var when: Date { Date(timeIntervalSince1970: TimeInterval(date)) }
}

struct Attachment: Codable, Hashable {
    var name: String
    var size: Int
}

struct MailBody: Codable {
    var text: String
    var html: String?
    var attachments: [Attachment]
}

struct Draft: Codable {
    var to: [String] = []
    var cc: [String] = []
    var bcc: [String] = []
    var subject = ""
    var body = ""
    var html: String?
    var inReplyTo: String?
    var references: String?
    var attachments: [String] = []
}

struct DraftReply: Codable {
    var from: String
    var draft: Draft
}

struct Config: Codable {
    var googleClientId: String?
    var googleClientSecret: String?
    var microsoftClientId: String?
    var syncPassphrase: String?
    var syncIntervalSecs: Int
    var initialLimit: Int
    var prefetchKb: Int
}

struct SyncResult: Codable {
    var account: String
    var ok: Bool
    var new: Int?
    var error: String?
}

struct CoreEvent: Codable {
    var type: String
    var url: String?
    var results: [SyncResult]?
}

enum Nav: Hashable {
    case inbox, people, notifications, newsletters, unread, flagged
    case folder(account: String, name: String)
}

// MARK: - Formatting

enum Fmt {
    private static let time: DateFormatter = { let f = DateFormatter(); f.dateFormat = "HH:mm"; return f }()
    private static let monthDay: DateFormatter = { let f = DateFormatter(); f.dateFormat = "M月d日"; return f }()
    private static let ymd: DateFormatter = { let f = DateFormatter(); f.dateFormat = "yyyy/M/d"; return f }()
    private static let full: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "yyyy年M月d日 HH:mm"
        return f
    }()

    static func short(_ d: Date) -> String {
        let cal = Calendar.current
        if cal.isDateInToday(d) { return time.string(from: d) }
        if cal.isDateInYesterday(d) { return "昨天" }
        if cal.isDate(d, equalTo: Date(), toGranularity: .year) { return monthDay.string(from: d) }
        return ymd.string(from: d)
    }

    static func long(_ d: Date) -> String { full.string(from: d) }

    static func size(_ n: Int) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(n), countStyle: .file)
    }
}

/// Stable per-address colour (FNV-1a hue), matching the other Apark front-ends.
func addressColor(_ key: String) -> Color {
    var h: UInt32 = 0x811C_9DC5
    for b in key.utf8 { h = (h ^ UInt32(b)) &* 0x0100_0193 }
    return Color(hue: Double(h % 3600) / 3600, saturation: 0.55, brightness: 0.85)
}
