//! readerc.go: the input decoded into the parser's buffer, a raw chunk of
//! at most 512 bytes at a time as Go's reads it, so a broken character is
//! found when Go finds it.

use super::h::{Encoding, Parser};
use super::h::{ErrorType, INPUT_BUFFER_SIZE, INPUT_RAW_BUFFER_SIZE};

// Byte order marks.
const BOM_UTF8: &[u8] = b"\xef\xbb\xbf";
const BOM_UTF16LE: &[u8] = b"\xff\xfe";
const BOM_UTF16BE: &[u8] = b"\xfe\xff";

impl Parser {
    /// new is a parser of input, read as Go's reader handlers read it.
    pub(super) fn new(input: Vec<u8>) -> Parser {
        Parser {
            input,
            raw_buffer: Vec::with_capacity(INPUT_RAW_BUFFER_SIZE),
            buffer: Vec::with_capacity(INPUT_BUFFER_SIZE),
            ..Parser::default()
        }
    }

    // Set the reader error and return false.
    fn set_reader_error(&mut self, problem: &str, offset: i64, value: i64) -> bool {
        self.error = ErrorType::Reader;
        self.problem = problem.to_string();
        self.problem_offset = offset;
        self.problem_value = value;
        false
    }

    // Determine the input stream encoding by checking the BOM symbol. If no BOM is
    // found, the UTF-8 encoding is assumed. Return 1 on success, 0 on failure.
    fn determine_encoding(&mut self) -> bool {
        // Ensure that we had enough bytes in the raw buffer.
        while !self.eof && self.raw_buffer.len() - self.raw_buffer_pos < 3 {
            if !self.update_raw_buffer() {
                return false;
            }
        }

        // Determine the encoding.
        let buf = &self.raw_buffer;
        let pos = self.raw_buffer_pos;
        let avail = buf.len() - pos;
        if avail >= 2 && buf[pos] == BOM_UTF16LE[0] && buf[pos + 1] == BOM_UTF16LE[1] {
            self.encoding = Encoding::Utf16le;
            self.raw_buffer_pos += 2;
            self.offset += 2;
        } else if avail >= 2 && buf[pos] == BOM_UTF16BE[0] && buf[pos + 1] == BOM_UTF16BE[1] {
            self.encoding = Encoding::Utf16be;
            self.raw_buffer_pos += 2;
            self.offset += 2;
        } else if avail >= 3 && buf[pos] == BOM_UTF8[0] && buf[pos + 1] == BOM_UTF8[1] && buf[pos + 2] == BOM_UTF8[2] {
            self.encoding = Encoding::Utf8;
            self.raw_buffer_pos += 3;
            self.offset += 3;
        } else {
            self.encoding = Encoding::Utf8;
        }
        true
    }

    // Update the raw buffer.
    fn update_raw_buffer(&mut self) -> bool {
        // Return if the raw buffer is full.
        if self.raw_buffer_pos == 0 && self.raw_buffer.len() == INPUT_RAW_BUFFER_SIZE {
            return true;
        }

        // Return on EOF.
        if self.eof {
            return true;
        }

        // Move the remaining bytes in the raw buffer to the beginning.
        self.raw_buffer.drain(..self.raw_buffer_pos);
        self.raw_buffer_pos = 0;

        // Call the read handler to fill the buffer: the input's next bytes,
        // as many as there's room for, or the end of it.
        if self.input_pos == self.input.len() {
            self.eof = true;
            return true;
        }
        let room = INPUT_RAW_BUFFER_SIZE - self.raw_buffer.len();
        let n = room.min(self.input.len() - self.input_pos);
        self.raw_buffer.extend_from_slice(&self.input[self.input_pos..self.input_pos + n]);
        self.input_pos += n;
        true
    }

    // Ensure that the buffer contains at least `length` characters.
    // Return true on success, false on failure.
    //
    // The length is supposed to be significantly less that the buffer size.
    pub(super) fn update_buffer(&mut self, length: usize) -> bool {
        // [Go] This function was changed to guarantee the requested length size at EOF.
        // The fact we need to do this is pretty awful, but the description above implies
        // for that to be the case, and there are tests

        // Return if the buffer contains enough characters.
        if self.unread >= length {
            return true;
        }

        // Determine the input encoding if it is not known yet.
        if self.encoding == Encoding::Any && !self.determine_encoding() {
            return false;
        }

        // Move the unread characters to the beginning of the buffer.
        let mut buffer_len = self.buffer.len();
        if self.buffer_pos > 0 && self.buffer_pos < buffer_len {
            self.buffer.drain(..self.buffer_pos);
            buffer_len -= self.buffer_pos;
            self.buffer_pos = 0;
        } else if self.buffer_pos == buffer_len {
            buffer_len = 0;
            self.buffer_pos = 0;
        }

        // Open the whole buffer for writing, and cut it before returning.
        self.buffer.resize(INPUT_BUFFER_SIZE.max(buffer_len), 0);

        // Fill the buffer until it has enough characters.
        let mut first = true;
        while self.unread < length {
            // Fill the raw buffer if necessary.
            if (!first || self.raw_buffer_pos == self.raw_buffer.len()) && !self.update_raw_buffer() {
                self.buffer.truncate(buffer_len);
                return false;
            }
            first = false;

            // Decode the raw buffer.
            while self.raw_buffer_pos != self.raw_buffer.len() {
                let raw_unread = self.raw_buffer.len() - self.raw_buffer_pos;
                let pos = self.raw_buffer_pos;
                let mut value: u32;
                let width: usize;

                // Decode the next character.
                match self.encoding {
                    Encoding::Utf8 => {
                        // Decode a UTF-8 character.  Check RFC 3629
                        // (http://www.ietf.org/rfc/rfc3629.txt) for more details.
                        //
                        // The following table (taken from the RFC) is used for
                        // decoding.
                        //
                        //    Char. number range |        UTF-8 octet sequence
                        //      (hexadecimal)    |              (binary)
                        //   --------------------+------------------------------------
                        //   0000 0000-0000 007F | 0xxxxxxx
                        //   0000 0080-0000 07FF | 110xxxxx 10xxxxxx
                        //   0000 0800-0000 FFFF | 1110xxxx 10xxxxxx 10xxxxxx
                        //   0001 0000-0010 FFFF | 11110xxx 10xxxxxx 10xxxxxx 10xxxxxx
                        //
                        // Additionally, the characters in the range 0xD800-0xDFFF
                        // are prohibited as they are reserved for use with UTF-16
                        // surrogate pairs.

                        // Determine the length of the UTF-8 sequence.
                        let octet = self.raw_buffer[pos];
                        width = match octet {
                            o if o & 0x80 == 0x00 => 1,
                            o if o & 0xE0 == 0xC0 => 2,
                            o if o & 0xF0 == 0xE0 => 3,
                            o if o & 0xF8 == 0xF0 => 4,
                            _ => {
                                // The leading octet is invalid.
                                return self.set_reader_error(
                                    "invalid leading UTF-8 octet",
                                    self.offset,
                                    i64::from(octet),
                                );
                            }
                        };

                        // Check if the raw buffer contains an incomplete character.
                        if width > raw_unread {
                            if self.eof {
                                return self.set_reader_error("incomplete UTF-8 octet sequence", self.offset, -1);
                            }
                            break;
                        }

                        // Decode the leading octet.
                        value = match width {
                            1 => u32::from(octet & 0x7F),
                            2 => u32::from(octet & 0x1F),
                            3 => u32::from(octet & 0x0F),
                            _ => u32::from(octet & 0x07),
                        };

                        // Check and decode the trailing octets.
                        for k in 1..width {
                            let octet = self.raw_buffer[pos + k];

                            // Check if the octet is valid.
                            if (octet & 0xC0) != 0x80 {
                                return self.set_reader_error(
                                    "invalid trailing UTF-8 octet",
                                    self.offset + k as i64,
                                    i64::from(octet),
                                );
                            }

                            // Decode the octet.
                            value = (value << 6) + u32::from(octet & 0x3F);
                        }

                        // Check the length of the sequence against the value.
                        let ok = match width {
                            1 => true,
                            2 => value >= 0x80,
                            3 => value >= 0x800,
                            _ => value >= 0x10000,
                        };
                        if !ok {
                            return self.set_reader_error("invalid length of a UTF-8 sequence", self.offset, -1);
                        }

                        // Check the range of the value.
                        if (0xD800..=0xDFFF).contains(&value) || value > 0x10FFFF {
                            return self.set_reader_error("invalid Unicode character", self.offset, i64::from(value));
                        }
                    }

                    Encoding::Utf16le | Encoding::Utf16be => {
                        let (low, high) = if self.encoding == Encoding::Utf16le { (0, 1) } else { (1, 0) };

                        // The UTF-16 encoding is not as simple as one might
                        // naively think.  Check RFC 2781
                        // (http://www.ietf.org/rfc/rfc2781.txt).
                        //
                        // Normally, two subsequent bytes describe a Unicode
                        // character.  However a special technique (called a
                        // surrogate pair) is used for specifying character
                        // values larger than 0xFFFF.
                        //
                        // A surrogate pair consists of two pseudo-characters:
                        //      high surrogate area (0xD800-0xDBFF)
                        //      low surrogate area (0xDC00-0xDFFF)
                        //
                        // The following formulas are used for decoding
                        // and encoding characters using surrogate pairs:
                        //
                        //  U  = U' + 0x10000   (0x01 00 00 <= U <= 0x10 FF FF)
                        //  U' = yyyyyyyyyyxxxxxxxxxx   (0 <= U' <= 0x0F FF FF)
                        //  W1 = 110110yyyyyyyyyy
                        //  W2 = 110111xxxxxxxxxx
                        //
                        // where U is the character value, W1 is the high surrogate
                        // area, W2 is the low surrogate area.

                        // Check for incomplete UTF-16 character.
                        if raw_unread < 2 {
                            if self.eof {
                                return self.set_reader_error("incomplete UTF-16 character", self.offset, -1);
                            }
                            break;
                        }

                        // Get the character.
                        value = u32::from(self.raw_buffer[pos + low]) + (u32::from(self.raw_buffer[pos + high]) << 8);

                        // Check for unexpected low surrogate area.
                        if value & 0xFC00 == 0xDC00 {
                            return self.set_reader_error(
                                "unexpected low surrogate area",
                                self.offset,
                                i64::from(value),
                            );
                        }

                        // Check for a high surrogate area.
                        if value & 0xFC00 == 0xD800 {
                            width = 4;

                            // Check for incomplete surrogate pair.
                            if raw_unread < 4 {
                                if self.eof {
                                    return self.set_reader_error("incomplete UTF-16 surrogate pair", self.offset, -1);
                                }
                                break;
                            }

                            // Get the next character.
                            let value2 = u32::from(self.raw_buffer[pos + low + 2])
                                + (u32::from(self.raw_buffer[pos + high + 2]) << 8);

                            // Check for a low surrogate area.
                            if value2 & 0xFC00 != 0xDC00 {
                                return self.set_reader_error(
                                    "expected low surrogate area",
                                    self.offset + 2,
                                    i64::from(value2),
                                );
                            }

                            // Generate the value of the surrogate pair.
                            value = 0x10000 + ((value & 0x3FF) << 10) + (value2 & 0x3FF);
                        } else {
                            width = 2;
                        }
                    }

                    Encoding::Any => unreachable!("impossible"),
                }

                // Check if the character is in the allowed range:
                //      #x9 | #xA | #xD | [#x20-#x7E]               (8 bit)
                //      | #x85 | [#xA0-#xD7FF] | [#xE000-#xFFFD]    (16 bit)
                //      | [#x10000-#x10FFFF]                        (32 bit)
                let allowed = value == 0x09
                    || value == 0x0A
                    || value == 0x0D
                    || (0x20..=0x7E).contains(&value)
                    || value == 0x85
                    || (0xA0..=0xD7FF).contains(&value)
                    || (0xE000..=0xFFFD).contains(&value)
                    || (0x10000..=0x10FFFF).contains(&value);
                if !allowed {
                    return self.set_reader_error("control characters are not allowed", self.offset, i64::from(value));
                }

                // Move the raw pointers.
                self.raw_buffer_pos += width;
                self.offset += width as i64;

                // Finally put the character into the buffer.
                if self.buffer.len() < buffer_len + 4 {
                    self.buffer.resize(buffer_len + 4, 0);
                }
                if value <= 0x7F {
                    // 0000 0000-0000 007F . 0xxxxxxx
                    self.buffer[buffer_len] = value as u8;
                    buffer_len += 1;
                } else if value <= 0x7FF {
                    // 0000 0080-0000 07FF . 110xxxxx 10xxxxxx
                    self.buffer[buffer_len] = (0xC0 + (value >> 6)) as u8;
                    self.buffer[buffer_len + 1] = (0x80 + (value & 0x3F)) as u8;
                    buffer_len += 2;
                } else if value <= 0xFFFF {
                    // 0000 0800-0000 FFFF . 1110xxxx 10xxxxxx 10xxxxxx
                    self.buffer[buffer_len] = (0xE0 + (value >> 12)) as u8;
                    self.buffer[buffer_len + 1] = (0x80 + ((value >> 6) & 0x3F)) as u8;
                    self.buffer[buffer_len + 2] = (0x80 + (value & 0x3F)) as u8;
                    buffer_len += 3;
                } else {
                    // 0001 0000-0010 FFFF . 11110xxx 10xxxxxx 10xxxxxx 10xxxxxx
                    self.buffer[buffer_len] = (0xF0 + (value >> 18)) as u8;
                    self.buffer[buffer_len + 1] = (0x80 + ((value >> 12) & 0x3F)) as u8;
                    self.buffer[buffer_len + 2] = (0x80 + ((value >> 6) & 0x3F)) as u8;
                    self.buffer[buffer_len + 3] = (0x80 + (value & 0x3F)) as u8;
                    buffer_len += 4;
                }

                self.unread += 1;
            }

            // On EOF, put NUL into the buffer and return.
            if self.eof {
                if self.buffer.len() < buffer_len + 1 {
                    self.buffer.resize(buffer_len + 1, 0);
                }
                self.buffer[buffer_len] = 0;
                buffer_len += 1;
                self.unread += 1;
                break;
            }
        }
        // [Go] Read the documentation of this function above. To return true,
        // we need to have the given length in the buffer. Not doing that means
        // every single check that calls this function to make sure the buffer
        // has a given length is Go) panicking; or C) accessing invalid memory.
        // This happens here due to the EOF above breaking early.
        self.buffer.truncate(buffer_len);
        while buffer_len < length {
            self.buffer.push(0);
            buffer_len += 1;
        }
        true
    }
}
