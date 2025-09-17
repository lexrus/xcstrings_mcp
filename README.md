# xcstrings-mcp

A Rust implementation of a Model Context Protocol (MCP) server designed for working with Xcode `Localizable.xcstrings` files. It exposes the translation catalog as MCP tools and also serves a lightweight web UI so teams can browse, search, and edit strings from a browser.

## Features
- Async-safe store that loads and persists `Localizable.xcstrings` JSON on every change.
- MCP toolset for listing, retrieving, creating, updating, and deleting translations and comments.
- Tool for enumerating all languages discovered in the file.
- Embedded Axum web UI for browsing translations, filtering by query, editing values, and managing comments.
- JSON-first responses from tools to make automation and debugging easier.

## Prerequisites
- [Rust](https://www.rust-lang.org/tools/install) 1.75 or newer.

## Running the server
```bash
cargo run -- [path-to/Localizable.xcstrings] [port]
```
- `path-to/Localizable.xcstrings`: Optional. Defaults to `./Localizable.xcstrings`.
- `port`: Optional. Defaults to `8787`.

You can also configure the server via environment variables:

| Variable | Description | Default |
| --- | --- | --- |
| `XCSTRINGS_PATH` | Path to the `.xcstrings` file | `Localizable.xcstrings` |
| `XCSTRINGS_WEB_HOST` | Host/interface for the web UI | `127.0.0.1` |
| `XCSTRINGS_WEB_PORT` | Port for the web UI | `8787` |

The web interface becomes available at `http://<host>:<port>/`.

### MCP usage
Run the binary with stdio transport (default) and wire it into an MCP-enabled client. The following tools are exposed:

- `list_translations(query?)`
- `get_translation(key, language)`
- `upsert_translation(key, language, value?, state?)`
- `delete_translation(key, language)`
- `delete_key(key)`
- `set_comment(key, comment?)`
- `list_languages()`

Each tool returns JSON payloads encoded into text content for easier consumption.

## Development
Install dependencies and run the full test suite:

```bash
cargo test
```

`cargo fmt --all` is recommended before submitting changes.

## Project layout
- `src/store.rs` – async storage layer for `.xcstrings` files.
- `src/mcp_server.rs` – MCP tool definitions exposing translation functionality.
- `src/web.rs` – Axum HTTP routes and HTML/JS single page view.
- `src/main.rs` – entrypoint that launches both web and MCP services.
