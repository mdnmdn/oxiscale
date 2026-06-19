# 02 — Target Rust Architecture

This document proposes the architecture for the Rust port of Headscale's
**client-facing control plane** — an **embeddable Rust crate** that hosts a
minimal Tailscale control plane.

The design goal is a **general-purpose Tailscale control server** delivered
as a library the host application links against and drives through a typed
Rust API. The sensor/central-service scenario is the **first target
configuration**, not a special case wired into the core: the data model,
policy engine, and tenancy hooks are general; the scenario is expressed as
configuration on top of them. A thin binary wrapper is provided for testing
and standalone use, but it is a client of the same crate API.

---

## 0. Scope & deployment scenario

The end goal is a **general-purpose Tailscale control server, in Rust,
embeddable as a library** — a Headscale-equivalent core. The *sensor /
central-service* scenario below is the **first target use-case**, not a
constraint baked into the design. We deliver the narrow scenario first, but
keep the architecture multipurpose so it can grow into a full Headscale
alternative (OIDC, web/gRPC API, full ACLs, multiple tailnets) later.

**In scope (phase 1):**

- The *data plane* — Noise/TS2021 handshake + long-poll network-map stream
- The *minimal control plane* — pre-auth-key registration, IP allocation,
  peer/map building, a real but small policy/filter engine, DERP-map
  distribution
- **Native Rust APIs** the host application calls directly (`control-api`)

**Out of scope (phase 1), but designed for:**

- gRPC/REST management API, web admin UI, OIDC / interactive browser
  registration, CLI, embedded DERP relay — additive layers on the same core,
  never re-architectures (see §6)

### Deployment scenario

> Several **independent networks**, each containing one or more **remote
> sensors** plus a **central service** node. All sensors talk to the
> central service over the tailnet. The control service may run on a
> different host than the central service. Connection keys are generated
> out-of-band and delivered to each sensor over a **secondary channel**
> for first connection.

The "connection keys delivered out-of-band" maps directly onto Headscale's
**pre-auth keys** — which is why OIDC / interactive browser flows are dropped
from phase 1. The topology is **hub-and-spoke**: the central service reaches
all sensors; sensors need not see each other. Phase 1 models this in a
**single tailnet with a hub-and-spoke ACL**. Supporting *several independent
central servers* (each its own isolated network) is a designed-for extension
— the multi-network model — rather than a phase-1 requirement. See
[04-critical-points.md](04-critical-points.md) §2 and §4 below.

### Reusing Tailscale's official Rust crates

Tailscale ships an official (experimental, **BSD-3-Clause**) Rust
implementation, [`tailscale-rs`](https://github.com/tailscale/tailscale-rs)
(vendored at `_refs/tailscale-rs/`). It is *client*-side, but its workspace
contains crates we can reuse or mirror server-side: `ts_control_serde` (the
`tailcfg` wire types), `ts_keys` (key types + encoding), `ts_noise` /
`ts_control_noise` (the Noise/TS2021 handshake), `ts_derp`. This materially
lowers the two highest risks (handshake + wire-type fidelity) — see
[04-critical-points.md](04-critical-points.md). It also doubles as a Rust
**test client** for our server alongside a real `tailscaled`.

---

## 1. Design principles

1. **Protocol fidelity over elegance.** The wire format is defined by the
   Tailscale client, which we do not control. Every `tailcfg` struct, key
   encoding, and framing byte must match what the Go client expects. We
   port behaviour, not aesthetics.
2. **Library first.** The public surface is a Rust API
   (`ControlServer`, `Network`, `PreAuthKey`, …). HTTP is an internal
   detail. No gRPC, no REST, no web UI in phase 1.
3. **General core, scenario as configuration.** Build a real (if initially
   small) policy engine and keep a tenancy seam, so the same core serves
   the hub-and-spoke sensor case *and* can grow into a full multi-tailnet
   Headscale alternative. Phase 1 ships single-tailnet + hub-and-spoke ACL;
   multiple independent networks is a designed-for extension (see §4), not a
   rewrite.
4. **Reuse Tailscale's official Rust crates where possible.**
   [`tailscale-rs`](https://github.com/tailscale/tailscale-rs) (BSD-3) gives
   us `tailcfg` wire types, key encoding, and a Noise reference for free —
   don't hand-reimplement what already exists in Rust (see §2). Treat them
   as upstream to track, not to fork casually.
5. **Async, not goroutines.** Tokio replaces Go's runtime. The
   copy-on-write NodeStore maps cleanly onto `arc-swap`; the long-poll
   sessions map onto Tokio tasks + channels.
6. **Keep the data model recognisable.** Field-for-field parity with the
   Go `Node`/`User`/`PreAuthKey` types eases verification against the
   reference implementation and against captured wire traffic.

---

## 2. Crate layout

A Cargo workspace. The protocol types are isolated in their own crates so
they can be tested independently — and, where possible, **sourced from
`tailscale-rs`** rather than written from scratch.

> **Build-vs-reuse decision (do this in the phase-1 spike):** evaluate
> depending on `tailscale-rs`'s `ts_control_serde` (wire types), `ts_keys`
> (keys), and `ts_noise`/`ts_control_noise` (handshake) directly. They are
> internal workspace crates (not necessarily published on crates.io), so
> reuse likely means a **git dependency on a pinned revision** or vendoring
> the needed crates into this workspace (BSD-3 permits this with
> attribution). If reuse proves impractical (e.g. the Noise crate is too
> tightly coupled to the *initiator* role), the crates below are the
> fallback hand-port — but `tailscale-rs` remains the authoritative
> reference and the byte-level oracle.

```
tailcontrol/                     # workspace root
├── crates/
│   ├── tailcfg/                 # ── PREFER tailscale-rs `ts_control_serde` ──
│   │   └── wire types: MapRequest, MapResponse, Node, FilterRule,
│   │       DERPMap, Hostinfo, NetInfo, DNSConfig, RegisterRequest/Response,
│   │       EarlyNoise, OverTLSPublicKeyResponse, PeerChange, CapabilityVersion
│   │       (reuse upstream if viable; else hand-port from tailscale.com/tailcfg)
│   │
│   ├── tskey/                   # ── PREFER tailscale-rs `ts_keys` ──
│   │   └── MachinePrivate/Public, NodePublic, DiscoPublic, ChallengePrivate
│   │       + mkey:/nodekey:/discokey: text codecs
│   │
│   ├── noise/                   # ── TS2021 Noise IK (RESPONDER side) ──
│   │   └── handshake + EarlyNoise framing + HTTP upgrade; mirror tailscale-rs
│   │       `ts_noise`/`ts_control_noise` (which do the initiator side) onto
│   │       the responder role. snow underneath if hand-built.
│   │
│   ├── control-core/            # ── the control plane ──
│   │   ├── state/               # State coordinator + NodeStore (arc-swap)
│   │   ├── store/               # persistence PORT: `Store` trait +
│   │   │                        #   bundled backends (memory, file)
│   │   ├── ipalloc/             # IP allocation
│   │   ├── mapper/              # MapResponse build + batched fan-out
│   │   ├── policy/              # minimal filter compilation
│   │   ├── derp/                # DERP map handling (+ optional relay shim)
│   │   └── server/              # HTTP routes (/key, /ts2021, /machine/*)
│   │
│   └── control-api/             # ── the public embedding API ──
│       └── ControlServer, Network, NodeHandle, PreAuthKey, events
│
└── bin/
    └── tailcontrold/            # thin standalone binary (testing/standalone)
```

### Recommended crates

| Concern | Go | Rust |
|---------|----|----|
| **Wire types (`tailcfg`)** | `tailscale.com/tailcfg` | **`tailscale-rs` `ts_control_serde`** (reuse) ▸ else serde hand-port |
| **Key types + encoding** | `tailscale.com/types/key` | **`tailscale-rs` `ts_keys`** (reuse) ▸ else `x25519-dalek` + custom codecs |
| **Noise/TS2021 handshake** | `control/controlbase` | mirror **`tailscale-rs` `ts_noise`** to responder ▸ else `snow` (Noise_IK, 25519+ChaChaPoly+BLAKE2s) |
| DERP client/reference | `derp` | `tailscale-rs` `ts_derp` (reference) |
| Async runtime | goroutines | `tokio` |
| HTTP server / routing | `chi` | `axum` (+ `hyper`) |
| HTTP/2 over hijacked conn | `golang.org/x/net/http2` | `h2` with a custom `AsyncRead+AsyncWrite` over the Noise `Conn` |
| JSON | `go-json-experiment` | `serde_json` |
| HuJSON | `tailscale/hujson` | strip comments + `serde_json`; or a hjson crate. Small subset suffices in phase 1 |
| zstd framing | `util/zstdframe` | `zstd` crate (frame mode) |
| Copy-on-write store | `atomic.Pointer` | `arc-swap` |
| Concurrent map | `xsync.Map` | `dashmap` or `scc` |
| Persistence | GORM (hardwired SQL) | **`Store` trait** (host-injectable port) — see §3.5 |
| ⮡ default backend | — | `MemoryStore` (in-process, volatile) |
| ⮡ file backend | — | `FileStore`: serde snapshot to JSON / YAML / TOML (`serde_json` / `serde_yaml` / `toml`) |
| ⮡ SQL backend | GORM | **deferred** — a future `SqlStore` (`sqlx`) implements the same trait; no SQL in phase 1 |
| Migrations | `gormigrate` | n/a for memory/file (versioned snapshot envelope); `sqlx::migrate!` only when the SQL backend lands |
| IP set arithmetic | `go4.org/netipx` | `ipnet` + custom range-set (no direct equivalent) |
| Logging | `zerolog` | `tracing` |
| Config | `viper` | `figment` / `serde` |
| Metrics | `prometheus/client_golang` | `metrics` + `metrics-exporter-prometheus` |
| bcrypt (pre-auth keys) | `golang.org/x/crypto/bcrypt` | `bcrypt` crate |

---

## 3. Component architecture

```
                         host application
                                │  (Rust calls)
                ┌───────────────▼────────────────┐
                │          control-api            │
                │  ControlServer / Network /      │
                │  PreAuthKey / event stream      │
                └───────────────┬────────────────┘
                                │
   ┌────────────────────────────┼─────────────────────────────┐
   │                       control-core                         │
   │                                                            │
   │   server (axum)                 State (coordinator)        │
   │   ├ GET  /key            ┌────────────┴───────────┐        │
   │   ├ POST /ts2021 ──noise─┤  NodeStore (arc-swap)  │        │
   │   ├ POST /machine/register│  IpAllocator           │        │
   │   └ POST /machine/map ────┤  PolicyManager         │        │
   │            │              │  dyn Store ─────────┐  │        │
   │            ▼              └────────────┬────────┼──┘        │
   │      Mapper / Batcher  ◀── change events ──┘    │           │
   │      (per-node tasks + channels)                │           │
   └─────────────────────────────────────────────────┼───────────┘
                                                      │  Store trait
                                                      ▼  (host-injectable)
                  ┌───────────────────────────────────────────────┐
                  │  MemoryStore   FileStore        …host's own    │
                  │  (volatile)    (JSON/YAML/TOML)  `impl Store`  │
                  └───────────────────────────────────────────────┘
```

### 3.1 NodeStore — copy-on-write in Rust

The Go `atomic.Pointer[Snapshot]` becomes `ArcSwap<Snapshot>`. Reads are
`store.load()` (a cheap `Arc` clone). Writes are serialised through a single
writer task fed by an `mpsc` channel, exactly mirroring the Go writer
goroutine:

```rust
pub struct NodeStore {
    data: ArcSwap<Snapshot>,
    tx:   mpsc::Sender<Work>,        // writes go here; one writer task drains
    peers_fn: Arc<dyn Fn(&[NodeView]) -> PeerMap + Send + Sync>,
}

pub struct Snapshot {
    nodes_by_id:    HashMap<NodeId, Arc<Node>>,
    by_node_key:    HashMap<NodePublic, NodeId>,
    by_machine_key: HashMap<MachinePublic, HashMap<UserId, NodeId>>,
    peers_by_node:  HashMap<NodeId, Vec<NodeId>>,      // policy-filtered
    all_nodes:      Vec<Arc<Node>>,
    routes:         HashMap<IpNet, NodeId>,            // HA primary election
}
```

`NodeView` (Go's mutation-guard wrapper) becomes `Arc<Node>` — sharing
without copying, immutable by construction. The writer applies a batch of
`Work`, rebuilds the `Snapshot` (re-running `peers_fn` and primary-route
election), and `store.store(Arc::new(new_snapshot))`. Same O(n²) peer-map
cost as Go; same batching mitigation.

### 3.2 Long-poll session

Each `/machine/map` stream is a Tokio task holding the response body. A
`tokio::sync::mpsc::Receiver<MapResponse>` is the delivery pipe (Go's
buffered `chan *MapResponse`). The select loop becomes `tokio::select!`
over: the receiver, a keepalive `interval`, and the connection's cancel
token. Frames are written `[u32 LE length][zstd-or-json body]` and flushed.

### 3.3 Mapper / Batcher

A `Batcher` owns a `DashMap<NodeId, NodeConn>` registry. A `change::Change`
enters, is filtered per node, queued, and coalesced on an interval tick;
a worker pool (`tokio` tasks pulling from an `mpsc`) turns queued changes
into `MapResponse`s and sends them to each node's channel. The peer-diff
(`last_sent_peers`) logic ports directly. This is the most intricate
subsystem to port and should be covered by a dedicated property/fuzz test
harness (see [03-migration-plan.md](03-migration-plan.md)).

### 3.4 Noise transport

`crates/noise` implements the **responder** side of TS2021. The companion
crate in `tailscale-rs` (`ts_noise` / `ts_control_noise`) implements the
**initiator** side — it is the authoritative Rust reference for the Noise
parameters, the EarlyNoise framing, and the HTTP-upgrade dance, and may be
partly reusable. If hand-built, it wraps `snow` configured for the Noise_IK
pattern Tailscale uses, plus:

- the `/key` response (`OverTLSPublicKeyResponse`),
- the HTTP upgrade dance on `/ts2021` (read the upgrade request, run the
  handshake over the hijacked TCP stream),
- the **EarlyNoise** frame: `\xff\xff\xffTS` + 4-byte BE length + JSON
  `EarlyNoise{ nodeKeyChallenge }`,
- exposing the resulting encrypted stream as an `AsyncRead + AsyncWrite`
  that the `h2` server runs an HTTP/2 connection over.

> This crate is the **single biggest risk** and should be prototyped and
> proven against a real `tailscaled` client **before** anything else is
> built. See critical points.

### 3.5 Persistence — the `Store` trait (host-injectable)

Persistence is a **port, not a fixed backend.** The control plane never
talks to a database directly; it depends on a single `Store` trait, and the
**host application chooses (or supplies) the implementation.** This keeps the
core free of any SQL dependency in phase 1 and lets an embedder decide where
state lives — RAM, a config-style file, or their own datastore.

`NodeStore` (§3.1) stays the in-memory, copy-on-write **runtime** source of
truth for the hot read path. `Store` is the **durability** seam behind it:
state is loaded through `Store` at startup and mutations are written back
through `Store`. The two are distinct — `NodeStore` is about fast concurrent
reads, `Store` is about where bytes persist.

```rust
/// Durable persistence port. The control plane owns a `dyn Store`;
/// the host picks the backend (or implements this trait itself).
#[async_trait]
pub trait Store: Send + Sync + 'static {
    // --- Networks (one default row in phase 1; the network_id seam, §4) ---
    async fn list_networks(&self) -> Result<Vec<Network>>;
    async fn upsert_network(&self, net: &Network) -> Result<()>;

    // --- Users ---
    async fn list_users(&self, network: NetworkId) -> Result<Vec<User>>;
    async fn upsert_user(&self, user: &User) -> Result<()>;

    // --- Nodes ---
    async fn list_nodes(&self, network: NetworkId) -> Result<Vec<Node>>;
    async fn get_node(&self, id: NodeId) -> Result<Option<Node>>;
    async fn upsert_node(&self, node: &Node) -> Result<()>;
    async fn delete_node(&self, id: NodeId) -> Result<()>;

    // --- Pre-auth keys ---
    async fn list_preauth_keys(&self, network: NetworkId) -> Result<Vec<PreAuthKey>>;
    async fn upsert_preauth_key(&self, key: &PreAuthKey) -> Result<()>;
    /// Atomic single-use consumption. Returns `true` iff this call won the
    /// race and flipped an unused key to used. (SQL: `UPDATE … WHERE used=false`;
    /// memory/file: guarded by a mutex.) See [03 §Phase 2].
    async fn consume_preauth_key(&self, id: PreAuthKeyId) -> Result<bool>;

    // --- Policy ---
    async fn get_policy(&self, network: NetworkId) -> Result<Option<String>>;
    async fn set_policy(&self, network: NetworkId, policy: &str) -> Result<()>;
}
```

The trait is intentionally **aggregate-oriented CRUD** (per `Node`, `User`,
`PreAuthKey`, `Network`, policy) rather than snapshot-oriented, so a future
row-based SQL backend maps onto it cleanly while the file backend can still
satisfy it by snapshotting. The one non-CRUD method,
`consume_preauth_key`, exists because single-use key redemption must be
atomic regardless of backend.

#### Bundled backends (phase 1)

| Backend | Shape | Use |
|---------|-------|-----|
| **`MemoryStore`** | `RwLock<HashMap<…>>` per aggregate; nothing on disk | default; tests; ephemeral deployments |
| **`FileStore`** | wraps the in-memory maps for reads/writes, **serialises the whole dataset to one file** on each mutation (debounced) and loads it at startup | small, persistent hub-and-spoke deployments — exactly the sensor-network target |

`FileStore` serialises a single versioned document via `serde`:

```rust
/// On-disk envelope; format chosen by extension or builder.
#[derive(Serialize, Deserialize)]
struct StoreDocument {
    version:  u32,                 // snapshot schema version (migration hook)
    networks: Vec<Network>,
    users:    Vec<User>,
    nodes:    Vec<Node>,
    preauth_keys: Vec<PreAuthKey>,
    policies: HashMap<NetworkId, String>,
}

pub enum Format { Json, Yaml, Toml }   // serde_json / serde_yaml / toml
```

- **Format is `serde`-driven and interchangeable**: the same
  `StoreDocument` round-trips through any of the three. The format is picked
  from the file extension (`.json` / `.yaml` / `.toml`) or set explicitly on
  the builder.
- Writes are **whole-file snapshots** (write-temp-then-rename for atomicity),
  debounced on a short interval so a burst of node updates collapses into one
  flush — acceptable because the target deployments are small (tens of
  sensors), and the NodeStore, not the file, serves reads.
- The `version` field is the **migration hook**: snapshot-schema changes bump
  it and `FileStore` upconverts on load. This replaces SQL migrations for the
  file/memory backends (see [04 §8](04-critical-points.md)).

#### Host-supplied backends

Because `Store` is a public trait, an embedder can implement it over their
own storage (their existing Postgres, an embedded KV like `sled`, an object
store) without any change to the core. The deferred `SqlStore` (`sqlx`,
[03 §Phase 6 / decision log]) is just one more implementor of this trait — it
is **not** a privileged or built-in path, which is the whole point of the
abstraction.

---

## 4. Tenancy: hub-and-spoke now, independent networks later

The scenario is **hub-and-spoke**: one central service must reach *all*
sensors; sensors do not need to reach each other. This is the textbook case
for an **ACL**, not for data-model isolation — sensors being mutually
invisible is a convenience, not a cross-customer security boundary, as long
as one operator runs one central service over its own sensors.

**Phase-1 model (chosen): single tailnet + hub-and-spoke ACL.** Tag sensors
`tag:sensor` and the central service `tag:central`. The policy grants
`tag:sensor ↔ tag:central` and **omits** `tag:sensor ↔ tag:sensor`. The
peer-visibility computation (`peers_fn`) then naturally shows each sensor
only the central service, and shows the central service all sensors. This
needs a *real but small* policy engine (tags + a couple of grants), not just
default-open — which is good, because a general server needs a policy engine
anyway.

**Designed-for extension: several independent central servers → the
`Network` entity.** When there are multiple *independent* central servers
(separate operators/customers) sharing one deployment, ACL-only isolation in
a shared tailnet becomes risky (shared IP space, isolation hinges on policy
correctness across tenants). For that, promote the tenancy seam to a
first-class `Network`: every `Node`/`User`/`PreAuthKey`/IP-pool/policy scoped
to a `network_id`, and the NodeStore **partitioned by network** so the O(n²)
peer computation never crosses a boundary and isolation is *structural*.
A "central server that connects to all sensors together" then becomes either
a single hub tailnet (phase-1 model) or a node that is a **member of multiple
networks** (multi-network model) — both are supported by the same core.

This is an extension, not a rewrite, **if** the schema carries `network_id`
from day one (see below). We pay one cheap column now to keep the door open;
we do not partition the NodeStore until the multi-network case is actually
needed.

### Data-model seam (carry from phase 2, even while single-tenant)

The seam is a `network_id` **field on every aggregate**, independent of how
`Store` persists it (a column for SQL, a field in the JSON/YAML/TOML
snapshot for `FileStore`, a map key for `MemoryStore`):

```
Network { id, name, server_host, ipv4_prefix, ipv6_prefix,
          noise_key_id, policy, created_at }      // one default instance in phase 1
User         { …, network_id }   (role: "central" | "sensor" via tags)
Node         { …, network_id }
PreAuthKey   { …, network_id }
```

In phase 1 there is a single default `Network`; the field is present but
unexercised — and it is `Store`-agnostic, so promoting to real multi-network
later is a data-model change, not a database migration. IP pools are *per
network*, so when multi-network arrives, sensors in different networks may
reuse `100.64.0.0/10` without collision.

> **Decision (recorded):** phase 1 = single tailnet + hub-and-spoke ACL;
> multi-network kept open via the `network_id` seam. See
> [04-critical-points.md](04-critical-points.md) §2.

### How a client selects its tenant — three strategies

The tenant is resolved at the server from the incoming request. Which
strategy applies depends on what clients the deployment serves.

---

#### Strategy A — Subdomain (works with all clients, recommended for multi-tenant)

`tenant1.ctl.example.com`, `tenant2.ctl.example.com`. The server (or reverse
proxy) selects the network from the `Host` header / SNI, which **every**
client preserves on every request. A wildcard cert (`*.ctl.example.com`)
covers all tenants.

This is the only strategy compatible with **stock `tailscaled` (Go)** and
**unmodified `tailscale-rs` clients**, because they use only the control
URL's hostname+port for the Noise handshake — the path is discarded
(`tailscale.com/control/ts2021/client.go`: `host: u.Hostname()`;
`control/controlhttp/constants.go`: `/ts2021` is hardcoded at the host root).

---

#### Strategy B — Path prefix (Rust client only, 2-line change)

The Rust client (`tailscale-rs`) discards path via `Url::join("/ts2021")`
(see `ts_control/src/tokio/connect.rs:239`), which URL-resolution rules
replace the base path. The fix is a relative-join:

```rust
// Before:  control_url.join("/ts2021")  → "https://host/ts2021"
// After:   control_url.join("ts2021")   → "https://host/tenant1/ts2021"
```

Two lines in one file, touching no protocol logic. The same change applies
to the `/key` join at line 200. TCP dial and TLS SNI use host+port only and
are unaffected.

This lets a single host serve multiple tenants at:
```
https://ctl.example.com/tenant1/key
https://ctl.example.com/tenant1/ts2021
https://ctl.example.com/tenant2/key
https://ctl.example.com/tenant2/ts2021
```

The server mounts the same route group at both root and path-prefixed
endpoints (dual-mount in axum), so **stock Go clients still hit root**
(`/key`, `/ts2021`) and the modified Rust clients hit their path-prefixed
tenant endpoints:

```rust
let tenant_routes = Router::new()
    .route("/key", get(key_handler))
    .route("/ts2021", post(noise_handler))
    .route("/machine/register", post(register_handler))
    .route("/machine/map", post(map_handler))
    .with_state(state);

let app = Router::new()
    .merge(tenant_routes.clone())          // root for stock clients
    .nest("/:tenant", tenant_routes);      // path-prefixed for Rust clients
```

The tenant is extracted from the path parameter (`:tenant`) on the server
side, or from `Host` when subdomain is used — the `Network`'s
`server_host` field is the routing key regardless of strategy.

---

#### Strategy C — Optional mTLS (Rust client only, additive)

The Rust client uses `rustls` (via `ts_tls_util`). Adding a client
certificate requires changing one builder call:

```rust
// Before: .with_no_client_auth()
// After:  .with_client_auth(cert_chain, private_key)?
```

Plus plumbing an `Option<Identity>` through ~5 function signatures
(`connect_alpn` → `dial_tls` → `connect`). The server uses
`WebPkiClientVerifier::builder(ca).allow_unauthenticated()`, which **asks
for** a client cert but does not reject connections without one:

```rust
rustls::ServerConfig::builder()
    .with_client_cert_verifier(
        WebPkiClientVerifier::builder(client_ca.into())
            .allow_unauthenticated()?  // ← optional: no rejection when absent
            .build()?,
    )
    .with_single_cert(server_certs, server_key)?;
```

The server extracts the tenant from the verified cert's CN or SAN in a
middleware layer. When no client cert is presented (stock Go clients), the
request falls through to whichever default routing strategy is configured
(subdomain or path).

**Compatibility summary:**

| Strategy | Stock Go `tailscaled` | Modified Rust client |
|----------|----------------------|---------------------|
| Subdomain | ✅ works | ✅ works |
| Path | ❌ dials `/ts2021` at root | ✅ works (2-line patch) |
| mTLS | ❌ no cert sent → falls through | ✅ works (CN/SAN routing) |

All three can coexist: the server applies subdomain routing first (from
`Host`), then path (from the URL), then mTLS (from the client cert). The
first match wins. This lets a single deployment serve stock clients at root,
Rust clients at path-prefixed endpoints, and optionally use cert-based
tenant assignment for the Rust ones.

The Rust `control-api` should accept the tenant routing strategy as a
builder option; the `Network` carries a `server_host` (canonical host,
possibly with path prefix) that the routing layer uses to dispatch.

### Deployment: TLS and reverse proxy

Two facts from the protocol shape deployment (both verified against the
Tailscale client and Headscale's `noise.go`):

- **TLS is optional / can be self-signed.** The Noise layer authenticates
  the server cryptographically via the machine key the client fetches from
  `/key`, so outer TLS is not the trust anchor. Options:
  - *Self-signed HTTPS:* works **if each client trusts the CA** (system trust
    store / `SSL_CERT_FILE`); the stock client has no "skip verify" flag but
    does honour `ExtraRootCAs`/system roots.
  - *Plain HTTP, no TLS:* for a **private/loopback host** the client will not
    attempt HTTPS at all (`ts2021/client.go` clears the HTTPS port for
    private hosts), so `http://` to a private address is accepted — ideal for
    a controlled sensor network or when a proxy terminates TLS.
- **Reverse proxies (nginx/traefik/caddy) are supported** — Headscale
  documents this — but the proxy must respect the protocol:
  - **Forward the HTTP `Upgrade`/`Connection` headers** for `/ts2021`. It is
    an HTTP/1.1 connection upgrade (like WebSocket); the handshake hijacks
    the socket and then speaks HTTP/2 *inside* the Noise tunnel, so the proxy
    only relays an opaque upgraded byte stream — it does **not** need HTTP/2
    itself. (Headscale's handler returns 500 if the `Upgrade` header is
    missing — the classic "misconfigured proxy" symptom.)
  - **Disable response buffering** (`proxy_buffering off`) and set **long
    read timeouts** so the `/machine/map` long-poll streams frames through
    (keepalives every ~50 s).
  - **Route by `Host`/SNI** — which is exactly how subdomain tenancy is
    implemented at the proxy: terminate TLS (e.g. a wildcard Let's Encrypt
    cert), then forward to one backend that picks the tenant by `Host`, or to
    per-tenant backends.
  - Pass `X-Forwarded-For` if the server uses trusted-proxy/real-IP handling.

---

## 5. The embedding API (control-api)

This replaces the gRPC/REST management surface with native Rust. Sketch:

```rust
// Construction — the host injects the persistence backend (the `Store` trait)
let server = ControlServer::builder()
    .store(MemoryStore::new())                  // volatile (default), or…
    // .store(FileStore::open("control.yaml")?) //   snapshot to YAML/JSON/TOML, or…
    // .store(my_own_store)                      //   any host `impl Store`
    .listen("0.0.0.0:443")
    .derp(DerpConfig::Default)          // or ::Embedded / ::Urls(...)
    .build().await?;
server.serve().await?;                   // runs the HTTP/Noise listener

// Tenancy
let net: Network = server.create_network("customer-42").await?;

// Onboarding (the "secondary channel" hands these strings to a sensor)
let sensor_key = net.create_preauth_key(PreAuthKeyOpts {
    reusable: false, ephemeral: false, expiry: Some(Duration::days(7)),
    tags: vec![],                        // or vec!["tag:sensor"]
}).await?;
println!("{}", sensor_key.key());        // "hskey-auth-<prefix>-<secret>"

// Introspection / control
let nodes: Vec<NodeHandle> = net.list_nodes().await?;
net.delete_node(node_id).await?;
net.set_policy(policy_text).await?;      // hub-and-spoke ACL by default

// Events (so the host app reacts to sensors coming/going)
let mut events = server.subscribe();     // broadcast stream
while let Some(ev) = events.next().await {
    match ev { Event::NodeOnline{network, node}, Event::NodeRegistered{..}, .. }
}
```

The key string format (`hskey-auth-<12 hex>-<64 hex>`, only the secret
bcrypt-hashed) is preserved so the onboarding token is identical to
Headscale's and the existing client tooling works unchanged.

---

## 6. What is deliberately omitted in phase 1

| Omitted | Replacement / rationale |
|---------|-------------------------|
| gRPC + REST API, swagger | Native Rust `control-api` |
| Web admin UI | Host application owns the UI |
| OIDC, interactive browser registration | Pre-auth keys only (matches scenario) |
| SSH ACLs (`HoldAndDelegate`) | Not needed for sensor↔service traffic |
| Taildrop, logtail, autoupdate | Disabled via node-attrs/Debug flags |
| Full HuJSON ACL grammar | Tags + grants subset (enough for hub-and-spoke); grammar grows later |
| Multi-tailnet partitioning | `network_id` seam present; partitioning deferred to multi-network phase |
| SQL persistence backend (SQLite/Postgres) | `Store` trait shipped with `MemoryStore` + `FileStore` (JSON/YAML/TOML); a `SqlStore` is an additive trait impl, deferred |
| Embedded DERP relay | Point at public/standalone DERP first; embed later if needed |

These are *additive* later — the whole point of building a general core. The
architecture keeps the gRPC boundary clean (it never touched the protocol
path in Go), the policy engine extensible, and the tenancy seam in place, so
OIDC, a management API, the full ACL grammar, and multi-tailnet isolation can
each be added on top of the same core without disturbing the data plane.

---

## 7. Mapping table (Go → Rust), at a glance

| Go construct | Rust target |
|--------------|-------------|
| `hscontrol/state/State` | `control_core::state::State` |
| `atomic.Pointer[Snapshot]` | `ArcSwap<Snapshot>` |
| `types.NodeView` | `Arc<Node>` |
| `chan *tailcfg.MapResponse` | `tokio::sync::mpsc<MapResponse>` |
| writer goroutine | single Tokio writer task draining an `mpsc` |
| `mapper.Batcher` + workers | `Batcher` + worker tasks + `DashMap` registry |
| `controlbase.Conn` | `noise::Conn` (snow + framing) |
| `key.MachinePrivate` etc. | `tskey::MachinePrivate` etc. |
| `tailcfg.*` | `tailcfg::*` (serde structs) |
| GORM models + DB access | `Store` trait (port) + serde-able domain structs |
| GORM/SQLite/Postgres backend | `MemoryStore` / `FileStore` now; `SqlStore` (`sqlx`) deferred — same trait |
| `gormigrate` migrations | versioned snapshot envelope (`StoreDocument.version`); `sqlx` migrations only when `SqlStore` lands |
| `IPAllocator` | `ipalloc::IpAllocator` |
| `policy/v2` | `policy` (minimal compiler) |
| `change.Change` | `change::Change` |
