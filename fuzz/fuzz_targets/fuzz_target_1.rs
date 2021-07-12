#![no_main]
use libfuzzer_sys::fuzz_target;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

// We're not checking if the parser does something reasonable here,
// just making sure that it doesn't panic or hang.
fuzz_target!(|data: &[u8]| {
    let mut data = data;
    let mut decisions;
    if data.len() > 4 {
        // Decide whether to call .next() or .value(), 32 times
        decisions = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        data = &data[4..];
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
        if decisions & 1 == 0 {
            match p.next() {
                Err(_) => (),
                Ok(Some(_)) => (),
                Ok(None) => break,
            }
        } else {
            // We never break here, but decisions reaches 0 eventually
            // and then we can break
            let _ = p.value();
        }
        decisions >>= 1;
    }
    assert!(matches!(p.next(), Ok(None)));
    assert!(matches!(p.next(), Ok(None)));
    assert!(matches!(p.next(), Ok(None)));
});
