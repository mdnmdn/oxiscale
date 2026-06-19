# 06 — End-to-End Integration Tests

Part of the `_docs/` set. This document defines the **multi-actor,
behaviour-level** test suite that complements the per-phase **verification
gates** in [03-migration-plan.md](03-migration-plan.md) and the unit /
golden-fixture tests described there.

Where the phase gates state the *minimum bar* ("a real client completes the
handshake"), the e2e tests are the *richer behavioural proof*: spawn a server
plus **several clients**, drive a realistic scenario, and assert observable
outcomes — connectivity, **discoverability (positive and negative)**, data
transfer, and update propagation. These are the highest-value regression
tests; they catch integration bugs that unit and serialization tests miss.

---

## 1. Principles

- **One headline e2e test per capability, added the moment the phase makes it
  possible.** Each phase contributes the e2e scenario it newly enables.
- **Progressive and self-pruning.** Later tests *supersede* the scaffolding of
  earlier ones. When a richer scenario fully covers an earlier behaviour, the
  obsolete e2e test is **deleted**, not kept — CI runs only the relevant,
  most-comprehensive set. The supersession ledger (§4) is authoritative; there
  must be **no dead or duplicated e2e tests** in the tree.
- **Assert behaviour, not internals.** Drive through the real wire protocol
  and the public embedding API; assert on what a client observes (its netmap,
  peer set, reachability) — not on private server state.
- **Both judges.** Use scriptable in-process clients for fast, precise
  assertions and real `tailscaled` for the ultimate data-path verdict (§2).

---

## 2. Test harness & actors

**Server under test**
- In-process `ControlServer` (via `control-api`) with a `MemoryStore` for
  speed; a `FileStore` variant for the persistence/restart tests. The
  `tailcontrold` binary is exercised by at least one smoke test.

**Clients** — two kinds, used together:
- **Scriptable** — `tailscale-rs` driven in-process. Fast and programmable;
  assert directly on the received `MapResponse` (own node, peer set, packet
  filters) and on incremental updates. The primary tool for discoverability
  and propagation assertions, and the bulk of CI.
- **Real** — `tailscaled` containers (≥ v1.80), the ultimate judge for the
  actual WireGuard data path (ping, TCP). Heavier; used where real packet
  flow or `tailscale status` output is the thing under test. Modelled on
  Headscale's `_refs/headscale/integration/` Docker harness.

**Supporting**
- **DERP:** a standalone/public DERP or a test relay, so NAT'd data-path
  tests can establish tunnels. Control-plane-only tests need no relay.
- **Onboarding:** pre-auth keys minted through the API / `Store`.

**Where they live & how to run**
- A dedicated `e2e/` integration crate (or workspace `tests/e2e/`), behind a
  `just test-e2e` recipe.
- **Two tiers:** *light* (scriptable clients, no Docker) run on every push;
  *heavy* (real `tailscaled` + DERP) run nightly / pre-release. Mark heavy
  tests with an `ignore`/feature gate so the default `just test` stays fast.

---

## 3. The progressive suite (by phase)

Each test lists the **phase** that unlocks it, the **actors**, what it
**asserts**, and its **supersession** status.

### E2E-1 · Handshake smoke — *phase 1* · *scaffold*

- **Actors:** 1 scriptable client + server.
- **Asserts:** `/key` returns the server machine key; `/ts2021` upgrade +
  Noise IK handshake + EarlyNoise complete; the client reaches
  `POST /machine/register` (a 5xx body is fine — only the handshake + HTTP/2
  framing must succeed).
- **Supersession:** scaffolding. **Deleted once E2E-3 is green** (E2E-3 drives
  the same handshake as a prerequisite to receiving a map).

### E2E-2 · Registration & persistence — *phase 2*

- **Actors:** 2 clients, distinct pre-auth keys, + server.
- **Asserts:** both nodes appear in the `Store` with correct machine/node keys
  and allocated IPs; each client gets `RegisterResponse{MachineAuthorized:true}`.
  **Restart** the server (FileStore) and assert both nodes survive the reload.
- **Negative:** a single-use key, once consumed, is rejected on reuse; an
  expired key is rejected.
- **Matrix:** runs against **both** `MemoryStore` and `FileStore`.

### E2E-3 · Connect, receive map, stay alive — *phase 3*

- **Actors:** 1 client + server.
- **Asserts:** the client completes register → `/machine/map`, receives a
  valid **initial netmap** (own node, assigned IP, correct DERP region), and
  the long-poll **stays open across ≥ 2 keepalives**. Compare the initial
  `MapResponse` byte-for-byte (modulo timestamps) against the Go server's for
  an equivalent node.
- **Supersession:** absorbs **E2E-1** (handshake) — delete E2E-1 here. Retained
  thereafter as the **single-client wire-fidelity** test (distinct from the
  multi-client E2E-4).

### E2E-4 · Three clients: connect + discover + data — *phase 4* · **headline**

The canonical scenario: a server and **three clients** connect, discover each
other per policy, and exchange data.

- **Actors:** 1 central (`tag:central`) + 2 sensors (`tag:sensor`) + server,
  under the hub-and-spoke ACL.
- **Connect:** all three register and receive their initial maps.
- **Discoverability (positive *and* negative):**
  - central's peer set = `{sensor-A, sensor-B}`;
  - each sensor's peer set = `{central}` **only** — sensor-A does **not** see
    sensor-B, and vice versa.
  - Asserted on the received netmaps (scriptable) **and** `tailscale status`
    (real client).
- **Data path:** central ↔ sensor ping / small payload **succeeds** (real
  `tailscaled`, direct or via DERP); sensor ↔ sensor is **blocked** (no route /
  filtered out).
- **Propagation:** bring a **third sensor** online → central receives a
  `PeersChanged` within one batch tick; the existing sensors receive nothing.
  Take a sensor offline → central receives the `Online:false` patch within the
  offline-grace window.

### E2E-5 · Discoverability matrix — *phase 4 (ACL focus)*

Generalises E2E-4's visibility check into a parametrised policy test.

- **Actors:** several clients with assorted tags + server.
- **Asserts:** for **every ordered pair** of clients, observed visibility
  equals the policy's expectation — a visible peer is also **pingable**, an
  invisible peer is **absent from the netmap and unreachable**. The matrix is
  the cross-product, so both the positive and negative cells are checked.
- **Live re-evaluation:** call `set_policy` to add/revoke a grant and assert
  the matrix updates *without reconnect* — a revoked grant emits `PeersRemoved`
  and drops reachability; a new grant emits `PeersChanged` and restores it.
- **Supersession:** takes ownership of the **discoverability assertions**;
  E2E-4 keeps only the **data-path + propagation** assertions to avoid
  overlap.

### E2E-6 · Embedding-API lifecycle + events — *phase 5*

- **Actors:** host code driving `control-api` + real/scriptable clients.
- **Asserts:** create a network, mint sensor + central keys, register a client
  each, `list_nodes`, then `delete_node` and assert the client drops. The
  `subscribe()` event stream observes `NodeRegistered` / `NodeOnline` /
  `NodeOffline` in order.
- **Optional (multi-network):** two networks, one client each — assert each
  client's map contains **only** its own network's peers, and that the two IP
  pools may **overlap** without conflict.

### E2E-7 · Resilience & soak — *phase 6*

- **Actors:** many clients + server, adversarial timing.
- **Asserts:** reconnection storms and overlapping sessions
  (`ActiveSessions`/epoch) do **not** flap peers beyond the grace window or
  delete nodes prematurely; ephemeral-node GC fires; NodeKey rotation
  mid-session is absorbed; expiry logs the node out. **Load:** N sensors —
  watch peer-map (O(n²)) cost and batch latency. **Soak:** across the
  supported Tailscale client-version (capver) window.

---

## 4. Supersession ledger

Keep this current — it is the contract that the e2e tree carries no dead or
duplicated tests.

| Test | Phase | Status | Superseded by / note |
|------|-------|--------|----------------------|
| E2E-1 Handshake smoke | 1 | **scaffold** | **Delete** when E2E-3 lands (handshake is a prerequisite there) |
| E2E-2 Registration & persistence | 2 | keep | — |
| E2E-3 Single-client map | 3 | keep | absorbs E2E-1; stays as wire-fidelity test |
| E2E-4 Three-client connect+data | 4 | keep | discoverability asserts move to E2E-5; keeps data-path + propagation |
| E2E-5 Discoverability matrix | 4 | keep | owns positive/negative visibility |
| E2E-6 API lifecycle + events | 5 | keep | — |
| E2E-7 Resilience & soak | 6 | keep | — |

**Rule:** when a phase makes a test in this table redundant, delete the file in
the same change and update this ledger. A "scaffold" test must not outlive the
test that supersedes it.

---

## 5. Mapping to the phase gates

The phase **gate** ([03](03-migration-plan.md)) is the go/no-go bar; the e2e
test is its automated, multi-actor expression. One gate may be proven by one
e2e test plus the golden-fixture / cross-serialization checks.

| Phase | Gate (03) | E2E test |
|-------|-----------|----------|
| 1 | Real `tailscaled` completes the handshake | E2E-1 |
| 2 | Client registers; node persisted & survives reload | E2E-2 |
| 3 | Client connected, IP assigned, long-poll alive | E2E-3 |
| 4 | Hub-and-spoke visibility + propagation | E2E-4, E2E-5 |
| 5 | Full lifecycle via the embedding API | E2E-6 |
| 6 | Stable under reconnect/soak | E2E-7 |

---

## 6. Keeping this document current

When a phase lands, add its e2e test here, delete any scaffold it supersedes
(updating §4), and keep the §5 mapping in sync with the gates in
[03-migration-plan.md](03-migration-plan.md). New e2e *recipes* go in the
`justfile`.
