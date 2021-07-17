//! POSIX [recommends](https://pubs.opengroup.org/onlinepubs/9699919799/basedefs/V1_chap12.html#tag_12_02)
//! that no more options are parsed after the first positional argument.
//! The other arguments are then all treated as positional arguments.
//!
//! There's a trick to get this behavior from lexopt. After seeing the first
//! positional argument (`Arg::Value`), repeatedly call `.value()` until it
//! returns an error. This works correctly because:
//!
//! - `Parser::value()` only returns an error when the command line is exhausted.
//!
//! - Seeing an `Arg::Value` means that you're not accidentally consuming
//!   an actual option-argument, like from `--option=value`.
//!
//! The most logical thing to then do is often to collect the values
//! into a `Vec`. This is shown below.

fn main() -> Result<(), lexopt::Error> {
    use lexopt::prelude::*;

    let mut parser = lexopt::Parser::from_env();
    let mut free = Vec::new();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('n') | Long("number") => {
                let num: u16 = parser.value()?.parse()?;
                println!("Got number {}", num);
            }
            Long("shout") => {
                println!("Got --shout");
            }
            Value(val) => {
                free.push(val);
                free.extend(std::iter::from_fn(|| parser.value().ok()));
            }
            _ => return Err(arg.unexpected()),
        }
    }
    println!("Got free args {:?}", free);
    Ok(())
}
