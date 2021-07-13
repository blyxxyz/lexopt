# Lexopt

Lexopt is an *imperative* argument parser. Most parsers are declarative: they are told which flags to expect. Lexopt instead returns flags as it encounters them and leaves their handling up to you.

A goal is to have the absolute simplest possible design that's still correct.

Lexopt is:
- Small: one file, no dependencies, no macros. Easy to audit or vendor.
- Correct: common conventions are supported and ambiguity is avoided. Tested and fuzzed.
- Pedantic: arguments are returned as [`OsString`](https://doc.rust-lang.org/std/ffi/struct.OsString.html)s, forcing you to convert them explicitly.
- Annoyingly minimalist: only the bare necessities are provided, things like subcommands and default values are up to the user to implement.
- Unhelpful: there is no help generation and error messages often lack context.

## Example
```rust
struct Args {
    thing: String,
    number: u32,
    shout: bool,
}

fn parse_args() -> Result<Args, lexopt::Error> {
    use lexopt::prelude::*;

    let mut thing = None;
    let mut number = 1;
    let mut shout = false;
    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('n') | Long("number") => {
                number = parser.value()?.parse()?;
            }
            Long("shout") => {
                shout = true;
            }
            Value(val) if thing.is_none() => {
                thing = Some(val.into_string()?);
            }
            Long("help") => {
                println!("Usage: hello [-n|--number=NUM] [--shout] THING");
                std::process::exit(0);
            }
            _ => return Err(arg.unexpected()),
        }
    }

    Ok(Args {
        thing: thing.ok_or("missing argument THING")?,
        number,
        shout,
    })
}

fn main() -> Result<(), lexopt::Error> {
    let args = parse_args()?;
    let mut message = format!("Hello {}", args.thing);
    if args.shout {
        message = message.to_uppercase();
    }
    for _ in 0..args.number {
        println!("{}", message);
    }
    Ok(())
}

```

Let's walk through this:
- We start parsing with `Parser::from_env()`.
- We call `parser.next()` in a loop to get all the arguments until they run out.
- We match on arguments. `Short` and `Long` indicate an option.
- To get the value that belongs to an option (like `10` in `-n 10`) we call `parser.value()`.
  - This returns a standard [`OsString`](https://doc.rust-lang.org/std/ffi/struct.OsString.html).
  - For convenience, `use lexopt::prelude::*` adds a `.parse()` method, analogous to [`str::parse`](https://doc.rust-lang.org/std/primitive.str.html#method.parse).
- `Value` indicates a free-standing argument. In this case, a filename.
  - `if thing.is_none()` is a useful pattern for positional arguments. If we already found `thing` we pass it on to another case.
  - It also contains an `OsString`.
    - The standard `.into_string()` method can decode it into a plain `String`.
- If we don't know what to do with an argument we use `return Err(arg.unexpected())` to turn it into an error message.
- Strings can be promoted to errors for custom error messages.

This covers almost all the functionality in the library. Lexopt does very little for you.

For a more complete example with useful patterns, see [`examples/httpie.rs`](examples/httpie.rs).

## Command line syntax
The following conventions are supported:
- Short flags (`-q`)
- Long flags (`--verbose`)
- `--` to mark the end of options
- `=` to separate long flags from values (`--flag=value`)
- Spaces to separate flags from values (`--flag value`, `-f value`)
- Unseparated short flags (`-fvalue`)
- Combined flags (`-abc` to mean `-a -b -c`)

These are not supported:
- `-f=value` for short flags
- Flags with optional arguments (like GNU sed's `-i`, which can be used standalone or as `-iSUFFIX`)
- Single-dash long flags (like find's `-name`)
- Abbreviated long flags (GNU's getopt lets you write `--num` instead of `--number` if it can be expanded unambiguously)

## Unicode
This library supports unicode while tolerating non-unicode arguments.

Short flags may be unicode, but only a single codepoint. (If you need whole grapheme clusters you can use a long flag. If you need normalization you're on your own, but it can be done.)

Flags can be combined with non-unicode arguments. That is, `--flag=���` will not cause an error. This is surprisingly tricky to support: see [`os_str_bytes`](https://crates.io/crates/os_str_bytes).

Flags that are invalid unicode will always cause an `Error::UnexpectedFlag`.

## Why?
For a particular application I was looking for a small parser that's pedantically correct. There are other compact argument parsing libraries, but I couldn't find one that handled `OsString`s and implemented all the fiddly details of the argument syntax faithfully.

This library may also be useful if a lot of control is desired, like when the exact argument order matters or not all options are known ahead of time. It could be considered more of a lexer than a parser.

## Why not?
This library may not be worth using if:
- You don't care about non-unicode arguments
- You don't care about exact compliance and correctness
- You hate boilerplate

## See also
- [`clap`](https://github.com/clap-rs/clap)/[`structopt`](https://github.com/TeXitoi/structopt): very fully-featured. The only other argument parser for Rust I know of that truly handles invalid unicode properly, if used right.
- [`argh`](https://github.com/google/argh) and [`gumdrop`](https://github.com/murarth/gumdrop): much leaner, yet still convenient and powerful enough for most purposes.
- [`pico-args`](https://github.com/RazrFalcon/pico-args): similar size to lexopt and easier to use (but less rigorous).
- [`ap`](https://docs.rs/ap): I have not used this, but it seems to support iterative parsing while being less bare-bones than lexopt.
- libc's [`getopt`](https://en.wikipedia.org/wiki/Getopt#Examples).

pico-args has a [nifty table](https://github.com/RazrFalcon/pico-args#alternatives) with build times and code sizes for different parsers. I've rerun the tests and added lexopt (using the program in `examples/pico_test_app.rs`):

|                        | null     | lexopt   | pico-args   | clap     | gumdrop  | structopt | argh     |
|------------------------|----------|----------|-------------|----------|----------|-----------|----------|
| Binary overhead        | 0KiB     | 14.4KiB  | **13.5KiB** | 372.8KiB | 17.7KiB  | 371.2KiB  | 16.8KiB  |
| Build time             | 0.9s     | 1.7s     | **1.6s**    | 13.0s    | 7.5s     | 17.0s     | 7.5s     |
| Number of dependencies | 0        | **0**    | **0**       | 8        | 4        | 19        | 6        |
| Tested version         | -        | 0.1.0    | 0.4.2       | 2.33.3   | 0.8.0    | 0.3.22    | 0.1.4    |

(tests were run on Linux with Rust 1.53 and cargo-bloat 0.10.1.)
