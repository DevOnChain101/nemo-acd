# R6 browser backend spike

Decision: **(c) own minimal CDP client** over `tokio-tungstenite`, driving an installed Chrome/Chromium/Edge.
Closes the open part of ADR-003 (backend under the Rust core); ADR-010 sidecar exception is not used.

## Evidence (checked 2026-10-27)

| Option | Maintenance | Fit |
|---|---|---|
| (a) chromiumoxide 0.9.1 | Last release 2026-02-25 (8 months stale); ~2.1M recent downloads. Large generated protocol surface. | Async, has a11y/console/network via generated CDP types and a fetcher that downloads Chrome (we must never download). Own process handling: no process-group/Job Object kill, so it fails our tree-kill rule; Windows quality unverified. |
| (b) headless_chrome 1.0.22 | Released 2026-06-11, ~0.75M recent downloads. | Synchronous, thread-per-connection; would need `spawn_blocking` around every call inside a tokio core. Same process-tree gap. |
| (c) own CDP client | `tokio-tungstenite` 0.30.0 (2026-07-11, ~99M recent downloads). | ~150 lines of framing/dispatch; we use about 15 CDP methods and 10 events. Fully ours: URL policy enforced per request through `Fetch.requestPaused`, process tree owned by `process-wrap` exactly like `synthesis-terminal`. |
| (d) Playwright sidecar | Mature. | Needs Node at runtime plus a downloaded browser bundle (Playwright pins its own Chromium), an RPC layer to write and secure, and a second process tree. Rejected: violates "never download a browser" and adds the most moving parts. |

Criteria for (c), all verified by the integration tests on macOS with Chrome 156:
- Accessibility tree: `Accessibility.getFullAXTree`, rendered to compact text with stable refs.
- Console, uncaught exceptions, network (status, timing, failures): `Runtime.consoleAPICalled`, `Runtime.exceptionThrown`, `Log.entryAdded`, `Network.*`. Bodies are never fetched.
- Screenshots: `Page.captureScreenshot` (viewport, or full page with a height cap).
- Isolation: fresh temp `--user-data-dir` plus `Target.createBrowserContext` per session.
- Headless launch of an installed browser. Transport is `BrowserOptions::transport`: `Pipe` (default on unix) uses `--remote-debugging-pipe`, so no TCP port exists; `Tcp` (default on Windows) uses `--remote-debugging-port=0` and reads the WebSocket URL from `DevToolsActivePort`.
- Pipe without `unsafe` (the workspace forbids it): Chrome needs the pipes on fd 3/4, which `pre_exec` or `dup2` on raw fds would require. Instead the child is `/bin/sh -c 'exec chrome ... 3<&0 4>&1 </dev/null >/dev/null'`: the shell moves our piped stdin/stdout to fd 3/4 and execs Chrome, so the pid and process group are Chrome's. Messages are NUL-terminated JSON. Windows needs `--remote-debugging-io-pipes=<handles>`, i.e. raw inheritable handles, so it stays on Tcp.
- Windows/macOS/Linux: only `process-wrap` (already used by `synthesis-terminal`), `tokio-tungstenite` and `url` are involved; discovery covers all three OS layouts.

## Risks and limits

- `Transport::Tcp` (Windows default): the debugging port listens on 127.0.0.1 with a random port. The browser WebSocket path id is unguessable, but Chrome's HTTP discovery endpoints (`/json/version`) hand it to any local process, so any local process can drive the browser. Residual risk until a Windows pipe transport exists.
- URL policy covers every target: browser-level and per-session `Target.setAutoAttach {waitForDebuggerOnStart, flatten}`; each popup, cross-site iframe and worker gets `Fetch.enable` plus `Network.setBlockedURLs` before `Runtime.runIfWaitingForDebugger`, and is closed (never resumed) if that fails. Verified on Chrome 156: neither `Fetch` nor `Network.setBlockedURLs` intercepts WebSocket handshakes (the handshake reached the server with `ws://*` blocked), so `WebSocket`/`WebTransport` are replaced by throwing stubs in every document and worker (script-level, best effort; `ws:`/`wss:` are also rejected by `check_request`). Chrome may open speculative preconnect sockets to a denied host; they carry no request.
- Policy hosts are normalised (lowercase, IDNA, trailing dots, IPv4 decimal/hex/octal, IPv4-mapped/compatible/NAT64 IPv6; a zone id makes the URL invalid and is refused) for URLs and for `allow_domains`/`deny_domains` entries.
- Screenshots: directory created 0700 only when missing (an existing directory keeps its mode), files 0600; Windows relies on default ACLs.
- Sandbox flags stay on. `BrowserOptions::no_sandbox` exists only for containers/root.
- DNS / SSRF, best effort, NOT a security boundary: the browser is not OS-sandboxed. (1) Every `Fetch.requestPaused` request is resolved with our own resolver (`UrlPolicy::check_resolved`: `ToSocketAddrs` on a blocking thread, 5 s timeout, 30 s cache, fails closed) and denied if ANY address is private/loopback/link-local/metadata/ULA/CGNAT/multicast/IPv4-mapped (same ranges as WebFetch). So `169.254.169.254.nip.io` is blocked before it is sent. (2) Chrome resolves again itself (own cache, DoH, hosts), so rebinding between the two lookups is possible. `Network.responseReceived.remoteIPAddress` is therefore checked: a private address stops the page (`Page.stopLoading`, documents are replaced by `about:blank`) and the request is recorded as failed with no status. The request itself HAS ALREADY BEEN SENT (a blind GET with side effects can land) and the headers were received; only the body is never exposed to snapshot/console/network output. Not covered: redirects Chrome follows internally are re-checked per hop only through Fetch, `requestWillBeSentExtraInfo`/`loadingFinished` are not used, preconnect sockets. Tests: `hostname_resolving_to_private_ip_is_blocked_before_sending`, `rebinding_to_private_ip_is_stopped_on_response` (real Chrome with `--host-resolver-rules`), `dns_names_resolving_to_private_ips_are_blocked` (unit). Callers who need a real boundary must run the browser in an OS sandbox/network namespace with egress filtering.
- Downloads are denied entirely.
- CDP method names can drift between Chrome versions; the integration tests are the guard.
- Not run: Windows, Linux, Edge, real-world sites.
