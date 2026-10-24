import Foundation
import Observation

/// A terminal size in cells.
public struct TerminalSize: Equatable, Hashable, Sendable, CustomStringConvertible {
    public var rows: Int
    public var cols: Int

    public init(rows: Int, cols: Int) {
        self.rows = rows
        self.cols = cols
    }

    public var description: String { "\(cols)×\(rows)" }
}

/// The emulator a `TerminalSession` drives, such as a Ghostty view.
@MainActor
public protocol TerminalSink: AnyObject {
    /// Clear everything and switch to this size; a snapshot's bytes follow.
    func terminalReset(size: TerminalSize)
    func terminalFeed(_ bytes: [UInt8])
    /// The agent's PTY changed size.
    func terminalResize(size: TerminalSize)
}

/// What a `TerminalSession` sends back to the daemon. `CosClient` is the real one.
public protocol TerminalControl: Sendable {
    func sendKeys(agentId: Int, _ bytes: [UInt8]) async throws
    func resizeAgent(id: Int, rows: Int, cols: Int) async throws
}

extension CosClient: TerminalControl {}

/// One agent's live terminal for a view, without any UI code: follows the
/// terminal stream into a `TerminalSink`, sends keystrokes in order
/// (batching the ones typed while a send is in flight), and resizes the
/// agent to fit the view, debounced.
@MainActor
@Observable
public final class TerminalSession {
    public enum Phase: Equatable, Sendable {
        /// Opening the stream; nothing shown yet.
        case connecting
        case live
        /// The stream dropped while the agent was alive; retrying.
        case reconnecting(String)
        /// The process ended. `nil`: it was killed (or interrupted).
        case exited(code: Int?)
        /// The daemon doesn't know this agent.
        case gone(String)
    }

    public let agentId: Int
    public private(set) var phase: Phase = .connecting
    /// The PTY's size as the daemon last said, or as we last set it.
    public private(set) var ptySize: TerminalSize?
    /// The size that fits the view, as the view last reported.
    public private(set) var viewSize: TerminalSize?
    /// Why the last keystroke or resize didn't go through, if it didn't.
    public private(set) var lastError: String?
    /// How many snapshots arrived: 1 after the first connect, more after reconnects.
    public private(set) var snapshots = 0

    @ObservationIgnored public weak var sink: TerminalSink?
    @ObservationIgnored private let source: TerminalSource
    @ObservationIgnored private let control: TerminalControl
    @ObservationIgnored private let backoff: Backoff
    @ObservationIgnored private let resizeDelay: Duration
    @ObservationIgnored private var streamTask: Task<Void, Never>?
    @ObservationIgnored private var resizeTask: Task<Void, Never>?
    @ObservationIgnored private var pendingKeys: [UInt8] = []
    @ObservationIgnored private var sendingKeys = false

    /// Largest keystroke batch per request; a big paste goes in several, in order.
    static let maxKeyBatch = 32 * 1024

    public convenience init(
        agentId: Int,
        client: CosClient,
        backoff: Backoff = Backoff(),
        resizeDelay: Duration = .milliseconds(150)
    ) {
        self.init(agentId: agentId, source: client, control: client, backoff: backoff, resizeDelay: resizeDelay)
    }

    public init(
        agentId: Int,
        source: TerminalSource,
        control: TerminalControl,
        backoff: Backoff = Backoff(),
        resizeDelay: Duration = .milliseconds(150)
    ) {
        self.agentId = agentId
        self.source = source
        self.control = control
        self.backoff = backoff
        self.resizeDelay = resizeDelay
    }

    public var isRunning: Bool { streamTask != nil }

    /// Opens the stream. Does nothing if it is already open.
    public func start() {
        guard streamTask == nil else { return }
        phase = .connecting
        let stream = TerminalStream(source: source, agentId: agentId, backoff: backoff)
        streamTask = Task { [weak self] in
            for await update in stream.updates() {
                guard let self else { return }
                self.apply(update)
            }
            // A restart cancels this task and has already replaced it.
            if !Task.isCancelled { self?.streamTask = nil }
        }
    }

    public func stop() {
        streamTask?.cancel()
        streamTask = nil
        resizeTask?.cancel()
        resizeTask = nil
    }

    /// A fresh stream, for example after the agent was resumed.
    public func restart() {
        stop()
        start()
    }

    func apply(_ update: TerminalUpdate) {
        switch update {
        case .event(.snapshot(let rows, let cols, let data)):
            let size = TerminalSize(rows: rows, cols: cols)
            ptySize = size
            snapshots += 1
            phase = .live
            sink?.terminalReset(size: size)
            sink?.terminalFeed(data)
            // We just (re)connected: make the agent fit this view.
            scheduleResize()
        case .event(.output(let data)):
            sink?.terminalFeed(data)
        case .event(.resize(let rows, let cols)):
            let size = TerminalSize(rows: rows, cols: cols)
            guard size != ptySize else { return }
            ptySize = size
            sink?.terminalResize(size: size)
        case .event(.exit(let code)):
            phase = .exited(code: code)
            resizeTask?.cancel()
        case .disconnected(let error, _):
            phase = .reconnecting(error.localizedDescription)
        case .gone(let error):
            phase = .gone(error.localizedDescription)
        }
    }

    // MARK: Input

    /// Keystrokes from the view, as bytes for the PTY. They go out in order;
    /// those typed while a request is in flight go together in the next one.
    public func send(_ bytes: [UInt8]) {
        guard !bytes.isEmpty, phase == .live || isReconnecting else { return }
        pendingKeys += bytes
        // Typing here means this view is the one in use: let it own the size.
        if viewSize != nil, viewSize != ptySize { scheduleResize() }
        guard !sendingKeys else { return }
        sendingKeys = true
        Task {
            while !pendingKeys.isEmpty {
                let count = min(pendingKeys.count, Self.maxKeyBatch)
                let batch = Array(pendingKeys.prefix(count))
                pendingKeys.removeFirst(count)
                do {
                    try await control.sendKeys(agentId: agentId, batch)
                    lastError = nil
                } catch {
                    lastError = error.localizedDescription
                    // It exited or went away; what's queued can't arrive either.
                    if case CosError.conflict = error { pendingKeys = [] }
                    if case CosError.notFound = error { pendingKeys = [] }
                }
            }
            sendingKeys = false
        }
    }

    private var isReconnecting: Bool {
        if case .reconnecting = phase { true } else { false }
    }

    /// The view's size in cells changed (window resize, new font). The agent
    /// is resized to match after `resizeDelay`, so a live resize sends one
    /// request at the end rather than one per frame.
    ///
    /// Reporting the same size again still re-claims the PTY if someone else
    /// resized it meanwhile: the view laid out at its own size.
    public func viewSizeChanged(_ size: TerminalSize) {
        guard size.rows > 0, size.cols > 0 else { return }
        viewSize = size
        scheduleResize()
    }

    private func scheduleResize() {
        resizeTask?.cancel()
        guard phase == .live, let target = viewSize, target != ptySize else { return }
        resizeTask = Task { [weak self, resizeDelay] in
            try? await Task.sleep(for: resizeDelay)
            guard let self, !Task.isCancelled else { return }
            await self.sendResize(target)
        }
    }

    private func sendResize(_ size: TerminalSize) async {
        guard phase == .live, size == viewSize, size != ptySize else { return }
        do {
            try await control.resizeAgent(id: agentId, rows: size.rows, cols: size.cols)
            // The daemon echoes a resize event; match it now so the emulator
            // and the PTY agree without waiting.
            if ptySize != size {
                ptySize = size
                sink?.terminalResize(size: size)
            }
            lastError = nil
        } catch {
            lastError = error.localizedDescription
        }
    }
}
