//! A simple command line tokenizer.
//!
//! Most argument parsers are declarative: you tell them what to parse,
//! and they do it.
//!
//! This one provides you with a stream of flags and values and lets you
//! figure out the rest.
//!
//! ## Example
//! ```no_run
//! #[derive(Debug)]
//! struct Args {
//!     follow: bool,
//!     number: u64,
//!     file: std::path::PathBuf,
//! }
//!
//! fn parse_args() -> Result<Args, optic::Error> {
//!     use optic::prelude::*;
//!
//!     let mut follow = false;
//!     let mut number = 10;
//!     let mut file = None;
//!
//!     let mut parser = optic::Parser::from_env();
//!     while let Some(arg) = parser.next()? {
//!         match arg {
//!             Short('f') | Long("follow") => {
//!                 follow = true;
//!             }
//!             Short('n') => {
//!                 number = parser.value()?.parse()?;
//!             }
//!             Value(value) if file.is_none() => {
//!                 file = Some(value.into());
//!             }
//!             Long("help") => {
//!                 println!("USAGE: tail [-f|--follow] [-n NUM] FILE");
//!                 std::process::exit(0);
//!             }
//!             _ => return Err(arg.error()),
//!         }
//!     }
//!     Ok(Args {
//!         follow,
//!         number,
//!         file: file.ok_or("missing FILE argument")?,
//!     })
//! }
//!
//! fn main() -> Result<(), optic::Error> {
//!     let args = parse_args()?;
//!     println!("{:#?}", args);
//!     Ok(())
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![allow(clippy::should_implement_trait)]

use std::{ffi::OsString, fmt::Display, str::FromStr};

// TODO:
// - Idiomatic way to find heterogenous positional arguments?
//   - Getopt saves positional arguments for the end
//     - Can we support that without making it easy to forget about them?
// - Fuzzing on other platforms
// - Pin down/document (formally or informally) recovery guarantees
// - Figure out how bin_name() should work
//   - At least three use cases:
//     - Put in help/error messages (unprocessed &str)
//     - Figure out variant of program (as in egrep vs grep) (strip dir and extension)
//     - Get the plain argv[0] to do something else (unprocessed &OsStr)
// - Reconsider use of word "option", maybe always call it "flag"?
// - Update table in README before release

/// A parser for command line arguments.
pub struct Parser {
    source: Box<dyn Iterator<Item = OsString> + 'static>,
    bin_name: Option<OsString>,
    // The current string of short flags being processed
    shorts: Option<(Vec<u8>, usize)>,
    #[cfg(windows)]
    // The same thing, but encoded as UTF-16
    shorts_utf16: Option<(Vec<u16>, usize)>,
    // Temporary storage for a long flag so it can be borrowed
    long: Option<String>,
    // The pending value for the last long flag
    long_value: Option<OsString>,
    // The last flag we emitted
    last_flag: LastFlag,
    // Whether we encountered "--" and know no more flags are coming
    finished_opts: bool,
}

// source may not implement Debug
impl std::fmt::Debug for Parser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut f = f.debug_struct("Parser");
        f.field("source", &"<iterator>")
            .field("bin_name", &self.bin_name)
            .field("shorts", &self.shorts);
        #[cfg(windows)]
        f.field("shorts_utf16", &self.shorts_utf16);
        f.field("long", &self.long)
            .field("long_value", &self.long_value)
            .field("last_flag", &self.last_flag)
            .field("finished_opts", &self.finished_opts)
            .finish()
    }
}

/// We use this to keep track of the last emitted flag, for error messages when
/// an expected value is not found.
///
/// A long flag can be recovered from the `long` field, so that variant doesn't
/// need to contain data. (We sometimes clear the field, but only for the
/// opposite error (a value is found but not expected), so that's fine.)
///
/// Our short flag storage is cleared more aggressively, so we do need to
/// duplicate that.
#[derive(Debug)]
enum LastFlag {
    None,
    Short(char),
    Long,
}

/// A command line argument, either a flag or a free-standing value.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// This will return an [`Error::UnexpectedValue`] if the last flag had a
    /// value that hasn't been consumed, as in `--flag=value`.
    ///
    /// It will also return an error for flags that are not valid unicode.
    ///
    /// It will return `Ok(None)` if the command line has been exhausted.
    pub fn next(&mut self) -> Result<Option<Arg<'_>>, Error> {
        if let Some(value) = self.long_value.take() {
            // Last time we got `--long=value`, and `value` hasn't been used.
            // TODO: take or clone?
            return Err(Error::UnexpectedValue {
                flag: self.long.take(),
                value,
            });
        }

        if let Some((ref arg, ref mut pos)) = self.shorts {
            // We're somewhere inside a -abc chain. Because we're in .next(),
            // not .value(), we can assume that the next character is another flag.
            match first_codepoint(&arg[*pos..]) {
                Ok(None) => {
                    self.shorts = None;
                }
                // If we find '=' here we assume it's part of a flag.
                // Another option would be to see it as a value separator.
                // `-=` as a flag exists in the wild!
                // See https://linux.die.net/man/1/a2ps
                Ok(Some(ch)) => {
                    *pos += ch.len_utf8();
                    self.last_flag = LastFlag::Short(ch);
                    return Ok(Some(Arg::Short(ch)));
                }
                Err(err) => {
                    *pos += 1; // This may allow recovery
                    return Err(err);
                }
            }
        }

        #[cfg(windows)]
        {
            if let Some((ref arg, ref mut pos)) = self.shorts_utf16 {
                match first_utf16_codepoint(&arg[*pos..]) {
                    Ok(None) => {
                        self.shorts_utf16 = None;
                    }
                    Ok(Some(ch)) => {
                        *pos += ch.len_utf16();
                        self.last_flag = LastFlag::Short(ch);
                        return Ok(Some(Arg::Short(ch)));
                    }
                    Err(err) => {
                        *pos += 1;
                        return Err(err);
                    }
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

        #[cfg(any(unix, target_os = "wasi"))]
        {
            // Fast solution for platforms where OsStrings are just UTF-8-ish bytes
            #[cfg(unix)]
            use std::os::unix::ffi::{OsStrExt, OsStringExt};
            #[cfg(target_os = "wasi")]
            use std::os::wasi::ffi::{OsStrExt, OsStringExt};

            let bytes = arg.as_bytes();
            if bytes.starts_with(b"--") {
                let flag = if let Some(ind) = bytes.iter().position(|&b| b == b'=') {
                    match String::from_utf8(bytes[..ind].into()) {
                        Ok(flag) => {
                            self.long_value = Some(OsString::from_vec(bytes[ind + 1..].into()));
                            flag
                        }
                        Err(_) => {
                            return Err(Error::UnexpectedFlag(
                                String::from_utf8_lossy(&bytes[..ind]).into(),
                            ))
                        }
                    }
                } else {
                    match arg.into_string() {
                        Ok(arg) => arg,
                        Err(arg) => {
                            return Err(Error::UnexpectedFlag(arg.to_string_lossy().into()))
                        }
                    }
                };
                // Store the flag so the caller can borrow it.
                // We go through this trouble because matching an owned string is a pain.
                let long = backport::insert(&mut self.long, flag);
                self.last_flag = LastFlag::Long;
                Ok(Some(Arg::Long(&long[2..])))
            } else if bytes.len() > 1 && bytes[0] == b'-' {
                self.shorts = Some((arg.into_vec(), 1));
                self.next()
            } else {
                Ok(Some(Arg::Value(arg)))
            }
        }

        #[cfg(not(any(unix, target_os = "wasi")))]
        {
            // Platforms where looking inside an OsString is harder

            #[cfg(windows)]
            {
                // Fast path for Windows
                use std::os::windows::ffi::OsStrExt;
                let mut bytes = arg.encode_wide();
                const DASH: u16 = b'-' as u16;
                match (bytes.next(), bytes.next()) {
                    (Some(DASH), Some(_)) => {
                        // This is a flag, we'll have to do more work.
                        // (We already checked for "--" earlier.)
                    }
                    _ => {
                        // Just a value, return early.
                        return Ok(Some(Arg::Value(arg)));
                    }
                }
            }

            let arg = match arg.into_string() {
                Ok(arg) => arg,
                Err(arg) => {
                    #[cfg(windows)]
                    {
                        // Unlike on Unix, we can't efficiently process invalid unicode.
                        // Semantically it's UTF-16, but internally it's WTF-8 (a superset of UTF-8).
                        // So we only process the raw version here, when we know we really have to.
                        use std::os::windows::ffi::{OsStrExt, OsStringExt};
                        let arg: Vec<u16> = arg.encode_wide().collect();
                        const DASH: u16 = b'-' as u16;
                        const EQ: u16 = b'=' as u16;
                        if arg.starts_with(&[DASH, DASH]) {
                            if let Some(ind) = arg.iter().position(|&u| u == EQ) {
                                if let Ok(flag) = String::from_utf16(&arg[..ind]) {
                                    self.long_value = Some(OsString::from_wide(&arg[ind + 1..]));
                                    let long = backport::insert(&mut self.long, flag);
                                    self.last_flag = LastFlag::Long;
                                    return Ok(Some(Arg::Long(&long[2..])));
                                } else {
                                    return Err(Error::UnexpectedFlag(String::from_utf16_lossy(
                                        &arg[..ind],
                                    )));
                                }
                            } else {
                                return Err(Error::UnexpectedFlag(String::from_utf16_lossy(&arg)));
                            }
                        } else {
                            assert!(arg.starts_with(&[DASH]));
                            assert!(arg.len() > 1);
                            self.shorts_utf16 = Some((arg, 1));
                            return self.next();
                        }
                    };

                    #[cfg(not(windows))]
                    {
                        // TODO: this code is really hard to test, because
                        // of a lack of platforms!
                        // wasm32-unknown-unknown qualifies, but I haven't figured out
                        // how to check the test outcome.
                        // We can at least use it to make sure this compiles.

                        // This allocates, sadly.
                        if arg.to_string_lossy().starts_with('-') {
                            // At this point it's game over.
                            // Even if the flag is fine and only the value is malformed,
                            // there's no way to separate it into its own OsString.
                            return Err(Error::NonUnicodeValue(arg));
                        } else {
                            // It didn't look like a flag, so return it as a value.
                            return Ok(Some(Arg::Value(arg)));
                        }
                    }
                }
            };

            if arg.starts_with("--") {
                if let Some((flag, value)) = backport::split_once(&arg) {
                    let long = &backport::insert(&mut self.long, flag.into())[2..];
                    self.long_value = Some(value.into());
                    self.last_flag = LastFlag::Long;
                    Ok(Some(Arg::Long(long)))
                } else {
                    let long = &backport::insert(&mut self.long, arg)[2..];
                    self.last_flag = LastFlag::Long;
                    Ok(Some(Arg::Long(long)))
                }
            } else if arg.len() > 1 && arg.starts_with('-') {
                self.shorts = Some((arg.into(), 1));
                self.next()
            } else {
                Ok(Some(Arg::Value(arg.into())))
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
        if let Some(value) = self.long_value.take() {
            return Ok(value);
        }

        if let Some((arg, pos)) = self.shorts.take() {
            if pos < arg.len() {
                #[cfg(any(unix, target_os = "wasi"))]
                {
                    #[cfg(unix)]
                    use std::os::unix::ffi::OsStringExt;
                    #[cfg(target_os = "wasi")]
                    use std::os::wasi::ffi::OsStringExt;
                    return Ok(OsString::from_vec(arg[pos..].into()));
                }
                #[cfg(not(any(unix, target_os = "wasi")))]
                {
                    let arg = String::from_utf8(arg[pos..].into())
                        .expect("short flag args on exotic platforms must be unicode");
                    return Ok(arg.into());
                }
            }
        }

        #[cfg(windows)]
        {
            if let Some((arg, pos)) = self.shorts_utf16.take() {
                if pos < arg.len() {
                    use std::os::windows::ffi::OsStringExt;
                    return Ok(OsString::from_wide(&arg[pos..]));
                }
            }
        }

        if let Some(value) = self.source.next() {
            return Ok(value);
        }

        let flag = match self.last_flag {
            LastFlag::None => None,
            LastFlag::Short(ch) => Some(format!("-{}", ch)),
            LastFlag::Long => self.long.take(),
        };
        Err(Error::MissingValue { flag })
    }

    /// Create a parser from the environment using [`std::env::args_os`].
    pub fn from_env() -> Parser {
        let mut source = std::env::args_os();
        let bin_name = source.next();
        Parser {
            source: Box::new(source),
            bin_name,
            shorts: None,
            #[cfg(windows)]
            shorts_utf16: None,
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
            #[cfg(windows)]
            shorts_utf16: None,
            long: None,
            long_value: None,
            last_flag: LastFlag::None,
            finished_opts: false,
        }
    }

    /// Get the name that was used to invoke the program.
    ///
    /// Only available if constructed by [`Parser::from_env`].
    pub fn bin_name(&self) -> Option<&str> {
        // We return a &str here, not an &OsStr. If argv[0] is not unicode
        // we return None.
        // Some considerations:
        // - Random filenames may be garbled, but you'd hopefully install an
        //   executable in a sane place.
        // - The intention of this method is to be used in help text and
        //   error messages. For that you'd always want a &str anyway.
        // - The fact that it's an Option will hopefully nudge people towards
        //   providing a default with unwrap_or().
        // - Someone might be tempted to use this to find the location of the
        //   executable. But that's a bad idea anyway:
        //   - If it's a relative path you have to resolve it
        //   - If it's a plain name you have to look it up in $PATH
        //   - argv[0] is not even guaranteed to hold the "real" name
        //   - It's not portable, and specific platforms have better solutions
        //     like /proc/self/exe
        //   So let's not do anything to support that use case.
        self.bin_name.as_ref()?.to_str()
    }
}

impl Arg<'_> {
    /// Convert an unexpected argument into an error.
    // TODO: rename to unexpected()?
    pub fn error(self) -> Error {
        match self {
            Arg::Short(short) => Error::UnexpectedFlag(format!("-{}", short)),
            Arg::Long(long) => Error::UnexpectedFlag(format!("--{}", long)),
            Arg::Value(value) => Error::UnexpectedArgument(value),
        }
    }
}

/// An error during argument parsing.
///
/// This implements [`From`] for `String` and `&str`, for easy ad-hoc error
/// messages.
///
/// It also implements `From` for [`OsString`], as that's used as an error type
/// by [`OsString::into_string`], so that method may be used with the try (`?`)
/// operator.
#[non_exhaustive]
pub enum Error {
    /// An option argument was expected but was not found.
    MissingValue {
        /// The most recently emitted flag.
        flag: Option<String>,
    },

    /// An unexpected flag was found.
    UnexpectedFlag(String),

    /// A free-standing argument was found when none was expected.
    UnexpectedArgument(OsString),

    /// A flag had a value when none was expected.
    UnexpectedValue {
        /// The flag. This is always a long flag.
        flag: Option<String>,
        /// The value.
        value: OsString,
    },

    /// Parsing a value failed. Returned by methods on [`ValueExt`].
    ParsingFailed {
        /// The string that failed to parse.
        value: String,
        /// The error returned while parsing.
        error: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    /// A value was found that was not valid unicode.
    ///
    /// This may be returned by some methods on [`ValueExt`].
    ///
    /// On exotic platforms (not Unix or Windows or WASI) it is also returned
    /// when such a value is combined with a flag (as in `-f[invalid]` and
    /// `--flag=[invalid]`), even if an [`OsString`] is requested.
    NonUnicodeValue(OsString),

    /// For custom error messages in application code.
    Custom(Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Error::*;
        match self {
            MissingValue { flag: None } => write!(f, "missing argument at end of command"),
            MissingValue { flag: Some(flag) } => {
                write!(f, "missing argument for option '{}'", flag)
            }
            UnexpectedFlag(flag) => write!(f, "invalid option '{}'", flag),
            UnexpectedArgument(value) => write!(f, "unexpected argument {:?}", value),
            UnexpectedValue {
                flag: Some(flag),
                value,
            } => {
                write!(f, "unexpected argument for option '{}': {:?}", flag, value)
            }
            UnexpectedValue { flag: None, value } => {
                write!(f, "unexpected argument for option: {:?}", value)
            }
            NonUnicodeValue(value) => write!(f, "argument is invalid unicode: {:?}", value),
            ParsingFailed { value, error } => {
                write!(f, "cannot parse argument {:?}: {}", value, error)
            }
            Custom(err) => write!(f, "{}", err),
        }
    }
}

// This is printed when returning an error from main(), so defer to Display
impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::ParsingFailed { error, .. } | Error::Custom(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

impl From<String> for Error {
    fn from(msg: String) -> Self {
        Error::Custom(msg.into())
    }
}

impl From<&'_ str> for Error {
    fn from(msg: &'_ str) -> Self {
        Error::Custom(msg.into())
    }
}

/// For [`OsString::into_string`].
impl From<OsString> for Error {
    fn from(arg: OsString) -> Self {
        Error::NonUnicodeValue(arg)
    }
}

mod private {
    pub trait Sealed {}
    impl Sealed for std::ffi::OsString {}
}

/// An optional extension trait with methods for parsing [`OsString`]s.
///
/// They may fail in two cases:
/// - The value cannot be decoded because it's invalid unicode
///   ([`Error::NonUnicodeValue`])
/// - The value can be decoded, but parsing fails ([`Error::ParsingFailed`])
///
/// If parsing fails the error will be wrapped in optic's own [`Error`] type.
pub trait ValueExt: private::Sealed {
    /// Decode the value and parse it using [`FromStr`].
    ///
    /// This will fail if the value is not valid unicode or if the subsequent
    /// parsing fails.
    fn parse<T: FromStr>(&self) -> Result<T, Error>
    where
        T::Err: Into<Box<dyn std::error::Error + Send + Sync + 'static>>;

    /// Decode the value and parse it using a custom function.
    fn parse_with<F, T, E>(&self, func: F) -> Result<T, Error>
    where
        F: FnOnce(&str) -> Result<T, E>,
        E: Into<Box<dyn std::error::Error + Send + Sync + 'static>>;

    // There is no parse_os_with() because I can't think of any useful
    // fallible operations on an OsString. Typically you'd either decode it,
    // use it as is, or do an infallible conversion to a PathBuf or such.
    //
    // If you have a use for parse_os_with() please open an issue with an
    // example.
}

impl ValueExt for OsString {
    fn parse<T: FromStr>(&self) -> Result<T, Error>
    where
        T::Err: Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    {
        self.parse_with(FromStr::from_str)
    }

    fn parse_with<F, T, E>(&self, func: F) -> Result<T, Error>
    where
        F: FnOnce(&str) -> Result<T, E>,
        E: Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    {
        match self.to_str() {
            Some(text) => match func(text) {
                Ok(value) => Ok(value),
                Err(err) => Err(Error::ParsingFailed {
                    value: text.to_owned(),
                    error: err.into(),
                }),
            },
            None => Err(Error::NonUnicodeValue(self.into())),
        }
    }
}

/// A small prelude for processing arguments.
///
/// It allows you to write `Short`/`Long`/`Value` without an [`Arg`] prefix
/// and adds convenience methods to [`OsString`].
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
    pub use super::ValueExt;
}

/// Take the first codepoint of a bytestring.
///
/// The rest of the bytestring does not have to be valid unicode.
fn first_codepoint(bytes: &[u8]) -> Result<Option<char>, Error> {
    // We only need the first 4 bytes
    let bytes = &bytes[..bytes.len().min(4)];
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) if err.valid_up_to() > 0 => {
            std::str::from_utf8(&bytes[..err.valid_up_to()]).unwrap()
        }
        Err(_) => return Err(Error::UnexpectedFlag(format!("-\\x{:02x}", bytes[0]))),
    };
    Ok(text.chars().next())
}

#[cfg(windows)]
/// As before, but for UTF-16.
fn first_utf16_codepoint(bytes: &[u16]) -> Result<Option<char>, Error> {
    match std::char::decode_utf16(bytes.iter().copied()).next() {
        Some(Ok(ch)) => Ok(Some(ch)),
        Some(Err(_)) => Err(Error::UnexpectedFlag(format!("-\\u{:04x}", bytes[0]))),
        None => Ok(None),
    }
}

/// Implementations of a few useful functions that didn't exist
/// yet in Rust 1.40 (our MSRV, when non_exhaustive stabilized).
///
/// Not generic but shamelessly specialized for our needs.
#[allow(dead_code)]
mod backport {
    /// [`str::split_once`]
    pub(super) fn split_once(text: &str) -> Option<(&str, &str)> {
        let mut iter = text.splitn(2, '=');
        Some((iter.next()?, iter.next()?))
    }

    /// [`Option::insert`]
    pub(super) fn insert(opt: &mut Option<String>, value: String) -> &mut String {
        *opt = None;
        opt.get_or_insert(value)
    }
}

#[cfg(test)]
mod tests {
    use super::prelude::*;
    use super::*;

    fn parse(args: &'static str) -> Parser {
        Parser::from_args(args.split_ascii_whitespace().map(bad_string))
    }

    #[test]
    fn test_basic() -> Result<(), Error> {
        let mut p = parse("-n 10 foo - -- baz -qux");
        assert_eq!(p.next()?.unwrap(), Short('n'));
        assert_eq!(p.value()?.parse::<i32>()?, 10);
        assert_eq!(p.next()?.unwrap(), Value("foo".into()));
        assert_eq!(p.next()?.unwrap(), Value("-".into()));
        assert_eq!(p.next()?.unwrap(), Value("baz".into()));
        assert_eq!(p.next()?.unwrap(), Value("-qux".into()));
        assert_eq!(p.next()?, None);
        assert_eq!(p.next()?, None);
        assert_eq!(p.next()?, None);
        Ok(())
    }

    #[test]
    fn test_combined() -> Result<(), Error> {
        let mut p = parse("-abc -fvalue -xfvalue");
        assert_eq!(p.next()?.unwrap(), Short('a'));
        assert_eq!(p.next()?.unwrap(), Short('b'));
        assert_eq!(p.next()?.unwrap(), Short('c'));
        assert_eq!(p.next()?.unwrap(), Short('f'));
        assert_eq!(p.value()?, "value");
        assert_eq!(p.next()?.unwrap(), Short('x'));
        assert_eq!(p.next()?.unwrap(), Short('f'));
        assert_eq!(p.value()?, "value");
        assert_eq!(p.next()?, None);
        Ok(())
    }

    #[test]
    fn test_long() -> Result<(), Error> {
        let mut p = parse("--foo --bar=qux --foobar=qux=baz");
        assert_eq!(p.next()?.unwrap(), Long("foo"));
        assert_eq!(p.next()?.unwrap(), Long("bar"));
        assert_eq!(p.value()?, "qux");
        assert_eq!(p.next()?.unwrap(), Long("foobar"));
        match p.next().unwrap_err() {
            Error::UnexpectedValue {
                flag: Some(flag),
                value,
            } => {
                assert_eq!(flag, "--foobar");
                assert_eq!(value, "qux=baz");
            }
            _ => panic!(),
        }
        assert_eq!(p.next()?, None);
        Ok(())
    }

    #[test]
    fn test_dash_args() -> Result<(), Error> {
        // "--" should indicate the end of the flags
        let mut p = parse("-x -- -y");
        assert_eq!(p.next()?.unwrap(), Short('x'));
        assert_eq!(p.next()?.unwrap(), Value("-y".into()));
        assert_eq!(p.next()?, None);

        // ...unless it's an argument of a flag
        let mut p = parse("-x -- -y");
        assert_eq!(p.next()?.unwrap(), Short('x'));
        assert_eq!(p.value()?, "--");
        assert_eq!(p.next()?.unwrap(), Short('y'));
        assert_eq!(p.next()?, None);

        // "-" is a valid value that should not be treated as a flag
        let mut p = parse("-x - -y");
        assert_eq!(p.next()?.unwrap(), Short('x'));
        assert_eq!(p.next()?.unwrap(), Value("-".into()));
        assert_eq!(p.next()?.unwrap(), Short('y'));
        assert_eq!(p.next()?, None);

        // '-' is a silly and hard to use short flag, but other parsers treat
        // it like a flag in this position
        let mut p = parse("-x-y");
        assert_eq!(p.next()?.unwrap(), Short('x'));
        assert_eq!(p.next()?.unwrap(), Short('-'));
        assert_eq!(p.next()?.unwrap(), Short('y'));
        assert_eq!(p.next()?, None);

        Ok(())
    }

    #[test]
    fn test_missing_value() -> Result<(), Error> {
        let mut p = parse("-o");
        assert_eq!(p.next()?.unwrap(), Short('o'));
        match p.value() {
            Err(Error::MissingValue { flag: Some(flag) }) => assert_eq!(flag, "-o"),
            _ => panic!(),
        }

        let mut q = parse("--out");
        assert_eq!(q.next()?.unwrap(), Long("out"));
        match q.value() {
            Err(Error::MissingValue { flag: Some(flag) }) => assert_eq!(flag, "--out"),
            _ => panic!(),
        }

        let mut r = parse("");
        match r.value() {
            Err(Error::MissingValue { flag: None }) => (),
            _ => panic!(),
        }

        Ok(())
    }

    #[test]
    fn test_unicode() -> Result<(), Error> {
        let mut p = parse("-aµ --µ=10 µ --foo=µ");
        assert_eq!(p.next()?.unwrap(), Short('a'));
        assert_eq!(p.next()?.unwrap(), Short('µ'));
        assert_eq!(p.next()?.unwrap(), Long("µ"));
        assert_eq!(p.value()?, "10");
        assert_eq!(p.next()?.unwrap(), Value("µ".into()));
        assert_eq!(p.next()?.unwrap(), Long("foo"));
        assert_eq!(p.value()?, "µ");
        Ok(())
    }

    #[test]
    fn test_mixed_invalid() -> Result<(), Error> {
        let mut p = parse("--foo=@@@");
        let mut q = parse("-💣@@@");
        let mut r = parse("-f@@@");
        #[cfg(any(unix, target_os = "wasi", windows))]
        {
            assert_eq!(p.next()?.unwrap(), Long("foo"));
            assert_eq!(p.value()?, bad_string("@@@"));

            assert_eq!(q.next()?.unwrap(), Short('💣'));
            assert_eq!(dbg!(q.value())?, bad_string("@@@"));

            assert_eq!(r.next()?.unwrap(), Short('f'));
            match r.next().unwrap_err() {
                Error::UnexpectedFlag(_) => (),
                _ => panic!(),
            }
        }
        #[cfg(not(any(unix, target_os = "wasi", windows)))]
        {
            match p.next().unwrap_err() {
                Error::NonUnicodeValue(value) => assert_eq!(value, bad_string("--foo=@@@")),
                _ => panic!(),
            }

            match q.next().unwrap_err() {
                Error::NonUnicodeValue(value) => assert_eq!(value, bad_string("-💣@@@")),
                _ => panic!(),
            }

            match r.next().unwrap_err() {
                Error::NonUnicodeValue(value) => assert_eq!(value, bad_string("-f@@@")),
                _ => panic!(),
            }
        }
        Ok(())
    }

    #[cfg(any(unix, target_os = "wasi", windows))]
    #[test]
    fn test_separate_invalid() -> Result<(), Error> {
        let mut p = parse("--foo @@@");
        assert_eq!(p.next()?.unwrap(), Long("foo"));
        assert_eq!(p.value()?, bad_string("@@@"));
        Ok(())
    }

    #[test]
    fn test_invalid_long_valued_flag_recovery() -> Result<(), Error> {
        let mut p = parse("--@=10");
        p.next().unwrap_err();
        assert_eq!(p.next()?, None);
        Ok(())
    }

    #[test]
    fn test_value_ext() -> Result<(), Error> {
        let s = OsString::from("-10");
        assert_eq!(s.parse::<i32>()?, -10);
        assert_eq!(
            s.parse_with(|s| match s {
                "-10" => Ok(0),
                _ => Err("bad"),
            })?,
            0,
        );
        match s.parse::<u32>() {
            Err(Error::ParsingFailed { value, .. }) => assert_eq!(value, "-10"),
            _ => panic!(),
        }
        match s.parse_with(|s| match s {
            "11" => Ok(0_i32),
            _ => Err("bad"),
        }) {
            Err(Error::ParsingFailed { value, .. }) => assert_eq!(value, "-10"),
            _ => panic!(),
        }
        assert_eq!(s.into_string()?, "-10");
        Ok(())
    }

    #[cfg(any(unix, target_os = "wasi", windows))]
    #[test]
    fn test_value_ext_invalid() -> Result<(), Error> {
        let s = bad_string("foo@");
        match s.parse::<i32>() {
            Err(Error::NonUnicodeValue(_)) => (),
            _ => panic!(),
        }
        match s.parse_with(<f32 as FromStr>::from_str) {
            Err(Error::NonUnicodeValue(_)) => (),
            _ => panic!(),
        }
        match s.into_string().map_err(Error::from) {
            Err(Error::NonUnicodeValue(_)) => (),
            _ => panic!(),
        }
        Ok(())
    }

    #[test]
    fn test_first_codepoint() {
        assert_eq!(first_codepoint(b"foo").unwrap(), Some('f'));
        assert_eq!(first_codepoint(b"").unwrap(), None);
        assert_eq!(first_codepoint(b"f\xFF\xFF").unwrap(), Some('f'));
        assert_eq!(first_codepoint(b"\xC2\xB5bar").unwrap(), Some('µ'));
        first_codepoint(b"\xFF").unwrap_err();
        assert_eq!(first_codepoint(b"foo\xC2\xB5").unwrap(), Some('f'));
    }

    /// Transform @ characters into invalid unicode.
    fn bad_string(text: &str) -> OsString {
        #[cfg(any(unix, target_os = "wasi"))]
        {
            #[cfg(unix)]
            use std::os::unix::ffi::OsStringExt;
            #[cfg(target_os = "wasi")]
            use std::os::wasi::ffi::OsStringExt;
            let mut text = text.as_bytes().to_vec();
            for ch in &mut text {
                if *ch == b'@' {
                    *ch = b'\xFF';
                }
            }
            OsString::from_vec(text)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            let mut out = Vec::new();
            for ch in text.chars() {
                if ch == '@' {
                    out.push(0xD800);
                    out.push(0xFFFF);
                } else {
                    let mut buf = [0; 2];
                    out.extend(&*ch.encode_utf16(&mut buf));
                }
            }
            OsString::from_wide(&out)
        }
        #[cfg(not(any(unix, target_os = "wasi", windows)))]
        {
            if text.contains('@') {
                unimplemented!("Don't know how to create invalid OsStrings on this platform");
            }
            text.into()
        }
    }
}
