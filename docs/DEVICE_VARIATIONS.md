# Device Variations Implementation

## Overview

This document describes the implementation of device-specific variations support in the xcstrings MCP server web UI. Device variations allow translations to be customized for different Apple devices (iPhone, iPad, Mac, Apple Watch, etc.).

## Features Implemented

### 1. Device Variation Display

- Device variations are displayed in the same style as plural variations
- Each device variation shows the device type label (e.g., "device: iPhone" instead of "device: iphone")
- Device variations can be edited inline with auto-resizing textareas
- Each device variation has a delete button (×) to remove it

### 2. "+ Device" Button

- Added a "+ Device" button next to the "+ Plural" button in the key tools section
- The button opens a dropdown picker with available device cases
- Supported device types:
  - Apple TV (`appletv`)
  - Apple Vision (`applevision`)
  - Apple Watch (`applewatch`)
  - iPad (`ipad`)
  - iPhone (`iphone`)
  - iPod (`ipod`)
  - Mac (`mac`)
  - Other (`other`)

### 3. Mutual Exclusivity Logic

The visibility of the "+ Plural" and "+ Device" buttons follows these rules:

#### When NO variations exist:

- ✓ Show "+ Plural" button (if plural cases are available)
- ✓ Show "+ Device" button (if device cases are available)

#### When ONLY plural variations exist:

- ✗ Hide "+ Plural" button
- ✓ Show "+ Device" button (if device cases are available)

#### When ONLY device variations exist:

- ✓ Show "+ Plural" button (if plural cases are available)
- ✗ Hide "+ Device" button

#### When BOTH variation types exist:

- ✗ Hide "+ Plural" button
- ✗ Hide "+ Device" button

This ensures that a translation can have either plural variations OR device variations at the top level, but not both simultaneously. However, device variations can contain nested plural variations within them.

## Implementation Details

### Code Changes in `src/web/index.html`

1. **Device variation rendering** (lines 2184-2375):
   - Extracts device variations from `translation.variations.device`
   - Renders each device variation with edit/delete capabilities
   - Uses `getCaseLabel("device", caseKey)` for proper labeling

2. **Button visibility logic** (lines 2387-2511):
   - Checks for existing plural variations: `pluralKeys.length`
   - Checks for existing device variations: `deviceKeys.length`
   - Shows "+ Plural" button only when `deviceKeys.length === 0`
   - Shows "+ Device" button only when `pluralKeys.length === 0`

3. **Search functionality update** (lines 1522-1541):
   - Added device variation values to search index
   - Searches through both `translation.variations?.device` and `substitution.variations?.device`

### Styling

- Reuses existing CSS classes from plural variations:
  - `.plural-list` for the container
  - `.plural-row` for each variation row
  - `.plural-picker` for the dropdown picker
  - `.plural-option` for picker options

### API Integration

The implementation uses the existing MCP API endpoints:

- `PUT /api/translations` with variations payload
- Supports `variations: { device: { [caseKey]: { value, state } } }`

## Testing

### Test Files

- **`schema/examples/DeviceVariations.xcstrings`**: Sample xcstrings file with various device variation scenarios demonstrating the feature

### Test Coverage

The implementation is tested through:

1. Rust unit tests (`cargo test`) - All existing tests continue to pass
2. Schema validation - The DeviceVariations.xcstrings example passes validation
3. Manual testing - The web UI can be tested by running the server with any xcstrings file

### Scenarios Covered

1. No variations - both buttons visible
2. Only plural variations - only "+ Device" button visible
3. Only device variations - only "+ Plural" button visible
4. Both variation types - no buttons visible (mutual exclusivity enforced)
5. Adding/editing/deleting device variations
6. Search functionality includes device variation values

## Usage Examples

### Adding a Device Variation

1. Click the "+ Device" button
2. Select a device type from the dropdown (e.g., "iPhone")
3. Enter the device-specific translation
4. The translation auto-saves on blur

### Editing a Device Variation

1. Click on the textarea for the device variation
2. Modify the text
3. The change saves automatically when you click away

### Removing a Device Variation

1. Click the × button next to the device variation
2. The variation is immediately removed

## Compatibility

- Works with the existing xcstrings JSON schema
- Compatible with Apple's localization format
- Server-side search already supports device variations through recursive search
- No breaking changes to existing functionality

## Future Enhancements

Potential improvements for future iterations:

1. Nested variations support (device variations with plural sub-variations)
2. Bulk operations for device variations
3. Device variation templates or copy functionality
4. Preview of how translations appear on different devices
5. Import/export of device-specific translation sets
