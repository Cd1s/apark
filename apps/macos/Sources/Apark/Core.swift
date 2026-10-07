import CApark
import Foundation

/// Bridge to the Rust core: one JSON call, executed off the main thread.
final class Core: @unchecked Sendable {
    static let shared = Core()

    private let queue = DispatchQueue(label: "apark.core", qos: .userInitiated, attributes: .concurrent)
    let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()
    let encoder: JSONEncoder = {
        let e = JSONEncoder()
        e.keyEncodingStrategy = .convertToSnakeCase
        return e
    }()

    struct Failure: LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    private struct Envelope<R: Decodable>: Decodable {
        let ok: Bool
        let result: R?
        let error: String?
    }

    private struct Ignored: Decodable {
        init(from decoder: Decoder) throws {}
    }

    private func raw(_ method: String, _ params: [String: Any]) async throws -> Data {
        let request = try JSONSerialization.data(withJSONObject: ["method": method, "params": params])
        let text = String(decoding: request, as: UTF8.self)
        return await withCheckedContinuation { cont in
            queue.async {
                let ptr = text.withCString { apark_call($0) }
                let reply = ptr.map { String(cString: $0) } ?? #"{"ok":false,"error":"no reply"}"#
                apark_free(ptr)
                cont.resume(returning: Data(reply.utf8))
            }
        }
    }

    /// Call a method that returns a value.
    func call<R: Decodable>(_ method: String, _ params: [String: Any] = [:], as _: R.Type = R.self) async throws -> R {
        let env = try decoder.decode(Envelope<R>.self, from: try await raw(method, params))
        guard env.ok else { throw Failure(message: env.error ?? "未知错误") }
        guard let result = env.result else { throw Failure(message: "空结果") }
        return result
    }

    /// Call a method whose result may be null.
    func callOptional<R: Decodable>(_ method: String, _ params: [String: Any] = [:], as _: R.Type = R.self) async throws -> R? {
        let env = try decoder.decode(Envelope<R>.self, from: try await raw(method, params))
        guard env.ok else { throw Failure(message: env.error ?? "未知错误") }
        return env.result
    }

    /// Call a method for its side effect.
    func run(_ method: String, _ params: [String: Any] = [:]) async throws {
        let env = try decoder.decode(Envelope<Ignored>.self, from: try await raw(method, params))
        guard env.ok else { throw Failure(message: env.error ?? "未知错误") }
    }

    /// Encodable value → JSON params dictionary.
    func params<T: Encodable>(_ value: T) -> [String: Any] {
        guard let data = try? encoder.encode(value),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
        return obj
    }

    /// Events from the core arrive on a background thread; hop to main and broadcast.
    func listen() {
        apark_set_event_callback { ptr in
            guard let ptr else { return }
            let data = Data(String(cString: ptr).utf8)
            DispatchQueue.main.async {
                NotificationCenter.default.post(name: .aparkEvent, object: data)
            }
        }
    }
}

extension Notification.Name {
    static let aparkEvent = Notification.Name("AparkEvent")
}
