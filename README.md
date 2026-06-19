# Oxiscale

A Rust reimplementation of the Tailscale control server — a Headscale-equivalent
core, embeddable as a library. The first deployment target is a hub-and-spoke
sensor network (remote sensors + a central service, onboarding via pre-auth keys
delivered out-of-band), but the core stays multipurpose so it can grow into a
full [Headscale](https://github.com/juanfont/headscale) alternative (OIDC,
gRPC/REST API, full ACLs, multiple tailnets).

## Status

Phase 1 (protocol foundations + TS2021 Noise handshake) is complete: a real
`tailscaled` completes the handshake and issues `POST /machine/register` over
HTTP/2-inside-Noise against the Rust server. See
[`_docs/project-structure.md`](_docs/project-structure.md) for the live status
map and [`_docs/03-migration-plan.md`](_docs/03-migration-plan.md) for the
phased build plan.

## Layout

- `crates/` — `tailcfg`, `tskey`, `noise`, `control-core`, `control-api`
- `bin/tailcontrold/` — standalone control-server binary
- `_docs/` — specifications (start with `02-target-architecture.md`)
- `_refs/` — read-only reference submodules (`headscale`, `tailscale-rs`)

## Development

```sh
just submodule   # initialise _refs/ submodules (required on a fresh clone)
just ci          # fmt check + clippy (-D warnings) + tests
```

Run `just --list` for all recipes.

## Acknowledgements & license

Oxiscale is **ported from [Headscale](https://github.com/juanfont/headscale)**
(BSD-3-Clause, © Juan Font), which serves as the behavioural oracle, and reuses
protocol/crypto crates from
[tailscale-rs](https://github.com/tailscale/tailscale-rs) (BSD-3-Clause,
© Tailscale Inc & contributors).

Oxiscale's own code is licensed under the **MIT License** — see
[`LICENSE`](LICENSE). Third-party attributions are in [`NOTICE`](NOTICE).
