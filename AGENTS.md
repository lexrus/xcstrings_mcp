# xcstrings-mcp — Agent Brief

## Current Status

- Rust MCP server exposes translation tooling and an Axum web UI for `.xcstrings` catalogs.
- Supports dynamic-path mode (no default file) and default-path mode with live web editing.
- Automatic discovery of catalogs when no path is provided; UI surfaces a selector and placeholders.
- JSON schema from `schema/xcstrings.schema.json` enforced through the store and validation utilities.

## Key Components

- `src/main.rs` wires CLI/env config (`STRINGS_PATH`, `WEB_HOST`, `WEB_PORT`), spawns MCP + web services, and handles shutdown.
- `src/mcp_server.rs` implements tools: `list_translations`, `get_translation`, `upsert_translation`, `delete_translation`, `delete_key`, `set_comment`, `set_extraction_state`, `list_languages`.
- `src/store.rs` manages async file access, caching with persistence on every change, variation/substitution updates, and catalog discovery.
- `src/web/` hosts the embedded UI (`index.html`) with search, inline edits, plural management, and runtime catalog switching.
- `examples/` contains sample catalogs for manual or automated validation; keep in sync with schema expectations.

## Dev Workflow

- `cargo build` / `cargo build --release` for debug or distribution binaries.
- `cargo run -- -- <path/to/Localizable.xcstrings> [port]` to launch MCP + web; omit the path to run in discovery mode.
- `cargo test` after non-trivial changes; add focused tests beside modified modules.
- `cargo fmt --all` before committing to satisfy CI style checks.
- `python3 validate_examples.py [path]` (if present) to confirm schema compliance when touching catalog logic.

## Style & Documentation

- Follow Rust `snake_case` naming, default `rustfmt`, and keep HTML/JS in `src/web/index.html` with two-space indentation.
- Document new configuration in both the `Config` implementation and `README.md` (env var name + default).
- Keep README feature lists aligned with implemented behavior, especially around dynamic path handling and web UI capabilities.

## Testing Expectations

- Place Rust tests alongside code (see `store.rs` for patterns) with descriptive snake_case names.
- Cover storage and MCP edge cases (plural variations, substitutions, comment handling, discovery) when modifying related code.
- Manually exercise the web UI via `cargo run` for UX-impacting changes; add backend tests to reflect new UI interactions.

## Outstanding Focus Areas

- Investigate cache invalidation when files change outside the app (web UI still showing stale content).
- Improve substitution layout (label/input on a single line) and reduce search box size for better ergonomics.
- Allow resizing translation textareas to support longer entries.

## MCP Docs Quick Links

- Use `/rust-lang/rust` for Rust stdlib and crates.io documentation.
- Use `docs.rs/rmcp/latest` for RMCP-specific APIs.
