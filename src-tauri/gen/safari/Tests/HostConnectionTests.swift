import XCTest

/// The real `HostConnection`, `Frame` and handler encoding against a fake app
/// on a temporary Unix socket (`FakeApp`).
final class HostConnectionTests: XCTestCase {
    private var app: FakeApp?

    override func tearDown() {
        app?.stop()
        app = nil
        super.tearDown()
    }

    private func start(_ handle: @escaping (FakeApp.Peer, [String: Any]) -> Void) throws -> HostConnection {
        let app = try FakeApp(handle: handle)
        self.app = app
        let path = app.path
        return HostConnection(socketPath: { path }, push: nil)
    }

    private func request(_ host: HostConnection, _ action: String) -> HostConnection.Outcome {
        let body = try! JSONSerialization.data(withJSONObject: ["action": action])
        return host.request(body, action: action)
    }

    private func actions(_ events: [HostConnection.Event]) -> [String] {
        events.map { event in
            switch event {
            case .signal(let signal):
                let object = try! JSONSerialization.jsonObject(with: Data(signal.message.utf8)) as! [String: Any]
                return "signal:\(object["action"] as! String)#\(signal.seq)"
            case .reply(let body):
                let object = try! JSONSerialization.jsonObject(with: body) as! [String: Any]
                return "reply:\(object["action"] as! String)"
            }
        }
    }

    // Echo every request as a successful reply.
    private let echo: (FakeApp.Peer, [String: Any]) -> Void = { peer, request in peer.reply(to: request) }

    func testNoAppIsNotRunning() {
        let host = HostConnection(socketPath: { "/tmp/rowel-safari-nobody-\(UUID().uuidString.prefix(8)).sock" }, push: nil)
        let outcome = request(host, "change-public-keys")
        XCTAssertEqual(outcome.failure?.code, "not-running")
        XCTAssertEqual(outcome.events, [])
        XCTAssertFalse(outcome.connected)
    }

    func testOnlyAKeyExchangeMakesAConnection() throws {
        let host = try start(echo)
        let refused = request(host, "get-logins")
        XCTAssertEqual(refused.failure?.code, "disconnected")
        XCTAssertEqual(app?.connections, 0, "no connection made for a request that has no keys")

        let exchanged = request(host, "change-public-keys")
        XCTAssertNil(exchanged.failure)
        XCTAssertEqual(actions(exchanged.events), ["reply:change-public-keys"])
        XCTAssertTrue(exchanged.connected)
        XCTAssertEqual(actions(request(host, "get-logins").events), ["reply:get-logins"])
        XCTAssertEqual(app?.connections, 1)
    }

    func testSignalsComeWhereTheAppSentThemAroundTheReply() throws {
        let host = try start { peer, request in
            if request["action"] as? String == "get-logins" {
                peer.signal("database-locked")
                peer.reply(to: request)
                peer.signal("database-unlocked")
            } else {
                peer.reply(to: request)
            }
        }
        _ = request(host, "change-public-keys")
        let outcome = request(host, "get-logins")
        XCTAssertEqual(outcome.after, 1)
        Thread.sleep(forTimeInterval: 0.2)
        let rest = host.poll()
        XCTAssertEqual(
            actions(outcome.events + rest.events),
            ["signal:database-locked#1", "reply:get-logins", "signal:database-unlocked#2"]
        )
        for event in outcome.events {
            if case .signal(let signal) = event, let after = outcome.after {
                XCTAssertLessThanOrEqual(signal.seq, after, "only signals sent before the reply precede it")
            }
        }
    }

    func testAPollWaitsForTheRequestOnTheWire() throws {
        let host = try start { peer, request in
            if request["action"] as? String == "associate" {
                // The user takes a while; the app locks meanwhile.
                peer.signal("database-locked")
                Thread.sleep(forTimeInterval: 0.5)
                peer.reply(to: request)
            } else {
                peer.reply(to: request)
            }
        }
        _ = request(host, "change-public-keys")

        let order = NSLock()
        var finished: [String] = []
        var associated: HostConnection.Outcome?
        var polled: HostConnection.Outcome?
        let done = DispatchGroup()
        DispatchQueue.global().async(group: done) {
            let outcome = self.request(host, "associate")
            order.lock(); finished.append("request"); associated = outcome; order.unlock()
        }
        Thread.sleep(forTimeInterval: 0.2)
        DispatchQueue.global().async(group: done) {
            let outcome = host.poll()
            order.lock(); finished.append("poll"); polled = outcome; order.unlock()
        }
        XCTAssertEqual(done.wait(timeout: .now() + 5), .success)

        XCTAssertEqual(finished, ["request", "poll"])
        XCTAssertEqual(actions(associated!.events), ["signal:database-locked#1", "reply:associate"])
        XCTAssertEqual(polled!.events, [], "the poll could not take the signal from before the reply")
    }

    func testNoSignalIsDropped() throws {
        let many = 200
        let host = try start { peer, request in
            if request["action"] as? String == "get-logins" {
                for index in 0..<many {
                    peer.signal(index % 2 == 0 ? "database-locked" : "database-unlocked")
                }
            }
            peer.reply(to: request)
        }
        _ = request(host, "change-public-keys")
        let outcome = request(host, "get-logins")
        let seqs = outcome.events.compactMap { event -> Int? in
            if case .signal(let signal) = event { return signal.seq }
            return nil
        }
        XCTAssertEqual(seqs, Array(1...many))
        XCTAssertEqual(actions(outcome.events).last, "reply:get-logins")
    }

    func testAQueueNobodyFetchesDropsTheConnectionNotASignal() throws {
        let host = try start { peer, request in
            if request["action"] as? String == "flood" {
                for _ in 0...HostConnection.signalLimit {
                    peer.signal("database-locked")
                }
            }
            peer.reply(to: request)
        }
        XCTAssertNil(request(host, "change-public-keys").failure)
        let flooded = request(host, "flood")
        XCTAssertEqual(flooded.failure?.code, "disconnected", "over the limit, the connection goes")
        XCTAssertEqual(flooded.events, [], "and nothing is handed on with a gap in it")
        XCTAssertFalse(flooded.connected)
        XCTAssertEqual(request(host, "get-logins").failure?.code, "disconnected")
    }

    func testAFrameOverTheCapEndsTheConnectionAndAKeyExchangeMakesANewOne() throws {
        let host = try start { peer, request in
            if request["action"] as? String == "oversize" {
                var length = UInt32(Frame.maxLength + 1)
                peer.sendRaw(Data(bytes: &length, count: 4))
            } else {
                peer.reply(to: request)
            }
        }
        _ = request(host, "change-public-keys")
        XCTAssertEqual(request(host, "oversize").failure?.code, "disconnected")
        XCTAssertEqual(request(host, "get-logins").failure?.code, "disconnected")
        XCTAssertNil(request(host, "change-public-keys").failure)
        XCTAssertEqual(app?.connections, 2)
    }

    func testTheAppQuitting() throws {
        let host = try start { peer, request in
            if request["action"] as? String == "quit" {
                peer.close()
            } else {
                peer.reply(to: request)
            }
        }
        _ = request(host, "change-public-keys")
        XCTAssertEqual(request(host, "quit").failure?.code, "disconnected")
        app?.stop()
        let polled = host.poll()
        XCTAssertFalse(polled.connected)
        XCTAssertEqual(request(host, "change-public-keys").failure?.code, "not-running")
    }

    func testAReplyJustBeforeTheAppQuitsStillCounts() throws {
        let host = try start { peer, request in
            peer.reply(to: request)
            if request["action"] as? String == "lock-database" {
                peer.close()
            }
        }
        _ = request(host, "change-public-keys")
        let outcome = request(host, "lock-database")
        XCTAssertNil(outcome.failure)
        XCTAssertEqual(actions(outcome.events), ["reply:lock-database"])
    }

    func testTheHandlerRefusesABadRequestAndEncodesInOrder() throws {
        let host = try start { peer, request in
            peer.signal("database-locked")
            peer.reply(to: request, ["count": NSNull()])
        }
        let bad = SafariWebExtensionHandler.answer(["request": "not json"], host: host)
        XCTAssertEqual(bad["error"] as? String, "bad-request")
        XCTAssertEqual(SafariWebExtensionHandler.answer("nonsense", host: host)["error"] as? String, "bad-request")
        XCTAssertEqual(app?.connections, 0)

        let answered = SafariWebExtensionHandler.answer(
            ["request": #"{"action":"change-public-keys"}"#], host: host
        )
        XCTAssertNil(answered["error"])
        XCTAssertEqual(answered["after"] as? Int, 1)
        XCTAssertEqual(answered["connected"] as? Bool, true)
        let events = answered["events"] as! [[String: Any]]
        XCTAssertEqual(events.count, 2)
        XCTAssertEqual(events[0]["seq"] as? Int, 1)
        XCTAssertEqual(events[0]["signal"] as? String, #"{"action":"database-locked"}"#)
        let reply = try JSONSerialization.jsonObject(with: Data((events[1]["reply"] as! String).utf8)) as! [String: Any]
        XCTAssertEqual(reply["action"] as? String, "change-public-keys")
        XCTAssertTrue(reply["count"] is NSNull, "a JSON null survives, as text")
    }
}
