# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a Rust implementation of a Model Context Protocol (MCP) server for working with Xcode `Localizable.xcstrings` files. The server exposes translation tools via MCP and serves a web UI for browsing/editing translations.

## Commands

### Development

- `cargo test` - Run the full test suite
- `cargo fmt --all` - Format code (recommended before submitting changes)
- `cargo run -- [path-to/Localizable.xcstrings] [port]` - Run the server

### Build & Release

- `cargo build` - Build in debug mode
- `cargo build --release` - Build optimized release binary

## Architecture

The codebase is organized into these core modules:

### `src/store.rs`

Async storage layer for `.xcstrings` files. Handles:

- Loading/parsing JSON xcstrings format
- Thread-safe in-memory caching with RwLock
- Persistence on every change
- Translation CRUD operations with plural variations support
- Multi-file store management (dynamic path mode vs default path)

### `src/mcp_server.rs`

MCP tool definitions exposing translation functionality:

- `list_translations` - Compact summaries with optional full values
- `get_translation` - Per-language translation details
- `upsert_translation` - Create/update with variations support
- `delete_translation`/`delete_key` - Remove operations
- `set_comment` - Manage translation comments
- `list_languages` - Enumerate discovered languages

### `src/web/` (was `src/web.rs`)

Axum-based web UI with single-page application for:

- Browsing translations with search/filtering
- Editing values and plural variations
- Managing comments
- Only enabled when default xcstrings path is configured

### `src/main.rs`

Application entrypoint that:

- Parses CLI args and environment variables
- Manages dual-mode operation (default path vs dynamic path)
- Concurrently runs MCP stdio transport and web server
- Handles graceful shutdown

## Key Concepts

- **Dynamic Path Mode**: No default xcstrings file configured, web UI disabled, all MCP calls must supply `path` parameter
- **Default Path Mode**: xcstrings file path configured via CLI arg or `STRINGS_PATH` env var, enables web UI, MCP calls can omit `path`
- **XcStrings Format**: Apple's JSON-based localization format with support for plurals and variations
- **MCP Tools**: Return JSON payloads encoded as text content for easier automation

## MCP server

Always use context7 when I need code generation, setup or configuration steps, or library/API documentation. This means you should automatically use the Context7 MCP tools to resolve library id and get library docs without me having to explicitly ask.

Skip the `resolve-library-id` request when possible since we can utilize these known *library ID*s for their corresponding scopes:
`/rust-lang/rust` for Rust standard library and crates.io dependencies
`docs.rs/rmcp/latest` for Rust MCP documentation
