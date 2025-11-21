# v0.2.0

## Breaking change
- Credentials schema changed: the `credentials` table now only stores `email`, `project_id`, `refresh_token`, `access_token`, `expiry`, and `status`. The former `client_id`, `client_secret`, and `scopes` columns were removed because OAuth client config now comes from build-time env vars. Existing SQLite databases from 0.1.x are incompatible—drop/recreate the DB (or `DROP TABLE credentials;`) and re-import credentials.

## Upgrade notes
- Stop the service and remove the old SQLite file (default `data.db`) or drop the `credentials` table, then restart to let it bootstrap the new schema.

