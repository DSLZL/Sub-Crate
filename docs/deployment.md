# Deployment

This guide covers installing Sub-Crate, connecting a frontend, and moving data from or back to Sub-Store.

Sub-Crate has no login. Anyone who can reach its port can read and change your subscriptions, so do not expose it to untrusted networks. The native binary listens on all IPv4 interfaces by default; set `SUB_STORE_BACKEND_API_HOST=127.0.0.1` to accept connections from this computer only.

## Native binary

Download from [Releases](https://github.com/DSLZL/Sub-Crate/releases):

| Platform | Archive | Requirement |
| --- | --- | --- |
| Linux x64 | `sub-crate-<version>-linux-x64.tar.gz` | glibc 2.38 or newer |
| Linux arm64 | `sub-crate-<version>-linux-arm64.tar.gz` | glibc 2.38 or newer |
| Windows x64 | `sub-crate-<version>-windows-x64.zip` | None; the C runtime is linked statically |

On older Linux distributions, use the [Docker image](#docker) instead.

Verify the archive against `SHA256SUMS` from the same release, then extract and start it.

Linux:

```sh
sha256sum --ignore-missing -c SHA256SUMS
tar -xzf sub-crate-<version>-linux-x64.tar.gz
cd sub-crate-<version>-linux-x64
mkdir -p data
SUB_STORE_DATA_BASE_PATH="$PWD/data" SUB_STORE_BACKEND_API_HOST=127.0.0.1 ./sub-crate
```

Windows PowerShell:

```powershell
(Get-FileHash .\sub-crate-<version>-windows-x64.zip -Algorithm SHA256).Hash  # compare with SHA256SUMS
Expand-Archive .\sub-crate-<version>-windows-x64.zip .
Set-Location .\sub-crate-<version>-windows-x64
New-Item -ItemType Directory -Force data | Out-Null
$env:SUB_STORE_DATA_BASE_PATH = (Join-Path $PWD data)
$env:SUB_STORE_BACKEND_API_HOST = '127.0.0.1'
.\sub-crate.exe
```

Open <http://127.0.0.1:3000/api/utils/env> to check that the backend is running. The native binary does not read `.env` files; set variables in the shell or service manager. See [Configuration](configuration.md) for every option.

## Docker

Images are published to `ghcr.io/dslzl/sub-crate` for `linux/amd64` and `linux/arm64`. Each release is tagged `vX.Y.Z` and `X.Y.Z`; `latest` follows the newest release. The image contains no Node.js, npm or Python.

### Compose

From a checkout of this repository:

```sh
cp .env.example .env
docker compose -f docker-compose.example.yml up -d
```

The example publishes `127.0.0.1:3000`, stores data in `./data` and runs with a read-only root filesystem. Edit `.env` to pin another image tag, change the bind address or port, and set runtime options; then run `up -d` again to apply the changes. Keep `./data` when recreating the container.

### docker run

```sh
mkdir -p data
docker run -d --name sub-crate --restart unless-stopped \
  --read-only --tmpfs /tmp --user "$(id -u):$(id -g)" \
  -p 127.0.0.1:3000:3000 -v "$PWD/data:/data" \
  ghcr.io/dslzl/sub-crate:latest
```

The mounted directory must be writable by the chosen user. Add `-e TZ=Asia/Shanghai` (or your zone) so schedules and export file names use local time.

### Build the image yourself

```sh
docker build -t sub-crate:local .
```

Set `SUB_CRATE_IMAGE=sub-crate:local` in `.env` and start Compose with `up -d --pull never`.

## Frontend

Sub-Crate does not bundle a frontend. Choose one of:

- **Hosted frontend.** Open <https://sub-store.vercel.app/subs> and enter your backend URL, such as `http://127.0.0.1:3000`. If the browser blocks the connection, allow the site to access your local network.
- **Served by Sub-Crate on the same port.** Build or download the [official frontend](https://github.com/sub-store-org/Sub-Store-Front-End), then set:

  ```sh
  SUB_STORE_FRONTEND_PATH=/path/to/dist
  SUB_STORE_BACKEND_MERGE=true
  SUB_STORE_FRONTEND_BACKEND_PATH=/backend
  ```

  Open `http://127.0.0.1:3000/subs?api=http%3A%2F%2F127.0.0.1%3A3000%2Fbackend`. With Docker, mount `dist` read-only at `/frontend` and set `SUB_STORE_FRONTEND_PATH=/frontend`.
- **Served on a separate port.** Leave `SUB_STORE_BACKEND_MERGE` unset, keep `SUB_STORE_FRONTEND_PATH` and `SUB_STORE_FRONTEND_BACKEND_PATH=/backend`, and add `SUB_STORE_BACKEND_PREFIX=true`, `SUB_STORE_FRONTEND_PORT=3001` and `SUB_STORE_CORS_ALLOWED_ORIGINS=http://127.0.0.1:3001`. Open `http://127.0.0.1:3001/subs?api=http%3A%2F%2F127.0.0.1%3A3001%2Fbackend`.

## Data

Sub-Crate uses Sub-Store's data files, `sub-store.json` and `root.json`, in `SUB_STORE_DATA_BASE_PATH`. The first time it starts with existing files, it copies them to `sub-store.pre-rust.json` and `root.pre-rust.json`; later starts never overwrite these snapshots.

- Run only one backend against a data directory at a time.
- If a data file is corrupt, Sub-Crate refuses to start and leaves the file untouched.
- If a restore is interrupted, the next start returns both files to their state before the restore.
- Backups made with the frontend or with Gist (plain, Base64 or age-encrypted) are interchangeable with Sub-Store. Keep your age identity somewhere safe.

## Migrate from Sub-Store

1. Stop Sub-Store.
2. Copy its whole data directory. Keep the original.
3. Start Sub-Crate with `SUB_STORE_DATA_BASE_PATH` pointing at the copy.
4. Check your subscriptions in the frontend. Scripts that use Node.js APIs keep their saved text but fail when run; switch them to their general JavaScript variants (see [Scripts](scripts.md)).

## Upgrade

Stop the running process or container, replace the binary or image tag, and start it again against the same data directory.

## Roll back to Sub-Store

1. If Sub-Crate stopped during a restore, start it once so it can undo the partial restore, then stop it.
2. Copy the data directory again.
3. Start Sub-Store against the copy. To discard every change made under Sub-Crate, use the `*.pre-rust.json` snapshots instead, renamed to `sub-store.json` and `root.json`.

Keep the earlier copies until you have checked the result in the frontend.
