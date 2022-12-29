#![no_main]
use libfuzzer_sys::fuzz_target;
use std::convert::TryInto;
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};

// We check some basic invariants but mainly make sure that it
// doesn't panic or hang.
fuzz_target!(|data: &[u8]| {
    let mut data = data;
    let mut decisions;
    if data.len() > 8 {
        // Decide which method to call, 64 / 4 = 16 times
        decisions = u64::from_le_bytes(data[..8].try_into().unwrap());
        data = &data[8..];
    } else {
        decisions = 0;
    }
    let data: Vec<_> = data
        // Arguments can't contain null bytes (on Unix) so it's a
        // reasonable separator
        .split(|&x| x == b'\0')
        .map(Into::into)
        .map(OsString::from_vec)
        .collect();
    let mut p = lexopt::Parser::from_args(data);
    loop {
        // 0 -> Parser::next()
        // 1 -> Parser::value()
        // 2 -> Parser::values()
        // 3 -> Parser::optional_value()
        match decisions % 4 {
            0 => match p.next() {
                Err(_) => assert_finished_arg(&mut p),
                Ok(Some(_)) => (),
                Ok(None) => break,
            },
            1 => match p.value() {
                Ok(_) => assert_finished_arg(&mut p),
                Err(_) => break,
            },
            2 => match p.values() {
                Ok(iter) => {
                    assert!(iter.count() > 0);
                    assert_finished_arg(&mut p);
                }
                Err(_) => {
                    // Either the command line ran out, or the next argument is an option
                    if let Some(next) = p.try_raw_args().unwrap().as_slice().first() {
                        let arg = next.as_bytes();
                        assert!(arg.starts_with(b"-"));
                        assert_ne!(arg, b"-");
                    }
                }
            },
            3 => {
                let could_get_raw = p.try_raw_args().is_some();
                let had_optional = p.optional_value().is_some();
                assert_ne!(could_get_raw, had_optional);
                assert_finished_arg(&mut p);
            }
            _ => unreachable!(),
        }
        decisions /= 4;
        // This should be safe to call all the time
        let _ = p.try_raw_args();
    }
    assert_eq!(p.try_raw_args().unwrap().as_slice().len(), 0);
    assert!(matches!(p.next(), Ok(None)));
    assert!(matches!(p.next(), Ok(None)));
    assert!(matches!(p.next(), Ok(None)));
});

fn assert_finished_arg(parser: &mut lexopt::Parser) {
    assert!(parser.try_raw_args().is_some());
    // These methods can mutate Parser so we maybe shouldn't call them here
    // in case they happen to repair the state.
    // assert!(parser.raw_args().is_ok());
    // assert!(parser.optional_value().is_none());
}
