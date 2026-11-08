# Configuration

Sub-Crate reads Sub-Store's environment variable names, so an existing Sub-Store configuration keeps working. Empty values count as unset. Flags marked *flag* are enabled by any non-empty value, including `false`; leave them unset to disable them.

The native binary does not read `.env` files. With Docker Compose, put these variables in `.env`.

## Listener and data

| Variable | Default | Description |
| --- | --- | --- |
| `SUB_STORE_BACKEND_API_HOST` | `0.0.0.0` | Address to listen on. Use `127.0.0.1` for local access only; IPv6 addresses are accepted. |
| `SUB_STORE_BACKEND_API_PORT` | `3000` | Port to listen on. |
| `SUB_STORE_DATA_BASE_PATH` | current directory | Directory holding `sub-store.json` and `root.json`. |
| `TZ` | system zone | Time zone for schedules and export file names, such as `Asia/Shanghai`. |

## Frontend hosting

See [Deployment](deployment.md#frontend) for complete examples.

| Variable | Default | Description |
| --- | --- | --- |
| `SUB_STORE_FRONTEND_PATH` | unset | Frontend `dist` directory to serve. |
| `SUB_STORE_BACKEND_MERGE` | unset | *Flag.* Serve the frontend on the backend port. Without it, the frontend gets its own listener. |
| `SUB_STORE_FRONTEND_HOST` | backend host | Address of the separate frontend listener. |
| `SUB_STORE_FRONTEND_PORT` | `3001` | Port of the separate frontend listener. |
| `SUB_STORE_FRONTEND_BACKEND_PATH` | unset | Path where the API is mounted, such as `/backend`. Must start with `/`. Required with `SUB_STORE_BACKEND_MERGE` or `SUB_STORE_BACKEND_PREFIX`. |
| `SUB_STORE_BACKEND_PREFIX` | unset | *Flag.* Serve the API under `SUB_STORE_FRONTEND_BACKEND_PATH` on the backend port. |

## HTTP

| Variable | Default | Description |
| --- | --- | --- |
| `SUB_STORE_CORS_ALLOWED_ORIGINS` | `https://sub-store.vercel.app`, `http://substore.stash`, `https://substore.stash` | Comma-separated origins allowed to call the API. `*` allows any origin. |
| `SUB_STORE_BACKEND_CUSTOM_NAME` | unset | Backend name shown in the frontend. With the default origin list, it also allows `http://localhost` and `http://127.0.0.1` origins. |
| `SUB_STORE_BODY_JSON_LIMIT` | `1mb` | Largest accepted JSON request body. Accepts units such as `512kb` and `1.5mb`. |
| `SUB_STORE_MAX_HEADER_SIZE` | `32768` | Largest response header, in bytes, accepted by script requests and downloads. |
| `SUB_STORE_X_POWERED_BY` | `Sub-Store` | Value of the `X-Powered-By` response header. |
| `NODE_ENV` | unset | `production` hides error traces in HTML error pages. |

## Scripts and downloads

| Variable | Default | Description |
| --- | --- | --- |
| `SUB_STORE_BACKEND_DEFAULT_PROXY` | unset | Fallback proxy for downloads and Gist requests when neither the request nor the frontend settings set one. |
| `SUB_STORE_PUSH_SERVICE` | unset | Notification target for scripts. An `http(s)://` URL is requested with `[推送标题]` and `[推送内容]` replaced by the title and content; other values go to the bundled notification library. |

## Schedules

Cron expressions have six fields with seconds (`0 0 */6 * * *`) or five without; they run in `TZ`.

| Variable | Description |
| --- | --- |
| `SUB_STORE_BACKEND_SYNC_CRON` | Synchronize artifacts. |
| `SUB_STORE_BACKEND_UPLOAD_CRON` | Upload a backup to the Gist configured in the frontend. |
| `SUB_STORE_BACKEND_DOWNLOAD_CRON` | Restore the backup from that Gist. |
| `SUB_STORE_PRODUCE_CRON` | Pre-generate outputs. Entries are `cron,sub,name` or `cron,col,name`, separated by `;`. |
| `SUB_STORE_MMDB_CRON` | Refresh the MMDB databases below. |
| `SUB_STORE_BACKEND_CRON`, `SUB_STORE_CRON` | Deprecated. Only logs a warning; use `SUB_STORE_BACKEND_SYNC_CRON`. |

## MMDB databases

Set each URL together with the file it is saved to.

| Variable | Description |
| --- | --- |
| `SUB_STORE_MMDB_COUNTRY_URL`, `SUB_STORE_MMDB_COUNTRY_PATH` | Country database download URL and local path. |
| `SUB_STORE_MMDB_ASN_URL`, `SUB_STORE_MMDB_ASN_PATH` | ASN database download URL and local path. |

## Startup restore

| Variable | Description |
| --- | --- |
| `SUB_STORE_DATA_URL` | Download data from this URL and restore it at every start. The process exits if the restore fails. |
| `SUB_STORE_DATA_URL_POST` | JavaScript that transforms the downloaded data before it is restored. |

## Diagnostics

| Variable | Description |
| --- | --- |
| `SUB_STORE_PROFILE` | `1` prints per-request stage timings to standard output. |
