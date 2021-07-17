Some rambling thoughts that don't deserve a place in the README.

# Cleanness
The API consists of two clean simple powerful methods with no arguments and no configurability, plus some more junk to make them convenient to use.

Language features are to be preferred over library features as much as possible. That way the library can stay smaller and code that uses the library is hopefully easier to understand in detail for people who aren't familiar with the library.

I don't really like the `ValueExt` extension trait, but I can't think of a nicer way to parse values. In my ideal workflow you would call `.into_string()?.parse()?` to parse a value, all built-in methods. But I don't think it's possible to have an error type that can be transformed from both methods' error types, `into_string` returns `OsString` and there are annoying rules around overlapping trait implementations. The error messages would also suffer.

Keeping the core API clean and generic means this could perhaps be used as the basis of a more complete parser.

# Possible enhancements
Some programs have options with optional arguments. `-fvalue` counts, `-f value` does not. There's a private method that supports exactly this behavior but I don't know if exposing it is a good idea.

POSIX has a notion of subarguments, combining multiple values in a single option-argument by separating them with commas or spaces. This is easy enough to hand-roll for valid unicode (`.into_string()?.split(...)`) but we could provide a function that does it on `OsString`s. I can't think of a case where values may not be valid unicode but definitely don't contain commas or spaces, though.

# Language quirks
Sometimes Rust is a bother.

`Arg::Long` contains a borrowed string instead of an owned string because you can't match owned strings against string literals. That means `Arg` needs a lifetime, the iterator protocol cannot be used (it would also be a bad fit for other reasons), and error messages can be slightly better.

Arguments on Windows sometimes have to be transcoded three times: from UTF-16 to WTF-8 by `args_os`, then back to UTF-16 to parse them, then to WTF-8 again to be used. This ensures we see the original invalid code unit if there's a problem, but it's a bit sad.

# Errors
There's not always enough information for a good error message. A plain `OsString` doesn't remember what the parser knows, like what the last option was.

`ValueExt::parse` exists to include the original string in an error message and to wrap all errors inside a uniform type. It's unclear if it earns its upkeep.

# Problems in other libraries
These are all defensible design choices, they're just a bad fit for some of the programs I want to write. All of them make some other kind of program easier to write.

## pico-args
- Results can be erratic in edge cases: option arguments may be interpreted as options, the order in which you request options matters, and arguments may get treated as if they're next to each other if the arguments inbetween get parsed first.
- `--` as a separator is not built in.
- Arguments that are not valid unicode are not recognized as options, even if they start with a dash.
- Left-over arguments are ignored by default. I prefer when the path of least resistance is strict.
- It uses `Vec::remove`, so it's potentially slow if you pass many thousands of options. (This is a bit academic, there's no problem for realistic workloads.)

These make the library simpler and smaller, which is the whole point.

## clap/structopt
- structopt nudges the user toward needlessly panicking on invalid unicode: even if a field has type `OsString` or `PathBuf` it'll round-trip through a unicode string and panic unless `from_os_str` is used. (I don't know if this is fixable even in theory while keeping the API ergonomic.)
- Invalid unicode can cause a panic instead of a soft error.
- Options with a variable number of arguments are supported, even though they're ambiguous. In structopt you need to take care not to enable this if you want an option that can occur multiple times with a single argument each time.
- They're large, both in API surface and in code size.

That said, it's still my first choice for complicated interfaces.

(I don't know how much of this applies to clap v3 and clap-derive.)

# Minimum Supported Rust Version
The current MSRV is 1.31, the first release of the 2018 edition.

The blocker for moving it even earlier is non-lexical lifetimes, there's some code that won't compile without it.

The `Value(arg) if foo.is_none() =>` pattern doesn't actually work until 1.39 ([`bind_by_move_pattern_guards`](https://github.com/rust-lang/rust/pull/63118)), so not all of the examples compile on the MSRV.
