//! Hello World program modeled after GNU Hello

fn parse_args() -> Result<String, optic::Error> {
    let mut greeting = "Hello, world!".into();

    let mut parser = optic::Parser::from_env();
    while let Some(arg) = parser.next()? {
        use optic::prelude::*;
        match arg {
            Short('t') | Long("traditional") => {
                greeting = "hello, world".into();
            }
            Short('g') | Long("greeting") => {
                greeting = parser.value()?.string()?;
            }
            Long("help") => {
                println!("Usage: hello [OPTION]...");
                std::process::exit(0);
            }
            _ => return Err(arg.error()),
        }
    }

    Ok(greeting)
}

fn main() -> Result<(), optic::Error> {
    let greeting = parse_args()?;
    println!("{}", greeting);
    Ok(())
}
