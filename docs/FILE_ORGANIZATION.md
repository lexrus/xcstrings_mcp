# File Organization

This document describes the file structure and organization of the xcstrings MCP project.

## Project Structure

```
xcstrings_mcp/
├── src/                      # Source code
│   ├── lib.rs               # Library entry point
│   ├── main.rs              # Application entry point
│   ├── mcp_server.rs        # MCP server implementation
│   ├── store.rs             # Storage layer for xcstrings files
│   └── web/                 # Web UI implementation
│       ├── mod.rs           # Web server and API endpoints
│       └── index.html       # Single-page application UI
│
├── schema/                   # JSON Schema and validation
│   ├── xcstrings.schema.json    # JSON Schema for xcstrings format
│   ├── validate_examples.py     # Validation script
│   └── examples/                # Example xcstrings files
│       ├── DeviceVariations.xcstrings  # Device-specific variations
│       ├── Extractor.xcstrings         # Extraction examples
│       ├── InfoPlist.xcstrings         # Info.plist strings
│       └── ...                          # Other test cases
│
├── docs/                     # Documentation
│   ├── DEVICE_VARIATIONS.md # Device variations feature documentation
│   └── FILE_ORGANIZATION.md # This file
│
├── screenshots/              # UI screenshots
│   └── screenshot_alpha.jpg # Main UI screenshot
│
├── Cargo.toml               # Rust package manifest
├── Cargo.lock               # Dependency lock file
├── README.md                # Main project documentation
├── LICENSE                  # MIT License
├── AGENTS.md                # Agent workflow documentation
└── CLAUDE.md                # Claude AI guidance

```

## Directory Purposes

### `/src`

Core application code including:

- MCP server implementation with translation tools
- Async storage layer for xcstrings files
- Web UI server and embedded HTML interface

### `/schema`

JSON Schema definition and validation:

- Official xcstrings schema (Draft 7)
- Python validation script
- Comprehensive example files covering all features

### `/schema/examples`

Reference xcstrings files demonstrating:

- Multi-language localizations
- Plural variations (zero, one, few, many, other)
- Device-specific variations (iPhone, iPad, Mac, etc.)
- Substitutions and format specifiers
- Various extraction and translation states

### `/docs`

Project documentation:

- Feature documentation (e.g., device variations)
- Architecture decisions
- Implementation details

### `/screenshots`

Visual documentation:

- UI screenshots for README
- Feature demonstrations

## File Naming Conventions

### Source Files

- Snake_case for Rust modules: `mcp_server.rs`, `store.rs`
- Lowercase for web assets: `index.html`

### Example Files

- PascalCase for xcstrings examples: `DeviceVariations.xcstrings`
- Descriptive names indicating content: `InfoPlist.xcstrings`, `Extractor.xcstrings`

### Documentation

- UPPERCASE for top-level docs: `README.md`, `LICENSE`, `AGENTS.md`
- UPPERCASE_WITH_UNDERSCORES for feature docs: `DEVICE_VARIATIONS.md`, `FILE_ORGANIZATION.md`

## Build Artifacts

The following directories are generated during build/development and are excluded from version control:

- `/target` - Rust build artifacts (via .gitignore)
- `/.git` - Git repository metadata

## Configuration Files

- `.gitignore` - Git ignore patterns
- `.gitmodules` - Git submodule configuration (if applicable)

## Testing

### Unit Tests

Located inline with source code in `/src` using Rust's `#[cfg(test)]` modules.

### Schema Validation

Python script `validate_examples.py` tests all files in `/schema/examples`.

## Adding New Files

When adding new files, follow these guidelines:

1. **Source code**: Add to appropriate module in `/src`
2. **Test xcstrings**: Add to `/schema/examples` and ensure validation passes
3. **Documentation**: Add to `/docs` for feature-specific docs
4. **Examples**: Ensure new xcstrings examples pass schema validation

## Maintenance

Regular maintenance tasks:

- Run `cargo fmt --all` before commits
- Validate examples: `cd schema && python3 validate_examples.py`
- Run tests: `cargo test`
- Update documentation when adding features
