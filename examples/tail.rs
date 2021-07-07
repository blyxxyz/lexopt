#[derive(Debug)]
struct Args {
    follow: bool,
    number: u64,
    file: Option<std::path::PathBuf>,
}

fn parse_args() -> Result<Args, optic::Error> {
    use optic::prelude::*;

    let mut follow = false;
    let mut number = 10;
    let mut file = None;

    let mut parser = optic::Parser::from_env();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('f') | Long("follow") => {
                follow = true;
            }
            Short('n') => {
                number = parser.value()?.parse()?;
            }
            Value(value) if file.is_none() => {
                file = Some(value.into());
            }
            Long("help") => {
                println!("USAGE: tail [-f|--follow] [-n NUM] [FILE]");
                std::process::exit(0);
            }
            _ => return Err(arg.error()),
        }
    }
    Ok(Args {
        follow,
        number,
        file,
    })
}

fn main() -> Result<(), optic::Error> {
    let args = parse_args()?;
    println!("{:#?}", args);
    Ok(())
}
