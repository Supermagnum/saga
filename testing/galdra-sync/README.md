# Galdra sync export fixture (Part D)

Schema-faithful SQLite package for the Saga one-way importer.

- `fixture-export.sqlite` — `schema_migrations` versions 1–8 and an `identities`
  table matching Galdralag-firmware `galdra-core-host` (pinned rev
  `b781bf6fa5ccf8700a5fb061583c31cfa00241a1`).
- Importable row: **Alice Import** with raw Brainpool P-256 SEC1 in `pgp_pubkey`.
- Non-importable rows: garbage pubkey and missing key.
