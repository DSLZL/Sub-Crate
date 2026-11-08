# Sub-Crate

Sub-Crate is a Rust backend for Sub-Store. It needs no Node.js.

## Start

- Linux: `./sub-crate`
- Windows: `sub-crate.exe` (in PowerShell, `.\sub-crate.exe`)

Open <http://127.0.0.1:3000/api/utils/env> to check that it is running. Then open <https://sub-store.vercel.app/subs> and set the backend address to `http://127.0.0.1:3000`. If the page cannot connect, allow it to access your local network in the browser.

## Settings

Set these environment variables before starting:

| Variable | Default | Purpose |
| --- | --- | --- |
| `SUB_STORE_BACKEND_API_HOST` | `0.0.0.0` (all interfaces) | Use `127.0.0.1` to accept only this computer |
| `SUB_STORE_BACKEND_API_PORT` | `3000` | Port |
| `SUB_STORE_DATA_BASE_PATH` | current directory | Where `sub-store.json` and `root.json` are kept |

Command Prompt: `set "SUB_STORE_BACKEND_API_HOST=127.0.0.1" && sub-crate.exe`
PowerShell: `$env:SUB_STORE_BACKEND_API_HOST = '127.0.0.1'; .\sub-crate.exe`

Sub-Crate has no login, so do not expose its port to untrusted networks. All options are listed in the project's `docs/configuration.md`.

## Upgrading and moving from Sub-Store

Stop the old backend first, and run only one backend per data directory. Keep a copy of your data. On its first start Sub-Crate also saves your existing files as `*.pre-rust.json`.

## Scripts

Use the general JavaScript version of a script. Node.js modules, `require`, npm packages and native addons are not available.

## Package contents

`sub-crate` or `sub-crate.exe`, this `README.md`, `LICENSE`, `THIRD_PARTY_NOTICES.txt` and `SOURCE.md`, which links the exact source. Verify the archive with the release's `SHA256SUMS`. Linux builds need glibc 2.38 or newer.
