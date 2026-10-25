//! Go's encoding/base64, StdEncoding.DecodeString, as yaml.v3 reads a
//! !!binary value: taken from aemui's aem-local (its Keychain's).

/// go_base64_decode is Go's base64.StdEncoding.DecodeString: the bytes, and
/// on an error ("illegal base64 data at input byte N") the ones decoded
/// before it. Line breaks are skipped, as Go skips them.
pub(super) fn go_base64_decode(s: &str) -> (Vec<u8>, Option<String>) {
    let src = s.as_bytes();
    let mut dst = Vec::with_capacity(src.len() / 4 * 3);
    let mut si = 0;
    while si < src.len() {
        let (nsi, _, err) = decode_quantum(&mut dst, src, si);
        si = nsi;
        if let Some(at) = err {
            return (dst, Some(format!("illegal base64 data at input byte {at}")));
        }
    }
    (dst, None)
}

fn decode_byte(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

// decodeQuantum decodes up to 4 base64 bytes. The received parameters are
// the destination buffer dst, the source buffer src and an index in the
// source buffer si. It returns the number of bytes read from src, the number
// of bytes written to dst, and an error, if any.
fn decode_quantum(dst: &mut Vec<u8>, src: &[u8], mut si: usize) -> (usize, usize, Option<usize>) {
    // Decode quantum using the base64 alphabet
    let mut dbuf = [0u8; 4];
    let mut dlen = 4;
    let mut err = None;

    let mut j = 0;
    while j < dbuf.len() {
        if src.len() == si {
            if j == 0 {
                return (si, 0, None);
            }
            // j == 1, or padding expected
            return (si, 0, Some(si - j));
        }
        let c = src[si];
        si += 1;
        if let Some(out) = decode_byte(c) {
            dbuf[j] = out;
            j += 1;
            continue;
        }

        if c == b'\n' || c == b'\r' {
            continue;
        }

        if c != b'=' {
            return (si, 0, Some(si - 1));
        }

        // We've reached the end and there's padding
        match j {
            0 | 1 => {
                // incorrect padding
                return (si, 0, Some(si - 1));
            }
            2 => {
                // "==" is expected, the first "=" is already consumed.
                // skip over newlines
                while si < src.len() && (src[si] == b'\n' || src[si] == b'\r') {
                    si += 1;
                }
                if si == src.len() {
                    // not enough padding
                    return (si, 0, Some(src.len()));
                }
                if src[si] != b'=' {
                    // incorrect padding
                    return (si, 0, Some(si - 1));
                }

                si += 1;
            }
            _ => {}
        }

        // skip over newlines
        while si < src.len() && (src[si] == b'\n' || src[si] == b'\r') {
            si += 1;
        }
        if si < src.len() {
            // trailing garbage
            err = Some(si);
        }
        dlen = j;
        break;
    }

    // Convert 4x 6bit source bytes into 3 bytes
    let val = u32::from(dbuf[0]) << 18 | u32::from(dbuf[1]) << 12 | u32::from(dbuf[2]) << 6 | u32::from(dbuf[3]);
    let out = [(val >> 16) as u8, (val >> 8) as u8, val as u8];
    dst.extend_from_slice(&out[..dlen - 1]);
    (si, dlen - 1, err)
}
