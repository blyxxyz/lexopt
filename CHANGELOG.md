## 0.2.1 (unreleased)

New:

- Add `Parser::raw_args()` for collecting raw unparsed arguments.
- Implement `Debug` for `ValuesIter`.

## 0.2.0 (2021-10-23)

While this release is not strictly backward-compatible it should break very few programs.

New:

- Add `Parser::values()` for options with multiple arguments.
- Add `Parser::optional_value()` for options with optional arguments.
- Add `Parser::from_iter()` to construct from an iterator that includes the binary name.
- Document how to use `Parser::value()` to collect all remaining arguments.

Changes:

- Support `=` as a separator for short options (as in `-o=value`).
- Sanitize the binary name if it's invalid unicode instead of ignoring it.
- Make `Error::UnexpectedValue.option` a `String` instead of an `Option<String>`.

Bug fixes:

- Include `bin_name` in `Parser`'s `Debug` output.

## 0.1.0 (2021-07-16)
Initial release.
