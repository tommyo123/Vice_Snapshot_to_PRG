# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
### Changed
### Fixed
### Removed

## [0.9.0] - 2025-10-14

### Added
- Initial beta release
- GUI application for converting VICE 3.9 x64sc snapshots to PRG files
- LZSA1 compression for efficient snapshot compression
- Automatic memory patching and restoration code generation
- MSI installer with WiX
- Smart vcruntime140.dll bundling (only if VC++ Redistributable not installed)
- Desktop and Start Menu shortcuts
- Complete documentation in README.md

### Known Limitations
- Only supports VICE 3.9 x64sc snapshots
- Requires memory initialization (`f 0000 ffff 00` + `reset`) before snapshot
- Stack pointer placement may be risky in edge cases
- "Smart attach..." feature in VICE should be avoided

[Unreleased]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/compare/v0.9.0...HEAD
[0.9.0]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases/tag/v0.9.0
