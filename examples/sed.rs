use std::{ffi::OsString, path::PathBuf};

#[derive(Default, Debug)]
struct Args {
    scripts: Vec<String>,
    files: Vec<PathBuf>,
    in_place: bool,
    in_place_suffix: Option<OsString>,
    line_length: Option<u32>,
    sandbox: bool,
}

fn parse_args() -> Result<Args, optic::Error> {
    use optic::prelude::*;

    let mut args = Args::default();

    let mut parser = optic::Parser::from_env();
    let mut scripts_or_files = Vec::new();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('e') | Long("expression") => {
                args.scripts.push(parser.value()?.string()?);
            }
            Short('i') | Long("in-place") => {
                args.in_place = true;
                args.in_place_suffix = parser.value_immediate();
            }
            Short('l') | Long("line-length") => {
                args.line_length = Some(parser.value()?.parse()?);
            }
            Long("sandbox") => {
                args.sandbox = true;
            }
            Value(value) => scripts_or_files.push(value),
            _ => return Err(arg.error()),
        }
    }

    let mut scripts_or_files = scripts_or_files.into_iter();
    if args.scripts.is_empty() {
        let script = scripts_or_files
            .next()
            .ok_or_else(|| Error::Custom(format!("missing script")))?
            .string()?;
        args.scripts.push(script);
    }
    args.files.extend(scripts_or_files.map(Into::into));

    Ok(args)
}

fn main() -> Result<(), optic::Error> {
    println!("{:#?}", parse_args()?);
    Ok(())
}
