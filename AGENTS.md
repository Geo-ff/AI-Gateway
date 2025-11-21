# Repository Guidelines

## Project Structure & Module Organization
- Root Rust service crate in `src/` (`server`, `routing`, `providers`, `db`, `config`, `admin`, etc.).
- Benchmarks live in `benches/`; sample data and templates in `data/`; top‑level TOML files hold runtime configuration.
- Web dashboard: `getway/` (Vue + Vite). Terminal UI: `tui/` (Rust crate).

## Build, Test, and Development Commands
- Gateway API: run `cargo build` and `cargo run` from the repo root.
- Benchmarks: `cargo bench --bench endpoints` for performance baselines.
- TUI client: `cd tui && cargo run`.
- Web UI: `cd getway && pnpm install && pnpm dev` for local development, `pnpm build` for production assets.

## Coding Style & Naming Conventions
- Rust: use `cargo fmt` before committing; prefer `snake_case` for modules/functions, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants.
- Organize Rust code by domain (e.g., route handlers in `routing`, DB access in `db`, configuration in `config`).
- Vue/TypeScript: 2‑space indentation; Vue SFCs in `PascalCase.vue`; composables named `useX`.

## Testing Guidelines
- Rust tests live alongside code in `src/` or under `tests/` when added; run with `cargo test`.
- Prefer small, focused tests that exercise public interfaces and critical paths (auth, routing, DB access).
- For benchmarks, keep scenarios realistic and document assumptions in the benchmark module.

## Commit & Pull Request Guidelines
- Follow Conventional Commits: `feat(scope): summary`, `fix(scope): summary`, `chore(scope): summary`, etc. Scopes match domains like `admin`, `auth`, `deps`, `benchmark`.
- Every PR should describe the change, rationale, and impact; link related issues and mention migration steps or config changes.
- For UI changes (`getway`, `tui`), include screenshots or short recordings when helpful.

## Security & Configuration Tips
- Do not commit real credentials, API keys, or production database URLs. Use local overrides instead of editing shared example configs.
- Before opening a PR, verify that configuration changes are backward compatible or clearly documented.

