# 03 — Migration Plan & Specification

A phased plan to port Headscale's **client-facing control plane** from Go to
Rust. Scope, deployment scenario, and crate layout are defined in
[02-target-architecture.md](02-target-architecture.md) §0; this document
specifies **what to build, in what order**, with a **verification gate** per
phase — a concrete, testable proof that the phase works before the next begins.

Each gate is the go/no-go bar. Its richer, multi-actor automated form — spawn
a server plus several clients and assert connectivity, discoverability, data
flow, and propagation — is defined as a progressive **e2e test** in
[06-e2e-test.md](06-e2e-test.md), referenced per phase below.

The plan is deliberately **de-risking-first**: the riskiest, least
reversible component (the Noise handshake) is proven in phase 1, before any
investment in state, persistence, or fan-out.

---

## 0. Guiding strategy

- **Reference the Go implementation continuously.** Headscale stays the
  oracle. Run a real `tailscaled` against both the Go server and the Rust
  server and diff the wire traffic. The Go code is the spec.
- **Build the protocol types bottom-up, the server top-down.** `tskey` and
  `tailcfg` first (everything depends on them), then transport, then the
  control logic.
- **Use a real client as the test harness from day one.** A `tailscaled`
  in a container that tries to connect is worth more than any unit test for
  protocol fidelity.
- **Persistence is a trait, not a database.** The core depends on a `Store`
  trait (see [02 §3.5](02-target-architecture.md)); the host injects the
  backend. Phase 1–5 ship `MemoryStore` and `FileStore` (JSON/YAML/TOML
  serde snapshot) only — **no SQL**. A `SqlStore` (`sqlx`) is an additive
  trait impl deferred to later. The append-only migration discipline from
  `AGENTS.md` (`YYYYMMDDHHMM-slug` IDs, never reorder, never disable FKs)
  applies only when/if that SQL backend lands; the file/memory backends
  version state through the snapshot envelope instead.

---

## Phase 1 — Protocol foundations + Noise handshake (PROVE IT FIRST)

**Goal:** a Rust client can complete the `/key` + `/ts2021` exchange with a
Rust server and exchange a single encrypted byte. No state, no DB.

**Build (reuse-first — see step 0):**
0. **Build-vs-reuse spike.** Evaluate depending on `tailscale-rs`
   (BSD-3) crates: `ts_keys` (key types/encoding), `ts_control_serde`
   (`tailcfg` wire types), `ts_noise`/`ts_control_noise` (Noise/TS2021).
   They are internal workspace crates, so reuse means a **git dependency on
   a pinned revision** or vendoring into our workspace. Decide per crate;
   the rest of this phase assumes reuse where viable and hand-port where
   not. `tailscale-rs` is *also* wired up as a **Rust test client** here.
1. `tskey`: reuse `ts_keys` if viable; else port — `MachinePrivate/Public`,
   `NodePublic`, `DiscoPublic`, `ChallengePrivate/Public` (Curve25519 via
   `x25519-dalek`) with the `mkey:`/`nodekey:`/`discokey:` text codecs.
   Round-trip against Go fixtures (`key.MachinePublic.String()`) regardless
   of source.
2. `tailcfg` (minimal): reuse `ts_control_serde` if viable; else hand-port
   `OverTLSPublicKeyResponse`, `EarlyNoise`, `CapabilityVersion`. Serde
   shapes verified against Go JSON output.
3. `noise` (**responder side**): mirror `ts_noise`'s initiator logic onto
   the server role — the `/key` handler; the `/ts2021` HTTP upgrade +
   handshake; the EarlyNoise frame (`\xff\xff\xffTS` + BE length + JSON);
   expose the encrypted stream as `AsyncRead+AsyncWrite`. `ts_noise` is the
   authoritative reference for Noise params and framing; `snow` underneath
   if hand-built.
4. `capver` table (port `capver_generated.go`; min supported = 113).

**Verification gate (hard requirement):**
- A **real `tailscaled`** (≥ v1.80) pointed at the Rust server with
  `--login-server` completes the Noise handshake and issues
  `POST /machine/register` (which can 5xx for now — we only need the
  handshake + HTTP/2 framing to succeed). Capture with the Go server first,
  then prove byte-compatible behaviour against the Rust one. The
  `tailscale-rs` client is a second, scriptable judge.
- If this gate cannot be met **even with `tailscale-rs` as reference**,
  reconsider scope before proceeding (see
  [04-critical-points.md](04-critical-points.md) §1) — e.g. a hybrid that
  keeps the Go transport behind FFI.

**E2E:** [06 · E2E-1 Handshake smoke](06-e2e-test.md#e2e-1--handshake-smoke--phase-1--scaffold)
(scaffold — superseded by E2E-3).

**Estimate:** the long pole of the whole project. Budget generously.

---

## Phase 2 — Wire types + persistence + registration

**Goal:** a real client registers with a pre-auth key and the node is
persisted.

**Build:**
1. Full `tailcfg` port needed for register/map:
   `RegisterRequest/Response`, `Hostinfo`, `NetInfo`, `Node`, `MapRequest`,
   `MapResponse` (struct only; building comes in phase 3), `PeerChange`,
   `UserProfile`, `DNSConfig`, `DERPMap`. **Verify every struct's JSON
   against Go** (serialize the same value in both, diff bytes).
2. `store` crate: the **`Store` trait** (persistence port) plus its two
   bundled backends — `MemoryStore` (volatile, the default/test backend) and
   `FileStore` (loads at startup, serialises the whole dataset to one
   JSON/YAML/TOML file per the `StoreDocument` envelope, format by extension).
   Aggregates: `Network`, `User`, `Node`, `PreAuthKey`, policy — all carrying
   `network_id` (§seam). **No SQL backend** (deferred; see decision log).
   The snapshot envelope carries a `version` field as the migration hook.
3. `ipalloc`: per-network IPv4/IPv6 allocation, reserved-IP skipping,
   used-set loaded from the `Store` at startup.
4. Pre-auth keys: generation (`hskey-auth-<12hex>-<64hex>`, bcrypt the
   secret), lookup-by-prefix, **atomic single-use consumption** via
   `Store::consume_preauth_key` (memory/file: mutex-guarded flip;
   SQL later: `UPDATE … WHERE used=false`), validation
   (expiry/reusable/used).
5. `auth`: `handle_register` for the **authkey path only**; per-machine-key
   lock; create-or-update node; allocate IPs.

**Verification gate:**
- A real `tailscaled` registers via `tailscale up --authkey hskey-auth-…`
  against the Rust server; the node appears in the `Store` (assert via the
  trait, and inspect the `FileStore` snapshot file) with correct
  machine/node keys and allocated IPs; the client receives
  `RegisterResponse{MachineAuthorized:true}`.
- Cross-check the persisted node against what the Go server stores for the
  same registration. Run the gate against **both** `MemoryStore` and
  `FileStore` to prove the trait boundary holds (reload from the file and
  confirm the node survives a restart).

**E2E:** [06 · E2E-2 Registration & persistence](06-e2e-test.md#e2e-2--registration--persistence--phase-2).

---

## Phase 3 — Data plane: NodeStore + map building + streaming

**Goal:** a single registered client opens `/machine/map`, receives a valid
initial network map, and stays connected with keepalives.

**Build:**
1. `state` + `NodeStore` (`ArcSwap<Snapshot>` + writer task). Single-node
   case first.
2. `mapper` initial-map path: `MapResponseBuilder` →
   `MapResponse{Node, Peers:[], DERPMap, DNSConfig, PacketFilters, …}`.
   `NodeView.TailNode()` equivalent: build `tailcfg.Node` from a `Node`
   (addresses, allowed-IPs, endpoints, HomeDERP, keys, expiry).
3. `poll` streaming: the `serve_long_poll` task, `[len][zstd|json]`
   framing, keepalive interval, `Connect`/`Disconnect` lifecycle.
4. `UpdateNodeFromMapRequest`: absorb endpoints/hostinfo/routes into the
   NodeStore.
5. Minimal policy engine → `PacketFilters["base"]`. Start default-open for
   the single-node bring-up; the hub-and-spoke tag ACL (tags + grants) lands
   in phase 4 where visibility actually matters.
6. DERP map: load from config/URL and inject (point at public or
   standalone DERP; no embedded relay yet).

**Verification gate:**
- `tailscale status` on the client shows it connected to the tailnet, with
  an assigned IP and the correct DERP region. The long-poll stays open
  across keepalives. Compare the initial `MapResponse` byte-for-byte (modulo
  timestamps) against the Go server's for an equivalent node.

**E2E:** [06 · E2E-3 Connect, receive map, stay alive](06-e2e-test.md#e2e-3--connect-receive-map-stay-alive--phase-3)
(absorbs E2E-1).

---

## Phase 4 — Multi-node fan-out + peer visibility

**Goal:** the hub (central service) sees all sensors and a sensor sees only
the hub; a WireGuard tunnel establishes hub↔sensor; a change to one
propagates to those allowed to see it.

**Build:**
1. `mapper.Batcher` + per-node `NodeConn` registry + worker pool + tick
   coalescing.
2. Change fan-out: `change::Change`, `FilterForNode`, broadcast vs
   targeted, the three incremental forms (`PeersChangedPatch`,
   `PeersChanged`, `PeersRemoved`) and the `last_sent_peers` diff.
3. **Hub-and-spoke policy + `peers_fn` / `BuildPeerMap`:** the tag ACL
   (`tag:sensor ↔ tag:central`, no `tag:sensor ↔ tag:sensor`) drives the
   peer graph, so each sensor's peer set is `{central}` and central's is
   `{all sensors}`; per-peer `AllowedIPs` / `RoutesForPeer`. (The engine is
   general; hub-and-spoke is just the configured policy.)
4. Primary-route (HA) election in the snapshot rebuild — needed even for a
   single subnet-router sensor.

**Verification gate:**
- Hub-and-spoke visibility holds in a multi-node tailnet: each sensor sees
  **only** the central service; central sees all sensors; sensor↔central data
  flows while sensor↔sensor is blocked; an online/offline change propagates to
  the central within one batch tick and to no one else.

**E2E:** [06 · E2E-4 Three clients: connect + discover + data](06-e2e-test.md#e2e-4--three-clients-connect--discover--data--phase-4--headline)
and [E2E-5 Discoverability matrix](06-e2e-test.md#e2e-5--discoverability-matrix--phase-4-acl-focus)
own the full multi-actor scenario and the positive/negative visibility matrix.

---

## Phase 5 — Embedding API (+ optional multi-network)

**Goal:** the host application drives everything through Rust.

**Build:**
1. `control-api`: `ControlServer`, `Network` (a single default network in
   the hub-and-spoke case), `PreAuthKey`, `NodeHandle`, event stream
   (`subscribe()`). The `network_id` seam from phase 2 is exposed here.
2. **(Optional, only if several *independent* central servers are needed)**
   promote the seam to real multi-tenancy: per-network NodeStore partition,
   per-network IP pools/policy, per-network routing — subdomain, path
   prefix, or optional mTLS (see [02 §4](02-target-architecture.md) for
   strategies and client-compatibility notes). Defer until the requirement
   is real.

**Verification gate:**
- The full node lifecycle (create network → mint keys → register → list →
  delete) is driven entirely through the `control-api`, and the `subscribe()`
  event stream reflects it. If multi-network is built, two networks stay
  isolated with independently overlapping IP pools.

**E2E:** [06 · E2E-6 Embedding-API lifecycle + events](06-e2e-test.md#e2e-6--embedding-api-lifecycle--events--phase-5).

---

## Phase 6 — Hardening & production readiness

**Goal:** safe to run unattended.

**Build / verify:**
- Reconnection storms, overlapping sessions (the `ActiveSessions`/epoch
  logic), ephemeral-node GC, key rotation (NodeKey changes mid-session),
  expiry handling.
- Metrics (`tracing` + Prometheus exporter), structured logging.
- `FileStore` durability hardening: atomic write-temp-then-rename, debounced
  flush under update bursts, snapshot-version upconversion on load, crash/
  partial-write recovery.
- **(Optional, deferred) `SqlStore`:** if a SQL backend is wanted, add it as
  a new `Store` impl (`sqlx`, SQLite then Postgres) and run the existing
  trait conformance + registration gates against it unchanged — the rest of
  the core does not move. Append-only migration discipline applies here.
- Load test: N sensors per network, M networks; watch the O(n²) peer-map
  cost and batch latency.
- Soak test against a matrix of Tailscale client versions (the capver
  window — latest ~10 minor releases).

**E2E:** [06 · E2E-7 Resilience & soak](06-e2e-test.md#e2e-7--resilience--soak--phase-6).

---

## Dependencies — Go → Rust inventory

### Critical (protocol — must be ported faithfully)

Prefer reusing `tailscale-rs` (BSD-3) for the protocol crates; hand-port is
the fallback. "Reuse" = pinned git dependency or vendored crate.

| Go package | Provides | Rust approach |
|------------|----------|---------------|
| `tailscale.com/tailcfg` | All wire structs | **Reuse `tailscale-rs` `ts_control_serde`**; else hand-port serde + **byte-diff every one against Go** |
| `tailscale.com/types/key` | Curve25519 key types + text codecs | **Reuse `tailscale-rs` `ts_keys`**; else `x25519-dalek` + custom `mkey:`/`nodekey:`/`discokey:` codecs |
| `tailscale.com/control/controlbase` | Noise IK session | **Mirror `tailscale-rs` `ts_noise` to responder**; else `snow` (Noise_IK_25519_ChaChaPoly_BLAKE2s) |
| `tailscale.com/control/controlhttp/controlhttpserver` | Noise-over-HTTP upgrade + EarlyNoise | `axum`/`hyper` upgrade + `h2` over the noise conn (reference: `ts_control_noise`) |
| `tailscale.com/util/zstdframe` | zstd map framing | `zstd` crate (frame API) — match framing |
| `tailscale.com/net/tsaddr` | CGNAT/ULA ranges, exit-route test | constants + helpers, values copied from Go |
| `tailscale.com/types/dnstype` | `Resolver` | serde struct |
| `tailscale.com/util/rands` | secure hex strings | `rand` (`OsRng`) + `hex` |
| `tailscale.com/util/dnsname` | hostname/FQDN sanitisation | small port |
| `go4.org/netipx` | IP-set arithmetic | `ipnet` + custom range-set (no equivalent) |

### Optional (DERP — can stay Go initially)

| Go package | Provides |
|------------|----------|
| `tailscale.com/derp`, `derp/derpserver` | embedded relay |
| `tailscale.com/net/stun` | STUN responder |
| `tailscale.com/net/wsconn` | WS↔net.Conn bridge |

> **DERP strategy:** keep the existing Go DERP relay (or public Tailscale
> DERPs / a standalone `derper`) as a separate process. The control plane
> only needs to *distribute a DERP map*, not *be* a relay. Port the relay
> last, if ever.

### Non-protocol (ordinary equivalents — low risk)

GORM→**`Store` trait** (`MemoryStore` + `FileStore` via
`serde_json`/`serde_yaml`/`toml`; `sqlx` deferred behind the same trait),
zerolog→`tracing`, viper→`figment`, cobra→`clap`,
grpc→`tonic` (only if a management API returns later), bcrypt→`bcrypt`,
prometheus→`metrics`, xsync→`dashmap`, fsnotify→`notify`.

---

## Verification methodology (applies to every phase)

1. **Golden wire fixtures.** Capture real request/response bytes from the
   Go server talking to a real client (a small recording proxy, or
   `tcpdump` + the Noise keys). Replay against the Rust server; diff.
2. **Cross-serialization tests.** For every `tailcfg` type, marshal the
   same value in Go and Rust and assert byte-identical JSON. A tiny Go
   helper binary emits fixtures; a Rust test consumes them.
3. **Real-client integration.** A `tailscaled` container is the ultimate
   judge. The existing `integration/` Docker harness is a model to copy.
   `tailscale-rs` provides a second, **scriptable** Rust client for
   fast in-process protocol tests. The concrete multi-actor scenarios — and
   the rule that the suite carries no obsolete/duplicate tests — live in
   [06-e2e-test.md](06-e2e-test.md).
4. **Differential running.** Where feasible, run Go and Rust servers
   side-by-side against the same client and compare observable behaviour
   (`tailscale status`, ping, map contents).

---

## Effort shape (relative, not calendar)

```
Phase 1  ████████████████  Noise handshake — the long pole, high risk
Phase 2  ████████          types + DB + registration
Phase 3  ██████████        data plane streaming
Phase 4  ████████████      fan-out + visibility — intricate, bug-prone
Phase 5  ██████            tenancy + API
Phase 6  ████████          hardening
```

Phases 1 and 4 carry the most risk. Phase 1 is gating: do not staff phases
2–6 until the handshake gate is green.

---

## Decision log (resolve before/at each phase)

| When | Decision | Default / status |
|------|----------|------------------|
| Before phase 1 | **Reuse `tailscale-rs` crates vs hand-port** (per crate) | **decided: reuse** — `ts_keys`, `ts_control_serde`, `ts_capabilityversion`, `ts_noise`, `ts_control_noise` wired as path deps. Requires toolchain ≥1.92 (their MSRV); project bumped 1.85.0→**1.96.0**. `OverTLSPublicKeyResponse`/`EarlyNoise` hand-written (server-emitted, not in `ts_control_serde`) |
| Before phase 1 | Scope: general server core, sensor scenario first | **decided** |
| Before phase 2 | Persistence: `Store` trait + bundled backends | **`MemoryStore` + `FileStore` (JSON/YAML/TOML); no SQL in phase 1** (decided) |
| Later (optional) | SQL backend (`SqlStore`: SQLite/Postgres) as a `Store` impl | deferred until a deployment needs it |
| Before phase 4 | Policy model | **hub-and-spoke tag ACL** (decided) |
| Before phase 5 | Multi-tenancy: single tailnet + ACL, or `Network` entity | **ACL now; `network_id` seam kept open** (decided) |
| Before phase 5 | Tenant routing strategy | **All three** — subdomain (all clients), path prefix (Rust client, 2-line patch), optional mTLS (Rust client, cert CN/SAN). Server supports all strategies simultaneously; first match wins. See [02 §4](02-target-architecture.md) |
| Before phase 3 | DERP: public / standalone / embedded | standalone or public |
| Ongoing | Tailscale client version window to support | latest ~10 minor |
