#![allow(unsafe_code)]
use std::ffi::OsStr;
use std::ops::RangeBounds;

pub(crate) trait OsStrSlice {
    /// Takes a substring based on a range that corresponds to the return value of
    /// [`OsStr::as_encoded_bytes`].
    ///
    /// The range's start and end must lie on valid `OsStr` boundaries.
    ///
    /// On Unix any boundaries are valid, as OS strings may contain arbitrary bytes.
    ///
    /// On other platforms such as Windows the internal encoding is currently
    /// unspecified, and a valid `OsStr` boundary is one of:
    /// - The start of the string
    /// - The end of the string
    /// - Immediately before a valid non-empty UTF-8 substring
    /// - Immediately after a valid non-empty UTF-8 substring
    ///
    /// # Panics
    ///
    /// Panics if `range` does not lie on valid `OsStr` boundaries or if it
    /// exceeds the end of the string.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use std::ffi::OsStr;
    ///
    /// let os_str = OsStr::new("foo=bar");
    /// let bytes = os_str.as_encoded_bytes();
    /// if let Some(index) = bytes.iter().position(|b| *b == b'=') {
    ///     let key = os_str.slice_encoded_bytes(..index);
    ///     let value = os_str.slice_encoded_bytes(index + 1..);
    ///     assert_eq!(key, "foo");
    ///     assert_eq!(value, "bar");
    /// }
    /// ```
    fn slice_encoded_bytes<R: RangeBounds<usize>>(&self, range: R) -> &Self;
}

impl OsStrSlice for OsStr {
    fn slice_encoded_bytes<R: RangeBounds<usize>>(&self, range: R) -> &Self {
        let bytes = self.as_encoded_bytes();
        let range = std::slice::range(range, ..bytes.len());

        #[cfg(unix)]
        return std::os::unix::ffi::OsStrExt::from_bytes(&bytes[range]);

        #[cfg(not(unix))]
        {
            fn is_valid_boundary(bytes: &[u8], index: usize) -> bool {
                if index == 0 || index == bytes.len() {
                    return true;
                }

                let (before, after) = bytes.split_at(index);

                // UTF-8 takes at most 4 bytes per codepoint, so we don't
                // need to check more than that.
                let after = after.get(..4).unwrap_or(after);
                match std::str::from_utf8(after) {
                    Ok(_) => return true,
                    Err(err) if err.valid_up_to() != 0 => return true,
                    Err(_) => (),
                }

                for len in 1..=4.min(index) {
                    let before = &before[index - len..];
                    if std::str::from_utf8(before).is_ok() {
                        return true;
                    }
                }

                false
            }

            assert!(is_valid_boundary(bytes, range.start));
            assert!(is_valid_boundary(bytes, range.end));

            // SAFETY: bytes was obtained from an OsStr just now, and we validated
            // that we only slice immediately before or after a valid non-empty
            // UTF-8 substring.
            unsafe { Self::from_encoded_bytes_unchecked(&bytes[range]) }
        }
    }
}
