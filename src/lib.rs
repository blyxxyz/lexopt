//! Experimental imperative argument parsing library.

use std::{
    ffi::{OsStr, OsString},
    fmt::Display,
    str::FromStr,
};

// TODO:
// - Idiomatic way to find heterogenous positional arguments?
//   - Getopt saves positional arguments for the end

/// A parser for command line arguments.
pub struct Parser {
    source: Box<dyn Iterator<Item = OsString> + 'static>,
    bin_name: Option<OsString>,
    // The current string of short flags being processed
    shorts: Option<(Vec<u8>, usize)>,
    // Temporary storage for a long flag so it can be borrowed
    long: Option<String>,
    // The pending value for the last long flag
    long_value: Option<OsString>,
    // Data about the last flag we looked at, for error messages
    last_flag: LastFlag,
    // Whether we encountered "--" and know no more flags are coming
    finished_opts: bool,
}

enum LastFlag {
    None,
    Short(char),
    Long,
}

/// A command line argument, either a flag or a free-standing value.
#[derive(Debug, Clone)]
pub enum Arg<'a> {
    /// A short flag, e.g. `-q`.
    Short(char),
    /// A long flag, e.g. `--verbose`.
    Long(&'a str),
    /// A free-standing argument, e.g. `/dev/null`.
    Value(OsString),
}

impl Parser {
    /// Get the next flag or free-standing argument.
    ///
    /// This will return an [`Error::UnexpectedValue`] if the last flag had an
    /// associated value that hasn't been consumed, as in `--flag=value`.
    ///
    /// It will also return an error for flags that are not valid unicode.
    ///
    /// It will return `None` if the command line has been exhausted.
    // TODO: pick other name or add clippy lint ignore
    pub fn next(&mut self) -> Result<Option<Arg<'_>>, Error> {
        if let Some(value) = self.long_value.take() {
            // Last time we got `--long=value`, and `value` hasn't been used.
            return Err(Error::UnexpectedValue(value));
        }

        if let Some((ref arg, ref mut pos)) = self.shorts {
            // We're somewhere inside a -abc chain. Because we're in .next(),
            // not .value(), we can assume that the next character is another flag.
            match take_char(&arg, pos)? {
                None => {
                    self.shorts = None;
                }
                // If we find '=' here we assume it's part of a flag.
                // Perversely, `-=` as a flag exists in the wild!
                // See https://linux.die.net/man/1/a2ps
                // This means -n=10 can have two meanings (but so can -n10).
                Some(ch) => {
                    self.last_flag = LastFlag::Short(ch);
                    return Ok(Some(Arg::Short(ch)));
                }
            }
        }

        let arg = match self.source.next() {
            Some(arg) => arg,
            None => return Ok(None),
        };

        if self.finished_opts {
            return Ok(Some(Arg::Value(arg)));
        }
        if arg == "--" {
            self.finished_opts = true;
            return self.next();
        }

        let arg = match arg.into_string() {
            Ok(arg) => arg,
            Err(arg) => {
                // Here's where it gets tricky.
                // The argument is not valid unicode. If it's a free-standing
                // value then that's fine. But if it starts with a - then we
                // have to treat it like a flag.
                // OsString is very limited. What we do next depends on the platform.

                #[cfg(unix)]
                {
                    // Unix lets us turn OsStrings into bytes and back.
                    use std::os::unix::ffi::{OsStrExt, OsStringExt};

                    let bytes = arg.as_bytes();
                    if bytes.get(0) == Some(&b'-') {
                        if bytes.get(1) == Some(&b'-') {
                            // Long flag
                            if let Some(ind) = bytes.iter().position(|&b| b == b'=') {
                                // Long flag with value
                                if let Ok(flag) = String::from_utf8(bytes[2..ind].into()) {
                                    // The flag is valid unicode, only the value is messed up
                                    // We can handle that
                                    let long = self.long.insert(flag);
                                    self.last_flag = LastFlag::Long;
                                    self.long_value =
                                        Some(OsString::from_vec(bytes[ind + 1..].into()));
                                    return Ok(Some(Arg::Long(long)));
                                } else {
                                    // Even the flag is invalid, so error out
                                    return Err(Error::UnexpectedFlag(
                                        String::from_utf8_lossy(&bytes[..ind]).into(),
                                    ));
                                }
                            } else {
                                // There's no value, so the flag must be invalid
                                return Err(Error::UnexpectedFlag(
                                    String::from_utf8_lossy(bytes).into(),
                                ));
                            }
                        } else {
                            // Short flag
                            self.shorts = Some((arg.into_vec(), 1));
                            return self.next();
                        }
                    }
                }

                #[cfg(not(unix))]
                {
                    // Allocates for all non-unicode arguments, sadly
                    let text = arg.to_string_lossy();
                    if text.starts_with('-') {
                        // At this point it's game over.
                        // Even if we have a valid flag with malformed value,
                        // there's no way to separate it into its own OsString.
                        // So we just try to give a reasonable error message.
                        if let Some((flag, _)) = text.split_once('=') {
                            if flag.contains('\u{FFFD}') {
                                // Looks like the flag was invalid.
                                // This means you shouldn't use U+FFFD REPLACEMENT CHARACTER
                                // in an actual flag.
                                // But nobody would ever do that, right?
                                return Err(Error::UnexpectedFlag(flag.into()));
                            } else {
                                // This error should be considered a bug in the library.
                                return Err(Error::NonUnicodeValue(arg));
                            }
                        } else {
                            return Err(Error::UnexpectedFlag(text.into()));
                        }
                    }
                }

                // Apparently it didn't look like a flag, so return it as a value.
                return Ok(Some(Arg::Value(arg)));
            }
        };

        if arg.starts_with("--") {
            // Store the flag so the caller can borrow it.
            // We go through this trouble because matching an owned string is a pain.
            let arg = &self.long.insert(arg)[2..];
            self.last_flag = LastFlag::Long;
            if let Some((flag, value)) = arg.split_once('=') {
                self.long_value = Some(value.into());
                return Ok(Some(Arg::Long(flag)));
            } else {
                return Ok(Some(Arg::Long(arg)));
            }
        }

        if arg.len() > 1 && arg.starts_with('-') {
            self.shorts = Some((arg.into(), 1));
            return self.next();
        }

        Ok(Some(Arg::Value(arg.into())))
    }

    /// Reconstruct the last flag we parsed.
    fn last_flag(&self) -> Option<String> {
        match self.last_flag {
            LastFlag::None => None,
            LastFlag::Short(ch) => Some(format!("-{}", ch)),
            LastFlag::Long => {
                // In principle self.long should always be filled, but avoid
                // unwrap just in case
                let long = self.long.as_ref()?;
                if let Some((flag, _)) = long.split_once('=') {
                    Some(flag.into())
                } else {
                    Some(long.into())
                }
            }
        }
    }

    /// Get a value for a flag.
    ///
    /// This function should be called right after seeing a flag that
    /// expects a value. Free-standing arguments are collected using
    /// [`next()`][Parser::next].
    ///
    /// It fails if the end of the command line is reached.
    ///
    /// A value is collected even if it looks like a flag
    /// (i.e., starts with `-`).
    pub fn value(&mut self) -> Result<OsString, Error> {
        self.value_immediate()
            .or_else(|| self.source.next())
            .ok_or_else(|| Error::MissingValue(self.last_flag()))
    }

    /// Get a value directly connected to a flag, if it exists.
    ///
    /// This returns `Some("value")` for `-fvalue`, `-f=value`, and
    /// `--flag=value`.
    ///
    /// It returns `None` for `-f value` and `--flag value`.
    ///
    /// This can be used to emulate GNU sed's `-i[SUFFIX]` flag, which only
    /// optionally takes a value. Using it for new APIs is not recommended as
    /// the behavior can be confusing.
    pub fn value_immediate(&mut self) -> Option<OsString> {
        if let Some(value) = self.long_value.take() {
            return Some(value);
        }

        if let Some((arg, mut pos)) = self.shorts.take() {
            if pos < arg.len() {
                // We recognize the `=` even for short flags.
                // This means that -n"$var" is not strictly safe if $var
                // starts with an =. Use -n "$var" or -n="$var" instead.
                // TODO: do we really want this?
                if arg[pos] == b'=' {
                    pos += 1;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStringExt;
                    return Some(OsString::from_vec(arg[pos..].into()));
                }
                #[cfg(not(unix))]
                {
                    return Some(String::from_utf8(arg[pos..].into()).unwrap().into());
                }
            }
        }

        None
    }

    /// Create a parser from the environment using [`std::env::args_os`].
    pub fn from_env() -> Parser {
        let mut source = std::env::args_os();
        Parser {
            bin_name: source.next(),
            source: Box::new(source),
            shorts: None,
            long: None,
            long_value: None,
            last_flag: LastFlag::None,
            finished_opts: false,
        }
    }

    /// Create a parser from an iterator. This may be useful for testing.
    ///
    /// The executable name must not be included.
    pub fn from_args<I>(args: I) -> Parser
    where
        I: IntoIterator + 'static,
        I::Item: Into<OsString>,
    {
        Parser {
            source: Box::new(args.into_iter().map(Into::into)),
            bin_name: None,
            shorts: None,
            long: None,
            long_value: None,
            last_flag: LastFlag::None,
            finished_opts: false,
        }
    }

    /// Get the name that was used to invoke the program.
    ///
    /// Only available if constructed by [`Parser::from_env`].
    pub fn bin_name(&self) -> Option<&OsStr> {
        self.bin_name.as_deref()
    }
}

impl Arg<'_> {
    /// Convert an unexpected argument into an error.
    pub fn error(self) -> Error {
        match self {
            Arg::Short(short) => Error::UnexpectedFlag(format!("-{}", short)),
            Arg::Long(long) => Error::UnexpectedFlag(format!("--{}", long)),
            Arg::Value(value) => Error::UnexpectedValue(value),
        }
    }
}

/// An error during argument parsing.
#[non_exhaustive]
#[derive(Clone)]
pub enum Error {
    /// An option argument was expected but was not found.
    MissingValue(Option<String>),

    /// An unexpected flag was found.
    UnexpectedFlag(String),

    /// A free-standing argument was found when none was expected.
    UnexpectedValue(OsString),

    /// Parsing a value failed. Returned by some methods on [`ValueExt`].
    // TODO: it would be really nice to include the flag here.
    // But we don't have access to the parser.
    // Replace OsString by type with extra data?
    ParsingFailed(String),

    /// A value was found that was not valid unicode.
    ///
    /// This may be returned by some methods on [`ValueExt`].
    ///
    /// On non-Unix platforms it is also returned when such a value is combined
    /// with a flag (as in `-f[invalid]` and `--flag=[invalid]`), even if an
    /// [`OsString`] is requested, because of limitations in Rust's standard
    /// library.
    NonUnicodeValue(OsString),

    /// For custom error messages in application code.
    Custom(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Error::*;
        match self {
            // TODO: it would be nice to be able to say which option
            // (but it's always the last one in the command)
            // perhaps .next() can set a field?
            MissingValue(Some(flag)) => write!(f, "missing argument for option '{}'", flag),
            MissingValue(None) => write!(f, "missing argument"),
            UnexpectedFlag(flag) => write!(f, "invalid option '{}'", flag),
            UnexpectedValue(value) => write!(f, "unexpected argument {:?}", value),
            NonUnicodeValue(value) => write!(f, "argument is invalid unicode: {:?}", value),
            ParsingFailed(err) => write!(f, "cannot parse argument: {}", err),
            Custom(msg) => write!(f, "{}", msg),
        }
    }
}

// This is printed when returning an error from main(), so defer to Display
impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl std::error::Error for Error {}

mod private {
    pub trait Sealed {}
    impl Sealed for std::ffi::OsString {}
}

/// An optional extension trait with methods for processing [`OsString`]s.
///
/// These methods return optic's own [`Error`] type on failure for easy mixing
/// with other fallible functions.
///
/// Depending on the method, they may fail in two cases:
/// - The value cannot be decoded because it's invalid unicode
/// - The value cannot be parsed
pub trait ValueExt: private::Sealed {
    /// Decode the value and parse it using [`FromStr`].
    fn parse<T: FromStr>(&self) -> Result<T, Error>
    where
        T::Err: Display;

    /// Decode the value and parse it using a custom function.
    fn parse_with<F, T, E>(&self, func: F) -> Result<T, Error>
    where
        F: FnOnce(&str) -> Result<T, E>,
        E: Display;

    /// Process the value as an [`OsStr`] using a custom function.
    fn parse_os_with<F, T, E>(&self, func: F) -> Result<T, Error>
    where
        F: FnOnce(&OsStr) -> Result<T, E>,
        E: Display;

    /// Decode the value into a [`String`].
    ///
    /// This is identical to [`OsString::into_string`] except for the
    /// error type.
    fn string(self) -> Result<String, Error>;
}

impl ValueExt for OsString {
    fn parse<T: FromStr>(&self) -> Result<T, Error>
    where
        T::Err: Display,
    {
        self.parse_with(FromStr::from_str)
    }

    fn parse_with<F, T, E>(&self, func: F) -> Result<T, Error>
    where
        F: FnOnce(&str) -> Result<T, E>,
        E: Display,
    {
        match self.to_str() {
            Some(text) => match func(text) {
                Ok(value) => Ok(value),
                Err(err) => Err(Error::ParsingFailed(err.to_string())),
            },
            None => Err(Error::NonUnicodeValue(self.into())),
        }
    }

    fn parse_os_with<F, T, E>(&self, func: F) -> Result<T, Error>
    where
        F: FnOnce(&OsStr) -> Result<T, E>,
        E: Display,
    {
        match func(&self) {
            Ok(value) => Ok(value),
            Err(err) => Err(Error::ParsingFailed(err.to_string())),
        }
    }

    fn string(self) -> Result<String, Error> {
        self.into_string().map_err(Error::NonUnicodeValue)
    }
}

/// A small prelude for the most common functionality.
///
/// If this is used it's best to import it inside a function, not in module
/// scope. For example:
/// ```ignore
/// fn parse_args() -> Result<Args, optic::Error> {
///     use optic::prelude::*;
///     ...
/// }
/// ```
pub mod prelude {
    pub use super::Arg::*;
    pub use super::Error;
    pub use super::Parser;
    pub use super::ValueExt;
}

/// Try to take a codepoint from the start of `bytes` and advance `pos`.
///
/// `pos` will be advanced even if an error is returned, to allow recovery.
fn take_char(bytes: &[u8], pos: &mut usize) -> Result<Option<char>, Error> {
    match partial_decode(&bytes[*pos..]) {
        Some(text) => match text.chars().next() {
            None => Ok(None),
            Some(ch) => {
                *pos += ch.len_utf8();
                Ok(Some(ch))
            }
        },
        None => {
            let byte = bytes[*pos];
            *pos += 1;
            Err(Error::UnexpectedFlag(format!("-\\x{:02x}", byte)))
        }
    }
}

/// Decode at least one codepoint of the start of a string.
///
/// Returns None if decoding fails and Some("") if the string is empty.
fn partial_decode(bytes: &[u8]) -> Option<&str> {
    // We only need the first char, which must be in the first 4 bytes
    let bytes = &bytes[..bytes.len().min(4)];
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(err) if err.valid_up_to() > 0 => {
            Some(std::str::from_utf8(&bytes[..err.valid_up_to()]).unwrap())
        }
        Err(_) => None,
    }
}
