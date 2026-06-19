# Project Structure — Rust Workspace

Part of the `_docs/` specification set for porting Headscale's
**client-facing control plane** from Go to Rust. This document describes the
**effective current** and **target** layout of the Oxiscale Rust
implementation — the living map of the codebase. Update it when crates,
modules, or implementation status change.

For project scope and deployment scenario, see
[02-target-architecture.md](02-target-architecture.md) §0. For design
rationale and embedding API details, see the rest of
[02-target-architecture.md](02-target-architecture.md). For build order and
verification gates, see [03-migration-plan.md](03-migration-plan.md).
Documentation navigation and agent rules live in
[AGENTS.md](../AGENTS.md).

---

## 1. Repository layout

```
oxiscale/                          # Cargo workspace root
├── README.md                      # project intro + Headscale attribution
├── LICENSE                        # MIT (Oxiscale's own code)
├── NOTICE                         # BSD-3 attributions (Headscale, tailscale-rs)
├── AGENTS.md                      # agent + contributor behavioural rules
├── justfile                       # common dev commands (`just --list`)
├── Cargo.toml                     # workspace manifest (license = MIT)
├── rust-toolchain.toml            # pinned Rust (1.96.0)
├── crates/                        # library crates (bottom-up dependency order)
│   ├── tailcfg/
│   ├── tskey/
│   ├── noise/
│   ├── control-core/
│   └── control-api/
├── bin/
│   └── tailcontrold/              # thin standalone binary (testing / dev)
├── _docs/                         # specifications and this file
└── _refs/                         # read-only references — do not edit
    ├── headscale/                 # Go Headscale snapshot (behavioural oracle)
    └── tailscale-rs/              # git submodule (protocol oracle + test client)
```

---

## 2. Crate dependency graph

```
                    ┌──────────────┐
                    │ tailcontrold │  bin/
                    └──────┬───────┘
                           │
                    ┌──────▼──────┐
                    │ control-api │  public embedding API
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │control-core │  state, store, server (axum + h2-over-noise)
                    └──┬───┬───┬──┘
                       │   │   │
              ┌────────┘   │   └────────┐
              │            │            │
       ┌──────▼──────┐ ┌───▼───┐ ┌──────▼──────┐
       │    noise    │─┤ tskey │◄┤   tailcfg   │
       └──────┬──────┘ └───┬───┘ └──────┬──────┘
              │ (also ─────┘            │
              │  ─► tailcfg)            │
              ▼            ▼            ▼
        reused tailscale-rs crates (path deps, wired in phase 1):
          noise   ─► ts_noise, ts_control_noise
          tailcfg ─► ts_control_serde, ts_capabilityversion
          tskey   ─► ts_keys
```

Internal edges: `noise ─► {tailcfg, tskey}`, `tailcfg ─► tskey`. The HTTP edge
in `control-core` additionally pulls `axum`/`hyper`/`hyper-util`.

**Rule:** dependencies flow upward only. Protocol crates (`tailcfg`, `tskey`,
`noise`) must not depend on `control-core` or `control-api`.

---

## 3. Current state (effective)

Legend: **done** = implemented and tested · **partial** = skeleton with some
logic · **stub** = module/file exists, no behaviour yet · **planned** = not
started

### 3.1 `crates/tailcfg` — wire types & protocol constants

| Item | Status | Notes |
|------|--------|-------|
| `capver::MIN_SUPPORTED_CAPABILITY_VERSION` | **done** | constant = 113 (`u16`); `CapabilityVersion` reused from `ts_capabilityversion`; **enforced in `noise::accept`** (clients below it are rejected) |
| `protocol` (paths, EarlyNoise magic, keepalive) | **done** | fixed: magic is 5 bytes `\xff\xff\xffTS`, upgrade token `tailscale-control-protocol` |
| `OverTLSPublicKeyResponse`, `EarlyNoise` | **done** | hand-written (server-emitted; not in `ts_control_serde`); PascalCase casing **confirmed against real `tailscaled`** at the gate |
| `MapRequest`, `MapResponse`, `RegisterRequest`, … | **done** | re-exported from `ts_control_serde` |
| Reuse spike vs hand-port | **done** | phase-1 step 0 — **reuse** confirmed (all 5 `ts_*` crates build under 1.96.0) |

### 3.2 `crates/tskey` — key types & text codecs

| Item | Status | Notes |
|------|--------|-------|
| `MachinePublicKey/PrivateKey`, `NodePublicKey`, `DiscoPublicKey`, `ChallengePublicKey`, codecs | **done** | re-exported from `ts_keys` (reuse); round-trip + casing tests. Errors use `ts_keys::ParseError` (the dead `KeyError` stub was removed) |

### 3.3 `crates/noise` — TS2021 responder handshake

| Item | Status | Notes |
|------|--------|-------|
| `NoiseError` | **done** | base64 / bad-initiation / handshake-failed / unsupported-version / serde / io |
| `accept()` responder (Noise IK msg1→msg2) | **done** | built on reused `ts_noise::ik::ReceivedHandshake`; in-process initiator (`ts_control_noise::Handshake`) completes + exchanges encrypted data |
| EarlyNoise framing | **done** | `write_early_noise()` — 5-byte magic + u32 BE len + JSON; verified on wire |
| Encrypted `AsyncRead+AsyncWrite` (`NoiseStream`) | **done** | `FramedIo` over `Framed<T, BiCodec>` (mirrors unexported `ts_control_noise` helper) |
| `/key` + `/ts2021` HTTP handlers | **done** | `control-core::server` (axum upgrade + hijack); proven in-process |
| HTTP/2-over-Noise per-connection server | **done** | hyper `http2::serve_connection` over `NoiseStream`; **real `tailscaled` (capver 138) completes the handshake + `POST /machine/register` over the tunnel** — phase-1 gate green. `/machine/*` bodies are placeholders until phases 2–3 |

### 3.4 `crates/control-core` — control plane

| Module | Status | Notes |
|--------|--------|-------|
| `store::traits::Store` | **done** | async trait, full CRUD surface |
| `store::MemoryStore` | **partial** | working in-memory impl; `consume_preauth_key` honours reusable + `expires_at` (tested); broader conformance suite still phase 2 |
| `store::FileStore` | **done** | async `open` loads snapshot; `flush` writes atomically (temp+rename); JSON/YAML/TOML; persist+reload tested |
| `store::types` | **partial** | domain structs with `network_id` seam |
| `store::document::StoreDocument` | **done** | versioned snapshot envelope |
| `state` | **stub** | NodeStore (`ArcSwap`) — phase 3 |
| `mapper` | **stub** | MapResponse build + batcher — phase 3–4 |
| `policy` | **stub** | hub-and-spoke ACL — phase 4 |
| `ipalloc` | **stub** | per-network IP pools — phase 2 |
| `auth` | **stub** | pre-auth-key registration — phase 2 |
| `change` | **stub** | fan-out change types — phase 4 |
| `derp` | **stub** | DERP map distribution — phase 3 |
| `server` | **partial** | axum `GET /key` + `POST /ts2021` upgrade→noise→EarlyNoise→HTTP/2; `/machine/*` placeholder — phase 1 |

### 3.5 `crates/control-api` — embedding API

| Item | Status | Notes |
|------|--------|-------|
| `ControlServerBuilder` | **partial** | `store`, `listen`, `tenant_routing`, `server_key`; builds + serves |
| `ControlServer` | **partial** | holds store + Noise key + `tenant_routing`; `serve()` threads the store into `ServerState` (read by phase-2 register/auth) |
| `TenantRouting` | **done** | subdomain / path / mTLS flags |
| `Event` | **stub** | enum defined; no broadcast stream yet |
| `Network`, `PreAuthKey`, `NodeHandle` | **planned** | phase 5 |

### 3.6 `bin/tailcontrold`

| Item | Status | Notes |
|------|--------|-------|
| CLI + listener | **done** | `#[tokio::main]`, `OXISCALE_LISTEN`, serves `GET /key` + `/ts2021`; verified manually |

### 3.7 Tests

Multi-actor end-to-end scenarios (the progressive suite + supersession ledger)
are specified in [06-e2e-test.md](06-e2e-test.md).

| Area | Status |
|------|--------|
| Unit tests per crate | **in progress** — 24 passing (tskey codecs, tailcfg JSON, noise handshake + capver reject, store preauth + FileStore persist) |
| `Store` conformance suite | **planned** — phase 2 (preauth + FileStore persist/reload covered now) |
| Wire golden fixtures | **planned** — phase 1+ |
| In-process server smoke (`control-core/tests/server_smoke.rs`) | **done** — `/key` + `/ts2021` + handshake + EarlyNoise via hyper client + reused initiator |
| E2E suite (`e2e/`, see [06](06-e2e-test.md)) | **planned** — one scenario per phase (manual one-off done for E2E-1) |
| `tailscaled` integration (heavy e2e tier) | **done (manual)** — phase-1 gate green; not yet scripted/CI |

---

## 4. Target architecture (by migration phase)

Aligned with [03-migration-plan.md](03-migration-plan.md).

### Phase 1 — Protocol + Noise handshake ✅ **complete**

```
tailcfg  ──▶ wire types reused from ts_control_serde (+ hand-written /key, EarlyNoise)
tskey    ──▶ key codecs reused from ts_keys
noise    ──▶ responder: /ts2021 handshake, EarlyNoise, encrypted stream
server   ──▶ control-core::server: GET /key, POST /ts2021, h2-over-noise
```

**Gate:** ✅ **met** — real `tailscaled` (capver 138) completes the handshake +
issues `POST /machine/register` over HTTP/2-in-Noise (verified via Docker,
2026-06-19).

### Phase 2 — Registration + persistence

```
control-core::store   ──▶ MemoryStore + FileStore complete
control-core::ipalloc ──▶ per-network IP allocation
control-core::auth    ──▶ pre-auth-key path only
```

**Gate:** client registers via auth key; node persisted; survives `FileStore` reload.

### Phase 3 — Data plane streaming

```
control-core::state   ──▶ NodeStore (ArcSwap + writer task)
control-core::mapper  ──▶ initial MapResponse
control-core::server  ──▶ /machine/map long-poll + zstd framing
control-core::derp    ──▶ DERP map injection
control-core::policy  ──▶ default-open filters (hub-and-spoke in phase 4)
```

**Gate:** `tailscale status` shows connected client with assigned IP.

### Phase 4 — Multi-node fan-out

```
control-core::mapper  ──▶ Batcher + NodeConn + worker pool
control-core::change  ──▶ PeersChanged / PeersRemoved / patch forms
control-core::policy  ──▶ tag ACL (sensor ↔ central, not sensor ↔ sensor)
control-core::state   ──▶ peers_fn / BuildPeerMap
```

**Gate:** hub-and-spoke visibility; sensor↔central ping works.

### Phase 5 — Embedding API

```
control-api ──▶ Network, PreAuthKey, NodeHandle, subscribe() events
              ──▶ optional multi-network partition (if required)
```

### Phase 6 — Hardening

```
FileStore durability, metrics, soak tests
optional SqlStore (deferred trait impl — same Store port)
```

---

## 5. `control-core` module map (target)

```
control-core/src/
├── lib.rs
├── auth/           # handle_register (authkey path)
├── change/         # Change fan-out types for mapper
├── derp/           # DERP map load + inject into MapResponse
├── ipalloc/        # IPv4/IPv6 per network
├── mapper/         # MapResponseBuilder, Batcher, NodeConn
├── policy/         # minimal ACL compiler → PacketFilters
├── server/         # axum: GET /key, POST /ts2021, POST /machine/*
├── state/          # State coordinator, NodeStore (arc-swap)
└── store/
    ├── traits.rs   # Store trait (host-injectable port)
    ├── types.rs    # Network, User, Node, PreAuthKey (+ network_id)
    ├── document.rs # StoreDocument { version, … }
    ├── memory.rs   # MemoryStore
    └── file.rs     # FileStore (JSON / YAML / TOML snapshot)
```

**Persistence rule:** no SQL in phase 1. `SqlStore` is a future `impl Store`,
not a separate architectural path.

---

## 6. External references (not part of the workspace)

| Path | Role |
|------|------|
| `_refs/headscale/` | Headscale (Go) submodule — behavioural oracle |
| `_refs/tailscale-rs/` | Rust protocol oracle + scriptable test client (submodule) |

Workspace `Cargo.toml` declares path dependencies to `tailscale-rs` crates and
`exclude`s `_refs/tailscale-rs` (its own nested workspace). The phase-1 spike
decided **reuse**, and the crates are wired in: `tskey ─► ts_keys`,
`tailcfg ─► ts_control_serde`/`ts_capabilityversion`,
`noise ─► ts_noise`/`ts_control_noise`. This requires the toolchain bump to
1.96.0 (their MSRV ≥ 1.92).

---

## 7. Conventions & Rust best practices

- **Edition / toolchain:** Rust 2021, MSRV 1.96.0 (`rust-toolchain.toml`).
  Bumped from 1.85.0 in the phase-1 spike: reusing the `tailscale-rs` crates
  requires their de-facto MSRV of 1.92.0+ (edition 2024); we pin 1.96.0.
- **Crate naming:** directory `control-core`, package `control-core`, Rust
  import `control_core`.
- **Dependency direction:** flows upward only — protocol crates (`tailcfg`,
  `tskey`, `noise`) never depend on `control-core`/`control-api` (see §2).
- **Error types:** per-crate `thiserror` enums; `anyhow` only in binaries.
- **Safety:** `#![forbid(unsafe_code)]` unless there is a documented,
  reviewed reason.
- **API surface:** public API documented; internal modules kept private.
- **Async:** Tokio throughout `control-core`/`control-api`; reach for `async`
  only where I/O or concurrency requires it.
- **Shared state:** prefer `Arc`, explicit ownership, and `Send + Sync`
  bounds.
- **Tests:** colocated `#[cfg(test)]` modules or `tests/` integration dirs per
  crate; run via `just test`.
- **Quality gate:** `just ci` at the end of every task (fmt + clippy + test).

---

## 8. Keeping this document current

When you add, rename, or materially implement a crate or module:

1. Update the **Current state** table in §3.
2. If the target layout changes, update §4–§5 and
   [02-target-architecture.md](02-target-architecture.md).
3. If new dev commands are added, update `justfile` and
   [AGENTS.md](../AGENTS.md) §Commands.