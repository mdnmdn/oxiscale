# 04 — Critical Points

The load-bearing risks of this migration. If you read one document before
deciding whether to proceed, read this one. Each point states the risk, why
it is hard, and how to mitigate it.

Ordered by severity.

---

## 1. The Noise/TS2021 handshake — still the hardest part 🟠

> **Severity downgraded from 🔴 to 🟠** thanks to `tailscale-rs`: an
> authoritative Rust implementation of the Noise/TS2021 protocol now exists
> (BSD-3). It is *client*-side, so we still build the **responder**, but we
> no longer work only from the Go source.

**Risk:** the effort is gated on faithfully implementing Tailscale's TS2021
control protocol — a **Noise IK** handshake wrapped in a custom HTTP
upgrade, with an "EarlyNoise" side-channel, followed by HTTP/2 multiplexed
*inside* the encrypted stream. Headscale delegates this to
`tailscale.com/control/controlbase` + `control/controlhttp`. We must produce
the server/responder equivalent in Rust.

**Why it's still hard (even with a reference):**
- `tailscale-rs`'s `ts_noise`/`ts_control_noise` implement the **initiator**
  (client) role. The server needs the **responder** role — we mirror their
  logic, we don't drop it in. The Noise params, prologue, and the magic-byte
  EarlyNoise frame (`\xff\xff\xffTS` + 4-byte BE length + JSON) become
  *known* (read them from the Rust reference) rather than guessed, but the
  responder state machine and HTTP-upgrade handling are ours to write.
- After the handshake you must run an HTTP/2 server over a raw, non-TLS
  encrypted `net.Conn`. The `h2` crate expects an `AsyncRead+AsyncWrite`;
  bridging it over the Noise `Conn` is fiddly.
- The reusable crates are **experimental and unaudited** (see §10) — a
  reference and likely a dependency, but not a turnkey solution.

**Mitigation:**
- **Prove it in phase 1, before anything else.** Gate all later work on a
  real `tailscaled` completing the handshake against the Rust server; use
  `tailscale-rs` as a second scriptable client and as the byte-level oracle.
- Capture a real handshake from the Go server (you control the Noise key,
  so you can decrypt) and replay/diff.
- Fallback if the responder proves intractable: keep the Noise transport in
  Go behind a thin FFI/IPC boundary and port only the control logic. Hedges
  the largest risk at the cost of a hybrid binary.

---

## 2. Tenancy: ACL now, network isolation later 🟡

> **Severity downgraded from 🔴 to 🟡** by the decision to start with a
> single tailnet + hub-and-spoke ACL, keeping multi-network as a designed-for
> extension rather than a phase-1 requirement.

**Decided model.** The scenario is hub-and-spoke (central reaches all
sensors; sensors need not see each other), so **phase 1 uses one tailnet
with a tag ACL** (`tag:sensor ↔ tag:central`, no `tag:sensor ↔ tag:sensor`).
Sensors being mutually invisible is a convenience here, not a cross-tenant
security boundary — acceptable when one operator runs one central service
over its own sensors.

**The remaining risk** is the *several independent central servers* case. If
multiple independent operators ever share one deployment in a single
tailnet, ACL-only isolation becomes load-bearing security: one policy bug
leaks one operator's sensors into another's, and the shared IP space invites
collisions. That is when isolation must become **structural**, not
policy-enforced.

**Mitigation (cheap insurance):**
- Carry a `network_id` column on `users`/`nodes`/`pre_auth_keys` **from
  phase 2**, with a single default network in the hub-and-spoke case.
  Retrofitting this later is a painful migration; adding one unused column
  now is free.
- When independent central servers are actually required, promote the seam:
  partition the NodeStore by network (the O(n²) peer computation never
  crosses a boundary), per-network IP pools/policy/`ServerURL`. This is an
  extension of the same core, not a rewrite — *provided* the seam exists.

**Open questions (only matter if/when multi-network is built):**
- How many independent central servers, and how many sensors each?
- Can a "central service" be a member of *multiple* networks at once (the
  "connect to all sensors together" case across networks), or is it
  one-service-per-network?

**Resolved — routing strategy:** the server supports three simultaneous
strategies (see [02 §4](02-target-architecture.md)):
- **Subdomain** (`Host` header) — compatible with all clients.
- **Path prefix** — requires a 2-line patch to `tailscale-rs`
  (`Url::join("ts2021")` instead of `Url::join("/ts2021")`); stock Go
  clients fall through to root.
- **Optional mTLS** — client cert CN/SAN routed; stock Go clients skip mTLS
  and fall through to subdomain or root.
All three coexist on the same listener; first match wins. This is
sufficiently flexible that it is not a risk — it is a standard axum/rustls
pattern.

---

## 3. `tailcfg` wire fidelity — large surface, now with a Rust source 🟡

> **Severity downgraded from 🟠 to 🟡** if we reuse `tailscale-rs`'s
> `ts_control_serde`, which already defines these types in Rust with the
> correct serde shapes. The risk drops from "hand-port and byte-verify
> dozens of structs" to "track an upstream crate".

**Risk:** `tailscale.com/tailcfg` is the entire control protocol schema —
`MapRequest`, `MapResponse`, `Node`, `FilterRule`, `Hostinfo`, `DERPMap`,
`DNSConfig`, `PeerChange`, and dozens more. Every field name, JSON tag,
omitempty, and type must match what the client (de)serializes. It is large
and evolves with each Tailscale release.

**Why it's still a watch item:**
- If we *hand-port* instead of reuse, Go's `encoding/json` /
  `go-json-experiment/json` subtleties (zero-value omission, `null` vs
  absent, `opt.Bool` encodings) must be reproduced exactly in serde.
- Either way the schema is a moving target tracked via `CapabilityVersion`;
  the Rust port inherits the obligation to follow the client versions it
  supports.

**Mitigation:**
- **Prefer reuse of `ts_control_serde`.** It is maintained by Tailscale
  against the real client.
- If hand-porting: **cross-serialization fixtures** (plan §Verification) — a
  Go helper emits golden JSON; Rust tests assert byte-identical output.
- Pin to a **capver window** and test against that matrix; isolate the wire
  types in their own crate and keep them mechanical.

---

## 4. Curve25519 key encoding 🟡

> **Severity downgraded from 🟠 to 🟡** if we reuse `tailscale-rs`'s
> `ts_keys`, which already implements these key types and codecs in Rust.

**Risk:** keys are not just 32 bytes — Tailscale wraps them in a typed text
encoding (`mkey:`, `nodekey:`, `discokey:` + hex) and uses them as map keys,
DB columns, and wire fields. The crypto is standard X25519; the **encoding**
is Tailscale-specific.

**Mitigation:** prefer `ts_keys`. If hand-porting `types/key` into the
`tskey` crate, build the codecs first and round-trip them against fixtures
captured from Go's `key.*Public.String()` / parse paths. Cheap to get
exactly right early; expensive to discover wrong late.

---

## 5. The copy-on-write concurrency model 🟠

**Risk:** the NodeStore (`atomic.Pointer[Snapshot]` + single writer
goroutine) and the mapper's batched fan-out are performance-critical and
concurrency-subtle. Naively translating goroutines/channels to Tokio can
introduce deadlocks, lost updates, or unbounded memory.

**Why it's hard:**
- The snapshot rebuild runs an **O(n²)** peer-map computation; it is the
  main scaling limit. Per-network partitioning (point 2) bounds it, but it
  remains the hot path.
- The batcher's `in_flight` guard, per-node ordering, the
  `last_sent_peers` diff, and the `pending`-list retry semantics are easy to
  get subtly wrong — producing maps that *look* fine but drop or duplicate
  peer updates.

**Mitigation:**
- Mirror the Go structure exactly: `ArcSwap<Snapshot>`, one writer task
  draining an `mpsc`, `Arc<Node>` as the `NodeView` analogue. Don't
  redesign the concurrency model during the port.
- Build a **property/differential test harness** for the batcher: feed the
  same change sequence to the Go and Rust mappers and assert equivalent
  emitted `MapResponse`s. This subsystem (plan phase 4) is the most
  bug-prone after the handshake.

---

## 6. Session lifecycle edge cases 🟡

**Risk:** Headscale handles reconnection storms, overlapping sessions
(`ActiveSessions` count + `SessionEpoch`), a 10-second grace before marking
a node offline, ephemeral-node GC, and NodeKey rotation mid-session. These
are easy to omit and cause flapping peers or premature node deletion.

**Mitigation:** port the `Connect`/`Disconnect` epoch logic faithfully
(phase 6); test with deliberate reconnect/restart loops, not just
happy-path connects.

---

## 7. DERP / NAT traversal 🟡

**Risk:** without a reachable DERP relay, clients behind NAT may fail to
establish tunnels even when the control plane is perfect — and it will look
like a control-plane bug.

**Mitigation:** decouple early. Distribute a DERP map pointing at public
Tailscale DERPs or a standalone `derper` (or the existing Go relay as a
separate process). Only embed a relay if the deployment truly needs it
(point sensors at a self-hosted relay for air-gapped networks). Do not let
DERP block control-plane progress.

---

## 8. Persistence abstraction & state durability 🟡

> **Reframed:** persistence is a **`Store` trait** the host injects, not a
> hardwired database (see [02 §3.5](02-target-architecture.md)). Phase 1 ships
> `MemoryStore` + `FileStore` (JSON/YAML/TOML snapshot); SQL is a deferred,
> additive trait impl. This removes the SQL/migration machinery from the
> phase-1 critical path but introduces two new watch items.

**Risk A — trait boundary correctness.** Every backend must behave
identically, or a deployment that swaps `MemoryStore` for `FileStore` (or a
host's own impl) gets subtly different semantics. The sharp edge is
**atomic single-use pre-auth-key consumption** (`Store::consume_preauth_key`):
SQL does it with `UPDATE … WHERE used=false`, but `MemoryStore`/`FileStore`
must reproduce the same race-free single-winner guarantee with a mutex.

**Risk B — `FileStore` durability.** A snapshot-to-file backend can corrupt
or lose state on a crash mid-write, or thrash under update bursts.

**Risk C — versioning replaces migrations.** With no SQL schema there are no
`sqlx` migrations for the default backends; the snapshot `version` field is
the *only* forward-compat mechanism, so it must be honoured from day one.

**Mitigation:**
- Write a **backend conformance test suite** once and run every `Store` impl
  through it — especially a concurrent `consume_preauth_key` race test
  asserting exactly one winner.
- `FileStore`: **write-temp-then-atomic-rename**, debounce flushes, and treat
  a partial/corrupt file as a recoverable load error. The NodeStore (not the
  file) serves reads, so a short flush debounce is safe.
- Stamp `StoreDocument.version` from the first release and upconvert on load;
  this is the file/memory analogue of append-only migrations.
- **Greenfield by default.** No import of existing Headscale databases is in
  scope; design the domain model around the `Network` entity from day one.
  If/when the deferred `SqlStore` lands, the Headscale append-only migration
  discipline (`AGENTS.md` §Database Migration Rules: never reorder, never
  disable FKs, `YYYYMMDDHHMM-slug` IDs) applies to *its* migrations — but it
  is no longer on the phase-1 path.

---

## 9. Maintenance treadmill (strategic, not technical) 🟡

**Risk:** the Tailscale protocol is defined by an upstream you don't
control and which changes every release. Headscale absorbs this churn
through the `capver` system and a community. A private Rust fork inherits
that maintenance burden **alone**, forever.

**Mitigation:** scope ruthlessly (a fixed capver window, sensor-only
feature set), isolate the protocol crates so upstream changes touch a small
surface, and budget ongoing capacity to track client releases — not just a
one-time port. Re-evaluate periodically whether a hybrid (Go transport via
FFI) lowers total cost.

---

## 10. Depending on `tailscale-rs` is itself a risk 🟡

**Risk:** the reuse that de-risks points 1, 3, and 4 comes with strings:
`tailscale-rs` is **experimental and explicitly unaudited** — it ships with
warnings ("unstable and insecure") and requires
`TS_RS_EXPERIMENT=this_is_unstable_software` to even link. It is
**client-perspective** (we adapt it to the server role), its internal crates
(`ts_noise`, `ts_control_serde`, `ts_keys`) are **not necessarily published
on crates.io** (reuse means a pinned git dependency or vendoring), and it is
a **moving target** under active development.

**Why it matters:**
- Building a *server* on an *unaudited client* library means inheriting its
  cryptographic uncertainty on the security-critical handshake path.
- A pinned git/vendored dependency on a fast-moving upstream needs a
  deliberate update cadence, or it rots.

**Mitigation:**
- Treat reuse as **"reference-first, dependency-second"**: even where we
  depend on a crate, keep the boundary thin and the types mirrored, so we
  can swap to a hand-port if upstream diverges or an audit demands it.
- Pin to a known-good revision; vendor the handful of crates actually used
  rather than tracking `main`.
- Re-evaluate at each Tailscale/`tailscale-rs` release whether to bump.
- This does **not** erase point 1's fallback (Go transport via FFI) — keep
  it on the table until the responder is proven and, ideally, audited.

---

## Summary: gates before committing

1. **Can a real `tailscaled` complete the Noise handshake against a Rust
   responder prototype?** (Points 1, 10.) Everything depends on this — prove
   it in a spike, reusing/mirroring `tailscale-rs`, before funding the full
   port.
2. **Is the `tailscale-rs` build-vs-reuse call made per crate?** (Point 10.)
   Reuse `ts_control_serde`/`ts_keys` if viable; pin or vendor; keep the
   FFI fallback for the handshake on the table.
3. **Tenancy is decided** (Point 2): single tailnet + hub-and-spoke ACL now,
   `network_id` seam from phase 2 to keep independent-networks open. No
   blocker.
4. **Is there appetite for the ongoing protocol-tracking burden?**
   (Point 9.) A general-purpose server amplifies this.

Gate 1 is the one that can sink the project; the rest are manageable. Run
the handshake spike first and decide nothing else until it resolves.
