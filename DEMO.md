# Demonstration — two programs find and range each other

`demo.sh` runs three acts between **separate processes** over one shared
`.phext` file. No program is told about any other: each responder starts
knowing only its own name, home coordinate, and capabilities; each prober
starts knowing only the medium and the universe it wants to search.

- **Act 1** — `mira` probes universe `1.1.1` for `time-census`;
  `ni-knight-runner` answers with its signature, capabilities, state, and
  mailbox. Range: **201 ms** at 100 ms polling on both sides — about one
  poll cycle each way, exactly the floor SPEC §5 predicts.
- **Act 2** — `mira` probes *across* into universe `2.1.1`;
  `far-seat`, which inhabits only `2.1.1`, answers, and the echo crosses
  back into `1.1.1`'s echo channel carrying `home-universe: 2.1.1`.
  Two details in the transcript are the protocol working as specified,
  not noise: the echo lands at index **#2** because index 1 of that
  channel still holds Act 1's echo (channels are keyed by reply universe
  + slot; matching is by probe-id — see SPEC §2.3), and the **1 ms**
  range is a lucky tick alignment — the responder's poll fired inside
  the prober's first read window. A single range bounds; it does not
  pinpoint (SPEC §5).
- **Act 3** — `mira` probes universe `3.1.1`, where nobody listens. The
  probe expires. Silence is absence, never assent.

Transcript as captured 2026-11-22 (EE 99); lines from concurrent
processes interleave, timestamps are each writer's own. The full log is
in `demo-transcript.txt`.

```
=== Act 1: discovery in universe 1.1.1 ===
[2026-11-22T17:28:00Z EE 99] ni-knight-runner answering in universe 1.1.1 (listening: 1.1.1), home 1.1.1/1.1.1/9.4.1, capabilities: time-census, consensus, blend
[2026-11-22T17:28:01Z EE 99] mira emitted probe mira@1.1.1#99-1-0454 into universe 1.1.1 (intent: time-census, reply at 1.1.1/1.1.1/2.1.1, expires 2026-11-22T17:28:05Z)
[2026-11-22T17:28:01Z EE 99] ni-knight-runner echoed probe mira@1.1.1#99-1-0454 from mira at 1.1.1/1.1.1/2.1.1
[2026-11-22T17:28:06Z EE 99] mira got echo #1 from ni-knight-runner (home 1.1.1/1.1.1/9.4.1, home universe 1.1.1): capabilities [time-census, consensus, blend], state: round 34 complete; consensus +0.006 ms, mailbox: 1.1.1/1.4.1/1.1.1
[2026-11-22T17:28:06Z EE 99] mira range: 201 ms (poll: prober 100 ms; an upper bound — see SPEC §5)
[2026-11-22T17:28:08Z EE 99] ni-knight-runner done listening

=== Act 2: a probe across universes (1.1.1 -> 2.1.1) ===
[2026-11-22T17:28:08Z EE 99] far-seat answering in universe 2.1.1 (listening: 2.1.1), home 2.1.1/1.1.1/9.4.1, capabilities: cartography
[2026-11-22T17:28:09Z EE 99] far-seat echoed probe mira@2.1.1#99-1-a8f7 from mira at 1.1.1/1.1.1/2.1.2
[2026-11-22T17:28:09Z EE 99] mira emitted probe mira@2.1.1#99-1-a8f7 into universe 2.1.1 (intent: cartography, reply at 1.1.1/1.1.1/2.1.1, expires 2026-11-22T17:28:15Z)
[2026-11-22T17:28:09Z EE 99] mira got echo #2 from far-seat (home 2.1.1/1.1.1/9.4.1, home universe 2.1.1): capabilities [cartography], state: surveying the far shelf, mailbox: 2.1.1/1.4.1/1.1.1
[2026-11-22T17:28:09Z EE 99] mira range: 1 ms (poll: prober 100 ms; an upper bound — see SPEC §5)

=== Act 3: a probe into silence (universe 3.1.1) ===
[2026-11-22T17:28:16Z EE 99] mira emitted probe mira@3.1.1#99-1-9a79 into universe 3.1.1 (intent: any, reply at 1.1.1/1.1.1/2.1.1, expires 2026-11-22T17:28:19Z)
[2026-11-22T17:28:20Z EE 99] mira heard silence — probe expired with no echo (absence, not assent)

=== The medium afterwards: demo-run/medium.phext (1768 bytes) ===
```

(Act 1's prober reports at :06 rather than :02 because it ran in `all`
mode: it fixed the range at the first echo and kept collecting until the
probe's 4-second lifetime expired, per SPEC §6.1.)

## Tests

`cargo test` — 5 integration tests over a real shared file medium,
all passing (4.93 s):

- `happy_path_discovery_identity_and_range`
- `no_echo_expiry_is_a_normal_terminal_state`
- `multi_echo_collects_every_responder`
- `malformed_scrolls_are_skipped_never_fatal`
- `cross_universe_echo_crosses_back` (also asserts the echo's physical
  placement: written in the prober's universe, not the responder's)
