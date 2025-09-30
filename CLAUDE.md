# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a Rust implementation of a Model Context Protocol (MCP) server for working with Xcode `Localizable.xcstrings` files. The server exposes translation tools via MCP and serves a web UI for browsing/editing translations.

## Commands

### Development

- `cargo test` - Run the full test suite
- `cargo test <testname>` - Run specific tests by name pattern
- `cargo fmt --all` - Format code (recommended before submitting changes)
- `cargo run -- [path-to/Localizable.xcstrings] [port]` - Run the server

### Build & Release

- `cargo build` - Build in debug mode
- `cargo build --release` - Build optimized release binary
- `cargo install --path .` - Install binary to ~/.cargo/bin/

### Validation

- `python3 validate_examples.py` - Validates all example .xcstrings files against the schema
- `python3 validate_examples.py path/to/catalog.xcstrings` - Validates a specific catalog file
- `python3 -m json.tool <file>` - Pretty-prints JSON files for easier review

### Development Setup

- `python3 -m pip install jsonschema` - Installs the only required dependency

## Architecture

This repository implements a JSON Schema validator for Apple's .xcstrings string catalog format:

- **xcstrings.schema.json**: The core JSON Schema (Draft 7) defining the complete structure of .xcstrings files, including:
  - Root-level catalog properties (version, sourceLanguage, strings)
  - String entries with localizations, variations, and substitutions
  - Nested variation sets for pluralization and device-specific strings
  - State management for translation workflow (translated, needs_review, etc.)

- **validate_examples.py**: Validation script that:
  - Loads the schema using jsonschema's Draft7Validator
  - Iterates through all .xcstrings files in examples/
  - Reports validation status and detailed error messages
  - Returns non-zero exit code on validation failures

- **examples/**: Reference catalog files covering various real-world scenarios like:
  - Multi-language localizations
  - Plural variations
  - Format specifiers and substitutions
  - Different extraction states

### `src/store.rs`

Async storage layer for `.xcstrings` files. Handles:

- Loading/parsing JSON xcstrings format
- Thread-safe in-memory caching with RwLock
- Persistence on every change
- Translation CRUD operations with plural variations support
- Multi-file store management (dynamic path mode vs default path)
- Translation progress tracking (percentage calculation, untranslated keys detection)

### `src/mcp_server.rs`

MCP tool definitions exposing translation functionality:

- `list_translations` - Compact summaries with optional full values
- `get_translation` - Per-language translation details
- `upsert_translation` - Create/update with variations support
- `delete_translation`/`delete_key` - Remove operations
- `set_comment` - Manage translation comments
- `list_languages` - Enumerate discovered languages
- `add_language` - Add new language with placeholder entries
- `remove_language` - Remove language from catalog
- `update_language` - Rename/update language code
- `list_untranslated` - List untranslated keys per language

### `src/web/mod.rs`

Axum-based web UI with single-page application for:

- Browsing translations with search/filtering
- Editing values and plural variations
- Managing comments
- Translation progress display with percentages in language dropdown
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

## Key Implementation Details

The schema uses recursive definitions ($defs) to handle the nested nature of variations and substitutions. The validation enforces that localizations must have either a stringUnit or variations (but not neither).

When modifying the schema, always validate against all example files to ensure backward compatibility. New schema features should be accompanied by example files demonstrating their usage.

## MCP server

Always use context7 when I need code generation, setup or configuration steps, or library/API documentation. This means you should automatically use the Context7 MCP tools to resolve library id and get library docs without me having to explicitly ask.

Skip the `resolve-library-id` request when possible since we can utilize these known *library ID*s for their corresponding scopes:
`/rust-lang/rust` for Rust standard library and crates.io dependencies
`docs.rs/rmcp/latest` for Rust MCP documentation
