//! A partial implementation of HTTPie's command line.
//!
//! This gives an example of some hairier patterns.
//!
//! Based on https://github.com/ducaale/xh/blob/bdb97f/src/cli.rs

use std::{path::PathBuf, str::FromStr};

const HELP: &str = "\
Usage: http [OPTION]... [METHOD] URL [REQUEST_ITEM]...

  -j, --json                 Send as JSON (default)
  -f, --form                 Send as HTTP form
  -o, --output=FILE          Write to FILE instead of stdout
  --pretty=STYLE             Change output format [all, colors, format, none]
  --stream                   Stream output as it's received
  --no-stream                Do not stream output
  --proxy=PROTOCOL:URL...    Proxy PROTOCOL over URL
";

#[derive(Debug)]
struct Args {
    // These flags should conflict with and override each other.
    json: bool,
    form: bool,

    // Flag with argument.
    output: Option<PathBuf>,

    // An enum with a small set of possible values.
    pretty: Option<Pretty>,

    // Ordinary binary flag, but there's also a flag to negate it.
    stream: bool,

    // Can be passed multiple times to add more proxies.
    proxies: Vec<Proxy>,

    // Positional arguments with complex logic.
    method: String,
    url: Url,
    request_items: Vec<RequestItem>,
}

fn parse_args() -> Result<Args, optic::Error> {
    use optic::prelude::*;

    let mut json = true;
    let mut form = false;
    let mut output = None;
    let mut pretty = None;
    let mut stream = false;
    let mut proxies = Vec::new();
    let mut method = None;
    let mut url = None;
    let mut request_items = Vec::new();

    let mut parser = optic::Parser::from_env();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('j') | Long("json") => {
                json = true;
                form = false;
            }
            Short('f') | Long("form") => {
                json = false;
                form = true;
            }
            Short('o') | Long("output") => {
                // .value() grabs a value for the flag.
                // This is an OsString, which we convert into a PathBuf.
                output = Some(parser.value()?.into());
            }
            Long("pretty") => {
                // We can call .parse() to parse a value, if it implements FromStr.
                // The prelude added that method to OsString.
                pretty = Some(parser.value()?.parse()?);
            }
            Long("stream") => {
                stream = true;
            }
            Long("no-stream") => {
                stream = false;
            }
            Long("proxy") => {
                // If we don't have a FromStr implementation or it doesn't do
                // what we want we can use a custom function.
                proxies.push(parser.value()?.parse_with(|s| {
                    // Starting from Rust 1.52, use str::split_once instead:
                    // https://doc.rust-lang.org/std/primitive.str.html#method.split_once
                    let split_arg: Vec<&str> = s.splitn(2, ':').collect();
                    match split_arg[..] {
                        ["http", url] => Ok(Proxy::Http(url.parse()?)),
                        ["https", url] => Ok(Proxy::Https(url.parse()?)),
                        ["all", url] => Ok(Proxy::All(url.parse()?)),
                        [_, _] => Err("Unknown protocol. Pick from: http, https, all"),
                        _ => Err("Invalid proxy. Format as <PROTOCOL>:<PROXY_URL>"),
                    }
                })?);
            }
            Long("help") => {
                print!("{}", HELP);
                std::process::exit(0);
            }
            // The logic here is:
            // - If the first argument looks like an HTTP method then it is one and the
            //   second is the URL.
            // - Otherwise the first argument is the URL.
            // - The other arguments are request items.
            Value(arg) if url.is_none() => {
                // If we want a plain String we can call .into_string() (part of the stdlib).
                let arg = arg.into_string()?;
                if method.is_none() && arg.chars().all(|c| c.is_ascii_alphabetic()) {
                    method = Some(arg.to_uppercase());
                } else {
                    url = Some(arg.parse()?);
                }
            }
            Value(arg) => {
                request_items.push(arg.parse()?);
            }
            _ => return Err(arg.unexpected()),
        }
    }

    Ok(Args {
        json,
        form,
        output,
        pretty,
        stream,
        proxies,
        method: method.unwrap_or_else(|| "GET".to_owned()),
        url: url.ok_or("missing URL")?,
        request_items,
    })
}

#[derive(Debug)]
enum Pretty {
    All,
    Colors,
    Format,
    None,
}

// clap has a macro for this: https://docs.rs/clap/2.33.3/clap/macro.arg_enum.html
// We have to do it manually.
impl FromStr for Pretty {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "all" => Ok(Pretty::All),
            "colors" => Ok(Pretty::Colors),
            "format" => Ok(Pretty::Format),
            "none" => Ok(Pretty::None),
            _ => Err(format!(
                "Invalid style '{}' [pick from: all, colors, format, none]",
                s
            )),
        }
    }
}

// Simplified for the sake of the example.
#[derive(Debug)]
struct Url(String);

impl FromStr for Url {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains("://") {
            Ok(Url(s.into()))
        } else {
            Err("URL does not have a scheme")
        }
    }
}

#[derive(Debug)]
enum Proxy {
    Http(Url),
    Https(Url),
    All(Url),
}

// These are actually pretty complicated but we'll simplify things.
#[derive(Debug)]
struct RequestItem {
    key: String,
    value: String,
}

impl FromStr for RequestItem {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let split_arg: Vec<&str> = s.splitn(2, '=').collect();
        if let [key, value] = split_arg[..] {
            Ok(RequestItem {
                key: key.into(),
                value: value.into(),
            })
        } else {
            Err("missing = sign")
        }
    }
}

fn main() -> Result<(), optic::Error> {
    let args = parse_args()?;
    println!("{:#?}", args);
    Ok(())
}
