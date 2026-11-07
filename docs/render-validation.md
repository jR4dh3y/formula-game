# Render multiplayer work record

Base: PR #5, `459e9904c2a6438ba32f5293564aeddd9b7a35c4`.
Branch: `codex/render-multiplayer`. User authorized committing and pushing this
deployment branch on 2026-11-03. PR #5 itself is unchanged.

Target outcome: the native game joins a Render-hosted authoritative server using WSS,
while UDP LAN hosting and offline racing remain available.

## Hard constraints

- Preserve unrelated work; only stage task files if staging is authorized.
- Commit and push only this authorized deployment branch.
- Bevy remains the only active engine; physics/rules stay in `sim`.
- Do not edit, reactivate, or build `archive/custom-engine`.
- Cargo commands run from the checkout root; packaging belongs in root `tools`.
- Do not expose credentials, weaken TLS checks, or claim builds prove gameplay.
- Keep input ordering, identity/content checks, timeout and bounded-queue contracts.

## Surface and completion ledger

| Surface | Applies | Change | Check | State |
|---|---|---|---|---|
| Client connections | yes | UDP/WSS endpoint selection, TLS validation, async connection | Endpoint tests and real TLS proxy admission | proved locally |
| Bevy menu and CLI | yes | Accept Render URL and preserve LAN host | Options tests and game compile; GUI play untested | proved in tests |
| Headless authority | yes | WS listener, HTTP health, PORT, same race rules | HTTP 200, PORT, real WS lifecycle | proved locally |
| Leave/failure/rejoin | yes | Close worker, bound queues, disconnect and ownership | Disconnect/reset, duplicate ID, failed connect, paused reader tests | proved locally |
| Snapshot/input/commands | yes | Same Renet channels over WS | Racing snapshots and loading recovery sequence tests | proved locally |
| Native platforms | yes | Rust TLS, no Steam SDK | Linux release build; other OS untested | partial |
| Offline and UDP | yes | Preserve old connection mode | Workspace tests and 16-client lossy UDP test | proved in tests |
| Assets and packaging | yes | Only simulation assets on server | Server-only staged bundle boots; Docker daemon unavailable | partial |
| Render deployment | yes | Container, health, secure public URL | Authenticated browser reached card verification before creation | blocked |
| Account/matchmaking/Steam | no | Direct URL connection needs none | Not part of this task | excluded |
| Archive engine and liveries | no | Frozen/unrelated | No edits | excluded |

## Checks completed

- `cargo test --workspace --lib --bins --locked`: game 82, net 6, sim 45 passed;
  game 14 and sim 2 environment-dependent tests ignored.
- `cargo test -p racecraft-net --test lifecycle --locked`: 18 passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Linux release server and examples built with `--locked`.
- Exact container startup command with `PORT=19117`: HTTP `/health` returned 200.
- Local TLS proxy: untrusted certificate rejected, wrong hostname rejected,
  trusted WSS game admission and HTTPS health passed. No certificate checks disabled.
- Real Spa/RB6, 16 scripted WebSocket clients, 15 seconds: all clients current at
  tick 5436, 388–454 observed authoritative updates, 3,190-byte final snapshot,
  zero overloads. Worst observed server update was 8.56 ms on this machine.
  This is a local capacity diagnostic, not Render capacity or human-play proof.
- Prepared asset hashes match source simulation inputs. Missing input and path
  traversal are refused before writing a deployment bundle.

During verification, the TLS crypto backend was made explicit and WebSocket
read/write halves were separated. The socket-to-Renet handoff preserves packets;
Renet's unreliable state channel can still supersede stale snapshots, while its
reliable channel retries control messages. A paused-reader regression test covers recovery.

## Remaining deployment work

The deployment branch was pushed at `e92b4bb59d954f70b8302e22208bc0645385f1ce`.
The authenticated Render browser was configured for Docker, Singapore, Starter
at $7/month, this branch, `tools/package/render.Dockerfile`, `/health`, and
auto-deploy off. Creating the service is blocked by Render's Add Card dialog.
Render states that card verification places a temporary $1 authorization, not
a charge. No service was created and the reported $50 credit was not verified.

First resume action: after the user completes card verification in Render,
create the configured service, inspect its build, and verify public `/health`,
WSS admission, racing and service restart. No API key is needed for the current
authenticated browser session.

Untested: Docker image boot (no local daemon), Windows/macOS runtime, two people
driving on different computers, internet latency, and capacity on Render Starter.
