Some rambling thoughts that don't deserve a place in the README.

# Cleanness
The API consists of a handful of clean simple powerful methods with no arguments and no configurability, plus some more junk to make them convenient to use.

Language features are to be preferred over library features as much as possible. That way the library can stay smaller and code that uses the library is hopefully easier to understand in detail for people who aren't familiar with the library.

I don't really like the `ValueExt` extension trait, but I can't think of a nicer way to parse values. In my ideal workflow you would call `.into_string()?.parse()?` to parse a value, all built-in methods. But I don't think it's possible to have an error type that can be transformed from both methods' error types, `into_string` returns `OsString` and there are annoying rules around overlapping trait implementations. The error messages would also suffer.

(Update: as of 0.3.0, `ValueExt` has a `string` method as an alternative to `into_string` with a cleaner return type. In theory this opens the way to removing `From<OsString>`, but I don't think `lexopt::Error` should be catch-all. There's `anyhow` for that.)

Keeping the core API clean and generic means this could perhaps be used as the basis of a more complete parser.

# Possible enhancements
POSIX has a notion of subarguments, combining multiple values in a single option-argument by separating them with commas or spaces. This is easy enough to hand-roll for valid unicode (`.into_string()?.split(...)`) but we could provide a function that does it on `OsString`s. I can't think of a case where values may not be valid unicode but definitely don't contain commas or spaces, though.

# Language quirks
Sometimes Rust is a bother.

`Arg::Long` contains a borrowed string instead of an owned string because you can't match owned strings against string literals. That means `Arg` needs a lifetime, the iterator protocol cannot be used (it would also be a bad fit for other reasons), and some abstractions are hard or impossible to build. On the plus side, error messages can be slightly better.

(Deref patterns would fix this, and if/when they're released I'll probably do a breaking release with a huge MSRV bump. I consider them a prerequisite for 1.0.)

Arguments on Windows sometimes have to be transcoded three times: from UTF-16 to WTF-8 by `args_os`, then back to UTF-16 to parse them, then to WTF-8 again to be used. This ensures we see the original invalid code unit if there's a problem, but it's a bit sad. (Luckily it only happens very rarely.)

# Errors
There's not always enough information for a good error message. A plain `OsString` doesn't remember what the parser knows, like what the last option was.

`ValueExt::parse` exists to include the original string in an error message and to wrap all errors inside a uniform type. It's unclear if it earns its upkeep.

# Iterator backing
I see three ways to store `Parser`'s internal iterator:

1. As a generic field (`Parser<I> where I: Iterator<Item = OsString>`)
2. As a trait object (`source: Box<dyn Iterator<Item = OsString> + 'static>`)
3. As a particular known type (`source: std::vec::IntoIter<OsString>`)

lexopt originally used option 2 but switched to option 3.

**Option 1** (generic field) is the most general and powerful but it's cumbersome and bloated. Benefits:

- The parser inherits the iterator's properties. You can have a non-`'static` parser, or a parser that is or isn't thread-safe.
- You can provide direct access to the original iterator.
- In theory, better optimization.

Drawbacks:

- Using a parser as an argument (or return value, or field) is difficult. You have to name the whole type (e.g. `Parser<std::env::ArgsOs>`), and you can't mix and match parsers created from different iterators.
- Code size and compile times are bloated, particularly if you use multiple iterator types.
- The benefits are pretty weak or niche.

**Option 2** (trait object) doesn't have the drawbacks of option 1, but it reduces everything to a lowest common denominator:

- Either the input must be `Send`/`Sync`, or the parser can't be `Send`/`Sync`. (To complicate things, `ArgsOs` is `!Send` and `!Sync` out of caution.)
- `Clone` can't be implemented. (Unless you exhaust the original iterator, which requires interior mutability and has bad edge cases.)
- `Debug` can't be derived.

**Option 3** (known type) means collecting the iterator into a `Vec` when the parser is constructed and then turning that into an iterator.

- The biggest benefit is that `vec::IntoIter` is a well-behaved type and everything becomes easy. It's `Sync` and `Send` and `Clone` and `Debug`. `Debug` even shows the raw arguments.
- We get unlimited lookahead through `vec::IntoIter::as_slice()`.
- `FromIterator` can be implemented. (I didn't implement it yet because I can't think of a reason to.)

There are also drawbacks:

- It's likely to be less efficient. But not disastrously so: `args_os()` allocates a brand-new `Vec` full of brand-new `OsString`s (each with their own allocation) before returning, and we only duplicate the `Vec` allocation.
- Iterators can't be infinite or otherwise avoid loading all arguments into memory at once.
- You can't use [clever tricks](https://gist.github.com/blyxxyz/06b45c82c4a4f1030a89e0289adebf09) to observe which argument is being processed.
  - [`RawArgs::as_slice()`](https://docs.rs/lexopt/latest/lexopt/struct.RawArgs.html#method.as_slice) mostly replaces this.

# Configuration
lexopt could not originally be configured. As of writing it has a single setting, [`set_short_equals`](https://docs.rs/lexopt/latest/lexopt/struct.Parser.html#method.set_short_equals). This setting is pretty niche but there's no way to implement it in user code.

There's also a request to [make `.value()` ignore arguments that look like options](https://github.com/blyxxyz/lexopt/issues/14) but this can be emulated by calling `.values()` in a wacky way.

Both settings are potentially context-sensitive. An option might need to take negative numbers as values, or arbitrary filenames. That means you might want to switch the option on/off just for the duration of parsing a single option. This is one reason that the setting was implemented as a `&mut self` method on `Parser`. However, you have to remember to revert the configuration once you're done.

Other possible APIs are:
- A `ParserBuilder` type that outputs a `Parser`
- A `Config` struct that's passed to `Parser` in some way
- A wrapper, e.g. `parser.allow_dash(false).value()` with `allow_dash() -> SomeWrapper`

I don't expect any more settings beyond `set_short_equals`. If the need does arise they can hopefully use the same signature.

# Problems in other libraries
These are all defensible design choices, they're just a bad fit for some of the programs I want to write. All of them make some other kind of program easier to write.

## pico-args
- Results can be erratic in edge cases: option arguments may be interpreted as options, the order in which you request options matters, and arguments may get treated as if they're next to each other if the arguments inbetween get parsed first.
- `--` as a separator is not built in.
- Arguments that are not valid unicode are not recognized as options, even if they start with a dash.
- Left-over arguments are ignored by default. I prefer when the path of least resistance is strict.
- It uses `Vec::remove`, so it's potentially slow if you pass many thousands of options. (This is a bit academic, there's no problem for realistic workloads.)

These make the library simpler and smaller, which is the whole point.

## clap
clap is very large, both in API surface and in code size.

It also [used](https://github.com/blyxxyz/lexopt/blob/d00be2711096c088cf198650d8853b2420516be3/DESIGN.md#clapstructopt) to suffer from bad defaults for OS strings and options that take multiple values. These problems are solved in current versions. (It still doesn't nudge people toward OS strings the way lexopt does but that's very reasonable.)

Despite the size and complexity it's my personal first choice for complicated interfaces.

# Minimum Supported Rust Version
The current MSRV is 1.31, the first release of the 2018 edition.

The blocker for moving it even earlier was non-lexical lifetimes, there's some code that won't compile without it.

The `Value(arg) if foo.is_none() =>` pattern doesn't actually work until 1.39 ([`bind_by_move_pattern_guards`](https://github.com/rust-lang/rust/pull/63118)), so not all of the examples compile on the MSRV. (And one of them uses `str::strip_prefix`, which requires at least 1.45.)

All these versions are very old. The MSRV will be raised dramatically once there is any reason to do so. Right now there isn't.
