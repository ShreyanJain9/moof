# transport

> **the wire layer that carries reflector envelopes between replicas
> and the reflector. small, predictable, doesn't try to be a
> messaging system. one logical channel per replicated session, plus
> a side-channel for snapshot transfer.**

`concepts/replication.md` describes the *what*. this doc describes
*how the bytes get there*.

## scope

transport handles:
- reflector ↔ replica envelope flow.
- snapshot transfer for late-joining replicas.
- handshake / authentication / capability negotiation.

transport does *not* handle:
- cross-vat far-ref messaging (uses a separate, simpler transport;
  point-to-point).
- federated workspaces (`docs/concepts/image-and-world.md`).
- general api / rpc.

## transports supported

at v0.1:

| transport | when |
|---|---|
| in-process channel | tests; same-process replicas; reflector-as-thread. |
| websocket (tcp) | localhost; same-machine demo with two terminals. |

at v0.2+:

| transport | when |
|---|---|
| webrtc (udp) | low-latency cross-internet for moofpaint sessions. |
| http(s) snapshot | one-shot snapshot transfer; works over any http. |

## the envelope wire format

every reflector → replica message is an envelope. on the wire:

```
| 4 bytes  | length-prefix (u32, big-endian)
| 1 byte   | format-version (currently 1)
| 1 byte   | envelope-kind:
|          |   0x01 = TurnEnvelope
|          |   0x02 = SnapshotChunk
|          |   0x03 = ControlMessage
|          |   0x04 = Heartbeat
| <body>   | per envelope-kind
```

### TurnEnvelope (kind 0x01)

```
| 16 bytes | session-id (UUIDv7)
|  4 bytes | epoch (u32)
|  8 bytes | turn-seq (u64)
| 16 bytes | author-vat-id (UUIDv7)
|  8 bytes | logical-now (i64, ticks since session start)
|  8 bytes | seed (u64, reflector-supplied entropy)
|  4 bytes | input-event-len (u32)
| <bytes>  | input-event (canonical-encoded Form)
| 64 bytes | reflector-signature (ed25519 over above)
```

replicas verify the signature before processing. a replica without
the reflector's public key cannot accept envelopes.

### SnapshotChunk (kind 0x02)

snapshots are streamed in chunks because they may be large. each
chunk:

```
| 16 bytes | snapshot-id (UUIDv7)
|  8 bytes | chunk-index (u64)
|  8 bytes | total-chunks (u64)
|  8 bytes | committed-turn-seq (u64; the snapshot reflects state at
            this turn-seq)
|  4 bytes | chunk-len (u32)
| <bytes>  | chunk-bytes (zstd-compressed canonical heap encoding)
| 32 bytes | chunk-hash (blake3)
```

snapshot transfer happens on a separate connection from the live
envelope stream. the receiver re-assembles, verifies, decompresses,
loads.

### ControlMessage (kind 0x03)

```
| 1 byte   | message-type:
|          |   0x01 = Hello (initial handshake)
|          |   0x02 = Welcome (reflector → replica after auth)
|          |   0x03 = SubscribeFromTurn (replica → reflector;
|                     "send me envelopes from turn-seq N")
|          |   0x04 = LeaderAnnounce (replica → reflector;
|                     "i am leader for epoch N+1")
|          |   0x05 = Disconnect
| <body>   | per message-type
```

### Heartbeat (kind 0x04)

reflector sends every N seconds (default: 5s). replica acknowledges.
disconnect detection.

```
| 8 bytes  | reflector-now (logical-now, mirrors current envelopes)
| 8 bytes  | replica-last-seen-turn-seq (echoed back in ack)
```

## handshake

new replica connects:

1. opens connection to `wss://reflector.host/session/<session-id>`.
2. sends `Hello`:
   ```
   | 16 bytes | client-vat-id
   | 32 bytes | client-public-key (ed25519)
   | 64 bytes | signature-of-(session-id || vat-id) by client-key
   ```
3. reflector verifies signature, looks up session, decides
   accept/reject.
4. reflector replies `Welcome` with current turn-seq, current epoch,
   reflector public key, and a snapshot-server URL.
5. replica fetches snapshot (out-of-band).
6. replica replays from snapshot-turn-seq to current.
7. replica sends `SubscribeFromTurn current-turn-seq`.
8. reflector starts streaming envelopes.

## reconnect

a replica that drops:

1. reconnects to the reflector.
2. sends `Hello` plus its `last-committed-turn-seq`.
3. reflector replies `Welcome`.
4. replica sends `SubscribeFromTurn last-committed-turn-seq + 1`.
5. reflector streams catch-up envelopes from a small recent buffer
   (default: 10000 turns, configurable). if too far behind, it
   instead returns "snapshot-required" and the replica re-syncs.
6. once caught up, normal flow resumes.

## leader failover

(see `concepts/replication.md` for the full protocol.)

on the wire:

1. follower decides to take over (timeout on leader heartbeat).
2. follower sends `LeaderAnnounce(epoch+1)` to reflector.
3. reflector either accepts (and promotes) or rejects (current
   leader is fine; election failed).
4. if accepted, reflector closes epoch N's input log; opens
   epoch N+1; broadcasts `epoch=N+1` to all replicas.
5. all replicas process `EpochOpen(N+1)` envelope; clear in-flight
   intents; new leader replays outbox.

## back-pressure and rate-limiting

the reflector emits a tick every 50ms by default. between ticks, it
batches input events received from replicas into the next outgoing
envelope. high-rate inputs (mouse-move at 240Hz) get coalesced.

if a replica falls behind (slow consumer), the reflector buffers up
to 10000 envelopes for it. beyond that, the replica is disconnected;
it must re-handshake with snapshot fetch.

producers (replicas submitting input events) can flood the reflector.
the reflector enforces a per-replica rate limit (default: 100 events
per second). over-limit events are dropped with a `RateLimited`
control message back to the offender.

## security model

at v0.1: trust within the session. all replicas are assumed honest;
all input is signed by replicas; the reflector's signature is the
session's authority.

beyond v0.1: per-replica capability authorization (replica X is
allowed to submit `Stroke` but not `ProtoEdit`); message-level
encryption; replica revocation.

byzantine replicas (sending malformed signed envelopes) are out of
scope. the reflector is the trust anchor; if it's compromised, the
session is compromised.

## what we deliberately do *not* do

- **out-of-order delivery within an epoch.** envelopes are totally
  ordered.
- **multicast at the network layer.** the reflector unicasts to each
  replica. (multicast can be added later for many-replica sessions.)
- **client-driven retransmission.** if a replica needs an envelope
  it didn't get, it asks the reflector by turn-seq. tcp + websocket
  handles within-session reliability for us.
- **udp by default.** websocket-over-tcp is reliable enough for v0.1.
  webrtc/udp comes when latency demands.

## the in-process variant

for tests and same-process replicas: the "transport" is a
`crossbeam::channel::Sender<Envelope>` per replica. the reflector
holds all senders; `broadcast(env)` clones the envelope and pushes to
each. trivial, fast, deterministic.

## inspirations

- **websocket protocol**: rfc 6455.
- **raft's log replication & snapshot transfer**: ongaro &
  ousterhout 2014.
- **kafka's broker model**: jay kreps et al. log + sequential
  consumers.
- **quic / http3**: for future multi-streaming over udp.
- **noise protocol**: trevor perrin. for v0.2 encryption.
- **erlang/OTP's distribution protocol**: for the "trusted cluster"
  trust model.

## see also

- `concepts/replication.md` — what envelopes mean.
- `concepts/persistence.md` — input log on disk.
- `laws/isolation-laws.md` — vat boundaries.
- `concepts/references.md` — far-refs (a separate transport).
- `concepts/effect-intents.md` — receipts arrive via the same
  transport as inputs.
