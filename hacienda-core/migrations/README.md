# Migration numbering convention

`sqlx`'s `_sqlx_migrations` tracking table is one table per physical Postgres
database, not one per crate. Every crate in this workspace that embeds its own
`sqlx::migrate!("./migrations")` and may run against the same database as
another such crate (e.g. via one shared `ApiState`/`HaciendaFacade` deployment)
must reserve a non-overlapping block of migration version numbers here, so two
crates never both claim the same version.

| Range       | Crate           | Migrations directory                |
|-------------|-----------------|--------------------------------------|
| `0001-0099` | `hacienda-core` | `hacienda-core/migrations/`          |
| `0100-0199` | `hacienda-rag`  | `crates/hacienda-rag/migrations/`    |

Reserve the next hundred-block here (in a new row above) before adding a
`sqlx::migrate!`-backed migrations directory to another crate. Version numbers
come from the file's `<VERSION>_<DESCRIPTION>.sql` prefix; sqlx resolves them by
that integer regardless of directory, so gaps within a reserved range are fine.
