#![no_main]
use libfuzzer_sys::fuzz_target;
use std::convert::TryInto;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

// We're not checking if the parser does something reasonable here,
// just making sure that it doesn't panic or hang.
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
        match decisions % 4 {
            0 => match p.next() {
                Err(_) => (),
                Ok(Some(_)) => (),
                Ok(None) => break,
            },
            1 => match p.value() {
                Ok(_) => (),
                Err(_) => break,
            },
            2 => match p.values() {
                Ok(iter) => iter.for_each(drop),
                Err(_) => (),
            },
            3 => {
                let _ = p.optional_value();
            }
            _ => unreachable!(),
        }
        decisions /= 4;
    }
    assert!(matches!(p.next(), Ok(None)));
    assert!(matches!(p.next(), Ok(None)));
    assert!(matches!(p.next(), Ok(None)));
});
