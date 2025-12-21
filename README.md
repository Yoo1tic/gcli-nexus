# Gcli-Nexus

> High-performance Gemini CLI reverse proxy that talks to the raw Cloud Code Gemini endpoints while presenting Gemini-native responses.

## Highlights

- **Gemini-native proxy for the official CLI**: accepts `/v1beta/models/{model}:generateContent` and `:streamGenerateContent` payloads from `geminicli` while converting upstream CLI envelopes back into the standard Gemini response shape.
- **SSE-friendly normalization**: streaming events land in the Gemini-native `candidates/usageMetadata/modelVersion` shape so dashboards and SDKs can consume them directly.
- **Credential pool with actor scheduling**: a `ractor`-driven worker manages Google OAuth credentials stored in SQLite, separates “big” and “tiny” model queues, cools down projects that hit 429, and refreshes tokens only when a credential is near expiry or fails authentication.
- **Operable out of the box**: `.env` configuration via Figment/dotenvy, SQLite (`data.db`) that bootstraps automatically, structured tracing, and a `mimalloc` global allocator for predictable latency.
- **One-click browser auth**: hitting `/auth` in a browser jumps straight to Google OAuth for login/consent.

## Quick start

### Prerequisites

- Google Cloud projects that already have Gemini CLI access; export each account as the JSON blob that contains `client_id`, `client_secret`, `refresh_token`, `project_id`, etc.
- For the prebuilt binary: Linux host with SQLite available (no Rust toolchain required).
- For containers: Docker + docker compose. Building from source remains possible with Rust 1.78+ if needed.

### Run the prebuilt binary

1. Copy the sample environment file and fill in secrets:
   ```bash
   cp .env.example .env
   # edit NEXUS_KEY plus (optionally) DATABASE_URL, BIGMODEL_LIST, PROXY...
   ```
2. Drop every Gemini credential JSON into the folder referenced by `CRED_PATH` (default `./credentials`). On startup the actor will normalize, refresh, and persist them into SQLite. Additions today require a restart to be ingested.
3. Download the latest release binary for your platform, make it executable, and run it from the project root:
   ```bash
   chmod +x gcli-nexus
   ./gcli-nexus
   ```
   The server binds `0.0.0.0:8188`. Logs reveal how many credentials were activated and whether a proxy is in use.

### Run with docker compose

1. Copy the compose template and set secrets:
   ```bash
   cp docker-compose.yml.example docker-compose.yml
   # edit NEXUS_KEY and other options in docker-compose.yml
   ```
2. Ensure local folders exist for persistence and credentials:
   ```bash
   mkdir -p data credentials
   # place credential JSON files under ./credentials
   ```
3. Start the stack:
   ```bash
   docker compose up -d
   ```
   The service listens on `0.0.0.0:8188` and stores SQLite data under `./data`.

## Configuration

| Env var                              | Required | Default            | Description                                                                                                           |
| ------------------------------------ | -------- | ------------------ | --------------------------------------------------------------------------------------------------------------------- |
| `NEXUS_KEY`                          | Yes      | _none_             | Shared secret checked on every request via `x-goog-api-key`, `Authorization: Bearer`, or `?key=`.                     |
| `DATABASE_URL`                       | No       | `sqlite://data.db` | SQLite DSN; the actor creates the file/migrations automatically.                                                      |
| `LOGLEVEL`                           | No       | `info`             | Tracing level (`error`, `warn`, `info`, `debug`, `trace`). `RUST_LOG` still works as a fallback.                      |
| `BIGMODEL_LIST`                      | No       | `[]`               | JSON array of model names treated as “big”. They get their own queue/cooldown bucket to avoid starving lighter chats. |
| `CRED_PATH`                          | No       | unset              | Directory that is scanned once during startup for credential JSON; leave unset to rely purely on SQLite contents.     |
| `OAUTH_TPS`                          | No       | `10`               | OAuth refresh requests per second; refresh buffer/burst sizes are derived as `OAUTH_TPS * 2`.                          |
| `GEMINI_RETRY_MAX_TIMES`             | No       | `3`                | Max retry attempts for Gemini CLI upstream calls.                                                                      |
| `ENABLE_MULTIPLEXING`                | No       | `false`            | Allow outbound reqwest clients to use HTTP/2 multiplexing; keep `false` to force HTTP/1-only behavior.                |
| `PROXY`                              | No       | unset              | Outbound HTTP proxy applied to both the Gemini caller and the OAuth refresh client (supports HTTP/SOCKS).             |
| `DATABASE_URL`, `PROXY`, `CRED_PATH` | —        | —                  | Accept absolute or relative paths; Figment merges `.env` values automatically.                                        |

### Credential lifecycle

1. **Ingestion**: Each JSON file is parsed via `GoogleCredential::from_payload`, refreshed immediately, and upserted into SQLite. Duplicate `project_id`s are replaced atomically.
2. **Queues**: Active credentials are pushed into both the “big” and “tiny” queues; requests choose a queue based on whether `model` matches `BIGMODEL_LIST`.
3. **Rate limits**: When a 429 response contains `quotaResetTimeStamp`, the actor parks the credential for that many seconds before putting it back in queue.
4. **Refresh flow**: 401/403 responses trigger `ReportInvalid` → refresh pipeline → DB update → re-enqueue. Failing refreshes disable the credential (status=false).
5. **Persistence**: Because the DB is authoritative, restarts reuse the latest access tokens/expiry timestamps without re-reading every JSON file.

## API usage

### Authentication

- Send `x-goog-api-key: <NEXUS_KEY>` (preferred).
- Or append `?key=<NEXUS_KEY>` to the request URL.
- Visit `/auth` in a browser to be redirected to Google OAuth for login/consent.

### Generate content (non-streaming)

```bash
curl -X POST http://localhost:8188/v1beta/models/gemini-2.5-pro:generateContent \
  -H "x-goog-api-key: $NEXUS_KEY" \
  -H "Content-Type: application/json" \
  -d '{
        "contents":[{"role":"user","parts":[{"text":"hello from gcli-nexus"}]}]
      }'
```

### Streaming

```bash
curl --no-buffer -X POST \
  http://localhost:8188/v1beta/models/gemini-2.5-pro:streamGenerateContent \
  -H "x-goog-api-key: $NEXUS_KEY" \
  -H "Content-Type: application/json" \
  -d '{"contents":[{"role":"user","parts":[{"text":"stream"}]}]}'
```

### Error semantics

- `401/403` from upstream map to a temporary `502/500` locally after a refresh attempt; the credential is refreshed before reuse.
- `429` returns upstream headers/body untouched while the offending credential cools down.
- `503` with `{"error":"no available credential"}` means all queues are empty or cooling—add more credentials or wait for cooldowns.

## Operations

- **Logging**: Structured tracing goes to stdout; set `LOGLEVEL=debug` for detailed actor logs (queue lengths, refresh states). Use `RUST_LOG` for per-module overrides.
- **Database**: `data.db` lives at the path inside `DATABASE_URL`; backup the file periodically if you care about history.
- **Proxying**: Set `PROXY` (e.g. `http://127.0.0.1:1080`) if your network requires outbound proxying; both Gemini traffic and OAuth refresh calls use it.
- **Credential rotation**: Update the JSON file, restart the binary, or seed SQLite manually; the actor upserts by `project_id`.
- **Security**: Treat `.env`, `credentials/*.json`, and `data.db` as sensitive—they contain refresh and access tokens.

## License

This project is distributed under the [MIT License](LICENSE).
