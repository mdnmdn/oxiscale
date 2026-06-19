# 01 — Headscale Structure (Go Reference Map)

This document describes how the existing Go implementation (Headscale) is
organised and how a Tailscale client gets from "powered on" to "streaming
network maps". It is the reference map for the Rust port; the Rust target
lives in [02-target-architecture.md](02-target-architecture.md).

Headscale is ~36k lines of non-test Go under `hscontrol/`. Go 1.26, module
`github.com/juanfont/headscale`. It depends heavily on `tailscale.com/*`
packages for the wire protocol — those are the parts that make this a
*protocol re-implementation* rather than an ordinary rewrite.

> **File:line references** in `_docs/` point at Go source under
> [`_refs/headscale/`](../_refs/headscale/) (e.g. `hscontrol/noise.go` →
> `_refs/headscale/hscontrol/noise.go`). They are anchors for the Rust
> implementer, not permanent coordinates.

---

## 1. Top-level layout

```
headscale/
├── cmd/
│   ├── headscale/    # server binary + CLI (cobra)
│   └── hi/           # integration-test runner
├── hscontrol/        # the entire control plane  ← all phase-1 interest is here
├── proto/            # gRPC/protobuf management API (OUT of scope phase 1)
├── gen/              # generated protobuf code
├── integration/      # Docker-based end-to-end tests
└── docs/             # user docs
```

### `hscontrol/` packages

| Path | Role | Phase-1 relevance |
|------|------|-------------------|
| `app.go` | Server bootstrap, HTTP router (`createRouter`), goroutine wiring | **Core** — defines the route table |
| `noise.go` | TS2021 Noise upgrade + per-connection HTTP/2 router | **Core** — the handshake |
| `auth.go` | Registration dispatch (authkey / interactive / OIDC) | **Core** (authkey only) |
| `oidc.go` | OIDC browser flow | Out of scope |
| `poll.go` | Long-poll map session (`serveLongPoll`) | **Core** — the data plane |
| `handlers.go` | `/key`, `/health`, `/verify`, misc HTTP | **Core** (`/key`) |
| `mapper/` | Map building + batching + fan-out to clients | **Core** |
| `state/` | Central coordinator + copy-on-write NodeStore | **Core** |
| `db/` | GORM persistence, migrations, IP allocation | **Core** (model + IP alloc) |
| `policy/v2/` | ACL/filter compilation (HuJSON) | **Minimal subset** |
| `dns/` | MagicDNS config + extra records | Optional |
| `derp/` | DERP map + embedded relay server | Optional (can stay Go) |
| `types/` | Core domain types (`Node`, `User`, `PreAuthKey`, `Config`) | **Core** |
| `grpcv1.go` | gRPC management API | Replaced by Rust API |
| `capver/` | Capability-version ↔ Tailscale-version table | **Core** |

---

## 2. The connection lifecycle (the thing we are porting)

A first-time client connection runs four HTTP exchanges. All but the first
happen *inside* a Noise-encrypted, HTTP/2-framed tunnel.

```
Client                                Server
  │                                      │
  │ 1. GET /key?v=<capver>               │   discover server Noise pubkey
  │ ◀── {"PublicKey":"mkey:…"} ──────────│   handlers.go: KeyHandler
  │                                      │
  │ 2. POST /ts2021 (Upgrade)            │   Noise IK handshake
  │ ◀═══ Noise handshake + EarlyNoise ══▶│   noise.go: NoiseUpgradeHandler
  │                                      │
  │ ══ Noise-encrypted HTTP/2 tunnel ════│
  │                                      │
  │ 3. POST /machine/register            │   register node (pre-auth key)
  │ ◀── {MachineAuthorized:true} ────────│   auth.go: handleRegister
  │                                      │
  │ 4. POST /machine/map  (Stream=true)  │   open the long-poll
  │ ◀── [len][MapResponse] (initial) ────│   poll.go: serveLongPoll
  │ ◀── [keepalive every ~50s] ──────────│
  │ ◀── [len][MapResponse] (on change) ──│   pushed by mapper.Batcher
```

### Step 1 — `/key` (handlers.go:191 `KeyHandler`)

Client GETs `/key?v=<CapabilityVersion>`. For all currently supported
clients (capver ≥ 39) the server returns
`tailcfg.OverTLSPublicKeyResponse{PublicKey: noisePrivateKey.Public()}` —
i.e. its long-lived **Curve25519 machine public key**, text-encoded as
`mkey:<hex>`. The client needs this to perform Noise IK (Initiator Knows
responder static key).

### Step 2 — `/ts2021` (noise.go:92 `NoiseUpgradeHandler`)

This is the heart of the protocol.

- The handler hands the hijacked socket to
  `controlhttpserver.AcceptHTTP(ctx, w, req, noisePrivateKey, earlyNoise)`
  (`tailscale.com/control/controlhttp/controlhttpserver`). That library
  performs the **Noise IK handshake** and returns a `*controlbase.Conn` —
  an authenticated, encrypted bidirectional channel.
- During the handshake the server's `earlyNoise` callback (noise.go:228)
  writes an **early payload** into the channel: a 9-byte frame
  (`\xff\xff\xffTS` magic + 4-byte big-endian length) followed by JSON
  `tailcfg.EarlyNoise{NodeKeyChallenge: <ChallengePublic>}`. The challenge
  lets the client prove ownership of its node key without an extra
  round-trip.
- The same callback enforces the minimum capability version
  (`capver.MinSupportedCapabilityVersion`, currently **113** = Tailscale
  v1.80); older clients are rejected before any app data flows.
- After the handshake, the encrypted `net.Conn` is wrapped in a
  **single-connection HTTP/2 server** (`http2.Server.ServeConn`) running a
  fresh `chi.Router`. Every later request from this client is HTTP/2
  multiplexed inside the Noise tunnel.

Routes mounted on the per-connection router:
`POST /machine/register`, `POST /machine/map`,
`GET /machine/ssh/action/...`, plus several `NotImplemented` stubs. A 1 MiB
body limit guards every route.

**Key crypto detail:** the machine key (`key.MachinePublic`, the Noise
static identity) is extracted from the session via `conn.Peer()` and is the
client's hardware identity. It is *not* re-validated per request — anything
reaching `/machine/*` is already inside an authenticated Noise session.

### Step 3 — `/machine/register` (auth.go:38 `handleRegister`)

Decodes a `tailcfg.RegisterRequest` (JSON). Dispatch tree:

1. Past `Expiry` → logout.
2. `Auth == nil` → look up node by `NodeKey`; return current state or
   logout.
3. `Followup` URL present → block in `waitForFollowup` until an interactive
   flow completes.
4. **`Auth.AuthKey != ""` → `handleRegisterWithAuthKey`** ← *the only path
   phase 1 needs.*
5. else → `handleRegisterInteractive` (web/OIDC).

The auth-key path calls
`state.HandleNodeFromPreAuthKey(req, machineKey)` (state.go:2275):
validates the key (bcrypt), takes a per-machine-key lock, then either
updates the existing node or creates a new one (allocating IPs), and
atomically consumes single-use keys (`UsePreAuthKey`, conditional
`UPDATE … WHERE used=false`). Returns `RegisterResponse{MachineAuthorized:
true}`. `DiscoKey` is zero at this point; it arrives with the first map
request.

### Step 4 — `/machine/map` (poll.go `serveLongPoll`)

The client POSTs `tailcfg.MapRequest{Stream:true, Compress:"zstd", …}`.
The server:

1. `UpdateNodeFromMapRequest` — absorb endpoints/hostinfo/routes into the
   NodeStore (state.go:2796).
2. `state.Connect(id)` — mark online, bump session epoch.
3. `mapBatcher.AddNode(id, ch, capVer, stop)` — register the channel and
   **synchronously deliver the initial full map** as the first frame.
4. Enter a `select` loop writing frames from `ch`, sending keepalives
   (`{KeepAlive:true}` every ~50 s + jitter), until context cancel.

**Wire framing** (poll.go `writeMap`): `[4-byte LE length][JSON or
zstd-JSON body]`, flushed after every write. The 4-byte prefix is
application-level framing *inside* the already-encrypted Noise/HTTP2
stream.

---

## 3. State, NodeStore, and persistence

### `State` (state/state.go) — the central coordinator

Everything cross-cutting goes through `State`. It owns the DB handle, the
`NodeStore`, the IP allocator, the policy manager, an LRU auth cache, and
per-machine-key registration locks. All mutating methods return one or more
`change.Change` values describing what must go into the next `MapResponse`;
these are fed to the mapper's batcher and fanned out.

```go
type Change struct {
    TargetNode    NodeID            // 0 = broadcast
    OriginNode    NodeID
    PeersChanged  []NodeID
    PeersRemoved  []NodeID
    PeerPatches   []*tailcfg.PeerChange
    IncludeDERPMap, IncludeDNS, IncludePolicy bool
    RequiresRuntimePeerComputation bool
    // …
}
```

### `NodeStore` (state/node_store.go) — copy-on-write cache

A read-optimised, **lock-free-read** cache: `atomic.Pointer[Snapshot]`.
Reads (the hot path — every map request from every node) are a single
atomic pointer load. Writes go through a single writer goroutine that
batches work, rebuilds a new immutable `Snapshot`, and atomically swaps it
in.

```go
type Snapshot struct {
    nodesByID         map[NodeID]Node              // source of truth
    nodesByNodeKey    map[key.NodePublic]NodeView  // derived indices…
    nodesByMachineKey map[key.MachinePublic]map[UserID]NodeView
    peersByNode       map[NodeID][]NodeView        // policy-filtered peer map
    nodesByUser       map[UserID][]NodeView
    allNodes          []NodeView
    routes            map[netip.Prefix]NodeID       // HA primary-route election
    isPrimaryRoute    map[NodeID]bool
}
```

The expensive part of each snapshot rebuild is `peersFunc` — a closure over
`policy.BuildPeerMap` that recomputes who-can-see-whom. It is **O(n²)** in
node count and is the main scaling pressure on the hot path.

### Database (`db/`)

- GORM over **SQLite** (`glebarez/sqlite`, pure-Go, pool forced to 1
  connection) or **PostgreSQL** (`gorm.io/driver/postgres`).
- Schema is embedded (`schema.sql`) and validated post-migration with
  `squibble` (SQLite only).
- Migrations via `gormigrate`, IDs `YYYYMMDDHHMM-slug`, **append-only**,
  never reordered (see `AGENTS.md` Database Migration Rules).
- Main tables: `nodes`, `users`, `pre_auth_keys`, `api_keys`, `policies`,
  `database_versions`. (The old `routes` table was folded into
  `nodes.approved_routes` JSON in v0.26.)

### IP allocation (`db/ip.go`)

`IPAllocator` hands out one IPv4 (from the CGNAT prefix, default
`100.64.0.0/10` subset) and one IPv6 (ULA `fd7a:115c:a1e0::/48` subset) per
node, sequential or random strategy, skipping Tailscale-reserved IPs. The
used-IP set is loaded from the DB at startup and held in memory
(`netipx.IPSetBuilder`).

### Core `Node` type (types/node.go)

```go
type Node struct {
    ID         NodeID
    MachineKey key.MachinePublic   // Noise identity (hardware)
    NodeKey    key.NodePublic      // WireGuard session key (rotates)
    DiscoKey   key.DiscoPublic     // peer path-discovery key
    Endpoints  []netip.AddrPort
    Hostinfo   *tailcfg.Hostinfo   // OS, routes (RoutableIPs), PreferredDERP
    IPv4, IPv6 *netip.Addr
    GivenName  string              // DNS label
    UserID     *uint               // nil for tagged nodes
    Tags       []string            // non-empty ⇒ "tagged" (owned by tags, not user)
    AuthKeyID  *uint64
    Expiry     *time.Time
    ApprovedRoutes []netip.Prefix
    // runtime-only (gorm:"-"): IsOnline, Unhealthy, ActiveSessions, SessionEpoch
}
```

**Tags-as-identity rule** (load-bearing): a node is *either* tagged *or*
user-owned, never both. `IsTagged()` (`len(Tags)>0`) is authoritative for
ownership — not `UserID`.

---

## 4. The mapper subsystem (data-plane fan-out)

When one node changes, every node that can see it must get an updated map.
This is what `mapper/` does. Four files:

| File | Role |
|------|------|
| `batcher.go` | Worker pool + per-node registry (`xsync.Map`) + change fan-out, coalesced on a tick (`BatchChangeDelay`) |
| `node_conn.go` | Per-node multi-connection state, delivery with 50 ms timeout, peer-diff tracking (`lastSentPeers`) |
| `mapper.go` | Builds the actual `MapResponse` (policy filtering, DNS) |
| `builder.go` | Fluent `MapResponseBuilder` (`WithDERPMap()`, `WithDNSConfig()`, `WithPeerChangedPatch()`, …) |

Flow: a `change.Change` enters `Batcher.AddWork`, is filtered per node
(`FilterForNode`), queued in each node's `pending`, and on the next tick a
worker turns it into a `MapResponse` and pushes it to the node's channel.
Incremental updates are expressed three ways, cheapest first:

- `PeersChangedPatch []*tailcfg.PeerChange` — endpoint/DERP/online deltas;
- `PeersChanged []*tailcfg.Node` — full node objects for added/changed peers;
- `PeersRemoved []NodeID` — peers no longer visible (computed by diffing
  `lastSentPeers`).

### Peer visibility / filtering

What a node sees = (all other nodes) ∩ (nodes with an ACL path to/from it).
Computed by `policy.ReduceNodes(node, peers, matchers)` where matchers come
from the compiled policy. With **no policy**, every node sees every node in
its tailnet (default-open). Per-peer `AllowedIPs` (including subnet-route
steering) come from `RoutesForPeer`.

---

## 5. Policy engine (`policy/v2/`)

Reads a **HuJSON** ACL document (comments + trailing commas) and compiles
it into per-node `tailcfg.FilterRule` sets plus SSH policies, tag-owner
maps, and route auto-approver maps. Outputs land in
`MapResponse.PacketFilters["base"]` and `MapResponse.SSHPolicy`.

For the sensor/central-service scenario the full ACL grammar is overkill;
phase 1 needs only enough to express "every node may talk to every node"
(default-open) or a simple allowlist. The compiler's key output type is:

```go
type FilterRule struct {
    SrcIPs   []string        // CIDRs or "*"
    DstPorts []NetPortRange  // {IP, PortRange}
    IPProto  []int           // nil = TCP+UDP+ICMP
    CapGrant []CapGrant
}
```

---

## 6. DERP (`derp/`)

Two responsibilities, both optional for phase 1:

1. **DERP map distribution** — a `tailcfg.DERPMap` (relay topology) is
   stored atomically in `State` and injected into every initial map and
   whenever it changes. Clients use it to find relays for NAT traversal.
2. **Embedded relay server** — Headscale can *be* a DERP relay
   (`tailscale.com/derp/derpserver` + STUN). This is a self-contained
   sub-service that clients connect to independently of the control plane.

In a controlled sensor deployment you can point at public Tailscale DERPs,
run a standalone DERP, or (if endpoints are directly reachable) lean on
direct connections. The relay can remain a separate Go process initially.

---

## 7. Dependency reality check

The Go code leans on `tailscale.com/*` for everything protocol-shaped.
These are the packages that define the porting effort (full inventory and
Rust mappings in [03-migration-plan.md](03-migration-plan.md) §Dependencies):

| Go package | What it provides | Why it's hard |
|------------|------------------|---------------|
| `tailscale.com/tailcfg` | **All** wire types (`MapRequest/Response`, `Node`, `FilterRule`, `DERPMap`, `Hostinfo`, …) | Large surface; JSON shape must match byte-for-byte |
| `tailscale.com/types/key` | `MachinePrivate/Public`, `NodePublic`, `DiscoPublic`, `ChallengePrivate` | Curve25519 + Tailscale's typed text encoding (`mkey:`/`nodekey:`/`discokey:`) |
| `tailscale.com/control/controlbase` + `controlhttp/controlhttpserver` | The TS2021 Noise IK handshake + HTTP upgrade | The single highest-risk component |
| `tailscale.com/util/zstdframe` | zstd framing of map bodies | Must match framing exactly |
| `tailscale.com/net/tsaddr` | CGNAT/ULA ranges, exit-route detection | Constants must match |
| `go4.org/netipx` | IP-set arithmetic | No direct Rust equivalent — custom |

Non-protocol deps (GORM, zerolog, viper, cobra, grpc) have ordinary Rust
equivalents (tracing, serde/figment, clap, tonic) and are not the risk.
GORM specifically is **not** mapped to a fixed ORM: persistence becomes a
host-injectable `Store` trait with bundled in-memory and file
(JSON/YAML/TOML) backends, SQL deferred — see
[02 §3.5](02-target-architecture.md).

---

## 8. What "minimal" actually means here

For the target scenario, the phase-1 server must implement:

- **Serve** `/key`, `/ts2021` (Noise), `/machine/register` (authkey only),
  `/machine/map` (streaming).
- **Persist** nodes, users (or "networks"), pre-auth keys; allocate IPs.
- **Build** per-node `MapResponse`s and fan out changes to peers.
- **Apply** a minimal filter (default-open or simple allowlist).
- **Distribute** a DERP map (pointing at existing relays) so NAT traversal
  works.

It can **omit**: OIDC, interactive web registration, the gRPC/REST API, the
admin UI, SSH ACLs, Taildrop, logtail, autoupdate, exit-node UX niceties.
Those are deferred or replaced by the embedding API (see
[02-target-architecture.md](02-target-architecture.md)).
