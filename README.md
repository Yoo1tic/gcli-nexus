# Gcli-Nexus

[![GitHub Release](https://img.shields.io/github/v/release/Yoo1tic/gcli-nexus?style=flat&logo=github&color=blue)](https://github.com/Yoo1tic/gcli-nexus/releases/latest)
[![License](https://img.shields.io/badge/License-MIT-green?style=flat)](LICENSE)

**Gcli-Nexus is a high-performance Rust adapter that bridges Gemini CLI (Cloud Code) to the Standard Gemini API.**

It acts as a headless protocol bridge: feeding raw GCP service accounts turns into a drop-in `/v1beta/models` interface. It normalizes proprietary CLI streams into standard JSON/SSE, compatible with LangChain, curl, and modern AI clients.

### Highlights

- **Protocol Standardization**: Exposes native Gemini API endpoints (`:generateContent`, `:streamGenerateContent`) backed by Cloud Code credentials.
- **Actor-Driven Concurrency**: Built on `ractor` for zero-lock scheduling, enabling high throughput with minimal resource overhead.
- **Headless & Self-Healing**: Zero management UI. Traffic automatically scrubs invalid tokens and repairs the pool asynchronously.
- **Hot-Swapping**: Scale capacity instantly via the `/auth` endpoint without restarting the service or dropping connections.
- **Portable**: Ships as a single static binary (Linux/macOS/Windows) or a lightweight Docker container.

## API Endpoints

Authentication requires the `x-goog-api-key` header (or `?key=` query parameter).

| Endpoint                                       | Method | Auth | Description                                                       |
| :--------------------------------------------- | :----- | :--- | :---------------------------------------------------------------- |
| `/v1beta/models/{model}:generateContent`       | `POST` | ✅   | **Core Interface**. Standard chat completion (unary).             |
| `/v1beta/models/{model}:streamGenerateContent` | `POST` | ✅   | **Core Interface**. Standard chat completion (streaming).         |
| `/v1beta/models`                               | `GET`  | ✅   | Lists supported models in standard Gemini JSON format.            |
| `/v1beta/openai/models`                        | `GET`  | ✅   | Lists supported models in OpenAI-compatible format.               |
| `/auth`                                        | `GET`  | ❌   | **Hot-Swapping**. Initiates OAuth flow to inject new credentials. |
| `/oauth2callback`                              | `GET`  | ❌   | Internal callback handler for Google OAuth redirects.             |

## Quick Start

### Prerequisites

- **Google Account**: A Google account with access to Gemini CLI (Cloud Code).
- **Environment**:
- **Docker** (Recommended) for containerized deployment.
- **Linux Host** with SQLite if running the binary directly.

### 1. Start the Service

You can start Gcli-Nexus immediately with an empty credential pool.

#### Option A: Docker Compose (Recommended)

1. **Setup Directories**:

```bash
mkdir -p gcli-nexus/data
cd gcli-nexus
```

2. **Create Compose File**:
   Copy `docker-compose.yml.example` or create a new `docker-compose.yml`:

3. **Launch**:

```bash
docker compose up -d
```

#### Option B: Prebuilt Binary

1. **Prepare Environment**:

```bash
cp .env.example .env
# Edit .env to set NEXUS_KEY and MODEL_LIST
```

2. **Run**:

```bash
chmod +x gcli-nexus
./gcli-nexus
```

The server binds to `0.0.0.0:8188` by default.

### 2. Onboard Credentials (Instant & Dynamic)

Gcli-Nexus supports **Hot-Swapping**. You can add credentials at runtime without restarting the service.

#### Method A: Browser-Based Auto Ingestion (Easiest)

1. Navigate to `http://<your-server-ip>:8188/auth` in your browser.
2. Complete the Google OAuth login flow.
3. **Done.** The credential is automatically captured, persisted to SQLite, and **immediately injected** into the scheduling queue.
4. Repeat for as many accounts as needed.

#### Method B: Manual JSON File (Legacy)

If you already have credential JSON files (containing `project_id` and `refresh_token`), place them into the `credentials/` directory.

- **Docker**: Place files in the mapped `./credentials` volume.
- **Binary**: Place files in the directory referenced by `CRED_PATH`.

_Note: Files added manually usually require a restart to be ingested, whereas Method A is instant._

### Credential JSON Format (For Method B)

```json
{
  "project_id": "my-gcp-project",
  "refresh_token": "1//0gExampleRefreshToken"
}
```

_Only `project_id` and `refresh_token` are strictly required. Missing fields (like `access_token`) are automatically filled during the first refresh._

### Usage

Gcli-Nexus exposes a standard Gemini-compatible surface.

**Generate Content:**

```bash
curl -X POST http://localhost:8188/v1beta/models/gemini-2.5-pro:generateContent \
  -H "x-goog-api-key: $NEXUS_KEY" \
  -H "Content-Type: application/json" \
  -d '{
        "contents":[{"role":"user","parts":[{"text":"Hello World"}]}]
      }'

```

## Configuration

| Env var                  | Required | Default                                                      | Description                                                                      |
| ------------------------ | -------- | ------------------------------------------------------------ | -------------------------------------------------------------------------------- |
| `LOGLEVEL`               | No       | `info`                                                       | Logging verbosity for tracing (e.g. `error`, `warn`, `info`, `debug`, `trace`).  |
| `LISTEN_ADDR`            | No       | `0.0.0.0`                                                    | HTTP server listen address.                                                      |
| `LISTEN_PORT`            | No       | `8188`                                                       | HTTP server listen port.                                                         |
| `NEXUS_KEY`              | Yes      | `pwd`                                                        | Required Nexus API key used to authorize inbound requests.                       |
| `MODEL_LIST`             | No       | `"[gemini-2.5-flash, gemini-2.5-pro, gemini-3-pro-preview]"` | JSON array of Gemini models.                                                     |
| `CRED_PATH`              | No       | `./credentials`                                              | Optional directory containing credential JSON files to preload.                  |
| `OAUTH_TPS`              | No       | `5`                                                          | OAuth refresh requests per second (TPS) for the refresh worker.                  |
| `ENABLE_MULTIPLEXING`    | No       | `false`                                                      | Allow reqwest clients to use HTTP/2 multiplexing. Leave `false` to force HTTP/1. |
| `GEMINI_RETRY_MAX_TIMES` | No       | `3`                                                          | Max retry attempts for Gemini CLI upstream calls.                                |
| `PROXY`                  | No       | unset                                                        | Optional outbound HTTP proxy (`scheme://user:pass@host:port`). Remove if unused. |

## Technical Details

### 1. Dynamic Scalability (Hot-Swapping)

Adding capacity is instantaneous.

- **Zero-Touch Ingestion**: Visit the `/auth` endpoint to authenticate a new account. The credential is automatically persisted to the database and **immediately injected** into the scheduling loop.
- **No Restarts**: Scale your pool from 1 to 1,000 credentials at runtime without dropping a single connection.

### 2. Traffic-Driven Maintenance

We don't run expensive background cron jobs to check for expired tokens. Instead, we use live traffic as a probe.

- **Lazy Self-Healing**: A credential's validity is verified only when a request hits the proxy. Invalid tokens (401/403) are instantly quarantined and repaired asynchronously.
- **Auto-Convergence**: The higher the concurrency, the faster the system converges to a clean state.

### 3. Zero-Lock Concurrency

Built on the **Actor Model (Ractor)**, Gcli-Nexus eliminates the mutex contention that plagues traditional multi-threaded proxies.

- **In-Memory Scheduling**: The critical path (Client -> Actor -> Client) is purely single-threaded and non-blocking.
- **Decoupled IO**: Database writes (SQLite WAL) and OAuth refreshes are offloaded to detached workers, ensuring the proxy latency remains stable under load.

### 4. Precision Rate Limiting

Handling upstream Rate Limits (429) is a scheduling problem, not an error handling problem.

- **The Waiting Room**: Rate-limited credentials are parked in a **Binary Heap**.
- **O(1) Wakeups**: We strictly avoid polling. Credentials are reclaimed into the active queue at the exact millisecond their quota resets.

## License

This project is distributed under the [MIT License](LICENSE).
