struct Args {
    thing: String,
    number: u32,
    shout: bool,
}

fn parse_args() -> Result<Args, optic::Error> {
    use optic::prelude::*;

    let mut thing = None;
    let mut number = 1;
    let mut shout = false;
    let mut parser = optic::Parser::from_env();
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

fn main() -> Result<(), optic::Error> {
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
