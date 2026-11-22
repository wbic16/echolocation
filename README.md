# echolocation

A generic **multiversal phext pathway between any two programs**: emit a
probe into a shared phext medium, and whoever is listening answers with an
echo carrying their identity, capabilities, and state — while you measure
the round trip. Discovery, identity, and ranging in one exchange, with no
prior introduction and no central directory.

The pattern descends from the dolphin organization of the Quantum Time
Sync work (emit a timestamped probe, read the echo) generalized from
clock-sync to any two programs that share a phext.

- **SPEC.md** — the protocol, implementable from the text alone: the
  medium and its well-known channels, probe/echo scroll formats, the
  exchange state machine, signature atoms, multiversal crossing rules,
  composition with the Choir mailbox pattern, and the failure catalogue.
- **src/lib.rs** — the reference implementation (Rust, on libphext-rs).
- **src/bin/echolocate.rs** — the reference CLI, both roles in one binary.
- **tests/exchange.rs** — the exchange, tested: happy path, no-echo
  expiry, multi-echo, malformed scrolls, cross-universe echo.
- **demo.sh / DEMO.md** — three acts between separate processes over one
  shared `.phext` file: discovery, a cross-universe echo, and a probe
  into silence.

## Quick start

```
cargo build
cargo test

# a responder (starts knowing nothing about any prober)
./target/debug/echolocate answer --medium /tmp/m.phext \
  --name ni-knight-runner --home 1.1.1/1.1.1/9.4.1 \
  --capabilities time-census,consensus --state "ready" \
  --mailbox 1.1.1/1.4.1/1.1.1 --for 30 &

# a prober (starts knowing nothing about the responder)
./target/debug/echolocate probe --medium /tmp/m.phext \
  --name mira --home 1.1.1/1.1.1/9.1.1 \
  --intent time-census --ttl 10
```

## Library surface

- `Signature` — the identity atom: name (a claim, not a proof), home
  coordinate, capability list. `publish_signature` writes the signature
  scroll at the home coordinate.
- `emit_probe(medium, sig, home_universe, target_universe, intent, ttl,
  mode, poll_ms) -> ProbeHandle`
- `await_echo(medium, handle) -> Exchange` — echoes, the measured
  `range_ms`, and the outcome (`Echoed` / `Expired`; expiry is a normal
  terminal state — silence is absence, never assent).
- `answer_once(medium, sig, home_universe, inhabits, state, mailbox,
  poll_ms)` — the responder step: scan the probe channels of the
  universes you inhabit, answer at most one outstanding probe, never an
  expired one, never the same probe-id twice.
- `Medium` — the transport trait. `FileMedium` (a shared `.phext` file;
  lock-file serialized writes, rename commits) and `MemoryMedium` ship;
  an SQ-hosted medium implements the same two methods against SQ's
  select/replace surface.

## Dependencies

The whole tree, enumerated (house rule: no massive chains):

- `libphext 0.6.6` — the phext reference implementation; all phext
  reading/writing and the Exocortical Epoch stamps (`libphext::epoch`)
  go through it. Default features only (no base64/xz2).
- `xxhash-rust 0.8.21` (feature `xxh64`) — probe-id nonces; also
  libphext's own checksum dependency.

Everything else is `std`. No async runtime, no serialization framework:
probe and echo scrolls are plain text, `key: value` lines under a version
header, per the house preference for text over bracket formats.

## Honest limitations (v1)

- **Range is an upper bound dominated by polling**, not a distance. Its
  resolution floor is roughly the two poll intervals added; SPEC §5
  requires ranges to be reported with both poll values. Over an SQ-hosted
  medium the floor rises by the server's visibility latency.
- **Identity is claimed, not proven.** Signatures carry no cryptography
  in v1 — spoofing and name collisions are possible and are named in
  SPEC §7 with the mitigations that exist (home-scroll corroboration,
  `(name, home, first-seen EE)` triple-keyed records).
- **The file medium serializes writers with a lock file.** That is
  correct for two programs on one host, which is what the demo runs;
  high-fan-out discovery wants a server-serialized medium (SQ), which
  the `Medium` trait exists to allow.
