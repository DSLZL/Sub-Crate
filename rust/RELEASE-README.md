# Sub-Store Rust

Run `sub-store-rust.exe` from Command Prompt on Windows, or
`./sub-store-rust` on Linux. In PowerShell, use `./sub-store-rust.exe`.
No Node, npm or other application runtime is required.

The backend listens on all IPv4 interfaces at `0.0.0.0:3000` by default.
Open <http://127.0.0.1:3000/api/utils/env> to check the connection, then enter
`http://127.0.0.1:3000` in the backend settings at
<https://sub-store.vercel.app/subs>.

To restrict access to this computer, set `SUB_STORE_BACKEND_API_HOST=127.0.0.1`.
For example, in Command Prompt:

```cmd
set "SUB_STORE_BACKEND_API_HOST=127.0.0.1" && sub-store-rust.exe
```

Or in PowerShell:

```powershell
$env:SUB_STORE_BACKEND_API_HOST = '127.0.0.1'
.\sub-store-rust.exe
```

Set `SUB_STORE_BACKEND_API_PORT` to change the port. An explicit IPv6 host
is still supported. If the direct check opens but the online frontend cannot
connect, check the browser's local network access permission for that site.

Data is stored in the current working directory. Set `SUB_STORE_DATA_BASE_PATH`
to use another directory. Preserve `sub-store.json`, `root.json` and their
`*.pre-rust.json` backups when upgrading. Stop the previous process before
starting its replacement; run only one backend against a writable data directory.

Use the general JavaScript variant of subscription scripts. Node-specific APIs,
`require`, npm packages and native addons are unsupported.

The package contains the executable, this README, `LICENSE`,
`THIRD_PARTY_NOTICES.txt`, `SOURCE.md` and `manifest.json`. The notices retain
dependency copyright and license texts. `SOURCE.md` links the exact source and
full build/configuration documentation; `manifest.json` records file hashes,
target and build provenance. Linux binaries require glibc 2.38 or newer.
