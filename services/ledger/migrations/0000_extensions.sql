-- Required Postgres extensions.
-- pgcrypto: digest() function used by post_ledger_transaction stored proc.

CREATE EXTENSION IF NOT EXISTS pgcrypto;
