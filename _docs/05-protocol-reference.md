# 05 — Protocol Reference

This document describes the **wire protocol** that a Tailscale client speaks
to a control server. It is the byte-level specification for the Rust
implementation — independent of both Go (Headscale) and Rust (`tailscale-rs`)
implementations, though both are authoritative references.

---

## 1. Protocol stack

```
┌──────────────────────────────────────────────────┐
│  Application layer                                │
│  POST /machine/register  POST /machine/map        │
│  (tailcfg JSON over HTTP/2)                       │
├──────────────────────────────────────────────────┤
│  Session layer                                     │
│  Noise IK handshake + encrypted stream            │
│  (TS2021 / controlbase)                            │
├──────────────────────────────────────────────────┤
│  Transport layer                                   │
│  HTTP/1.1 Upgrade (→ Noise) then HTTP/2 inside    │
├──────────────────────────────────────────────────┤
│  Outer security (optional: not the trust anchor)  │
│  TLS 1.3 (rustls / Go crypto/tls)                 │
├──────────────────────────────────────────────────┤
│  Network                                           │
│  TCP                                               │
└──────────────────────────────────────────────────┘
```

TLS is **optional** because the Noise handshake provides mutual
authentication via Curve25519 keys. The client fetches the server's Noise
public key from `/key` over plain HTTP (or HTTPS) before the handshake;
after the handshake, **all** control traffic runs inside the Noise-encrypted
channel. TLS is useful for hiding the `/key` response from passive observers
and for operator compliance, but is never the trust anchor.

---

## 2. Connection lifecycle

Every client-server interaction follows four sequential HTTP exchanges:

```
Client                                Server
  │                                      │
  │ 1. GET /key?v=<capver>               │  discover server Noise pubkey
  │ ◀── {"PublicKey":"mkey:…"} ──────────│
  │                                      │
  │ 2. POST /ts2021 (Upgrade)            │  Noise IK handshake
  │ ◀═══ Noise handshake + EarlyNoise ══▶│
  │                                      │
  │ ══ Noise-encrypted HTTP/2 tunnel ════│
  │                                      │
  │ 3. POST /machine/register            │  register node
  │ ◀── {MachineAuthorized:true} ────────│
  │                                      │
  │ 4. POST /machine/map (Stream=true)   │  open the long-poll
  │ ◀── [len][MapResponse] (initial) ────│
  │ ◀── [keepalive every ~50 s] ────────│
  │ ◀── [len][MapResponse] (on change) ──│
```

Steps 1–2 run over **HTTP/1.1** (possibly over TLS). Steps 3–4 run over
**HTTP/2 multiplexed inside the Noise tunnel**.

---

## 3. Endpoint reference

### 3.1 `GET /key` — Noise public key discovery

**Purpose:** The client fetches the server's long-lived Curve25519 public key
(the Noise static identity) before initiating the handshake.

**Request:**
```
GET /key?v=113 HTTP/1.1
Host: ctl.example.com
```

The `v` query parameter is the client's `CapabilityVersion`. Servers may
reject clients below `MinSupportedCapabilityVersion` (currently **113**,
Tailscale v1.80).

**Response (200 OK):**
```json
{
  "LegacyPublicKey": "mkey:0000...",
  "PublicKey":       "mkey:abcdef0123456789..."
}
```

The response type is `tailcfg.OverTLSPublicKeyResponse`, which has **two**
fields: `PublicKey` (the current Noise static key, `mkey:` + 64 hex) and
`LegacyPublicKey` (a deprecated pre-Noise control key; may be the zero key).
A current client uses `PublicKey`; the legacy field is retained for
compatibility and must still be present.

> **Verify the JSON casing against a Go fixture.** The Go struct has no
> explicit JSON tags, so `encoding/json` emits PascalCase
> (`PublicKey`/`LegacyPublicKey`), whereas `tailscale-rs`'s `ControlPublicKeys`
> deserializes camelCase (`publicKey`/`legacyPublicKey`). Go decoding is
> case-insensitive, so both interoperate, but the **responder must emit the
> exact casing the real server emits** — capture a `/key` response from the
> Go server and match it byte-for-byte before freezing this.

**Routing note:** When using path-based tenancy, this endpoint is also
available at `/:tenant/key` — see §6.

---

### 3.2 `POST /ts2021` — Noise handshake upgrade

**Purpose:** Perform the TS2021 Noise IK handshake and upgrade the connection
to an encrypted tunnel. After this handshake succeeds, all further
communication happens inside the Noise channel.

**Request:**
```
POST /ts2021 HTTP/1.1
Host: ctl.example.com
Connection: Upgrade
Upgrade: tailscale-control-protocol
X-Tailscale-Handshake: <base64(controlbase initiation message)>
```

> **The upgrade token is `tailscale-control-protocol`, not `TS2021`.**
> "TS2021" is the informal name of the protocol and the URL path; it never
> appears as the `Upgrade` value on the wire. The server only requires the
> `Upgrade` header to be **present and non-empty** (Headscale's
> `NoiseUpgradeHandler` checks `req.Header.Get("Upgrade") != ""` and
> delegates the rest to `controlhttpserver.AcceptHTTP`).

The Noise **initiation message (msg1) is carried in the
`X-Tailscale-Handshake` request header, base64-encoded** — not written on
the raw socket. The server completes the upgrade (HTTP `101`), hijacks the
TCP connection, and runs the rest of the Noise IK handshake over the now-raw
socket. (Subtlety: only msg2 onward flows over the hijacked socket; msg1
already arrived in the header.)

**Handshake protocol — Noise IK is a *two-message* pattern (see §4):**

1. **msg1 (client → server)** carries `e, es, s, ss` + payload — the client's
   ephemeral **and** static keys travel together in one message. Delivered in
   the `X-Tailscale-Handshake` header above.
2. **msg2 (server → client)** carries `e, ee, se` + payload, written over the
   hijacked socket. The handshake is complete after this message.
3. The server *optionally* writes an **EarlyNoise** frame into the encrypted
   channel: 5-byte magic `\xff\xff\xffTS` + 4-byte BE length + JSON
   `EarlyNoise{NodeKeyChallenge}` (§4.3). It is optional — the client probes
   for the magic and, if absent, treats the bytes as the HTTP/2 preface.
   Headscale always sends it.
4. The encrypted stream is handed to an **HTTP/2 server**; all subsequent
   `/machine/*` requests run as HTTP/2 inside Noise (wrapped in controlbase
   `Record` frames — §4.4).

> There is no separate third handshake message. A spec or implementation
> that expects `s, es, ss` as a distinct msg3 has mismodelled IK as an
> interactive (XX-style) exchange and will not interoperate.

**Error responses:**

| Condition | Response |
|-----------|----------|
| Missing/empty `Upgrade` header | `500` (Headscale: logs "no upgrade header"; classic misconfigured-proxy symptom) |
| Below min capver | Rejected during the EarlyNoise / accept callback |
| Handshake failure | Connection closed |

**Routing note:** When using path-based tenancy, this endpoint is also
available at `/:tenant/ts2021` — see §6.

---

### 3.3 `POST /machine/register` — Node registration

**Purpose:** Register (or re-authenticate) a node with the control server.
This is the first request sent inside the Noise-encrypted HTTP/2 tunnel.

**Request (JSON, `tailcfg.RegisterRequest`):**
```json
{
  "Version": 113,
  "NodeKey": "nodekey:...",
  "OldNodeKey": "",
  "Auth": {
    "AuthKey": "hskey-auth-abcdef123456-..."
  },
  "Expiry": null,
  "Hostinfo": { ... },
  "Followup": "",
  "Capabilities": [...]
}
```

Key fields:
- `NodeKey` — the node's current WireGuard public key (text-encoded).
- `Auth.AuthKey` — the pre-auth key for first-time registration. Absent for
  subsequent re-authentication (re-auth by `NodeKey`).
- `Hostinfo` — OS, Tailscale version, network interfaces, etc.

**Response (JSON, `tailcfg.RegisterResponse`):**
```json
{
  "MachineAuthorized": true,
  "User": { "LoginName": "sensor-42", ... },
  "NodeKeyExpiry": "2025-01-01T00:00:00Z",
  "Error": ""
}
```

**Registration flow (auth-key path):**

1. Server looks up pre-auth key by its 12-hex prefix.
2. Validates bcrypt hash of the 64-hex secret.
3. Checks expiry and single-use status (atomic consume).
4. Creates or updates the node record with the machine key, node key, and
   allocated IPs.
5. Returns `MachineAuthorized: true`.

---

### 3.4 `POST /machine/map` — Long-poll network map stream

**Purpose:** Open a streaming connection that delivers the current network
topology and pushes incremental updates. This is the **data plane** — the
connection stays open as long as the client is online.

**Request (JSON, `tailcfg.MapRequest`):**
```json
{
  "Version": 113,
  "Compress": "zstd",
  "Stream": true,
  "NodeKey": "nodekey:...",
  "DiscoKey": "discokey:...",
  "Endpoints": ["192.0.2.42:12345"],
  "Hostinfo": { ... },
  "IncludeHealth": false,
  "IncludeRoutes": true
}
```

`Stream: true` is the standard keepalive mode. `Stream: false` is a
poll-once equivalent (not used in normal operation).

**Response stream:**

The server responds with `200 OK` and a stream of frames, each structured
as:

```
┌─────────────────────────────────────────────────┐
│  4 bytes  │  Body (JSON or zstd-compressed JSON) │
│  LE uint32│                                     │
│  length   │  MapResponse                         │
└─────────────────────────────────────────────────┘
```

Framed inside the HTTP/2 data frames (each HTTP/2 DATA frame carries one
or more of these sub-frames).

**Frame types delivered over the stream:**

| Frame type | Trigger | Contents |
|------------|---------|----------|
| **Initial map** | On open | Full `Node` + `Peers` + `DERPMap` + `DNSConfig` + `PacketFilters` |
| **Keepalive** | Every ~50 s | `{ "KeepAlive": true }` |
| **PeersChangedPatch** | Endpoint/DERP/online change | Delta: `[]PeerChange{Endpoint, DERPRegion, Online}` |
| **PeersChanged** | New/updated peer | Full `[]*Node` for added or changed peers |
| **PeersRemoved** | Peer gone | `[]NodeID` for peers no longer visible |

---

## 4. Noise/TS2021 handshake

### 4.1 Protocol parameters

| Parameter | Value |
|-----------|-------|
| Pattern | **IK** (Initiator Knows responder static key) |
| DH function | **Curve25519** |
| Cipher | **ChaCha20-Poly1305** |
| Hash | **BLAKE2s** (256-bit) |
| Protocol name | `Noise_IK_25519_ChaChaPoly_BLAKE2s` |
| Prologue | **non-empty:** the ASCII string `"Tailscale Control Protocol v<capver>"` |
| PSK | None |

The full Noise protocol-name string is
`Noise_IK_25519_ChaChaPoly_BLAKE2s` (verified in
`_refs/tailscale-rs/ts_noise/src/ik.rs`).

> **The prologue is not empty.** Both peers mix the ASCII string
> `"Tailscale Control Protocol v<CapabilityVersion>"` into the handshake
> hash as the Noise prologue (`_refs/tailscale-rs/ts_control/.../connect.rs`
> → `Handshake::initialize(prologue, …)` → `State::new(PROTOCOL).mix_hash(prologue)`).
> A responder that uses an empty prologue derives a different handshake hash
> and the `ss`/payload tags will fail to verify — the handshake aborts. The
> responder must bind the *same* string the client used; because it embeds
> the capability version, it is part of version negotiation, not a constant.

The server's static keypair is the `MachineKey` — the same key served by
`/key` and stored in the server's private key.

### 4.2 Handshake flow (responder perspective)

IK pre-message: the initiator already knows the responder's static key `s`
(fetched from `/key`), so it is mixed in before msg1; there is no message
that carries it.

```
Client (Initiator)                          Server (Responder)
  │                                              │   (knows responder s from /key)
  │  ── e, es, s, ss  (+payload) ───────────────▶│  msg1: client ephemeral
  │      via X-Tailscale-Handshake header        │        AND static, + DHs
  │                                              │  Decrypt s, derive secrets
  │  ◀── e, ee, se  (+payload) ──────────────────│  msg2: server ephemeral + DHs
  │                                              │  Handshake complete (both sides
  │                                              │  hold transport keys)
  │  ◀┄┄ EarlyNoise frame (optional) ┄┄┄┄┄┄┄┄┄┄┄┄│  Encrypted under transport key
```

**Two messages, not three.** In Noise IK the initiator's static key `s` is
sent inside msg1 (encrypted under the `es`-derived key), so there is no
separate msg3. `_refs/tailscale-rs/ts_noise/src/ik.rs` confirms it: the
responder's `ReceivedHandshake::new` opens `e, es, s, ss` from the single
initiation packet, and `finish` emits the `e, ee, se` response.

### 4.3 EarlyNoise frame

Immediately after the handshake completes (msg2 sent, transport keys
derived), the **server** *may* write this frame to the encrypted channel.
The 9-byte header is **5 bytes of magic followed by a 4-byte length** (this
9-byte total is deliberately the same size as an HTTP/2 frame header, so the
client can read 9 bytes and disambiguate):

```
Byte 0-4:    0xFF 0xFF 0xFF 'T' 'S'   (5-byte magic)
Byte 5-8:    big-endian u32           (payload length, N; ≤ 1024)
Byte 9..9+N: JSON                     (tailcfg.EarlyNoise)
```

> **The frame is optional.** A client reads the first 9 bytes and, if the
> magic does not match, treats them as the beginning of the HTTP/2 preface
> instead (`_refs/tailscale-rs/.../connect.rs` `read_challenge_packet`).
> Headscale always emits it (`_refs/headscale/hscontrol/noise.go`,
> `earlyPayloadMagic = "\xff\xff\xffTS"`), so a responder *should* send it,
> but the wire contract permits its absence.

The `EarlyNoise` struct:
```json
{
  "NodeKeyChallenge": "chalpub:abcdef..."
}
```

> **Prefix is `chalpub:`, not `challenge:`.** Tailscale's `key.ChallengePublic`
> (and `ts_keys::ChallengePublicKey`) text-encodes as `chalpub:` + 64 hex.
> §5.3 below is corrected accordingly.

`NodeKeyChallenge` is a temporary Curve25519 public key. The client must
sign a message with its `NodePrivate` key to prove ownership during
registration or map requests. This prevents node-key impersonation.

### 4.4 Controlbase message framing (the layer under HTTP/2)

Everything on the connection — the handshake *and* the post-handshake
encrypted bytes — is wrapped in **controlbase framing**: a 3-byte header
followed by a body.

```
Field   Size      Description
─────   ────      ───────────
type    u8        message type (see below)
len     u16 BE    body length (not including this 3-byte header)
body    len bytes type-specific
```

| `type` | Name | Body |
|--------|------|------|
| `0x01` | Initiation | Noise msg1. **Prefixed by a 2-byte capability version** before the 3-byte header (this message alone). |
| `0x02` | Response | Noise msg2. |
| `0x03` | Error | cleartext UTF-8 error string. |
| `0x04` | Record | an encrypted application-data chunk. |

After the handshake, the encrypted stream is **not** raw over the Noise
socket: the HTTP/2 bytes are carried as a sequence of `Record (0x04)`
frames, each `[0x04][u16 BE len][ciphertext]`, and the Noise transport
cipher is applied per record. A responder must therefore wrap/unwrap this
framing *beneath* the HTTP/2 server, not hand the `h2` crate the raw socket.
Source: `_refs/tailscale-rs/ts_control_noise/src/messages.rs`.

> Note the three distinct length encodings on this connection, easy to
> conflate: controlbase `len` is **u16 BE**, the EarlyNoise length (§4.3) is
> **u32 BE**, and the map sub-frame length (§5.1) is **u32 LE**.

### 4.5 HTTP/2 inside Noise

Above the controlbase `Record` framing (§4.4), the decrypted byte stream is
an ordinary HTTP/2 connection. The encrypted `Conn`, exposed as
`AsyncRead + AsyncWrite`, is handed to a single-connection HTTP/2 server
(`h2` crate). Routes mounted on this per-connection server:

| Route | Handler |
|-------|---------|
| `POST /machine/register` | Registration handler |
| `POST /machine/map` | Long-poll streaming handler |
| `POST /machine/ssh/action/...` | SSH (deferred) |

HTTP/2 is required precisely **because** these are multiplexed: the
long-lived `/machine/map` long-poll stream must share the one Noise
connection with other `/machine/*` requests (and HTTP/2 keepalive/flow
control), so they run as concurrent HTTP/2 streams rather than serially.
There is only ever one client per connection — so there is no cross-*client*
contention — but the per-connection stream multiplexing is the whole reason
HTTP/2, not HTTP/1.1, runs inside the tunnel.

---

## 5. Wire framing details

### 5.1 Map response sub-framing

This is an **application-level** framing that lives in the HTTP/2 response
body of `/machine/map` — a *third* framing layer, distinct from the
controlbase `Record` framing of §4.4 (which is u16 BE and wraps the whole
HTTP/2 connection). Here, inside the `/machine/map` stream body, each map
response is length-prefixed:

```
Field       Size    Description
─────       ────    ───────────
Length      u32 LE  Body length in bytes (0 for keepalive)
Body        N bytes JSON or zstd-compressed JSON

Compression is per-frame, indicated by the initial MapRequest.Compress field.
```

Keepalive frames have `Length = 0` (empty body). Note the endianness:
**little-endian** here, versus big-endian for both the controlbase header
(§4.4) and the EarlyNoise length (§4.3).

### 5.2 zstd framing

When `Compress: "zstd"` is set in the `MapRequest`, map response bodies are
compressed using the zstd frame format (`rfc8878`). The server selects the
compression level (typically 3–15); the client decompresses using the `zstd`
crate's streaming decoder.

### 5.3 Key text encoding

All Curve25519 keys on the wire use Tailscale's typed text encoding:

| Type | Prefix | Example |
|------|--------|---------|
| Machine public | `mkey:` | `mkey:abcdef0123456789...` |
| Machine private | (never on wire) | — |
| Node public | `nodekey:` | `nodekey:abcdef0123456789...` |
| Disco public | `discokey:` | `discokey:abcdef0123456789...` |
| Challenge | `chalpub:` | `chalpub:abcdef0123456789...` |

The encoding is: `prefix` + 64 lowercase hex chars (32 bytes). Parsing must
validate the prefix, decode hex, and reject wrong-length keys.

### 5.4 Pre-auth key format

```
hskey-auth-<12 hex prefix>-<64 hex secret>
             │                  │
             └── key lookup     └── bcrypt-hashed secret
```

The server stores the prefix and the bcrypt hash of the secret. On use, the
client sends the full string; the server looks up the key by prefix, then
verifies the secret against the stored bcrypt hash.

---

## 6. Routing & tenancy

The server supports three simultaneous strategies for dispatching a request
to the correct `Network` (tenant). They are checked in order and the first
match wins.

### 6.1 Subdomain (all clients)

```
Host: tenant1.ctl.example.com   →  Network{id: "tenant1"}
```

The tenant is extracted from the `Host` header. Compatible with **every**
Tailscale client — stock Go `tailscaled`, unmodified `tailscale-rs`, and
modified Rust clients alike.

### 6.2 Path prefix (Rust client only)

```
https://ctl.example.com/tenant1/key    →  GET /key
https://ctl.example.com/tenant1/ts2021 →  POST /ts2021
```

Requires a 2-line patch to `tailscale-rs` (`ts_control/src/tokio/connect.rs`)
changing `Url::join("/key")` and `Url::join("/ts2021")` to relative joins:

```rust
// Stock: control_url.join("/ts2021")  → "https://host/ts2021"
// Patched: control_url.join("ts2021") → "https://host/tenant1/ts2021"
```

The server mounts routes at both `/:tenant/key`, `/:tenant/ts2021`, etc. **and**
at root (`/key`, `/ts2021`) so stock clients continue to work unchanged.

### 6.3 Optional mTLS (Rust client only)

When the client presents a TLS certificate, the server extracts the tenant
from the certificate's CN or SAN:

```
Client cert CN="tenant1" → Network{id: "tenant1"}
```

The server uses `WebPkiClientVerifier::builder(ca).allow_unauthenticated()`
— it requests a client cert but does **not** reject connections without one.
Stock Go clients send nothing and fall through to subdomain or root.

### 6.4 Coexistence

```
Request arrives
  ├─ mTLS cert present?  → route by CN/SAN
  ├─ Host matches tenant? → route by subdomain
  ├─ Path matches tenant? → route by path prefix
  └─ none match           → default network (or 404)
```

---

## 7. Capability versions

The `CapabilityVersion` is a monotonically increasing integer that encodes
what protocol features a client supports. The server uses it to shape
responses compatibly.

| Version | Tailscale release | Notes |
|---------|-------------------|-------|
| 1 | v0.91 | Initial |
| ... | ... | |
| 39 | v1.0 | `/key` returns `OverTLSPublicKeyResponse` |
| ... | ... | |
| 113 | v1.80 | **Current minimum supported** |
| 126 | v1.84 | Latest at time of writing |

The server advertises its minimum supported version via:
- Rejecting handshakes below the threshold in the EarlyNoise callback.
- Including `CapMap` in `MapResponse` to negotiate per-feature support.

The capver table (`capver` crate) is ported from Headscale's
`hscontrol/capver/capver_generated.go` and defines the boundary for which
client versions the server supports. The target window is the latest ~10
minor releases.

---

## 8. Key types summary

| Key | Purpose | Who generates | Where used |
|-----|---------|---------------|------------|
| **MachineKey** | Long-lived hardware identity (Noise static key) | Factory (burned in) / `tailscale up` | Noise handshake, `/machine/register` |
| **NodeKey** | WireGuard session key (rotates) | Client, on each session | `/machine/register`, `/machine/map` |
| **DiscoKey** | Peer path-discovery key | Client, once per install | `/machine/map` |
| **ChallengeKey** | NodeKey-proof ephemeral key | Server, per-handshake | EarlyNoise frame |
| **ControlKey** (server) | Server Noise static key | Server, generated once | `/key`, Noise handshake |

The server stores the **public** half of each key for every registered node
and uses them for authentication, routing, and peer-map building.

---

## 9. References

| Concern | Primary source | Fallback |
|---------|---------------|----------|
| Wire types (`tailcfg`) | `_refs/tailscale-rs/ts_control_serde/` | `_refs/headscale/tailscale.com/tailcfg/` |
| Key types | `_refs/tailscale-rs/ts_keys/` | `_refs/headscale/.../types/key/` |
| Noise IK pattern + protocol name + prologue mixing | `_refs/tailscale-rs/ts_noise/src/ik.rs` | — |
| Controlbase framing (`Header`, message types, Initiation) | `_refs/tailscale-rs/ts_control_noise/src/messages.rs`, `handshake.rs` | — |
| Noise handshake (responder) | This document §4 + Go `control/controlhttp/controlhttpserver` | `_refs/headscale/hscontrol/noise.go` |
| HTTP upgrade flow (`Upgrade: tailscale-control-protocol`, `X-Tailscale-Handshake`, EarlyNoise probe) | `_refs/tailscale-rs/ts_control/src/tokio/connect.rs` | `_refs/headscale/hscontrol/noise.go` |
| Map streaming | `_refs/headscale/hscontrol/poll.go` | `_refs/tailscale-rs/ts_control_serde/` |
| Registration | `_refs/headscale/hscontrol/auth.go` | `_refs/headscale/hscontrol/state/state.go` |
