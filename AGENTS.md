# Repository Guidelines

## Project Structure & Module Organization

- `src/` holds the Rust application: runtime wiring in `main.rs`, MCP handlers in `mcp_server.rs`, storage logic in `store.rs`, and the embedded web UI assets in `web/` (notably `index.html`).
- `examples/` provides sample `.xcstrings` files that mirror real-world payloads for manual testing.
- `README.md` documents runtime configuration, environment variables, and MCP integration steps; keep it aligned with code changes.

## Build, Test, and Development Commands

- `cargo build` — compile the project in debug mode; use `cargo build --release` for binaries you plan to ship to MCP clients.
- `cargo run -- -- <path/to/Localizable.xcstrings>` — start the MCP server and bundled web UI against a specific strings file.
- `cargo test` — execute all unit and integration tests; run after every non-trivial change.
- `cargo fmt` — format Rust sources; run before committing to avoid CI churn.

## Coding Style & Naming Conventions

- Follow standard Rust style (`rustfmt` defaults). Prefer descriptive snake_case for modules, variables, and functions.
- Keep web assets self-contained in `src/web/index.html`; use two-space indentation within HTML/JS to match existing layout.
- When introducing new configuration, document the environment variable name and default in both `Config` and `README.md`.

## Testing Guidelines

- Rust tests live alongside modules in `src/` (see `store.rs` for examples). Name tests using descriptive snake_case (e.g., `plural_variations_round_trip`).
- Ensure new storage or MCP behaviors have coverage in `store.rs` or `mcp_server.rs`; prefer temp-directory fixtures when touching filesystem logic.
- For web changes, add companion Rust tests when behavior depends on API responses; manually validate UI flows locally via `cargo run`.

## Commit & Pull Request Guidelines

- Write imperative, present-tense commit subjects (`Add plural picker delete control`). Group related changes per commit for easier review.
- Pull requests should summarize the user-facing impact, list verification steps (tests, manual checks), and reference related issues or MCP manifests.
- Include screenshots or screencasts when altering the web UI so reviewers can confirm styling without rebuilding locally.

## MCP Servers

Skip the `resolve-library-id` request when possible since we can utilize these known *library ID*s for their corresponding scopes:
`/rust-lang/rust` for Rust standard library and crates.io dependencies
`docs.rs/rmcp/latest` for Rust MCP documentation
