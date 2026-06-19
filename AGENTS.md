# AGENTS.md

Behavioural guidance for AI agents working on **Oxiscale** — a Rust
reimplementation of the Tailscale control server, ported from Headscale.

This repository is a **migration workspace**, not a fork of Headscale. New
Rust code lives in the Cargo workspace at the repository root; the Go
Headscale codebase and Tailscale's official Rust client live under `_refs/`
as read-only references.

**What it is:** a general-purpose Tailscale control server, embeddable as a
library — a Headscale-equivalent core. The first deployment target is a
**hub-and-spoke sensor network** (remote sensors + a central service,
onboarding via pre-auth keys delivered out-of-band), but the core stays
multipurpose so it can grow into a full Headscale alternative (OIDC,
gRPC/REST API, full ACLs, multiple tailnets). The build is **phased and
de-risking-first**, with a verification gate per phase. For scope, phasing,
persistence/tenancy design, and what is deliberately deferred, see the
`_docs/` set below — start with [`02`](_docs/02-target-architecture.md)
(architecture) and [`03`](_docs/03-migration-plan.md) (migration plan).

## Repository layout

```
oxiscale/
├── AGENTS.md              # this file
├── justfile               # dev commands
├── Cargo.toml             # Rust workspace root
├── crates/
│   ├── tailcfg/           # wire types + protocol constants
│   ├── tskey/             # key types / codecs
│   ├── noise/             # TS2021 responder handshake
│   ├── control-core/      # state, store, mapper, policy, server, auth
│   └── control-api/       # public embedding API
├── bin/tailcontrold/      # thin standalone binary
├── _docs/                 # specifications (incl. project-structure.md)
└── _refs/                 # read-only reference material — do not edit
    ├── headscale/         # frozen Headscale (Go) snapshot
    └── tailscale-rs/      # git submodule — Tailscale's official Rust client
```

## Commands

[`justfile`](justfile) wraps the dev workflow — run `just --list` for all
recipes. Key ones:

- **`just check`** — type-check the workspace (fast iteration).
- **`just test`** — run all tests.
- **`just fmt`** — format Rust code.
- **`just clippy`** — lint with warnings denied.
- **`just lint`** — `fmt-check` + `clippy`.
- **`just ci`** — end-of-task quality gate (lint + test).
- **`just run`** — run the `tailcontrold` binary.
- **`just submodule`** — initialise `_refs/tailscale-rs`.

## Documentation (`_docs/`)

Authoritative specs for **what to build, in what order, and why**. Each topic
lives in one document — consult it rather than duplicating its content.

- **[`project-structure.md`](_docs/project-structure.md)** — living map of the
  Rust workspace: current implementation status per crate/module, target
  layout by phase, dependency graph. Read it before touching code; **update it
  organically** when crates or modules change.
- **[`01-headscale-structure.md`](_docs/01-headscale-structure.md)** — how the
  Go (Headscale) codebase is laid out and works.
- **[`02-target-architecture.md`](_docs/02-target-architecture.md)** — scope,
  deployment scenario, Rust crate architecture, design principles,
  persistence/tenancy seams, embedding API.
- **[`03-migration-plan.md`](_docs/03-migration-plan.md)** — phased build plan,
  verification gates, Go→Rust dependency inventory.
- **[`04-critical-points.md`](_docs/04-critical-points.md)** — load-bearing
  risks; read before deciding to proceed.
- **[`05-protocol-reference.md`](_docs/05-protocol-reference.md)** — wire
  protocol: endpoints, Noise handshake, framing, tenancy routing.
- **[`06-e2e-test.md`](_docs/06-e2e-test.md)** — progressive multi-actor
  end-to-end test suite, one scenario per phase, with a supersession ledger.

**Where to look:**

- **Deciding whether / how to proceed** → [`04`](_docs/04-critical-points.md).
- **Implementing** → [`02`](_docs/02-target-architecture.md) →
  [`03`](_docs/03-migration-plan.md), keeping [`04`](_docs/04-critical-points.md)
  and [`05`](_docs/05-protocol-reference.md) open, plus
  [`project-structure.md`](_docs/project-structure.md) for the current map.
- **Wire-protocol / byte-level questions** → [`05`](_docs/05-protocol-reference.md).
- **What exists / what's implemented** → [`project-structure.md`](_docs/project-structure.md).
- **Integration / e2e test scenarios** → [`06`](_docs/06-e2e-test.md).
- **Go behaviour (the oracle)** → `_refs/headscale/` (mapped in
  [`01`](_docs/01-headscale-structure.md); Go→Rust table in
  [`02`](_docs/02-target-architecture.md)).

File:line references in `_docs/` point at Go source under `_refs/headscale/`
(e.g. `hscontrol/noise.go` → `_refs/headscale/hscontrol/noise.go`) — anchors
for the implementer, not permanent coordinates.

## References (`_refs/` — read-only, do not modify)

- **`_refs/headscale/`** — [Headscale](https://github.com/juanfont/headscale)
  (Go, git submodule). The behavioural **oracle**: look up the authoritative
  implementation of any protocol detail, and diff wire traffic by running the
  Go server alongside the Rust one.
- **`_refs/tailscale-rs/`** —
  [Tailscale's official Rust client](https://github.com/tailscale/tailscale-rs)
  (BSD-3, git submodule). Reusable protocol crates: `ts_control_serde`
  (`tailcfg` wire types), `ts_keys` (keys), `ts_noise`/`ts_control_noise`
  (Noise/TS2021, initiator side), `ts_derp`. Use as a **byte-level oracle**
  and **scriptable Rust test client** alongside real `tailscaled` containers.
  Initialise with `just submodule`.

## Core directives

Apply to every task, alongside the interaction rules below.

- **Test every change.** Each feature, fix, or behavioural change ships with
  tests that would catch a regression — colocated `#[cfg(test)]`/`tests/`, or
  integration tests when behaviour crosses crates. No tests, not done.
- **Run the quality gate.** Finish with `just ci` (fmt check, clippy
  `-D warnings`, full test suite); fix every failure (`just fmt` first if
  formatting changed). Each phase also has a verification gate in
  [`03`](_docs/03-migration-plan.md) — honour it.
- **Keep it simple.** No premature abstraction or "just in case" code — three
  similar lines beat a premature trait. Match the current phase; generalise
  only when a second caller exists.
- when executing bash commands don't put echo message after and before, only direct
  commands to better efficiency and reuse.
- **Ask when uncertain.** If requirements, protocol behaviour, or design
  trade-offs are unclear, ask before guessing (comprehensive options — see
  below). Wrong calls on protocol fidelity or persistence are expensive to
  unwind.
- **Use subagents.** Delegate exploration, research, and parallelisable
  subtasks; use cheaper/faster models for bounded work (search, reading refs,
  single-crate tests) and reserve the primary agent for design and integration.
- **Update docs in the same change.** Crate/module status →
  [`project-structure.md`](_docs/project-structure.md); architecture →
  [`02`](_docs/02-target-architecture.md); protocol →
  [`05`](_docs/05-protocol-reference.md); new commands → [`justfile`](justfile)
  + the index table above.
- **Code conventions & Rust best practices** →
  [`project-structure.md`](_docs/project-structure.md) §7.

## Interaction rules

- **Ask with multiple-choice options.** When clarifying intent, present
  comprehensive options plus an "other — please describe" escape.
- **Read the plan first.** The migration plan is de-risking-first with a
  per-phase gate; read [`03`](_docs/03-migration-plan.md) and follow the phase
  order — don't skip ahead.
- **Map once, then act.** Orient before editing — check
  [`project-structure.md`](_docs/project-structure.md) and the doc index above;
  don't re-explore the same area repeatedly.
- **Fail fast, report up.** If a command fails twice with the same error, stop
  and report with context.
- **Confirm scope for multi-file changes.** Before touching more than three
  files, show which files will change and why.
- **Prefer editing existing files.** Don't create new files unless strictly
  necessary.
