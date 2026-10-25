//! Go's `time.Duration` in its own words, `d.String()`: 1h2m3s, 4.5s,
//! 300ms; and `d.Truncate(m)`. A duration is nanoseconds, as Go's is, and
//! may be below zero.

/// One second, in nanoseconds.
pub const SECOND: i64 = 1_000_000_000;

/// truncate is Go's `d.Truncate(m)`: d rounded toward zero to a multiple
/// of m; d itself when m isn't above zero.
pub fn truncate(d: i64, m: i64) -> i64 {
    if m <= 0 {
        return d;
    }
    d - d % m
}

/// duration_string is Go's `time.Duration.String`: "72h3m0.5s", a
/// duration under a second in smaller units ("1.2ms"), "0s" for none.
pub fn duration_string(d: i64) -> String {
    // Largest time is 2540400h10m10.000000000s
    let mut buf = [0u8; 32];
    let mut w = buf.len();

    let neg = d < 0;
    let mut u = d.unsigned_abs();

    if u < SECOND as u64 {
        // Special case: if duration is smaller than a second,
        // use smaller units, like 1.2ms
        let prec;
        w -= 1;
        buf[w] = b's';
        w -= 1;
        if u == 0 {
            buf[w] = b'0';
            return String::from_utf8_lossy(&buf[w..]).into_owned();
        } else if u < 1_000 {
            // print nanoseconds
            prec = 0;
            buf[w] = b'n';
        } else if u < 1_000_000 {
            // print microseconds
            prec = 3;
            // U+00B5 'µ' micro sign == 0xC2 0xB5
            w -= 1; // Need room for two bytes.
            buf[w..w + 2].copy_from_slice("µ".as_bytes());
        } else {
            // print milliseconds
            prec = 6;
            buf[w] = b'm';
        }
        (w, u) = fmt_frac(&mut buf, w, u, prec);
        w = fmt_int(&mut buf, w, u);
    } else {
        w -= 1;
        buf[w] = b's';

        (w, u) = fmt_frac(&mut buf, w, u, 9);

        // u is now integer seconds
        w = fmt_int(&mut buf, w, u % 60);
        u /= 60;

        // u is now integer minutes
        if u > 0 {
            w -= 1;
            buf[w] = b'm';
            w = fmt_int(&mut buf, w, u % 60);
            u /= 60;

            // u is now integer hours
            // Stop at hours because days can be different lengths.
            if u > 0 {
                w -= 1;
                buf[w] = b'h';
                w = fmt_int(&mut buf, w, u);
            }
        }
    }

    if neg {
        w -= 1;
        buf[w] = b'-';
    }
    String::from_utf8_lossy(&buf[w..]).into_owned()
}

// fmt_frac formats the fraction of v/10**prec (e.g., ".12345") into the
// tail of buf before w, omitting trailing zeros. It omits the decimal
// point too when the fraction is 0. It returns the index where the
// output bytes begin and the value v/10**prec.
fn fmt_frac(buf: &mut [u8], mut w: usize, mut v: u64, prec: usize) -> (usize, u64) {
    // Omit trailing zeros up to and including decimal point.
    let mut print = false;
    for _ in 0..prec {
        let digit = v % 10;
        print = print || digit != 0;
        if print {
            w -= 1;
            buf[w] = digit as u8 + b'0';
        }
        v /= 10;
    }
    if print {
        w -= 1;
        buf[w] = b'.';
    }
    (w, v)
}

// fmt_int formats v into the tail of buf before w. It returns the index
// where the output begins.
fn fmt_int(buf: &mut [u8], mut w: usize, mut v: u64) -> usize {
    if v == 0 {
        w -= 1;
        buf[w] = b'0';
    } else {
        while v > 0 {
            w -= 1;
            buf[w] = (v % 10) as u8 + b'0';
            v /= 10;
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_say_what_gos_say() {
        for (d, want) in [
            (0, "0s"),
            (1, "1ns"),
            (1_100, "1.1µs"),
            (2_200_000, "2.2ms"),
            (3 * SECOND, "3s"),
            (62 * SECOND, "1m2s"),
            (3600 * SECOND + 5 * SECOND, "1h0m5s"),
            (26 * 3600 * SECOND, "26h0m0s"),
            (4 * SECOND + SECOND / 2, "4.5s"),
            (-3 * SECOND, "-3s"),
            (i64::MIN, "-2562047h47m16.854775808s"),
        ] {
            assert_eq!(duration_string(d), want, "{d}");
        }
        assert_eq!(truncate(4 * SECOND + 900_000_000, SECOND), 4 * SECOND);
        assert_eq!(truncate(-(4 * SECOND + 900_000_000), SECOND), -4 * SECOND);
        assert_eq!(truncate(7, 0), 7);
    }
}
