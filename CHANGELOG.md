## 0.2.0 (unreleased)

New:

- Add `Parser::values()` for options with multiple arguments.
- Add `Parser::optional_value()` for options with optional arguments.
- Add `Parser::from_iter()` to construct from an iterator that includes the binary name.
- Document how to use `Parser::value()` to collect all remaining arguments.

Changes:

- Support `=` as a separator for short options (as in `-o=value`).
- Sanitize the binary name if it's invalid unicode instead of ignoring it.

These changes are not strictly backward-compatible, hence the version bump, but problems are unlikely.

Bug fixes:

- Include `bin_name` in `Parser`'s `Debug` output.

## 0.1.0 (2021-07-16)
Initial release.
