# The croc receiver

How Kopico receives transfers with [croc](https://github.com/schollz/croc), and the croc
behaviour it relies on. Code: `src/receiver.rs`.

## Why a loop

A croc receiver can't sit and wait for a sender: each run either finds a sender and
receives, or gives up. So while Kopico is enabled, a background thread runs croc again and
again:

| Mode | croc arguments | Poll every | Max wait per run | Rate-limit back-off |
|---|---|---|---|---|
| Local Network (default) | `--local` | 3 s | croc exits by itself (~0.5–1 s) | — |
| Local Network + Sender IP | `--ip <host[:port]>` | 3 s | croc exits by itself (~0.5 s) | — |
| Own Relay | `--relay <host:port>` (+ `CROC_PASS`) | 10 s | 8 s | 60 s |
| Public Relay | — | 30 s | 25 s | 60 s |

Every run also gets `--yes --ignore-stdin --rename --out ~/Downloads/Kopico`, and the code
in `CROC_SECRET`.

- **Intervals** count from the start of a run, so "every 3 s" means a new attempt starts
  every 3 s.
- **Public relays** admit about 30 joins per minute per IP (`croc relay` defaults:
  `--source-join-limit 30 --join-limit-window 1m`). Polling every 30 s stays far below that,
  and a rate-limit answer pauses the loop for 60 s. That's also why **Local** is the
  default: no relay, no limit.
- **Max wait** only matters for relays: a relay receiver waits in the relay's room until
  someone comes, so Kopico ends the run itself. It **never** ends a run once a sender has
  been found — a transfer is never cut off.
- Mode and timing constants are at the top of `src/receiver.rs`.

## croc's output, as Kopico reads it

Verified against **croc 11.5.4**. Re-check these after upgrading croc — the parser depends
on them.

**Progress goes to stderr**, separated by `\r` as well as `\n`; stdout carries only
errors (and received text). Phases on stderr:

```
connecting... → looking for sender... | waiting for sender...      (nobody yet)
→ authenticating code... → opening transfer channels...           (sender found)
→ Receiving 2 files (28.6 MB) | Receiving text message (17 B)    (transferring)
```

| Situation | What croc does | Kopico's outcome |
|---|---|---|
| `--local`, nobody sending | exit 1 after ~0.5–1 s; stdout `could not connect to : found no relay addresses` | no sender, try again |
| `--ip`, nobody at that address | exit 1 after ~0.5 s; stdout `… connection refused` | no sender |
| relay, nobody sending | waits forever (`waiting for sender...`) | Kopico kills it at the max wait |
| files received | exit 0 | **Received** — new names in the output folder |
| text received | exit 0; stderr `Receiving text message (N B)`; stdout = the exact text; no file | **Text** — copied to the clipboard |
| relay rate limit | exit 1, stdout `… relay admission rate limited`, or stderr `The relay is rate limiting reconnects; retrying in …` | back off 60 s |

Details that shaped the parser:

- croc never prints a reliable "received file X" line, and file names in the progress bar
  are truncated (`test fil...`). Kopico lists the output folder before and after a run and
  reports the new entries. `--rename` guarantees a new name even when a file with the same
  name exists — so nothing is overwritten and the difference is reliable.
- Rate limiting is matched on `rate limit` only; matching `429` or `too many` would trigger
  on a file name or a size like `(429 MB)`.
- croc refuses codes on the command line on Linux/macOS, so the code goes in
  `CROC_SECRET`. The relay password goes in `CROC_PASS` (keeps it out of `ps`).
  `CROC_RELAY`/`CROC_RELAY6`/`CROC_PASS` from your shell are removed, so the mode chosen
  in Kopico always applies.
- croc needs a code of at least 6 characters.

## Finding the sender on a local network

`--local` finds the sender by multicast discovery. In croc 11.5 the receiver gives up on
discovery in under a second (`peer discovery timed out` in `croc --debug`), and on some
machines — seen on macOS — it never finds a sender that is reachable. Hence the optional
**Sender IP**:

- Kopico then runs `croc --ip <host[:port]>` (without `--local`) — the same as typing
  `CROC_SECRET=… croc --ip 192.168.0.110:9009 --yes` by hand.
- croc connects straight to the small relay a normal `croc send` starts on the sender
  (port 9009 unless given). The sender must **not** use `--no-local`.
- If nobody is sending there, croc exits in ~0.5 s with *connection refused*, so the 3 s
  rhythm is kept.
- The sender needs a stable address (DHCP reservation) — otherwise update the field.

## Lifecycle and safety

- croc is located on `PATH`, then in `/usr/local/bin`, `/opt/homebrew/bin`,
  `/opt/local/bin`, `/usr/bin`, `~/.local/bin`, `~/go/bin` — apps started from Finder or a
  desktop launcher get a minimal `PATH`.
- Both pipes are drained by their own threads, so croc can never block on a full pipe.
- Stopping (Kopico off, settings changed, quit) kills a running croc within ~200 ms.
- If the app is force-killed while croc waits on a relay, that croc keeps waiting; a
  normal quit always stops it.
- Received text is never logged and never shown in notifications (they can appear on the
  lock screen).

## Reading the debug log

`KOPICO_DEBUG=1 cargo run` logs each run: the command line (code and password never
included), the exit status, croc's status lines, its stdout, and the outcome.

| Log | Meaning |
|---|---|
| `stdout: could not connect to : found no relay addresses` (Local) | discovery found nobody — normal when idle; if a sender *is* waiting, set a Sender IP |
| `stdout: … connection refused` (Sender IP) | nobody sending at that address right now |
| `→ NoSender` after ~8 s / 25 s (relay modes) | nobody joined the room this round |
| `→ RateLimited` | the relay refused us; next try in 60 s |
| `stderr: … authenticating code...` then `→ NoSender` | a sender was there but the transfer failed — often a code mismatch |

## Tests

`cargo test` checks the parsing rules. `cargo test -- --ignored --test-threads=1` runs
real croc processes: own relay, `--ip`, text, wait limit + stop latency, rate limiting
(using a local `croc relay --source-join-limit 1`). See
[development.md](development.md#tests).
