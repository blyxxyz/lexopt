//! A pathologically simple command line argument parser.
//!
//! Most argument parsers are declarative: you tell them what to parse,
//! and they do it.
//!
//! This one provides you with a stream of options and values and lets you
//! figure out the rest.
//!
//! ## Example
//! ```no_run
//! struct Args {
//!     thing: String,
//!     number: u32,
//!     shout: bool,
//! }
//!
//! fn parse_args() -> Result<Args, lexopt::Error> {
//!     use lexopt::prelude::*;
//!
//!     let mut thing = None;
//!     let mut number = 1;
//!     let mut shout = false;
//!     let mut parser = lexopt::Parser::from_env();
//!     while let Some(arg) = parser.next()? {
//!         match arg {
//!             Short('n') | Long("number") => {
//!                 number = parser.value()?.parse()?;
//!             }
//!             Long("shout") => {
//!                 shout = true;
//!             }
//!             Value(val) if thing.is_none() => {
//!                 thing = Some(val.string()?);
//!             }
//!             Long("help") => {
//!                 println!("Usage: hello [-n|--number=NUM] [--shout] THING");
//!                 std::process::exit(0);
//!             }
//!             _ => return Err(arg.unexpected()),
//!         }
//!     }
//!
//!     Ok(Args {
//!         thing: thing.ok_or("missing argument THING")?,
//!         number,
//!         shout,
//!     })
//! }
//!
//! fn main() -> Result<(), lexopt::Error> {
//!     let args = parse_args()?;
//!     let mut message = format!("Hello {}", args.thing);
//!     if args.shout {
//!         message = message.to_uppercase();
//!     }
//!     for _ in 0..args.number {
//!         println!("{}", message);
//!     }
//!     Ok(())
//! }
//! ```
//! Let's walk through this:
//! - We start parsing with [`Parser::from_env`].
//! - We call [`parser.next()`][Parser::next] in a loop to get all the arguments until they run out.
//! - We match on arguments. [`Short`][Arg::Short] and [`Long`][Arg::Long] indicate an option.
//! - To get the value that belongs to an option (like `10` in `-n 10`) we call [`parser.value()`][Parser::value].
//!   - This returns a standard [`OsString`][std::ffi::OsString].
//!   - For convenience, [`use lexopt::prelude::*`][prelude] adds a [`.parse()`][ValueExt::parse] method, analogous to [`str::parse`].
//!   - Calling `parser.value()` is how we tell `Parser` that `-n` takes a value at all.
//! - `Value` indicates a free-standing argument.
//!   - `if thing.is_none()` is a useful pattern for positional arguments. If we already found `thing` we pass it on to another case.
//!   - It also contains an `OsString`.
//!     - The [`.string()`][ValueExt::string] method decodes it into a plain `String`.
//! - If we don't know what to do with an argument we use [`return Err(arg.unexpected())`][Arg::unexpected] to turn it into an error message.
//! - Strings can be promoted to errors for custom error messages.

#![feature(slice_range)]
#![deny(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations, elided_lifetimes_in_paths)]
#![allow(clippy::should_implement_trait)]

use std::{
    ffi::{OsStr, OsString},
    fmt::Display,
    mem::replace,
    str::{FromStr, Utf8Error},
};

mod os_str_slice;

use os_str_slice::OsStrSlice;

type InnerIter = std::vec::IntoIter<OsString>;

fn make_iter(iter: impl Iterator<Item = OsString>) -> InnerIter {
    iter.collect::<Vec<_>>().into_iter()
}

/// A parser for command line arguments.
#[derive(Debug, Clone)]
pub struct Parser {
    source: InnerIter,
    state: State,
    /// The last option we emitted.
    last_option: LastOption,
    /// The name of the command (argv\[0\]).
    bin_name: Option<String>,
}

#[derive(Debug, Clone)]
enum State {
    /// Nothing interesting is going on.
    None,
    /// We have a value left over from --option=value.
    PendingValue(OsString),
    /// We're in the middle of -abc.
    ///
    /// In order to satisfy OsStr::slice_encoded_bytes() we make sure that the
    /// usize always point to the end of a valid UTF-8 substring.
    Shorts(OsString, usize),
    /// We saw -- and know no more options are coming.
    FinishedOpts,
}

/// We use this to keep track of the last emitted option, for error messages when
/// an expected value is not found.
///
/// We also use this as storage for long options so we can hand out &str
/// (because String doesn't support pattern matching).
#[derive(Debug, Clone)]
enum LastOption {
    None,
    Short(char),
    Long(String),
}

/// A command line argument found by [`Parser`], either an option or a positional argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg<'a> {
    /// A short option, e.g. `Short('q')` for `-q`.
    Short(char),
    /// A long option, e.g. `Long("verbose")` for `--verbose`. (The dashes are not included.)
    Long(&'a str),
    /// A positional argument, e.g. `/dev/null`.
    Value(OsString),
}

impl Parser {
    /// Get the next option or positional argument.
    ///
    /// A return value of `Ok(None)` means the command line has been exhausted.
    ///
    /// Options that are not valid unicode are transformed with replacement
    /// characters as by [`String::from_utf8_lossy`].
    ///
    /// # Errors
    ///
    /// [`Error::UnexpectedValue`] is returned if the last option had a
    /// value that hasn't been consumed, as in `--option=value` or `-o=value`.
    ///
    /// It's possible to continue parsing after an error (but this is rarely useful).
    pub fn next(&mut self) -> Result<Option<Arg<'_>>, Error> {
        match self.state {
            State::PendingValue(ref mut value) => {
                // Last time we got `--long=value`, and `value` hasn't been used.
                let value = replace(value, OsString::new());
                self.state = State::None;
                return Err(Error::UnexpectedValue {
                    option: self
                        .format_last_option()
                        .expect("Should only have pending value after long option"),
                    value,
                });
            }
            State::Shorts(ref arg, ref mut pos) => {
                // We're somewhere inside a -abc chain. Because we're in .next(),
                // not .value(), we can assume that the next character is another option.
                match first_codepoint(&arg.as_encoded_bytes()[*pos..]) {
                    Ok(None) => {
                        self.state = State::None;
                    }
                    // If we find "-=[...]" we interpret it as an option.
                    // If we find "-o=..." then there's an unexpected value.
                    // ('-=' as an option exists, see https://linux.die.net/man/1/a2ps.)
                    // clap always interprets it as a short flag in this case, but
                    // that feels sloppy.
                    Ok(Some('=')) if *pos > 1 => {
                        return Err(Error::UnexpectedValue {
                            option: self.format_last_option().unwrap(),
                            value: self.optional_value().unwrap(),
                        });
                    }
                    Ok(Some(ch)) => {
                        *pos += ch.len_utf8();
                        self.last_option = LastOption::Short(ch);
                        return Ok(Some(Arg::Short(ch)));
                    }
                    Err(_) => {
                        // Skip the rest of the argument. This makes it easy to maintain the
                        // OsString invariant, and the caller is almost certainly going to
                        // abort anyway.
                        self.state = State::None;
                        self.last_option = LastOption::Short('�');
                        return Ok(Some(Arg::Short('�')));
                    }
                }
            }
            State::FinishedOpts => {
                return Ok(self.source.next().map(Arg::Value));
            }
            State::None => (),
        }

        match self.state {
            State::None => (),
            ref state => panic!("unexpected state {:?}", state),
        }

        let arg = match self.source.next() {
            Some(arg) => arg,
            None => return Ok(None),
        };

        if arg == "--" {
            self.state = State::FinishedOpts;
            return self.next();
        }

        let arg_bytes = arg.as_encoded_bytes();
        if arg_bytes.starts_with(b"--") {
            let mut arg = arg.as_os_str();
            // Long options have two forms: --option and --option=value.
            if let Some(ind) = arg_bytes.iter().position(|&b| b == b'=') {
                // The value can be an OsString...
                let value = arg.slice_encoded_bytes(ind + 1..).to_owned();

                self.state = State::PendingValue(value);
                arg = arg.slice_encoded_bytes(..ind)
            }

            // ...but the option has to be a string.
            let arg = arg.to_string_lossy().into_owned();
            self.last_option = LastOption::Long(arg);
            let long = match self.last_option {
                LastOption::Long(ref option) => &option[2..],
                _ => unreachable!(),
            };
            Ok(Some(Arg::Long(long)))
        } else if arg_bytes.len() > 1 && arg_bytes[0] == b'-' {
            self.state = State::Shorts(arg, 1);
            self.next()
        } else {
            Ok(Some(Arg::Value(arg)))
        }
    }

    /// Get a value for an option.
    ///
    /// This function should normally be called right after seeing an option
    /// that expects a value, with positional arguments being collected
    /// using [`next()`][Parser::next].
    ///
    /// A value is collected even if it looks like an option
    /// (i.e., starts with `-`).
    ///
    /// # Errors
    ///
    /// An [`Error::MissingValue`] is returned if the end of the command
    /// line is reached.
    pub fn value(&mut self) -> Result<OsString, Error> {
        if let Some(value) = self.optional_value() {
            return Ok(value);
        }

        if let Some(value) = self.source.next() {
            return Ok(value);
        }

        Err(Error::MissingValue {
            option: self.format_last_option(),
        })
    }

    /// Gather multiple values for an option.
    ///
    /// This is used for options that take multiple arguments, such as a
    /// `--command` flag that's invoked as `app --command echo 'Hello world'`.
    ///
    /// It will gather arguments until another option is found, or `--` is found, or
    /// the end of the command line is reached. This differs from `.value()`, which
    /// takes a value even if it looks like an option.
    ///
    /// An equals sign (`=`) will limit this to a single value. That means `-a=b c` and
    /// `--opt=b c` will only yield `"b"` while `-a b c`, `-ab c` and `--opt b c` will
    /// yield `"b"`, `"c"`.
    ///
    /// # Errors
    /// If not at least one value is found then [`Error::MissingValue`] is returned.
    ///
    /// # Example
    /// ```
    /// # fn main() -> Result<(), lexopt::Error> {
    /// # use lexopt::prelude::*;
    /// # use std::ffi::OsString;
    /// # use std::path::PathBuf;
    /// # let mut parser = lexopt::Parser::from_args(&["a", "b", "-x", "one", "two", "three", "four"]);
    /// let arguments: Vec<OsString> = parser.values()?.collect();
    /// # assert_eq!(arguments, &["a", "b"]);
    /// # let _ = parser.next();
    /// let at_most_three_files: Vec<PathBuf> = parser.values()?.take(3).map(Into::into).collect();
    /// # assert_eq!(parser.raw_args()?.as_slice(), &["four"]);
    /// for value in parser.values()? {
    ///     // ...
    /// }
    /// # Ok(()) }
    /// ```
    pub fn values(&mut self) -> Result<ValuesIter<'_>, Error> {
        // This code is designed so that just calling .values() doesn't consume
        // any arguments as long as you don't use the iterator. It used to work
        // differently.
        // "--" is treated like an option and not consumed. This seems to me the
        // least unreasonable behavior, and it's the easiest to implement.
        if self.has_pending() || self.next_is_normal() {
            Ok(ValuesIter {
                took_first: false,
                parser: Some(self),
            })
        } else {
            Err(Error::MissingValue {
                option: self.format_last_option(),
            })
        }
    }

    /// Inspect an argument and consume it if it's "normal" (not an option or --).
    ///
    /// Used by [`Parser::values`].
    ///
    /// This method should not be called while partway through processing an
    /// argument.
    fn next_if_normal(&mut self) -> Option<OsString> {
        if self.next_is_normal() {
            self.source.next()
        } else {
            None
        }
    }

    /// Execute the check for next_if_normal().
    fn next_is_normal(&self) -> bool {
        assert!(!self.has_pending());
        let arg = match self.source.as_slice().first() {
            // There has to be a next argument.
            None => return false,
            Some(arg) => arg,
        };
        if let State::FinishedOpts = self.state {
            // If we already found a -- then we're really not supposed to be here,
            // but we shouldn't treat the next argument as an option.
            return true;
        }
        if arg == "-" {
            // "-" is the one argument with a leading '-' that's allowed.
            return true;
        }
        let lead_dash = arg.as_encoded_bytes().first() == Some(&b'-');

        !lead_dash
    }

    /// Take raw arguments from the original command line.
    ///
    /// This returns an iterator of [`OsString`]s. Any arguments that are not
    /// consumed are kept, so you can continue parsing after you're done with
    /// the iterator.
    ///
    /// To inspect an argument without consuming it, use [`RawArgs::peek`] or
    /// [`RawArgs::as_slice`].
    ///
    /// # Errors
    ///
    /// Returns an [`Error::UnexpectedValue`] if the last option had a left-over
    /// argument, as in `--option=value`, `-ovalue`, or if it was midway through
    /// an option chain, as in `-abc`. The iterator only yields whole arguments.
    /// To avoid this, use [`try_raw_args()`][Parser::try_raw_args].
    ///
    /// After this error the method is guaranteed to succeed, as it consumes the
    /// rest of the argument.
    ///
    /// # Example
    /// As soon as a free-standing argument is found, consume the other arguments
    /// as-is, and build them into a command.
    /// ```
    /// # fn main() -> Result<(), lexopt::Error> {
    /// # use lexopt::prelude::*;
    /// # use std::ffi::OsString;
    /// # use std::path::PathBuf;
    /// # let mut parser = lexopt::Parser::from_args(&["-x", "echo", "-n", "'Hello, world'"]);
    /// # while let Some(arg) = parser.next()? {
    /// # match arg {
    /// Value(prog) => {
    ///     let args: Vec<_> = parser.raw_args()?.collect();
    ///     let command = std::process::Command::new(prog).args(args);
    /// }
    /// # _ => (), }} Ok(()) }
    pub fn raw_args(&mut self) -> Result<RawArgs<'_>, Error> {
        if let Some(value) = self.optional_value() {
            return Err(Error::UnexpectedValue {
                option: self.format_last_option().unwrap(),
                value,
            });
        }

        Ok(RawArgs(&mut self.source))
    }

    /// Take raw arguments from the original command line, *if* the current argument
    /// has finished processing.
    ///
    /// Unlike [`raw_args()`][Parser::raw_args] this does not consume any value
    /// in case of a left-over argument. This makes it safe to call at any time.
    ///
    /// It returns `None` exactly when [`optional_value()`][Parser::optional_value]
    /// would return `Some`.
    ///
    /// Note: If no arguments are left then it returns an empty iterator (not `None`).
    ///
    /// # Example
    /// Process arguments of the form `-123` as numbers. For a complete runnable version of
    /// this example, see
    /// [`examples/nonstandard.rs`](https://github.com/blyxxyz/lexopt/blob/e3754e6f24506afb42394602fc257b1ad9258d84/examples/nonstandard.rs).
    /// ```
    /// # fn main() -> Result<(), lexopt::Error> {
    /// # use lexopt::prelude::*;
    /// # use std::ffi::OsString;
    /// # use std::path::PathBuf;
    /// # let mut parser = lexopt::Parser::from_iter(&["-13"]);
    /// fn parse_dashnum(parser: &mut lexopt::Parser) -> Option<u64> {
    ///     let mut raw = parser.try_raw_args()?;
    ///     let arg = raw.peek()?.to_str()?;
    ///     let num = arg.strip_prefix('-')?.parse::<u64>().ok()?;
    ///     raw.next(); // Consume the argument we just parsed
    ///     Some(num)
    /// }
    ///
    /// loop {
    ///     if let Some(num) = parse_dashnum(&mut parser) {
    ///         println!("Got number {}", num);
    ///     } else if let Some(arg) = parser.next()? {
    ///         match arg {
    ///             // ...
    ///             # _ => (),
    ///         }
    ///     } else {
    ///         break;
    ///     }
    /// }
    /// # Ok(()) }
    /// ```
    pub fn try_raw_args(&mut self) -> Option<RawArgs<'_>> {
        if self.has_pending() {
            None
        } else {
            Some(RawArgs(&mut self.source))
        }
    }

    /// Check whether we're halfway through an argument, or in other words,
    /// if [`Parser::optional_value()`] would return `Some`.
    fn has_pending(&self) -> bool {
        match self.state {
            State::None | State::FinishedOpts => false,
            State::PendingValue(_) => true,
            State::Shorts(ref arg, pos) => pos < arg.len(),
        }
    }

    #[inline(never)]
    fn format_last_option(&self) -> Option<String> {
        match self.last_option {
            LastOption::None => None,
            LastOption::Short(ch) => Some(format!("-{}", ch)),
            LastOption::Long(ref option) => Some(option.clone()),
        }
    }

    /// The name of the command, as in the zeroth argument of the process.
    ///
    /// This is intended for use in messages. If the name is not valid unicode
    /// it will be sanitized with replacement characters as by
    /// [`String::from_utf8_lossy`].
    ///
    /// To get the current executable, use [`std::env::current_exe`].
    ///
    /// # Example
    /// ```
    /// let mut parser = lexopt::Parser::from_env();
    /// let bin_name = parser.bin_name().unwrap_or("myapp");
    /// println!("{}: Some message", bin_name);
    /// ```
    pub fn bin_name(&self) -> Option<&str> {
        Some(self.bin_name.as_ref()?)
    }

    /// Get a value only if it's concatenated to an option, as in `-ovalue` or
    /// `--option=value` or `-o=value`, but not `-o value` or `--option value`.
    pub fn optional_value(&mut self) -> Option<OsString> {
        Some(self.raw_optional_value()?.0)
    }

    /// [`Parser::optional_value`], but indicate whether the value was joined
    /// with an = sign. This matters for [`Parser::values`].
    fn raw_optional_value(&mut self) -> Option<(OsString, bool)> {
        match replace(&mut self.state, State::None) {
            State::PendingValue(value) => Some((value, true)),
            State::Shorts(arg, mut pos) => {
                if pos >= arg.len() {
                    return None;
                }
                let mut had_eq_sign = false;
                if arg.as_encoded_bytes()[pos] == b'=' {
                    // -o=value.
                    // clap actually strips out all leading '='s, but that seems silly.
                    // We allow `-xo=value`. Python's argparse doesn't strip the = in that case.
                    pos += 1;
                    had_eq_sign = true;
                }
                let value = arg.slice_encoded_bytes(pos..).to_owned();
                Some((value, had_eq_sign))
            }
            State::FinishedOpts => {
                // Not really supposed to be here, but it's benign and not our fault
                self.state = State::FinishedOpts;
                None
            }
            State::None => None,
        }
    }

    fn new(bin_name: Option<OsString>, source: InnerIter) -> Parser {
        Parser {
            source,
            state: State::None,
            last_option: LastOption::None,
            bin_name: bin_name.map(|s| match s.into_string() {
                Ok(text) => text,
                Err(text) => text.to_string_lossy().into_owned(),
            }),
        }
    }

    /// Create a parser from the environment using [`std::env::args_os`].
    ///
    /// This is the usual way to create a `Parser`.
    pub fn from_env() -> Parser {
        let mut source = make_iter(std::env::args_os());
        Parser::new(source.next(), source)
    }

    // The collision with `FromIterator::from_iter` is a bit unfortunate.
    // This name is used because:
    // - `from_args()` was taken, and changing its behavior without changing
    //   its signature would be evil.
    // - structopt also had a method by that name, so there's a precedent.
    //   (clap_derive doesn't.)
    // - I couldn't think of a better one.
    // When this name was chosen `FromIterator` could not actually be implemented.
    // It can be implemented now, but I'm not sure there's a reason to.

    /// Create a parser from an iterator. This is useful for testing among other things.
    ///
    /// The first item from the iterator **must** be the binary name, as from [`std::env::args_os`].
    ///
    /// The iterator is consumed immediately.
    ///
    /// # Example
    /// ```
    /// let mut parser = lexopt::Parser::from_iter(&["myapp", "-n", "10", "./foo.bar"]);
    /// ```
    pub fn from_iter<I>(args: I) -> Parser
    where
        I: IntoIterator,
        I::Item: Into<OsString>,
    {
        let mut args = make_iter(args.into_iter().map(Into::into));
        Parser::new(args.next(), args)
    }

    /// Create a parser from an iterator that does **not** include the binary name.
    ///
    /// The iterator is consumed immediately.
    ///
    /// [`bin_name()`](`Parser::bin_name`) will return `None`. Consider using
    /// [`Parser::from_iter`] instead.
    pub fn from_args<I>(args: I) -> Parser
    where
        I: IntoIterator,
        I::Item: Into<OsString>,
    {
        Parser::new(None, make_iter(args.into_iter().map(Into::into)))
    }
}

impl Arg<'_> {
    /// Convert an unexpected argument into an error.
    pub fn unexpected(self) -> Error {
        match self {
            Arg::Short(short) => Error::UnexpectedOption(format!("-{}", short)),
            Arg::Long(long) => Error::UnexpectedOption(format!("--{}", long)),
            Arg::Value(value) => Error::UnexpectedArgument(value),
        }
    }
}

/// An iterator for multiple option-arguments, returned by [`Parser::values`].
///
/// It's guaranteed to yield at least one value.
#[derive(Debug)]
pub struct ValuesIter<'a> {
    took_first: bool,
    parser: Option<&'a mut Parser>,
}

impl Iterator for ValuesIter<'_> {
    type Item = OsString;

    fn next(&mut self) -> Option<Self::Item> {
        let parser = self.parser.as_mut()?;
        if self.took_first {
            parser.next_if_normal()
        } else if let Some((value, had_eq_sign)) = parser.raw_optional_value() {
            if had_eq_sign {
                self.parser = None;
            }
            self.took_first = true;
            Some(value)
        } else {
            let value = parser
                .next_if_normal()
                .expect("ValuesIter must yield at least one value");
            self.took_first = true;
            Some(value)
        }
    }
}

/// An iterator for the remaining raw arguments, returned by [`Parser::raw_args`].
#[derive(Debug)]
pub struct RawArgs<'a>(&'a mut InnerIter);

impl Iterator for RawArgs<'_> {
    type Item = OsString;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

impl RawArgs<'_> {
    /// Return a reference to the next() value without consuming it.
    ///
    /// An argument you peek but do not consume will still be seen by `Parser`
    /// if you resume parsing.
    ///
    /// See [`Iterator::peekable`], [`std::iter::Peekable::peek`].
    pub fn peek(&self) -> Option<&OsStr> {
        Some(self.0.as_slice().first()?.as_os_str())
    }

    /// Consume and return the next argument if a condition is true.
    ///
    /// See [`std::iter::Peekable::next_if`].
    pub fn next_if(&mut self, func: impl FnOnce(&OsStr) -> bool) -> Option<OsString> {
        match self.peek() {
            Some(arg) if func(arg) => self.next(),
            _ => None,
        }
    }

    /// Return the remaining arguments as a slice.
    pub fn as_slice(&self) -> &[OsString] {
        self.0.as_slice()
    }
}

// These would make sense:
// - fn RawArgs::iter(&self)
// - impl IntoIterator for &RawArgs
// - impl AsRef<[OsString]> for RawArgs
// But they're niche and constrain future design.
// Let's leave them out for now.
// (Open question: should iter() return std::slice::Iter<OsString> and get
// an optimized .nth() and so on for free, or should it return a novel type
// that yields &OsStr?)

/// An error during argument parsing.
///
/// This implements `From<String>` and `From<&str>`, for easy ad-hoc error
/// messages.
//
// This is not #[non_exhaustive] because of the MSRV. I'm hoping no more
// variants will turn out to be needed: this seems reasonable, if the scope
// of the library doesn't change. Worst case scenario it can be stuffed inside
// Error::Custom.
pub enum Error {
    /// An option argument was expected but was not found.
    MissingValue {
        /// The most recently emitted option.
        option: Option<String>,
    },

    /// An unexpected option was found.
    UnexpectedOption(String),

    /// A positional argument was found when none was expected.
    UnexpectedArgument(OsString),

    /// An option had a value when none was expected.
    UnexpectedValue {
        /// The option.
        option: String,
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
    /// This can be returned by the methods on [`ValueExt`].
    NonUnicodeValue(OsString),

    /// For custom error messages in application code.
    Custom(Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::Error::*;
        match self {
            MissingValue { option: None } => write!(f, "missing argument"),
            MissingValue {
                option: Some(option),
            } => {
                write!(f, "missing argument for option '{}'", option)
            }
            UnexpectedOption(option) => write!(f, "invalid option '{}'", option),
            UnexpectedArgument(value) => write!(f, "unexpected argument {:?}", value),
            UnexpectedValue { option, value } => {
                write!(
                    f,
                    "unexpected argument for option '{}': {:?}",
                    option, value
                )
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

impl<'a> From<&'a str> for Error {
    fn from(msg: &'a str) -> Self {
        Error::Custom(msg.into())
    }
}

/// For [`OsString::into_string`], so it may be used with the try (`?`) operator.
///
/// [`ValueExt::string`] is the new preferred method because it's compatible with
/// catch-all error types like `anyhow::Error`.
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
/// If parsing fails the error will be wrapped in lexopt's own [`Error`] type.
pub trait ValueExt: private::Sealed {
    /// Decode the value and parse it using [`FromStr`].
    ///
    /// This will fail if the value is not valid unicode or if the subsequent
    /// parsing fails.
    fn parse<T: FromStr>(&self) -> Result<T, Error>
    where
        T::Err: Into<Box<dyn std::error::Error + Send + Sync + 'static>>;

    // TODO: move the F parameter to the end for better turbofishing.
    // This is a breaking change that affects at least one real-world program.
    // But the code will be better off for it, so it's worth doing in the next
    // breaking release.

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

    /// Convert the `OsString` into a [`String`] if it's valid Unicode.
    ///
    /// This is like [`OsString::into_string`] but returns an
    /// [`Error::NonUnicodeValue`] on error instead of the original `OsString`.
    /// This makes it easier to propagate the failure with libraries like
    /// `anyhow`.
    fn string(self) -> Result<String, Error>;
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

    fn string(self) -> Result<String, Error> {
        match self.into_string() {
            Ok(string) => Ok(string),
            Err(raw) => Err(Error::NonUnicodeValue(raw)),
        }
    }
}

/// A small prelude for processing arguments.
///
/// It allows you to write `Short`/`Long`/`Value` without an [`Arg`] prefix
/// and adds convenience methods to [`OsString`].
///
/// If this is used it's best to import it inside a function, not in module
/// scope:
/// ```
/// # struct Args;
/// fn parse_args() -> Result<Args, lexopt::Error> {
///     use lexopt::prelude::*;
///     // ...
///     # Ok(Args)
/// }
/// ```
pub mod prelude {
    pub use super::Arg::*;
    pub use super::ValueExt;
}

/// Take the first codepoint from a UTF-8 bytestring.
///
/// The rest of the bytestring does not have to be valid unicode.
fn first_codepoint(bytes: &[u8]) -> Result<Option<char>, Utf8Error> {
    // UTF-8 takes at most 4 bytes per codepoint.
    let bytes = bytes.get(..4).unwrap_or(bytes);
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) if err.valid_up_to() > 0 => {
            std::str::from_utf8(&bytes[..err.valid_up_to()]).unwrap()
        }
        Err(err) => return Err(err),
    };
    Ok(text.chars().next())
}

#[cfg(test)]
mod tests {
    use std::error::Error as stdError;

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;
    #[cfg(target_os = "wasi")]
    use std::os::wasi::ffi::OsStringExt;
    #[cfg(windows)]
    use std::os::windows::ffi::OsStringExt;

    use super::prelude::*;
    use super::*;

    fn parse(args: &'static str) -> Parser {
        Parser::from_args(args.split_whitespace().map(bad_string))
    }

    /// Specialized backport of matches!()
    macro_rules! assert_matches {
        ($expression: expr, $( $pattern: pat )|+) => {
            match $expression {
                $( $pattern )|+ => (),
                _ => panic!(
                    "{:?} does not match {:?}",
                    stringify!($expression),
                    stringify!($( $pattern )|+)
                ),
            }
        };
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
        assert_eq!(
            p.next().unwrap_err().to_string(),
            r#"unexpected argument for option '--foobar': "qux=baz""#
        );
        assert_eq!(p.next()?, None);
        Ok(())
    }

    #[test]
    fn test_dash_args() -> Result<(), Error> {
        // "--" should indicate the end of the options
        let mut p = parse("-x -- -y");
        assert_eq!(p.next()?.unwrap(), Short('x'));
        assert_eq!(p.next()?.unwrap(), Value("-y".into()));
        assert_eq!(p.next()?, None);

        // ...unless it's an argument of an option
        let mut p = parse("-x -- -y");
        assert_eq!(p.next()?.unwrap(), Short('x'));
        assert_eq!(p.value()?, "--");
        assert_eq!(p.next()?.unwrap(), Short('y'));
        assert_eq!(p.next()?, None);

        // "-" is a valid value that should not be treated as an option
        let mut p = parse("-x - -y");
        assert_eq!(p.next()?.unwrap(), Short('x'));
        assert_eq!(p.next()?.unwrap(), Value("-".into()));
        assert_eq!(p.next()?.unwrap(), Short('y'));
        assert_eq!(p.next()?, None);

        // '-' is a silly and hard to use short option, but other parsers treat
        // it like an option in this position
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
        assert_eq!(
            p.value().unwrap_err().to_string(),
            "missing argument for option '-o'",
        );

        let mut q = parse("--out");
        assert_eq!(q.next()?.unwrap(), Long("out"));
        assert_eq!(
            q.value().unwrap_err().to_string(),
            "missing argument for option '--out'",
        );

        let mut r = parse("");
        assert_eq!(r.value().unwrap_err().to_string(), "missing argument");

        Ok(())
    }

    #[test]
    fn test_weird_args() -> Result<(), Error> {
        let mut p = Parser::from_args(&[
            "", "--=", "--=3", "-", "-x", "--", "-", "-x", "--", "", "-", "-x",
        ]);
        assert_eq!(p.next()?.unwrap(), Value(OsString::from("")));

        // These are weird and questionable, but this seems to be the standard
        // interpretation
        // GNU getopt_long and argparse complain that it could be an abbreviation
        // of every single long option
        // clap complains that "--" is not expected, which matches its treatment
        // of unknown long options
        assert_eq!(p.next()?.unwrap(), Long(""));
        assert_eq!(p.value()?, OsString::from(""));
        assert_eq!(p.next()?.unwrap(), Long(""));
        assert_eq!(p.value()?, OsString::from("3"));

        assert_eq!(p.next()?.unwrap(), Value(OsString::from("-")));
        assert_eq!(p.next()?.unwrap(), Short('x'));
        assert_eq!(p.value()?, OsString::from("--"));
        assert_eq!(p.next()?.unwrap(), Value(OsString::from("-")));
        assert_eq!(p.next()?.unwrap(), Short('x'));
        assert_eq!(p.next()?.unwrap(), Value(OsString::from("")));
        assert_eq!(p.next()?.unwrap(), Value(OsString::from("-")));
        assert_eq!(p.next()?.unwrap(), Value(OsString::from("-x")));
        assert_eq!(p.next()?, None);

        #[cfg(any(unix, target_os = "wasi", windows))]
        {
            let mut q = parse("--=@");
            assert_eq!(q.next()?.unwrap(), Long(""));
            assert_eq!(q.value()?, bad_string("@"));
            assert_eq!(q.next()?, None);
        }

        let mut r = parse("");
        assert_eq!(r.next()?, None);

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

    #[cfg(any(unix, target_os = "wasi", windows))]
    #[test]
    fn test_mixed_invalid() -> Result<(), Error> {
        let mut p = parse("--foo=@@@");
        assert_eq!(p.next()?.unwrap(), Long("foo"));
        assert_eq!(p.value()?, bad_string("@@@"));

        let mut q = parse("-💣@@@");
        assert_eq!(q.next()?.unwrap(), Short('💣'));
        assert_eq!(q.value()?, bad_string("@@@"));

        let mut r = parse("-f@@@");
        assert_eq!(r.next()?.unwrap(), Short('f'));
        assert_eq!(r.next()?.unwrap(), Short('�'));
        assert_eq!(r.next()?, None);

        let mut s = parse("--foo=bar=@@@");
        assert_eq!(s.next()?.unwrap(), Long("foo"));
        assert_eq!(s.value()?, bad_string("bar=@@@"));

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

    #[cfg(any(unix, target_os = "wasi", windows))]
    #[test]
    fn test_invalid_long_option() -> Result<(), Error> {
        let mut p = parse("--@=10");
        assert_eq!(p.next()?.unwrap(), Long("�"));
        assert_eq!(p.value().unwrap(), OsString::from("10"));
        assert_eq!(p.next()?, None);

        let mut q = parse("--@");
        assert_eq!(q.next()?.unwrap(), Long("�"));
        assert_eq!(q.next()?, None);

        Ok(())
    }

    #[test]
    fn short_opt_equals_sign() -> Result<(), Error> {
        let mut p = parse("-a=b");
        assert_eq!(p.next()?.unwrap(), Short('a'));
        assert_eq!(p.value()?, OsString::from("b"));
        assert_eq!(p.next()?, None);

        let mut p = parse("-a=b");
        assert_eq!(p.next()?.unwrap(), Short('a'));
        assert_eq!(
            p.next().unwrap_err().to_string(),
            r#"unexpected argument for option '-a': "b""#
        );
        assert_eq!(p.next()?, None);

        let mut p = parse("-a=");
        assert_eq!(p.next()?.unwrap(), Short('a'));
        assert_eq!(p.value()?, OsString::from(""));
        assert_eq!(p.next()?, None);

        let mut p = parse("-a=");
        assert_eq!(p.next()?.unwrap(), Short('a'));
        assert_eq!(
            p.next().unwrap_err().to_string(),
            r#"unexpected argument for option '-a': """#
        );
        assert_eq!(p.next()?, None);

        let mut p = parse("-=");
        assert_eq!(p.next()?.unwrap(), Short('='));
        assert_eq!(p.next()?, None);

        let mut p = parse("-=a");
        assert_eq!(p.next()?.unwrap(), Short('='));
        assert_eq!(p.value()?, "a");

        Ok(())
    }

    #[cfg(any(unix, target_os = "wasi", windows))]
    #[test]
    fn short_opt_equals_sign_invalid() -> Result<(), Error> {
        let mut p = parse("-a=@");
        assert_eq!(p.next()?.unwrap(), Short('a'));
        assert_eq!(p.value()?, bad_string("@"));
        assert_eq!(p.next()?, None);

        let mut p = parse("-a=@");
        assert_eq!(p.next()?.unwrap(), Short('a'));
        #[cfg(any(unix, target_os = "wasi"))]
        assert_eq!(
            p.next().unwrap_err().to_string(),
            r#"unexpected argument for option '-a': "\xFF""#
        );
        #[cfg(windows)]
        assert_eq!(
            p.next().unwrap_err().to_string(),
            r#"unexpected argument for option '-a': "\u{d800}""#
        );
        assert_eq!(p.next()?, None);

        let mut p = parse("-=@");
        assert_eq!(p.next()?.unwrap(), Short('='));
        assert_eq!(p.value()?, bad_string("@"));

        Ok(())
    }

    #[test]
    fn multi_values() -> Result<(), Error> {
        for &case in &["-a b c d", "-ab c d", "-a b c d --", "--a b c d"] {
            let mut p = parse(case);
            p.next()?.unwrap();
            let mut iter = p.values()?;
            let values: Vec<_> = iter.by_ref().collect();
            assert_eq!(values, &["b", "c", "d"]);
            assert!(iter.next().is_none());
            assert!(p.next()?.is_none());
        }

        for &case in &["-a=b c", "--a=b c"] {
            let mut p = parse(case);
            p.next()?.unwrap();
            let mut iter = p.values()?;
            let values: Vec<_> = iter.by_ref().collect();
            assert_eq!(values, &["b"]);
            assert!(iter.next().is_none());
            assert_eq!(p.next()?.unwrap(), Value("c".into()));
            assert!(p.next()?.is_none());
        }

        for &case in &["-a", "--a", "-a -b", "-a -- b", "-a --"] {
            let mut p = parse(case);
            p.next()?.unwrap();
            assert!(p.values().is_err());
            assert!(p.next().is_ok());
            assert!(p.next().unwrap().is_none());
        }

        for &case in &["-a=", "--a="] {
            let mut p = parse(case);
            p.next()?.unwrap();
            let mut iter = p.values()?;
            let values: Vec<_> = iter.by_ref().collect();
            assert_eq!(values, &[""]);
            assert!(iter.next().is_none());
            assert!(p.next()?.is_none());
        }

        // Test that .values() does not eagerly consume the first value
        for &case in &["-a=b", "--a=b", "-a b"] {
            let mut p = parse(case);
            p.next()?.unwrap();
            assert!(p.values().is_ok());
            assert_eq!(p.value()?, "b");
        }

        {
            let mut p = parse("-ab");
            p.next()?.unwrap();
            assert!(p.values().is_ok());
            assert_eq!(p.next()?.unwrap(), Short('b'));
        }

        Ok(())
    }

    #[test]
    fn raw_args() -> Result<(), Error> {
        let mut p = parse("-a b c d");
        assert!(p.try_raw_args().is_some());
        assert_eq!(p.raw_args()?.collect::<Vec<_>>(), &["-a", "b", "c", "d"]);
        assert!(p.try_raw_args().is_some());
        assert!(p.next()?.is_none());
        assert!(p.try_raw_args().is_some());
        assert_eq!(p.raw_args()?.as_slice().len(), 0);

        let mut p = parse("-ab c d");
        p.next()?;
        assert!(p.try_raw_args().is_none());
        assert!(p.raw_args().is_err());
        assert_eq!(p.try_raw_args().unwrap().collect::<Vec<_>>(), &["c", "d"]);
        assert!(p.next()?.is_none());
        assert_eq!(p.try_raw_args().unwrap().as_slice().len(), 0);

        let mut p = parse("-a b c d");
        assert_eq!(p.raw_args()?.take(3).collect::<Vec<_>>(), &["-a", "b", "c"]);
        assert_eq!(p.next()?, Some(Value("d".into())));
        assert!(p.next()?.is_none());

        let mut p = parse("a");
        let mut it = p.raw_args()?;
        assert_eq!(it.peek(), Some("a".as_ref()));
        assert_eq!(it.next_if(|_| false), None);
        assert_eq!(
            it.next_if(|arg| {
                assert_eq!(arg, "a");
                true
            }),
            Some("a".into())
        );
        assert!(p.next()?.is_none());

        Ok(())
    }

    #[test]
    fn bin_name() {
        assert_eq!(
            Parser::from_iter(&["foo", "bar", "baz"]).bin_name(),
            Some("foo")
        );
        assert_eq!(Parser::from_args(&["foo", "bar", "baz"]).bin_name(), None);
        assert_eq!(Parser::from_iter(&[] as &[&str]).bin_name(), None);
        assert_eq!(Parser::from_iter(&[""]).bin_name(), Some(""));
        #[cfg(any(unix, target_os = "wasi", windows))]
        {
            assert!(Parser::from_env().bin_name().is_some());
            assert_eq!(
                Parser::from_iter(vec![bad_string("foo@bar")]).bin_name(),
                Some("foo�bar")
            );
        }
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
        assert_eq!(
            s.parse::<u32>().unwrap_err().to_string(),
            r#"cannot parse argument "-10": invalid digit found in string"#,
        );
        assert_eq!(
            s.parse_with(|s| match s {
                "11" => Ok(0_i32),
                _ => Err("bad"),
            })
            .unwrap_err()
            .to_string(),
            r#"cannot parse argument "-10": bad"#,
        );
        assert_eq!(s.string()?, "-10");
        Ok(())
    }

    #[cfg(any(unix, target_os = "wasi", windows))]
    #[test]
    fn test_value_ext_invalid() -> Result<(), Error> {
        let s = bad_string("foo@");
        #[cfg(any(unix, target_os = "wasi"))]
        let message = r#"argument is invalid unicode: "foo\xFF""#;
        #[cfg(windows)]
        let message = r#"argument is invalid unicode: "foo\u{d800}""#;
        assert_eq!(s.parse::<i32>().unwrap_err().to_string(), message);
        assert_eq!(
            s.parse_with(<f32 as FromStr>::from_str)
                .unwrap_err()
                .to_string(),
            message,
        );
        assert_eq!(s.clone().string().unwrap_err().to_string(), message);
        assert_eq!(
            Error::from(s.into_string().unwrap_err()).to_string(),
            message,
        );
        Ok(())
    }

    #[test]
    fn test_errors() {
        assert_eq!(
            Arg::Short('o').unexpected().to_string(),
            "invalid option '-o'",
        );
        assert_eq!(
            Arg::Long("opt").unexpected().to_string(),
            "invalid option '--opt'",
        );
        assert_eq!(
            Arg::Value("foo".into()).unexpected().to_string(),
            r#"unexpected argument "foo""#,
        );
        assert_eq!(
            Error::from("this is an error message").to_string(),
            "this is an error message",
        );
        assert_eq!(
            Error::from("this is an error message".to_owned()).to_string(),
            "this is an error message",
        );
        assert!(Error::from("this is an error message").source().is_some());
        assert!(OsString::from("foo")
            .parse::<i32>()
            .unwrap_err()
            .source()
            .is_some());
        assert!(Arg::Short('o').unexpected().source().is_none());
        assert_eq!(
            format!("{:?}", Arg::Short('o').unexpected()),
            "invalid option '-o'",
        );
    }

    #[test]
    fn test_first_codepoint() {
        assert_eq!(first_codepoint(b"foo").unwrap(), Some('f'));
        assert_eq!(first_codepoint(b"").unwrap(), None);
        assert_eq!(first_codepoint(b"f\xFF\xFF").unwrap(), Some('f'));
        assert_eq!(first_codepoint(b"\xC2\xB5bar").unwrap(), Some('µ'));
        assert_eq!(first_codepoint(b"foo\xC2\xB5").unwrap(), Some('f'));
        assert_eq!(
            first_codepoint(b"\xFF\xFF").unwrap_err().error_len(),
            Some(1)
        );
        assert_eq!(first_codepoint(b"\xC2").unwrap_err().error_len(), None);
        assert_eq!(first_codepoint(b"\xC2a").unwrap_err().error_len(), Some(1));
        assert_eq!(first_codepoint(b"\xF0").unwrap_err().error_len(), None);
        assert_eq!(
            first_codepoint(b"\xF0\x9D\x84").unwrap_err().error_len(),
            None
        );
        assert_eq!(
            first_codepoint(b"\xF0\x9Da").unwrap_err().error_len(),
            Some(2)
        );
        assert_eq!(
            first_codepoint(b"\xF0\x9D\x84a").unwrap_err().error_len(),
            Some(3)
        );
        assert_eq!(first_codepoint(b"\xF0\x9D\x84\x9E").unwrap(), Some('𝄞'));
    }

    /// Transform @ characters into invalid unicode.
    fn bad_string(text: &str) -> OsString {
        #[cfg(any(unix, target_os = "wasi"))]
        {
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
            let mut out = Vec::new();
            for ch in text.chars() {
                if ch == '@' {
                    out.push(0xD800);
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

    /// Basic exhaustive testing of short combinations of "interesting"
    /// arguments. They should not panic, not hang, and pass some checks.
    ///
    /// The advantage compared to full fuzzing is that it runs on all platforms
    /// and together with the other tests. cargo-fuzz doesn't work on Windows
    /// and requires a special incantation.
    ///
    /// A disadvantage is that it's still limited by arguments I could think of
    /// and only does very short sequences. Another is that it's bad at
    /// reporting failure, though the println!() helps.
    ///
    /// This test takes a while to run.
    #[test]
    fn basic_fuzz() {
        #[cfg(any(windows, unix, target_os = "wasi"))]
        const VOCABULARY: &[&str] = &[
            "", "-", "--", "---", "a", "-a", "-aa", "@", "-@", "-a@", "-@a", "--a", "--@", "--a=a",
            "--a=", "--a=@", "--@=a", "--=", "--=@", "--=a", "-@@", "-a=a", "-a=", "-=", "-a-",
        ];
        #[cfg(not(any(windows, unix, target_os = "wasi")))]
        const VOCABULARY: &[&str] = &[
            "", "-", "--", "---", "a", "-a", "-aa", "--a", "--a=a", "--a=", "--=", "--=a", "-a=a",
            "-a=", "-=", "-a-",
        ];
        exhaust(Parser::new(None, Vec::new().into_iter()), 0);
        let vocabulary: Vec<OsString> = VOCABULARY.iter().map(|&s| bad_string(s)).collect();
        let mut permutations = vec![vec![]];
        for _ in 0..3 {
            let mut new = Vec::new();
            for old in permutations {
                for word in &vocabulary {
                    let mut extended = old.clone();
                    extended.push(word);
                    new.push(extended);
                }
            }
            permutations = new;
            for permutation in &permutations {
                println!("{:?}", permutation);
                let p = Parser::from_args(permutation);
                exhaust(p, 0);
            }
        }
    }

    /// Run many sequences of methods on a Parser.
    fn exhaust(mut parser: Parser, depth: u16) {
        if depth > 100 {
            panic!("Stuck in loop");
        }

        // has_pending() == optional_value().is_some()
        if parser.has_pending() {
            {
                let mut parser = parser.clone();
                assert!(parser.try_raw_args().is_none());
                assert!(parser.try_raw_args().is_none());
                assert!(parser.raw_args().is_err());
                // Recovery possible
                assert!(parser.raw_args().is_ok());
                assert!(parser.try_raw_args().is_some());
            }

            {
                let mut parser = parser.clone();
                assert!(parser.optional_value().is_some());
                exhaust(parser, depth + 1);
            }
        } else {
            let prev_state = parser.state.clone();
            let prev_remaining = parser.source.as_slice().len();
            assert!(parser.optional_value().is_none());
            assert!(parser.raw_args().is_ok());
            assert!(parser.try_raw_args().is_some());
            // Verify state transitions
            match prev_state {
                State::None | State::PendingValue(_) => {
                    assert_matches!(parser.state, State::None);
                }
                State::Shorts(arg, pos) => {
                    assert_eq!(pos, arg.len());
                    assert_matches!(parser.state, State::None);
                }
                State::FinishedOpts => assert_matches!(parser.state, State::FinishedOpts),
            }
            // No arguments were consumed
            assert_eq!(parser.source.as_slice().len(), prev_remaining);
        }

        {
            let mut parser = parser.clone();
            match parser.next() {
                Ok(None) => {
                    assert_matches!(parser.state, State::None | State::FinishedOpts);
                    assert_eq!(parser.source.as_slice().len(), 0);
                }
                _ => exhaust(parser, depth + 1),
            }
        }

        {
            let mut parser = parser.clone();
            match parser.value() {
                Err(_) => {
                    assert_matches!(parser.state, State::None | State::FinishedOpts);
                    assert_eq!(parser.source.as_slice().len(), 0);
                }
                Ok(_) => {
                    assert_matches!(parser.state, State::None | State::FinishedOpts);
                    exhaust(parser, depth + 1);
                }
            }
        }

        {
            match parser.values() {
                Err(_) => (),
                Ok(iter) => {
                    assert!(iter.count() > 0);
                    exhaust(parser, depth + 1);
                }
            }
        }
    }
}
