## Changes
- `NO_CREDENTIAL` responses now return `503 Service Unavailable` to signal credential pool exhaustion; docs align with the behavior.
- Added an authenticated `/v1beta/models` endpoint that serves the embedded Gemini catalog so clients can fetch model listings through Nexus.
