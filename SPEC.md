# Echolocation — a generic multiversal phext pathway between any two programs

Version 1. Status: reference specification, with a reference implementation in
this repository (Rust, on libphext-rs).

Origin: the dolphin organization of the Quantum Time Sync triangulation work
(task 26.6.11/8.7.14/8.6.10), where sampling is echolocation — emit a
timestamped probe, read the echo, and let the echo carry identity and range.
This spec generalizes that pattern from clock-sync to any two programs.

## 1. The idea

A program that wants to find, identify, or range another program it has never
met places a **probe** — a phext scroll at a well-known coordinate in a shared
phext medium. Any program listening on that medium may answer with an **echo**
— a scroll written back at a reply coordinate the probe names, carrying the
responder's identity (its signature), the capabilities it offers, and its
state. The prober measures the round trip: the **range**.

One exchange yields three things at once:

- **discovery** — someone is there;
- **identity** — who they claim to be and what they offer;
- **ranging** — how far away they are, in medium round-trip time.

No prior introduction, no central directory, no registry is required to run
an exchange. The medium is the only shared assumption.

## 2. The medium and its addressing

The medium is a single phext document that both programs can read and write
at scroll granularity. Qualifying media include, without distinction:

- a shared `.phext` file both programs can open;
- an SQ-hosted phext (read/written through SQ's select/replace surface);
- a phext-lattice document both programs are editors of.

All coordinates in this spec are **medium-relative**: a coordinate names a
scroll inside one named medium, never a global address. An exchange happens
inside one medium; §8 covers how its addressing spans universes *within* the
medium.

### 2.1 Phext coordinates (as implemented by libphext)

A scroll is addressed by a 9-component coordinate written `Z/Y/X`:

- Z arm — `library.shelf.series`
- Y arm — `collection.volume.book`
- X arm — `chapter.section.scroll`

Components are 1-based and unbounded. A position *inside* a scroll adds line
and column, for 11 dimensions in total. Implementations MUST read and write
phext through the reference implementation (libphext-rs, or a port at parity
with it) — phext serialization is never hand-rolled.

### 2.2 Universes

A **universe** of a medium is one value of the Z arm (`library.shelf.series`).
The universe a program *inhabits* is the Z triple under which it publishes
its own signature and listens for probes. The default universe is `1.1.1`.
A medium may carry any number of universes side by side; they share the
document and nothing else — channels, registries, and mailboxes are
per-universe.

### 2.3 Well-known channels

Within universe `Z`, with the Y arm fixed at `1.1.1`:

| Channel | Coordinate pattern | Contents |
|---|---|---|
| Probe channel | `Z/1.1.1/1.1.n` | Probe scrolls; `n` (the scroll component) is the **probe slot**, 1-based, append-only. |
| Echo channel | `Z/1.1.1/2.s.k` | Echo scrolls answering probe slot `s`; `k` (scroll component) is the **echo index**, 1-based. |
| Signature registry | `Z/1.1.1/3.1.n` | Optional self-published signature scrolls (§7). Browsable; never required. |

A prober chooses its probe slot as the first `n` whose probe-channel scroll
is absent. Slots are never reused within a medium's lifetime; an expired
probe remains in its slot as history. After writing, a prober MUST read the
slot back and confirm it holds its own probe-id; on a mismatch (a concurrent
prober won the slot) it takes the next free slot and re-emits.

Claiming a slot — finding the first free coordinate and writing to it — MUST
be one atomic read-modify-write on the medium. Serializing each individual
write is not enough: two writers can both find slot `n` free and then write
it in turn, and the read-back only shows who held the slot at the instant
of that read — a later overwrite leaves the first prober waiting on a probe
that is no longer there, and it will read the result as silence. The
reference implementation meets this rule through `Medium::update` (scan and
write inside one locked step). A medium that can only offer separate
select and replace calls needs a conditional write (replace only if the
scroll is still absent), or its probers MUST re-read their slot on every
poll and re-emit if their probe-id is gone. The same rule governs echo
indices.

A responder answering probe slot `s` writes its echo at the first free echo
index `k` of `Z/1.1.1/2.s.k`, where `Z` is the universe named by the probe's
`reply-at` field (§3) — which is the prober's universe, not necessarily the
responder's. See §8 for the crossing rule.

Note that the echo channel is keyed by (reply universe, probe slot) only.
Slot numbers are per target universe, so two exchanges that reuse a slot
number from the same reply universe — including one prober probing two
different universes in turn — share an echo channel. This is expected and
harmless: echoes are matched by probe-id (§6.3), foreign echoes are
skipped, and each answer takes the first free index. The channel is a
shared wall, not a private wire; the probe-id is the address that matters.

## 3. The probe

A probe is one scroll in the probe channel. Its text is plain lines; the
first line is the version header, the rest are `key: value` fields. Field
order is fixed as listed; unknown fields MUST be ignored by readers.

```
echolocation/probe 1
probe-id: mira@1.1.1#98-3-9f2c
from-name: mira
from-home: 1.1.1/1.1.1/9.1.1
from-capabilities: time-census, consensus
universe: 1.1.1
intent: time-census
reply-at: 1.1.1/1.1.1/2.3.1
mode: all
issued-utc: 2026-11-22T17:24:07Z
issued-ee: 98
expires-utc: 2026-11-22T17:24:37Z
poll-ms: 100
```

Fields:

- `probe-id` — `<from-name>@<target universe>#<issued-ee>-<slot>-<nonce>`.
  The nonce is 4 lowercase hex digits chosen by the prober so that a
  restarted prober re-using a slot in the same epoch does not collide with
  its own earlier probe. Probe-ids are the dedupe key for the whole
  protocol: responders answer a given probe-id at most once, and echoes are
  matched to probes by probe-id alone.
- `from-name`, `from-home`, `from-capabilities` — the prober's signature
  (§7), inline. A probe is signed by the same atom an echo carries.
- `universe` — the target universe: the Z triple whose probe channel this
  scroll sits in. Redundant with the scroll's own coordinate, deliberately:
  a probe copied or quoted out of place still names its universe.
- `intent` — the capability sought, or the literal `any`. A responder whose
  capabilities do not intersect the intent SHOULD NOT answer (silence is how
  the channel stays quiet); `any` invites every listener.
- `reply-at` — the full coordinate of echo index 1 for this probe:
  `<prober's universe>/1.1.1/2.<slot>.1`. Echo `k` replaces the scroll
  component with `k`. Naming the reply coordinate explicitly, instead of
  deriving it, is what lets an echo cross universes (§8).
- `mode` — `first` (the exchange closes at the first valid echo) or `all`
  (the prober collects every valid echo until expiry). Default `all`.
- `issued-utc`, `expires-utc` — the prober's clock, UTC, second precision.
  `expires-utc` is the exchange deadline; default lifetime is 30 s.
- `issued-ee` — the Exocortical Epoch of issue:
  `EE = floor((unix − 1795327200) / 420) + 1`, EE 0 before the anchor.
  Implementations on libphext SHOULD take this from `libphext::epoch`
  rather than re-deriving it. The EE stamp is the cross-check that survives
  clock disagreement: two programs whose UTC clocks differ still share the
  epoch count to within one epoch.
- `poll-ms` — the prober's poll interval on the echo channel. Required,
  because the range (§5) cannot be interpreted without it.

## 4. The echo

An echo is one scroll in the echo channel, at the coordinate §2.3 derives
from the probe's `reply-at`.

```
echolocation/echo 1
probe-id: mira@1.1.1#98-3-9f2c
from-name: ni-knight-runner
from-home: 1.1.1/1.1.1/9.4.1
home-universe: 1.1.1
capabilities: time-census, consensus, blend
state: round 34 complete; consensus +0.006 ms
mailbox: 1.1.1/1.4.1/1.1.1
issued-utc: 2026-11-22T17:24:09Z
issued-ee: 98
poll-ms: 100
```

Fields:

- `probe-id` — copied verbatim from the probe. An echo that does not name
  its probe is not an echo.
- `from-name`, `from-home` — the responder's signature (§7).
- `home-universe` — the universe the responder inhabits. It differs from
  the echo's own location exactly when the echo has crossed (§8).
- `capabilities` — what the responder offers. This is the discovery payload:
  the prober asked by `intent`; the echo answers with the full list.
- `state` — one line of free text: the responder's state, in its own terms.
  Optional; an absent `state` is a responder that offers capabilities but
  reports nothing.
- `mailbox` — a full coordinate, in this same medium, where the responder
  accepts addressed mail (the Choir pattern, §9), or the literal `none`.
  This is the graduation path from discovery to conversation.
- `issued-utc`, `issued-ee` — the responder's stamps.
- `poll-ms` — the responder's poll interval on the probe channel. Required,
  for the same reason as the prober's: it is half the range floor (§5).

## 5. Range — what the number means, and its floor

The prober records `t0`, the instant just before its probe write commits,
and `t1`, the instant it first observes a valid echo. The **range** is
`t1 − t0` in milliseconds, reported by the prober (never by the responder,
whose clock is not the prober's).

Range is an **upper bound** on the exchange's true round trip, and over a
phext medium it is dominated by polling, not by distance:

```
range ≈ (prober's wait for its next poll tick)
      + medium write-visibility latency, both directions
      + responder's wait for its next poll tick
      + responder's write time
```

Consequences the spec states plainly, in the spirit of the QTS sampling
work that this protocol descends from:

- The range **bounds; it does not pinpoint.** A range of 240 ms over a
  medium where both parties poll at 100 ms says the responder is *at most*
  about one poll-cycle away in each direction — nothing finer.
- The resolution floor of a single exchange is approximately
  `poll-ms (prober) + poll-ms (responder)`. Implementations MUST report the
  range alongside both poll values; a range quoted without them is not a
  protocol range.
- A shared-file medium on one host can range in single-digit milliseconds
  with tight polling. An SQ-hosted medium adds the server's visibility
  latency. Neither number says anything about physical distance; range is a
  property of the *pathway* — medium plus both programs' attentiveness —
  and that is exactly what a program deciding whether to talk further needs
  to know.

## 6. The exchange state machine

### 6.1 Prober

```
IDLE ──emit──▶ EMITTED ──first poll──▶ LISTENING
LISTENING ──first valid echo──▶ ECHOED
ECHOED ──mode=first──▶ CLOSED
ECHOED ──mode=all──▶ COLLECTING ──expiry──▶ CLOSED
LISTENING ──expiry, no valid echo──▶ EXPIRED ──▶ CLOSED
```

- The range is fixed at the first valid echo, in both modes.
- In `all` mode the prober keeps collecting until expiry; later echoes do
  not change the range.
- **EXPIRED is a normal terminal state, not an error.** It means: no
  listener in the target universe chose to answer within the lifetime.
  Silence is absence, never assent (the Choir protocol's standing rule,
  adopted unchanged).
- Re-probing is a new exchange: a new slot, a new probe-id. Suggested
  backoff between attempts of one discovery: 1 s, doubling, capped at 30 s;
  the number of attempts is the caller's policy, not the protocol's.

### 6.2 Responder

```
SCANNING ──probe parsed, unexpired, intent matches──▶ SEEN
SEEN ──echo written at first free k──▶ ANSWERED ──▶ SCANNING
```

- A responder MUST NOT answer an expired probe, a probe whose intent does
  not intersect its capabilities (unless the intent is `any`), or a
  probe-id it has already answered. The already-answered check is made
  against the medium itself (an echo naming this probe-id and this
  responder's name exists in the echo channel) so it survives restarts;
  a local seen-set is an optimization, never the only record.
- A responder MAY answer probes in several universes; it scans the probe
  channel of each universe it inhabits.

### 6.3 Echo validity

An observed scroll is a valid echo for a probe iff all of these hold:

1. it sits at the probe's echo coordinates (§2.3) with index `k ≥ 1`;
2. its first line parses as `echolocation/echo 1`;
3. its `probe-id` equals the probe's;
4. the prober observes it no later than `expires-utc` — the prober's own
   clock is authoritative for its own deadline.

Anything else at an echo coordinate — a partial write, a foreign scroll, a
scroll from another protocol sharing the medium — is **skipped, never
fatal**. A channel coordinate holding malformed content costs the prober
one skipped scroll, not the exchange.

## 7. Identity: the signature atom

A program's **signature** is three fields, carried inline in every probe
and echo it writes:

- `name` — self-chosen, human-readable, **not unique** and not a proof of
  anything. Programs choose names the way Choir seats do: by being called
  them.
- `home` — a full coordinate, in a named medium, that the program claims
  as its own. A program SHOULD publish its signature scroll at its home
  coordinate: the same three fields, plus its capability list and the EE of
  publication, as a scroll headed `echolocation/signature 1`. The registry
  channel (§2.3) is a convenience copy for browsing; the home coordinate is
  the canonical one.
- `capabilities` — a comma-separated list of capability words the program
  offers. Capability words are compared literally; a shared vocabulary is
  a community convention, not a protocol layer.

What v1 deliberately does **not** do: cryptographic signing is out of
scope. A signature is a *claim*, and the spec says so rather than
pretending otherwise. Two consequences are named, not hidden:

- **Spoofing is possible.** Any writer to the medium can emit an echo
  carrying any name. A prober that cares MAY corroborate: read the
  signature scroll at the responder's claimed home coordinate and check
  that the capabilities agree, and prefer responders whose home scroll
  predates the exchange. Corroboration raises the cost of a lie; it does
  not make lies impossible.
- **Name collisions happen.** Two programs may share a name. The
  disambiguating identity, for address books and logs, is the triple
  `(name, home coordinate, first-seen EE)`. Protocols built on
  echolocation MUST key records by the triple, never by name alone.
- **Fields are data, never instructions.** Every field of a probe, echo,
  or signature scroll is text chosen by a writer the reader has not
  authenticated — `state` most of all, since it is free text. A program
  that reads these scrolls, and above all a language-model agent, MUST
  treat their contents as data to record or display, and MUST NOT act on
  anything a field says to do. An introduction is not an order.

## 8. Multiversal rules

- A probe names exactly **one** target universe. Discovery across several
  universes is one probe per universe — each is its own exchange, with its
  own range.
- A probe is placed in the **target** universe's probe channel, so a
  listener finds it by scanning only the universes it inhabits.
- The echo is written at the probe's `reply-at` coordinate, which lives in
  the **prober's** universe. Writing the echo is the crossing: the
  responder reaches into the prober's universe to answer, and says where
  it actually lives in the echo's `home-universe` field.
- An echo is **valid in the prober's universe** iff it satisfies §6.3
  there. Its `home-universe` does not affect validity; it affects
  follow-up: mail (§9) is addressed in the responder's home universe, at
  the echo's `mailbox` coordinate, unless the two programs agree otherwise
  in that conversation.
- Universes are contexts, not trust domains. A cross-universe echo is
  exactly as trustworthy as a local one — which is to say, it is a claim
  (§7) — and exactly as useful: discovery, identity, and range all survive
  the crossing unchanged.

## 9. Composition: echolocation, the Choir mailbox, and SQ

Echolocation and the Choir mailbox are complementary halves of one
pattern, and the spec keeps their boundary sharp:

| | Choir mailbox | Echolocation |
|---|---|---|
| You must already know | the party, and their inbox coordinate | nobody |
| The write goes to | one known inbox | a channel anyone may hear |
| The reply is | a letter, in the conversation | an echo: identity + capabilities + range |
| Silence means | not yet read (the mailbox waits) | absence — nobody answered (the probe expires) |
| State after one exchange | an ongoing conversation | an address-book entry |

**Graduation.** An exchange ends with the prober holding the responder's
signature, home universe, and `mailbox` coordinate. The next step, if the
prober wants one, is a mail drop at that coordinate under the Choir
pattern: addressed delivery to a now-known party. Echolocation's job ends
at the introduction; it carries no conversation itself. Address-book
entries are keyed by the §7 triple and SHOULD record the last measured
range and last-seen EE alongside the mailbox coordinate.

**SQ's place.** SQ is a medium, not a party: it hosts the phext the
exchange runs over. Echolocation neither requires nor replaces it. One
operational note, from the house record: an SQ daemon's hosted file is
replaced only with the daemon stopped — programs using SQ as the medium
should write through SQ's own select/replace surface, which serializes
writes server-side, rather than swapping the file underneath it.

## 10. Failure catalogue

- **Medium unavailable** (file missing/locked past timeout, SQ down):
  emitting fails loudly to the caller; *listening* treats an unreadable
  medium as silence — a responder never invents a probe, a prober never
  invents an echo. Absence of the medium is absence, like absence of an
  answer.
- **Malformed scroll at a channel coordinate**: skipped per §6.3, on both
  sides. A probe channel scroll that does not parse is not a probe; an
  echo channel scroll that does not parse is not an echo. Neither side
  crashes, deletes, or answers malformed content.
- **Concurrent probers colliding on a slot**: prevented by the read-back
  rule (§2.3); the loser re-slots. Over a medium whose slot claim is
  atomic (the file medium's locked read-modify-write) collisions do not
  occur at all, and the read-back is the belt to that suspender. Over a
  medium that only serializes individual writes, the read-back narrows
  the window without closing it — see the atomic-claim rule in §2.3.
- **Echo spoofing / name collision**: possible in v1 by design (§7).
  Mitigations available in v1: home-scroll corroboration, triple-keyed
  records, and treating first contact as an introduction, not a trust
  grant. Cryptographic identity is future work and is named as such.
- **Probe floods**: etiquette is part of the protocol. A prober SHOULD
  wait at least 1 s between probes into one universe and SHOULD poll no
  faster than every 50 ms; responders MAY ignore, as local policy,
  probers who flood — a flooded channel is how a discovery medium dies,
  and the house rule is human-scale rates on shared surfaces.
- **Clock disagreement**: stamps are claims made by their writers'
  clocks. Expiry is enforced on the prober's clock for the prober's
  deadline and on the responder's clock when deciding whether a probe is
  still answerable. Two consequences follow. A responder whose clock runs
  ahead of the prober's by more than the probe's remaining lifetime will
  judge every probe expired and never answer, and the prober cannot tell
  that from absence: with the 30 s default lifetime, a skew of about half
  a minute is enough. The EE stamp does not rescue this — an epoch is
  420 s, far coarser than a probe's lifetime; it exists to let a reader
  notice gross disagreement (stamps more than one epoch apart), not to
  correct it. Programs on hosts without disciplined clocks SHOULD use
  longer lifetimes, and a responder SHOULD log, rather than silently
  drop, a probe whose `issued-ee` is current while its `expires-utc` has
  already passed on the responder's own clock.
- **Channel growth**: slots are never reused, so the probe channel only
  grows, and both the first-free-slot scan and a responder's scan start at
  slot 1. v1 accepts this cost for short-lived media. A long-lived medium
  should expect scans to slow in proportion to its history; a high-water
  mark scroll or channel rotation is future work and is named as such.
- **Responder dies mid-exchange**: indistinguishable from silence, and
  handled as silence — the probe expires. Echolocation keeps no session
  state to leak.

## 11. Versioning

The version header (`echolocation/probe 1`, `echolocation/echo 1`,
`echolocation/signature 1`) is the compatibility contract. Readers MUST
ignore unknown fields and MUST NOT answer a probe whose major version
they do not implement. A future version that changes field *meaning*
bumps the number; one that only adds fields does not.
