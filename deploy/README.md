# Running SolidCraft as a web app

SolidCraft runs entirely in the browser (WebAssembly). Two small containers serve it and keep your
designs:

| Container | What it does |
|---|---|
| `solidcraft` | Caddy: serves the web app and passes `/api` and `/library` to the store |
| `store` | A small Rust server that keeps your saved designs, and every earlier version, in a Docker volume |

You need Docker with the Compose plugin. Nothing else is installed on the host.

## Quick start

```sh
git clone <this repository> && cd <repository>
docker compose -f deploy/compose.yaml up --build -d
```

Open <http://localhost:8080>. The first build compiles the whole app and takes a while (10-20
minutes, and about 8-12 GB of free RAM for the final WebAssembly link). Later builds reuse Docker's
cache.

Short on memory? Lower the parallelism and build first:

```sh
docker compose -f deploy/compose.yaml build --build-arg JOBS=2
docker compose -f deploy/compose.yaml up -d
```

The Dockerfiles pick the right Trunk download for x86-64 and ARM64. The x86-64 path is the one that
has been run; ARM64 has not been tried yet.

## Where your designs live

Save a design in the app and it is sent to the store in the background. Open the app in another
browser or device and the designs appear in the **Files in this browser** panel (the app checks the
store on start and every 30 seconds). If the same design changed in both places, the newer one wins
and the older one is kept next to it as `name (server copy)` or `name (this browser)`; nothing is
overwritten without a copy.

- `http://localhost:8080/library` lists every design and every earlier version, including deleted
  designs, with download links. It is a plain page with no scripts, so it works on any device.
- The store keeps the newest 100 versions of each design (`STORE_KEEP_VERSIONS` in `compose.yaml`).
- Deleting a design in the app removes it from the store's current list; its versions stay in the history.

### Backups

Everything is in the Docker volume `deploy_projects` (`projects/` holds current designs and
`history/` holds earlier versions). To back it up:

```sh
docker run --rm -v deploy_projects:/data -v "$PWD":/backup alpine tar czf /backup/solidcraft-store.tgz -C /data .
```

To keep the data in a normal folder instead, replace the volume in `compose.yaml` with a bind mount.
The store runs as user `65532`, so the folder must belong to it (`sudo chown -R 65532:65532 ./data`),
and on SELinux systems (Fedora, RHEL) add `:z` to the mount (`./data:/data:z`).

## Using it from other devices

By default the app listens on `127.0.0.1` only. Browsers only enable the features SolidCraft needs
(private file storage, WebGPU) on **`localhost` or HTTPS**, so plain `http://192.168.x.x:8080` will
not work: the app falls back to local storage and does not sync.

Reach it from other devices through something that provides HTTPS, for example:

- **Tailscale:** `tailscale serve --bg 8080` on the host, then open the `https://<machine>.<tailnet>.ts.net` address.
- **A reverse proxy or tunnel with a certificate** (Caddy, Cloudflare Tunnel, ...) in front of port 8080.

Set `SOLIDCRAFT_BIND` and `SOLIDCRAFT_PORT` to change where it listens.

## Security

There are **no accounts**: anyone who can reach the port can read and change your designs. That is
fine on `localhost` or a private network such as Tailscale. Do not expose it to the internet without
putting authentication in front of it. The store is not published on any port; only the web server
container can reach it, and changes to it need a header that a web page from another site cannot send.

## Updating and developing

```sh
git pull
docker compose -f deploy/compose.yaml up --build -d
```

To iterate on the app quickly, build it on the host (Rust 1.95, `wasm32-unknown-unknown`, and
[Trunk](https://trunkrs.dev)) and package that instead of rebuilding inside Docker:

```sh
(cd apps/solidcraft-web && trunk build --release)
docker compose -f deploy/compose.yaml -f deploy/compose.prebuilt.yaml up --build -d
```

The store has its own tests: `cd deploy/store && cargo test`.

## Troubleshooting

- **The build is killed, or Docker restarts, during the WebAssembly link.** Not enough memory for
  Docker. Use `--build-arg JOBS=2`, or give Docker more RAM (Docker Desktop: Settings, Resources).
- **A blank page.** Open `http://localhost:8080/?webgl` (forces WebGL2 where WebGPU is missing).
- **Designs don't appear on another device.** Check that you opened it over HTTPS or on `localhost`
  (see above), then look at `http://<address>/library` to confirm the store has them.
- **Logs:** `docker compose -f deploy/compose.yaml logs store`.
