// Copyright (C) 2020-2026 Andy Kurnia.

// the lines of a text: each ends at "\n", "\r\n", or a lone "\r" (the line
// endings of unix, windows, and classic mac text), the ending left off, and the
// last with or without one; as str::lines reads them, but for the lone "\r",
// which it leaves on the line
#[inline]
pub fn lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let end = rest.find(['\n', '\r']).unwrap_or(rest.len());
        let line = &rest[..end];
        rest = match rest[end..].strip_prefix("\r\n") {
            Some(after) => after,
            None => rest[end..].get(1..).unwrap_or(""),
        };
        Some(line)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(text: &str) -> Vec<&str> {
        lines(text).collect()
    }

    #[test]
    #[inline]
    fn lines_end_at_any_of_the_three_endings() {
        assert_eq!(of(""), Vec::<&str>::new());
        assert_eq!(of("a"), ["a"]);
        assert_eq!(of("a\n"), ["a"]);
        assert_eq!(of("a\r\n"), ["a"]);
        assert_eq!(of("a\r"), ["a"]);
        assert_eq!(of("\n"), [""]);
        assert_eq!(of("a\n\nb"), ["a", "", "b"]);
        assert_eq!(of("a\r\n\r\nb"), ["a", "", "b"]);
        assert_eq!(of("a\r\rb"), ["a", "", "b"]);
        assert_eq!(of("a\n\r\nb\rc\n"), ["a", "", "b", "c"]);
        // as str::lines, but for the lone "\r"
        for text in ["", "a", "a\n", "a\r\n", "\n", "a\n\nb\r\n\r\nc", "\n\n"] {
            assert_eq!(of(text), text.lines().collect::<Vec<_>>(), "{text:?}");
        }
    }
}
