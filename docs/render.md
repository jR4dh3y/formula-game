# Host multiplayer on Render

The native game can connect directly to a Render web service using secure
WebSockets. No Steam account, Steam App ID, or Spacewar is involved. UDP remains
available for direct LAN/VPN hosting.

## Build and assets

Use the same source revision and converted simulation assets for the server and
every client. The existing release and the original UDP-only PR #5 binaries do
not contain this WebSocket transport. Build new clients from this revision:

```sh
cargo build --release --locked --workspace
```

Prepare a small server-only bundle from the assets used by your players:

```sh
python3 tools/package/prepare_render.py --assets /path/to/converted/assets --car rb6 --track spa
```

This writes `deploy/render/assets`, `rules.json` and an asset SHA-256 manifest.
It includes track geometry and car simulation inputs. It excludes models,
textures, audio and any GPU dependency. `--car gen22` includes its team roster.
The generated bundle must be included in the deployment's source revision.
Keep your source assets; the preparation command does not replace them.

The default rules are three laps, no AI, full AI power, automatic gears and
medium tyres. Legacy rules files without `ai_power` default that engine setting
to 1.0.
The first player to join owns the room and can change supported lobby rules.
Circuit and car are selected by the staged bundle and require a new deployment to change.

## Deploy

The root `render.yaml` defines one paid Starter web service in Singapore using
`tools/package/render.Dockerfile`. It does not provision a database, disk, or
additional services. Render listed Starter at $7/month when checked on 2026-11-03. Confirm the current
price and credit eligibility in your workspace before creating it. Starter's ability to handle your desired
player count needs measurement on the deployed machine.

Once the source and generated bundle are published to your repository, create a
Render Blueprint from that branch. Automatic deploys are disabled so code changes
do not unexpectedly end a race. Render builds only the headless Rust server,
binds HTTP to `0.0.0.0:$PORT`, and checks `/health`. The container runs as an
unprivileged user. Render terminates TLS and forwards `/ws` to the server.

A manual local startup uses the same command as the container:

```sh
PORT=10000 ./target/release/racecraft-server --websocket \
  --assets ./deploy/render/assets --settings ./deploy/render/rules.json
```

For a local container check:

```sh
docker build -f tools/package/render.Dockerfile -t racecraft-render .
docker run --rm -p 10000:10000 -e PORT=10000 racecraft-render
```

The health endpoint returns 503 if the simulation loop has not updated within
eight seconds. A process restart or redeploy ends the running race; rooms are
in memory. Players can join again to create a new room after startup. This is
not seamless reconnect to an ongoing race.

## Join

Open **Multiplayer**, enter your name and this join address:

```text
wss://YOUR-SERVICE.onrender.com
```

The game adds `/ws` automatically. The equivalent CLI command is:

```sh
./target/release/f1-the-game --assets ./assets --car rb6 \
  --join wss://YOUR-SERVICE.onrender.com --name Alice
```

Use the car matching your deployment. Select Ready, then the owner starts the
race. A local development server accepts `ws://127.0.0.1:10000`. Public Render
connections use `wss://`; certificates and hostnames are checked using the OS
trust store. An `https://` URL is not a game connection address.

The server still supports its original UDP mode:

```sh
./target/release/racecraft-server --assets ./assets --bind 0.0.0.0:7777
```

That command is useful on LAN/VPN or a UDP-capable host. Render does not expose
its UDP port to internet clients.

## Verify the deployment

1. Check `https://YOUR-SERVICE.onrender.com/health` returns HTTP 200.
2. Connect two clients from this revision using the public WSS URL.
3. Ready both, start, load the same grid and confirm advancing authoritative
   snapshots and player control in both clients.
4. Leave as owner and confirm ownership passes; leave everyone, then join again.
5. Restart the service and confirm the clients report disconnection and can join
   a new lobby afterward.

For a local 16-scripted-client WebSocket soak on converted Spa/RB6 assets:

```sh
cargo run --release --locked -p racecraft-net --example soak -- ./assets 30 --websocket
```

The existing headless diagnostic client accepts WSS URLs too:

```sh
cargo run --release --locked -p racecraft-net --example bot -- \
  wss://YOUR-SERVICE.onrender.com ./assets "Connection probe"
```

That probe proves admission. Two people driving, long-distance latency and the
capacity of a particular Render plan require separate runtime checks. WebSockets
run over TCP, so packet loss can delay newer updates behind an older packet.
