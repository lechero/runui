//! scannerc.go: the input read into tokens, with yaml.v3's comments
//! (head, line and foot) found and kept for the tokens they belong to.

use super::h::*;

impl Parser {
    // Ensure that the buffer contains the required number of characters.
    // Return true on success, false on failure (reader error or memory error).
    #[allow(dead_code)]
    fn cache(&mut self, length: usize) -> bool {
        // [Go] This was inlined: !cache(A, B) -> unread < B && !update(A, B)
        self.unread >= length || self.update_buffer(length)
    }

    // Advance the buffer pointer.
    fn skip(&mut self) {
        if !is_blank(&self.buffer, self.buffer_pos) {
            self.newlines = 0;
        }
        self.mark.index += 1;
        self.mark.column += 1;
        self.unread -= 1;
        self.buffer_pos += width(at(&self.buffer, self.buffer_pos));
    }

    fn skip_line(&mut self) {
        if is_crlf(&self.buffer, self.buffer_pos) {
            self.mark.index += 2;
            self.mark.column = 0;
            self.mark.line += 1;
            self.unread -= 2;
            self.buffer_pos += 2;
            self.newlines += 1;
        } else if is_break(&self.buffer, self.buffer_pos) {
            self.mark.index += 1;
            self.mark.column = 0;
            self.mark.line += 1;
            self.unread -= 1;
            self.buffer_pos += width(at(&self.buffer, self.buffer_pos));
            self.newlines += 1;
        }
    }

    // Copy a character to a string buffer and advance pointers.
    fn read(&mut self, s: &mut Vec<u8>) {
        if !is_blank(&self.buffer, self.buffer_pos) {
            self.newlines = 0;
        }
        let w = width(at(&self.buffer, self.buffer_pos));
        if w == 0 {
            panic!("invalid character sequence");
        }
        s.extend_from_slice(&self.buffer[self.buffer_pos..self.buffer_pos + w]);
        self.buffer_pos += w;
        self.mark.index += 1;
        self.mark.column += 1;
        self.unread -= 1;
    }

    // Copy a line break character to a string buffer and advance pointers.
    fn read_line(&mut self, s: &mut Vec<u8>) {
        let buf = &self.buffer;
        let pos = self.buffer_pos;
        if at(buf, pos) == b'\r' && at(buf, pos + 1) == b'\n' {
            // CR LF . LF
            s.push(b'\n');
            self.buffer_pos += 2;
            self.mark.index += 1;
            self.unread -= 1;
        } else if at(buf, pos) == b'\r' || at(buf, pos) == b'\n' {
            // CR|LF . LF
            s.push(b'\n');
            self.buffer_pos += 1;
        } else if at(buf, pos) == 0xC2 && at(buf, pos + 1) == 0x85 {
            // NEL . LF
            s.push(b'\n');
            self.buffer_pos += 2;
        } else if at(buf, pos) == 0xE2
            && at(buf, pos + 1) == 0x80
            && (at(buf, pos + 2) == 0xA8 || at(buf, pos + 2) == 0xA9)
        {
            // LS|PS . LS|PS
            s.extend_from_slice(&buf[pos..pos + 3]);
            self.buffer_pos += 3;
        } else {
            return;
        }
        self.mark.index += 1;
        self.mark.column = 0;
        self.mark.line += 1;
        self.unread -= 1;
        self.newlines += 1;
    }

    /// insert_token puts a token in the queue: at its end, or pos tokens
    /// after its head. A full queue is moved to the start of its buffer
    /// first, when tokens were taken from it, as Go's does.
    pub(super) fn insert_token(&mut self, pos: isize, token: Token) {
        // Check if we can move the queue at the beginning of the buffer.
        if self.tokens_head > 0 && self.tokens.len() == self.tokens_cap {
            self.tokens.drain(..self.tokens_head);
            self.tokens_head = 0;
        }
        if self.tokens.len() == self.tokens_cap {
            self.tokens_cap = next_cap(self.tokens_cap);
        }
        if pos < 0 {
            self.tokens.push(token);
            return;
        }
        self.tokens.insert(self.tokens_head + pos as usize, token);
    }

    // Set the scanner error and return false.
    fn set_scanner_error(&mut self, context: &str, context_mark: Mark, problem: &str) -> bool {
        self.error = ErrorType::Scanner;
        self.context = context.to_string();
        self.context_mark = context_mark;
        self.problem = problem.to_string();
        self.problem_mark = self.mark;
        false
    }

    fn set_scanner_tag_error(&mut self, directive: bool, context_mark: Mark, problem: &str) -> bool {
        let context = if directive { "while parsing a %TAG directive" } else { "while parsing a tag" };
        self.set_scanner_error(context, context_mark, problem)
    }

    // Ensure that the tokens queue contains at least one token which can be
    // returned to the Parser.
    pub(super) fn fetch_more_tokens(&mut self) -> bool {
        // While we need more tokens to fetch, do it.
        loop {
            // [Go] The comment parsing logic requires a lookahead of two tokens
            // so that foot comments may be parsed in time of associating them
            // with the tokens that are parsed before them, and also for line
            // comments to be transformed into head comments in some edge cases.
            if self.tokens_head + 2 < self.tokens.len() {
                // If a potential simple key is at the head position, we need to fetch
                // the next token to disambiguate it.
                let Some(&head_tok_idx) = self.simple_keys_by_tok.get(&self.tokens_parsed) else {
                    break;
                };
                let (valid, ok) = self.simple_key_is_valid(head_tok_idx);
                if !ok {
                    return false;
                } else if !valid {
                    break;
                }
            }
            // Fetch the next token.
            if !self.fetch_next_token() {
                return false;
            }
        }

        self.token_available = true;
        true
    }

    // The dispatcher for token fetchers.
    fn fetch_next_token(&mut self) -> bool {
        // Ensure that the buffer is initialized.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }

        // Check if we just started scanning.  Fetch STREAM-START then.
        if !self.stream_start_produced {
            return self.fetch_stream_start();
        }

        let scan_mark = self.mark;

        // Eat whitespaces and comments until we reach the next token.
        if !self.scan_to_next_token() {
            return false;
        }

        // [Go] While unrolling indents, transform the head comments of prior
        // indentation levels observed after scan_start into foot comments at
        // the respective indexes.

        // Check the indentation level against the current column.
        if !self.unroll_indent(self.mark.column, scan_mark) {
            return false;
        }

        // Ensure that the buffer contains at least 4 characters.  4 is the length
        // of the longest indicators ('--- ' and '... ').
        if self.unread < 4 && !self.update_buffer(4) {
            return false;
        }

        // Is it the end of the stream?
        if is_z(&self.buffer, self.buffer_pos) {
            return self.fetch_stream_end();
        }

        // Is it a directive?
        if self.mark.column == 0 && at(&self.buffer, self.buffer_pos) == b'%' {
            return self.fetch_directive();
        }

        let buf = &self.buffer;
        let pos = self.buffer_pos;

        // Is it the document start indicator?
        if self.mark.column == 0
            && at(buf, pos) == b'-'
            && at(buf, pos + 1) == b'-'
            && at(buf, pos + 2) == b'-'
            && is_blankz(buf, pos + 3)
        {
            return self.fetch_document_indicator(TokenType::DocumentStart);
        }

        // Is it the document end indicator?
        if self.mark.column == 0
            && at(buf, pos) == b'.'
            && at(buf, pos + 1) == b'.'
            && at(buf, pos + 2) == b'.'
            && is_blankz(buf, pos + 3)
        {
            return self.fetch_document_indicator(TokenType::DocumentEnd);
        }

        let mut comment_mark = self.mark;
        if !self.tokens.is_empty()
            && (self.flow_level == 0 && at(buf, pos) == b':' || self.flow_level > 0 && at(buf, pos) == b',')
        {
            // Associate any following comments with the prior token.
            comment_mark = self.tokens[self.tokens.len() - 1].start_mark;
        }

        if !self.fetch_token() {
            return false;
        }
        if self.tokens.last().is_some_and(|t| t.typ == TokenType::BlockEntry) {
            // Sequence indicators alone have no line comments. It becomes
            // a head comment for whatever follows.
            return true;
        }
        self.scan_line_comment(comment_mark)
    }

    // The token at the current position, fetched: the rest of the dispatcher.
    fn fetch_token(&mut self) -> bool {
        let c = at(&self.buffer, self.buffer_pos);

        // Is it the flow sequence start indicator?
        if c == b'[' {
            return self.fetch_flow_collection_start(TokenType::FlowSequenceStart);
        }

        // Is it the flow mapping start indicator?
        if c == b'{' {
            return self.fetch_flow_collection_start(TokenType::FlowMappingStart);
        }

        // Is it the flow sequence end indicator?
        if c == b']' {
            return self.fetch_flow_collection_end(TokenType::FlowSequenceEnd);
        }

        // Is it the flow mapping end indicator?
        if c == b'}' {
            return self.fetch_flow_collection_end(TokenType::FlowMappingEnd);
        }

        // Is it the flow entry indicator?
        if c == b',' {
            return self.fetch_flow_entry();
        }

        // Is it the block entry indicator?
        if c == b'-' && is_blankz(&self.buffer, self.buffer_pos + 1) {
            return self.fetch_block_entry();
        }

        // Is it the key indicator?
        if c == b'?' && (self.flow_level > 0 || is_blankz(&self.buffer, self.buffer_pos + 1)) {
            return self.fetch_key();
        }

        // Is it the value indicator?
        if c == b':' && (self.flow_level > 0 || is_blankz(&self.buffer, self.buffer_pos + 1)) {
            return self.fetch_value();
        }

        // Is it an alias?
        if c == b'*' {
            return self.fetch_anchor(TokenType::Alias);
        }

        // Is it an anchor?
        if c == b'&' {
            return self.fetch_anchor(TokenType::Anchor);
        }

        // Is it a tag?
        if c == b'!' {
            return self.fetch_tag();
        }

        // Is it a literal scalar?
        if c == b'|' && self.flow_level == 0 {
            return self.fetch_block_scalar(true);
        }

        // Is it a folded scalar?
        if c == b'>' && self.flow_level == 0 {
            return self.fetch_block_scalar(false);
        }

        // Is it a single-quoted scalar?
        if c == b'\'' {
            return self.fetch_flow_scalar(true);
        }

        // Is it a double-quoted scalar?
        if c == b'"' {
            return self.fetch_flow_scalar(false);
        }

        // Is it a plain scalar?
        //
        // A plain scalar may start with any non-blank characters except
        //
        //      '-', '?', ':', ',', '[', ']', '{', '}',
        //      '#', '&', '*', '!', '|', '>', '\'', '\"',
        //      '%', '@', '`'.
        //
        // In the block context (and, for the '-' indicator, in the flow context
        // too), it may also start with the characters
        //
        //      '-', '?', ':'
        //
        // if it is followed by a non-space character.
        //
        // The last rule is more restrictive than the specification requires.
        // [Go] TODO Make this logic more reasonable.
        let b = &self.buffer;
        let p = self.buffer_pos;
        if !(is_blankz(b, p)
            || matches!(
                c,
                b'-' | b'?'
                    | b':'
                    | b','
                    | b'['
                    | b']'
                    | b'{'
                    | b'}'
                    | b'#'
                    | b'&'
                    | b'*'
                    | b'!'
                    | b'|'
                    | b'>'
                    | b'\''
                    | b'"'
                    | b'%'
                    | b'@'
                    | b'`'
            ))
            || (c == b'-' && !is_blank(b, p + 1))
            || (self.flow_level == 0 && (c == b'?' || c == b':') && !is_blankz(b, p + 1))
        {
            return self.fetch_plain_scalar();
        }

        // If we don't determine the token type so far, it is an error.
        let mark = self.mark;
        self.set_scanner_error("while scanning for the next token", mark, "found character that cannot start any token")
    }

    fn simple_key_is_valid(&mut self, i: usize) -> (bool, bool) {
        let simple_key = self.simple_keys[i];
        if !simple_key.possible {
            return (false, true);
        }

        // The 1.2 specification says:
        //
        //     "If the ? indicator is omitted, parsing needs to see past the
        //     implicit key to recognize it as such. To limit the amount of
        //     lookahead required, the “:” indicator must appear at most 1024
        //     Unicode characters beyond the start of the key. In addition, the key
        //     is restricted to a single line."
        //
        if simple_key.mark.line < self.mark.line || simple_key.mark.index + 1024 < self.mark.index {
            // Check if the potential simple key to be removed is required.
            if simple_key.required {
                return (
                    false,
                    self.set_scanner_error(
                        "while scanning a simple key",
                        simple_key.mark,
                        "could not find expected ':'",
                    ),
                );
            }
            self.simple_keys[i].possible = false;
            return (false, true);
        }
        (true, true)
    }

    // Check if a simple key may start at the current position and add it if
    // needed.
    fn save_simple_key(&mut self) -> bool {
        // A simple key is required at the current position if the scanner is in
        // the block context and the current column coincides with the indentation
        // level.

        let required = self.flow_level == 0 && self.indent == self.mark.column;

        //
        // If the current position may start a simple key, save it.
        //
        if self.simple_key_allowed {
            let simple_key = SimpleKey {
                possible: true,
                required,
                token_number: self.tokens_parsed + (self.tokens.len() - self.tokens_head),
                mark: self.mark,
            };

            if !self.remove_simple_key() {
                return false;
            }
            let last = self.simple_keys.len() - 1;
            self.simple_keys[last] = simple_key;
            self.simple_keys_by_tok.insert(simple_key.token_number, last);
        }
        true
    }

    // Remove a potential simple key at the current flow level.
    fn remove_simple_key(&mut self) -> bool {
        let i = self.simple_keys.len() - 1;
        if self.simple_keys[i].possible {
            // If the key is required, it is an error.
            if self.simple_keys[i].required {
                let mark = self.simple_keys[i].mark;
                return self.set_scanner_error("while scanning a simple key", mark, "could not find expected ':'");
            }
            // Remove the key from the stack.
            self.simple_keys[i].possible = false;
            self.simple_keys_by_tok.remove(&self.simple_keys[i].token_number);
        }
        true
    }

    // Increase the flow level and resize the simple key list if needed.
    fn increase_flow_level(&mut self) -> bool {
        // Reset the simple key on the next level.
        self.simple_keys.push(SimpleKey {
            possible: false,
            required: false,
            token_number: self.tokens_parsed + (self.tokens.len() - self.tokens_head),
            mark: self.mark,
        });

        // Increase the flow level.
        self.flow_level += 1;
        if self.flow_level > MAX_FLOW_LEVEL {
            let mark = self.simple_keys[self.simple_keys.len() - 1].mark;
            return self.set_scanner_error(
                "while increasing flow level",
                mark,
                &format!("exceeded max depth of {MAX_FLOW_LEVEL}"),
            );
        }
        true
    }

    // Decrease the flow level.
    fn decrease_flow_level(&mut self) -> bool {
        if self.flow_level > 0 {
            self.flow_level -= 1;
            let last = self.simple_keys.len() - 1;
            self.simple_keys_by_tok.remove(&self.simple_keys[last].token_number);
            self.simple_keys.truncate(last);
        }
        true
    }

    // Push the current indentation level to the stack and set the new level
    // the current column is greater than the indentation level.  In this case,
    // append or insert the specified token into the token queue.
    fn roll_indent(&mut self, column: i64, number: i64, typ: TokenType, mark: Mark) -> bool {
        // In the flow context, do nothing.
        if self.flow_level > 0 {
            return true;
        }

        if self.indent < column {
            // Push the current indentation level to the stack and set the new
            // indentation level.
            self.indents.push(self.indent);
            self.indent = column;
            if self.indents.len() > MAX_INDENTS {
                let mark = self.simple_keys[self.simple_keys.len() - 1].mark;
                return self.set_scanner_error(
                    "while increasing indent level",
                    mark,
                    &format!("exceeded max depth of {MAX_INDENTS}"),
                );
            }

            // Create a token and insert it into the queue.
            let token = Token { typ, start_mark: mark, end_mark: mark, ..Token::default() };
            let mut number = number;
            if number > -1 {
                number -= self.tokens_parsed as i64;
            }
            self.insert_token(number as isize, token);
        }
        true
    }

    // Pop indentation levels from the indents stack until the current level
    // becomes less or equal to the column.  For each indentation level, append
    // the BLOCK-END token.
    fn unroll_indent(&mut self, column: i64, scan_mark: Mark) -> bool {
        // In the flow context, do nothing.
        if self.flow_level > 0 {
            return true;
        }

        let mut block_mark = scan_mark;
        block_mark.index -= 1;

        // Loop through the indentation levels in the stack.
        while self.indent > column {
            // [Go] Reposition the end token before potential following
            //      foot comments of parent blocks. For that, search
            //      backwards for recent comments that were at the same
            //      indent as the block that is ending now.
            let mut stop_index = block_mark.index;
            for i in (0..self.comments.len()).rev() {
                let comment = &self.comments[i];

                if comment.end_mark.index < stop_index {
                    // Don't go back beyond the start of the comment/whitespace scan, unless column < 0.
                    // If requested indent column is < 0, then the document is over and everything else
                    // is a foot anyway.
                    break;
                }
                if comment.start_mark.column == self.indent + 1 {
                    // This is a good match. But maybe there's a former comment
                    // at that same indent level, so keep searching.
                    block_mark = comment.start_mark;
                }

                // While the end of the former comment matches with
                // the start of the following one, we know there's
                // nothing in between and scanning is still safe.
                stop_index = comment.scan_mark.index;
            }

            // Create a token and append it to the queue.
            let token =
                Token { typ: TokenType::BlockEnd, start_mark: block_mark, end_mark: block_mark, ..Token::default() };
            self.insert_token(-1, token);

            // Pop the indentation level.
            self.indent = self.indents.pop().unwrap_or_default();
        }
        true
    }

    // Initialize the scanner and produce the STREAM-START token.
    fn fetch_stream_start(&mut self) -> bool {
        // Set the initial indentation.
        self.indent = -1;

        // Initialize the simple key stack.
        self.simple_keys.push(SimpleKey::default());

        self.simple_keys_by_tok.clear();

        // A simple key is allowed at the beginning of the stream.
        self.simple_key_allowed = true;

        // We have started.
        self.stream_start_produced = true;

        // Create the STREAM-START token and append it to the queue.
        let token = Token {
            typ: TokenType::StreamStart,
            start_mark: self.mark,
            end_mark: self.mark,
            encoding: self.encoding,
            ..Token::default()
        };
        self.insert_token(-1, token);
        true
    }

    // Produce the STREAM-END token and shut down the scanner.
    fn fetch_stream_end(&mut self) -> bool {
        // Force new line.
        if self.mark.column != 0 {
            self.mark.column = 0;
            self.mark.line += 1;
        }

        // Reset the indentation level.
        if !self.unroll_indent(-1, self.mark) {
            return false;
        }

        // Reset simple keys.
        if !self.remove_simple_key() {
            return false;
        }

        self.simple_key_allowed = false;

        // Create the STREAM-END token and append it to the queue.
        let token = Token { typ: TokenType::StreamEnd, start_mark: self.mark, end_mark: self.mark, ..Token::default() };
        self.insert_token(-1, token);
        true
    }

    // Produce a VERSION-DIRECTIVE or TAG-DIRECTIVE token.
    fn fetch_directive(&mut self) -> bool {
        // Reset the indentation level.
        if !self.unroll_indent(-1, self.mark) {
            return false;
        }

        // Reset simple keys.
        if !self.remove_simple_key() {
            return false;
        }

        self.simple_key_allowed = false;

        // Create the YAML-DIRECTIVE or TAG-DIRECTIVE token.
        let mut token = Token::default();
        if !self.scan_directive(&mut token) {
            return false;
        }
        // Append the token to the queue.
        self.insert_token(-1, token);
        true
    }

    // Produce the DOCUMENT-START or DOCUMENT-END token.
    fn fetch_document_indicator(&mut self, typ: TokenType) -> bool {
        // Reset the indentation level.
        if !self.unroll_indent(-1, self.mark) {
            return false;
        }

        // Reset simple keys.
        if !self.remove_simple_key() {
            return false;
        }

        self.simple_key_allowed = false;

        // Consume the token.
        let start_mark = self.mark;

        self.skip();
        self.skip();
        self.skip();

        let end_mark = self.mark;

        // Create the DOCUMENT-START or DOCUMENT-END token.
        let token = Token { typ, start_mark, end_mark, ..Token::default() };
        // Append the token to the queue.
        self.insert_token(-1, token);
        true
    }

    // Produce the FLOW-SEQUENCE-START or FLOW-MAPPING-START token.
    fn fetch_flow_collection_start(&mut self, typ: TokenType) -> bool {
        // The indicators '[' and '{' may start a simple key.
        if !self.save_simple_key() {
            return false;
        }

        // Increase the flow level.
        if !self.increase_flow_level() {
            return false;
        }

        // A simple key may follow the indicators '[' and '{'.
        self.simple_key_allowed = true;

        // Consume the token.
        let start_mark = self.mark;
        self.skip();
        let end_mark = self.mark;

        // Create the FLOW-SEQUENCE-START of FLOW-MAPPING-START token.
        let token = Token { typ, start_mark, end_mark, ..Token::default() };
        // Append the token to the queue.
        self.insert_token(-1, token);
        true
    }

    // Produce the FLOW-SEQUENCE-END or FLOW-MAPPING-END token.
    fn fetch_flow_collection_end(&mut self, typ: TokenType) -> bool {
        // Reset any potential simple key on the current flow level.
        if !self.remove_simple_key() {
            return false;
        }

        // Decrease the flow level.
        if !self.decrease_flow_level() {
            return false;
        }

        // No simple keys after the indicators ']' and '}'.
        self.simple_key_allowed = false;

        // Consume the token.

        let start_mark = self.mark;
        self.skip();
        let end_mark = self.mark;

        // Create the FLOW-SEQUENCE-END of FLOW-MAPPING-END token.
        let token = Token { typ, start_mark, end_mark, ..Token::default() };
        // Append the token to the queue.
        self.insert_token(-1, token);
        true
    }

    // Produce the FLOW-ENTRY token.
    fn fetch_flow_entry(&mut self) -> bool {
        // Reset any potential simple keys on the current flow level.
        if !self.remove_simple_key() {
            return false;
        }

        // Simple keys are allowed after ','.
        self.simple_key_allowed = true;

        // Consume the token.
        let start_mark = self.mark;
        self.skip();
        let end_mark = self.mark;

        // Create the FLOW-ENTRY token and append it to the queue.
        let token = Token { typ: TokenType::FlowEntry, start_mark, end_mark, ..Token::default() };
        self.insert_token(-1, token);
        true
    }

    // Produce the BLOCK-ENTRY token.
    fn fetch_block_entry(&mut self) -> bool {
        // Check if the scanner is in the block context.
        if self.flow_level == 0 {
            // Check if we are allowed to start a new entry.
            if !self.simple_key_allowed {
                let mark = self.mark;
                return self.set_scanner_error("", mark, "block sequence entries are not allowed in this context");
            }
            // Add the BLOCK-SEQUENCE-START token if needed.
            if !self.roll_indent(self.mark.column, -1, TokenType::BlockSequenceStart, self.mark) {
                return false;
            }
        } else {
            // It is an error for the '-' indicator to occur in the flow context,
            // but we let the Parser detect and report about it because the Parser
            // is able to point to the context.
        }

        // Reset any potential simple keys on the current flow level.
        if !self.remove_simple_key() {
            return false;
        }

        // Simple keys are allowed after '-'.
        self.simple_key_allowed = true;

        // Consume the token.
        let start_mark = self.mark;
        self.skip();
        let end_mark = self.mark;

        // Create the BLOCK-ENTRY token and append it to the queue.
        let token = Token { typ: TokenType::BlockEntry, start_mark, end_mark, ..Token::default() };
        self.insert_token(-1, token);
        true
    }

    // Produce the KEY token.
    fn fetch_key(&mut self) -> bool {
        // In the block context, additional checks are required.
        if self.flow_level == 0 {
            // Check if we are allowed to start a new key (not nessesary simple).
            if !self.simple_key_allowed {
                let mark = self.mark;
                return self.set_scanner_error("", mark, "mapping keys are not allowed in this context");
            }
            // Add the BLOCK-MAPPING-START token if needed.
            if !self.roll_indent(self.mark.column, -1, TokenType::BlockMappingStart, self.mark) {
                return false;
            }
        }

        // Reset any potential simple keys on the current flow level.
        if !self.remove_simple_key() {
            return false;
        }

        // Simple keys are allowed after '?' in the block context.
        self.simple_key_allowed = self.flow_level == 0;

        // Consume the token.
        let start_mark = self.mark;
        self.skip();
        let end_mark = self.mark;

        // Create the KEY token and append it to the queue.
        let token = Token { typ: TokenType::Key, start_mark, end_mark, ..Token::default() };
        self.insert_token(-1, token);
        true
    }

    // Produce the VALUE token.
    fn fetch_value(&mut self) -> bool {
        let last = self.simple_keys.len() - 1;

        // Have we found a simple key?
        let (valid, ok) = self.simple_key_is_valid(last);
        if !ok {
            return false;
        } else if valid {
            let simple_key = self.simple_keys[last];

            // Create the KEY token and insert it into the queue.
            let token = Token {
                typ: TokenType::Key,
                start_mark: simple_key.mark,
                end_mark: simple_key.mark,
                ..Token::default()
            };
            // [Go] an int: a key's token the parser took already is
            // negative, and appended
            self.insert_token(simple_key.token_number as isize - self.tokens_parsed as isize, token);

            // In the block context, we may need to add the BLOCK-MAPPING-START token.
            if !self.roll_indent(
                simple_key.mark.column,
                simple_key.token_number as i64,
                TokenType::BlockMappingStart,
                simple_key.mark,
            ) {
                return false;
            }

            // Remove the simple key.
            self.simple_keys[last].possible = false;
            self.simple_keys_by_tok.remove(&simple_key.token_number);

            // A simple key cannot follow another simple key.
            self.simple_key_allowed = false;
        } else {
            // The ':' indicator follows a complex key.

            // In the block context, extra checks are required.
            if self.flow_level == 0 {
                // Check if we are allowed to start a complex value.
                if !self.simple_key_allowed {
                    let mark = self.mark;
                    return self.set_scanner_error("", mark, "mapping values are not allowed in this context");
                }

                // Add the BLOCK-MAPPING-START token if needed.
                if !self.roll_indent(self.mark.column, -1, TokenType::BlockMappingStart, self.mark) {
                    return false;
                }
            }

            // Simple keys after ':' are allowed in the block context.
            self.simple_key_allowed = self.flow_level == 0;
        }

        // Consume the token.
        let start_mark = self.mark;
        self.skip();
        let end_mark = self.mark;

        // Create the VALUE token and append it to the queue.
        let token = Token { typ: TokenType::Value, start_mark, end_mark, ..Token::default() };
        self.insert_token(-1, token);
        true
    }

    // Produce the ALIAS or ANCHOR token.
    fn fetch_anchor(&mut self, typ: TokenType) -> bool {
        // An anchor or an alias could be a simple key.
        if !self.save_simple_key() {
            return false;
        }

        // A simple key cannot follow an anchor or an alias.
        self.simple_key_allowed = false;

        // Create the ALIAS or ANCHOR token and append it to the queue.
        let mut token = Token::default();
        if !self.scan_anchor(&mut token, typ) {
            return false;
        }
        self.insert_token(-1, token);
        true
    }

    // Produce the TAG token.
    fn fetch_tag(&mut self) -> bool {
        // A tag could be a simple key.
        if !self.save_simple_key() {
            return false;
        }

        // A simple key cannot follow a tag.
        self.simple_key_allowed = false;

        // Create the TAG token and append it to the queue.
        let mut token = Token::default();
        if !self.scan_tag(&mut token) {
            return false;
        }
        self.insert_token(-1, token);
        true
    }

    // Produce the SCALAR(...,literal) or SCALAR(...,folded) tokens.
    fn fetch_block_scalar(&mut self, literal: bool) -> bool {
        // Remove any potential simple keys.
        if !self.remove_simple_key() {
            return false;
        }

        // A simple key may follow a block scalar.
        self.simple_key_allowed = true;

        // Create the SCALAR token and append it to the queue.
        let mut token = Token::default();
        if !self.scan_block_scalar(&mut token, literal) {
            return false;
        }
        self.insert_token(-1, token);
        true
    }

    // Produce the SCALAR(...,single-quoted) or SCALAR(...,double-quoted) tokens.
    fn fetch_flow_scalar(&mut self, single: bool) -> bool {
        // A plain scalar could be a simple key.
        if !self.save_simple_key() {
            return false;
        }

        // A simple key cannot follow a flow scalar.
        self.simple_key_allowed = false;

        // Create the SCALAR token and append it to the queue.
        let mut token = Token::default();
        if !self.scan_flow_scalar(&mut token, single) {
            return false;
        }
        self.insert_token(-1, token);
        true
    }

    // Produce the SCALAR(...,plain) token.
    fn fetch_plain_scalar(&mut self) -> bool {
        // A plain scalar could be a simple key.
        if !self.save_simple_key() {
            return false;
        }

        // A simple key cannot follow a flow scalar.
        self.simple_key_allowed = false;

        // Create the SCALAR token and append it to the queue.
        let mut token = Token::default();
        if !self.scan_plain_scalar(&mut token) {
            return false;
        }
        self.insert_token(-1, token);
        true
    }

    // Eat whitespaces and comments until the next token is found.
    fn scan_to_next_token(&mut self) -> bool {
        let scan_mark = self.mark;

        // Until the next token is not found.
        loop {
            // Allow the BOM mark to start a line.
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
            if self.mark.column == 0 && is_bom(&self.buffer, self.buffer_pos) {
                self.skip();
            }

            // Eat whitespaces.
            // Tabs are allowed:
            //  - in the flow context
            //  - in the block context, but not at the beginning of the line or
            //  after '-', '?', or ':' (complex value).
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }

            while at(&self.buffer, self.buffer_pos) == b' '
                || ((self.flow_level > 0 || !self.simple_key_allowed) && at(&self.buffer, self.buffer_pos) == b'\t')
            {
                self.skip();
                if self.unread < 1 && !self.update_buffer(1) {
                    return false;
                }
            }

            // Check if we just had a line comment under a sequence entry that
            // looks more like a header to the following content. Similar to this:
            //
            // - # The comment
            //   - Some data
            //
            // If so, transform the line comment to a head comment and reposition.
            if !self.comments.is_empty() && self.tokens.len() > 1 {
                let token_a = self.tokens[self.tokens.len() - 2].typ;
                let token_b = self.tokens[self.tokens.len() - 1].typ;
                let is_break_here = is_break(&self.buffer, self.buffer_pos);
                let mark = self.mark;
                let last = self.comments.len() - 1;
                let comment = &mut self.comments[last];
                if token_a == TokenType::BlockSequenceStart
                    && token_b == TokenType::BlockEntry
                    && !comment.line.is_empty()
                    && !is_break_here
                {
                    // If it was in the prior line, reposition so it becomes a
                    // header of the follow up token. Otherwise, keep it in place
                    // so it becomes a header of the former.
                    comment.head = std::mem::take(&mut comment.line);
                    if comment.start_mark.line == mark.line - 1 {
                        comment.token_mark = mark;
                    }
                }
            }

            // Eat a comment until a line break.
            if at(&self.buffer, self.buffer_pos) == b'#' && !self.scan_comments(scan_mark) {
                return false;
            }

            // If it is a line break, eat it.
            if is_break(&self.buffer, self.buffer_pos) {
                if self.unread < 2 && !self.update_buffer(2) {
                    return false;
                }
                self.skip_line();

                // In the block context, a new line may start a simple key.
                if self.flow_level == 0 {
                    self.simple_key_allowed = true;
                }
            } else {
                break; // We have found a token.
            }
        }

        true
    }

    // Scan a YAML-DIRECTIVE or TAG-DIRECTIVE token.
    //
    // Scope:
    //
    //	%YAML    1.1    # a comment \n
    //	^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    //	%TAG    !yaml!  tag:yaml.org,2002:  \n
    //	^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    fn scan_directive(&mut self, token: &mut Token) -> bool {
        // Eat '%'.
        let start_mark = self.mark;
        self.skip();

        // Scan the directive name.
        let mut name = Vec::new();
        if !self.scan_directive_name(start_mark, &mut name) {
            return false;
        }

        // Is it a YAML directive?
        if name == b"YAML" {
            // Scan the VERSION directive value.
            let (mut major, mut minor) = (0i8, 0i8);
            if !self.scan_version_directive_value(start_mark, &mut major, &mut minor) {
                return false;
            }
            let end_mark = self.mark;

            // Create a VERSION-DIRECTIVE token.
            *token = Token { typ: TokenType::VersionDirective, start_mark, end_mark, major, minor, ..Token::default() };

            // Is it a TAG directive?
        } else if name == b"TAG" {
            // Scan the TAG directive value.
            let (mut handle, mut prefix) = (Vec::new(), Vec::new());
            if !self.scan_tag_directive_value(start_mark, &mut handle, &mut prefix) {
                return false;
            }
            let end_mark = self.mark;

            // Create a TAG-DIRECTIVE token.
            *token =
                Token { typ: TokenType::TagDirective, start_mark, end_mark, value: handle, prefix, ..Token::default() };

            // Unknown directive.
        } else {
            self.set_scanner_error("while scanning a directive", start_mark, "found unknown directive name");
            return false;
        }

        // Eat the rest of the line including any comments.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }

        while is_blank(&self.buffer, self.buffer_pos) {
            self.skip();
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
        }

        if at(&self.buffer, self.buffer_pos) == b'#' {
            // [Go] Discard this inline comment for the time being.
            //if !yaml_parser_scan_line_comment(parser, start_mark) {
            //	return false
            //}
            while !is_breakz(&self.buffer, self.buffer_pos) {
                self.skip();
                if self.unread < 1 && !self.update_buffer(1) {
                    return false;
                }
            }
        }

        // Check if we are at the end of the line.
        if !is_breakz(&self.buffer, self.buffer_pos) {
            self.set_scanner_error(
                "while scanning a directive",
                start_mark,
                "did not find expected comment or line break",
            );
            return false;
        }

        // Eat a line break.
        if is_break(&self.buffer, self.buffer_pos) {
            if self.unread < 2 && !self.update_buffer(2) {
                return false;
            }
            self.skip_line();
        }

        true
    }

    // Scan the directive name.
    //
    // Scope:
    //
    //	%YAML   1.1     # a comment \n
    //	 ^^^^
    //	%TAG    !yaml!  tag:yaml.org,2002:  \n
    //	 ^^^
    fn scan_directive_name(&mut self, start_mark: Mark, name: &mut Vec<u8>) -> bool {
        // Consume the directive name.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }

        let mut s = Vec::new();
        while is_alpha(&self.buffer, self.buffer_pos) {
            self.read(&mut s);
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
        }

        // Check if the name is empty.
        if s.is_empty() {
            self.set_scanner_error("while scanning a directive", start_mark, "could not find expected directive name");
            return false;
        }

        // Check for an blank character after the name.
        if !is_blankz(&self.buffer, self.buffer_pos) {
            self.set_scanner_error(
                "while scanning a directive",
                start_mark,
                "found unexpected non-alphabetical character",
            );
            return false;
        }
        *name = s;
        true
    }

    // Scan the value of VERSION-DIRECTIVE.
    //
    // Scope:
    //
    //	%YAML   1.1     # a comment \n
    //	     ^^^^^^
    fn scan_version_directive_value(&mut self, start_mark: Mark, major: &mut i8, minor: &mut i8) -> bool {
        // Eat whitespaces.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }
        while is_blank(&self.buffer, self.buffer_pos) {
            self.skip();
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
        }

        // Consume the major version number.
        if !self.scan_version_directive_number(start_mark, major) {
            return false;
        }

        // Eat '.'.
        if at(&self.buffer, self.buffer_pos) != b'.' {
            return self.set_scanner_error(
                "while scanning a %YAML directive",
                start_mark,
                "did not find expected digit or '.' character",
            );
        }

        self.skip();

        // Consume the minor version number.
        if !self.scan_version_directive_number(start_mark, minor) {
            return false;
        }
        true
    }

    // Scan the version number of VERSION-DIRECTIVE.
    //
    // Scope:
    //
    //	%YAML   1.1     # a comment \n
    //	        ^
    //	%YAML   1.1     # a comment \n
    //	          ^
    fn scan_version_directive_number(&mut self, start_mark: Mark, number: &mut i8) -> bool {
        // Repeat while the next character is digit.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }
        let (mut value, mut length) = (0i8, 0i8);
        while is_digit(&self.buffer, self.buffer_pos) {
            // Check if the number is too long.
            length += 1;
            if length > MAX_NUMBER_LENGTH {
                return self.set_scanner_error(
                    "while scanning a %YAML directive",
                    start_mark,
                    "found extremely long version number",
                );
            }
            value = value.wrapping_mul(10).wrapping_add(as_digit(&self.buffer, self.buffer_pos) as i8);
            self.skip();
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
        }

        // Check if the number was present.
        if length == 0 {
            return self.set_scanner_error(
                "while scanning a %YAML directive",
                start_mark,
                "did not find expected version number",
            );
        }
        *number = value;
        true
    }

    // Scan the value of a TAG-DIRECTIVE token.
    //
    // Scope:
    //
    //	%TAG    !yaml!  tag:yaml.org,2002:  \n
    //	    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    fn scan_tag_directive_value(&mut self, start_mark: Mark, handle: &mut Vec<u8>, prefix: &mut Vec<u8>) -> bool {
        let (mut handle_value, mut prefix_value) = (Vec::new(), Vec::new());

        // Eat whitespaces.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }

        while is_blank(&self.buffer, self.buffer_pos) {
            self.skip();
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
        }

        // Scan a handle.
        if !self.scan_tag_handle(true, start_mark, &mut handle_value) {
            return false;
        }

        // Expect a whitespace.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }
        if !is_blank(&self.buffer, self.buffer_pos) {
            self.set_scanner_error("while scanning a %TAG directive", start_mark, "did not find expected whitespace");
            return false;
        }

        // Eat whitespaces.
        while is_blank(&self.buffer, self.buffer_pos) {
            self.skip();
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
        }

        // Scan a prefix.
        if !self.scan_tag_uri(true, &[], start_mark, &mut prefix_value) {
            return false;
        }

        // Expect a whitespace or line break.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }
        if !is_blankz(&self.buffer, self.buffer_pos) {
            self.set_scanner_error(
                "while scanning a %TAG directive",
                start_mark,
                "did not find expected whitespace or line break",
            );
            return false;
        }

        *handle = handle_value;
        *prefix = prefix_value;
        true
    }

    fn scan_anchor(&mut self, token: &mut Token, typ: TokenType) -> bool {
        let mut s = Vec::new();

        // Eat the indicator character.
        let start_mark = self.mark;
        self.skip();

        // Consume the value.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }

        while is_alpha(&self.buffer, self.buffer_pos) {
            self.read(&mut s);
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
        }

        let end_mark = self.mark;

        /*
         * Check if length of the anchor is greater than 0 and it is followed by
         * a whitespace character or one of the indicators:
         *
         *      '?', ':', ',', ']', '}', '%', '@', '`'.
         */

        let c = at(&self.buffer, self.buffer_pos);
        if s.is_empty()
            || !(is_blankz(&self.buffer, self.buffer_pos)
                || matches!(c, b'?' | b':' | b',' | b']' | b'}' | b'%' | b'@' | b'`'))
        {
            let context = if typ == TokenType::Anchor { "while scanning an anchor" } else { "while scanning an alias" };
            self.set_scanner_error(context, start_mark, "did not find expected alphabetic or numeric character");
            return false;
        }

        // Create a token.
        *token = Token { typ, start_mark, end_mark, value: s, ..Token::default() };

        true
    }

    /*
     * Scan a TAG token.
     */

    fn scan_tag(&mut self, token: &mut Token) -> bool {
        let mut handle = Vec::new();
        let mut suffix = Vec::new();

        let start_mark = self.mark;

        // Check if the tag is in the canonical form.
        if self.unread < 2 && !self.update_buffer(2) {
            return false;
        }

        if at(&self.buffer, self.buffer_pos + 1) == b'<' {
            // Keep the handle as ''

            // Eat '!<'
            self.skip();
            self.skip();

            // Consume the tag value.
            if !self.scan_tag_uri(false, &[], start_mark, &mut suffix) {
                return false;
            }

            // Check for '>' and eat it.
            if at(&self.buffer, self.buffer_pos) != b'>' {
                self.set_scanner_error("while scanning a tag", start_mark, "did not find the expected '>'");
                return false;
            }

            self.skip();
        } else {
            // The tag has either the '!suffix' or the '!handle!suffix' form.

            // First, try to scan a handle.
            if !self.scan_tag_handle(false, start_mark, &mut handle) {
                return false;
            }

            // Check if it is, indeed, handle.
            if handle[0] == b'!' && handle.len() > 1 && handle[handle.len() - 1] == b'!' {
                // Scan the suffix now.
                if !self.scan_tag_uri(false, &[], start_mark, &mut suffix) {
                    return false;
                }
            } else {
                // It wasn't a handle after all.  Scan the rest of the tag.
                let head = std::mem::take(&mut handle);
                if !self.scan_tag_uri(false, &head, start_mark, &mut suffix) {
                    return false;
                }

                // Set the handle to '!'.
                handle = vec![b'!'];

                // A special case: the '!' tag.  Set the handle to '' and the
                // suffix to '!'.
                if suffix.is_empty() {
                    std::mem::swap(&mut handle, &mut suffix);
                }
            }
        }

        // Check the character which ends the tag.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }
        if !is_blankz(&self.buffer, self.buffer_pos) {
            self.set_scanner_error(
                "while scanning a tag",
                start_mark,
                "did not find expected whitespace or line break",
            );
            return false;
        }

        let end_mark = self.mark;

        // Create a token.
        *token = Token { typ: TokenType::Tag, start_mark, end_mark, value: handle, suffix, ..Token::default() };
        true
    }

    // Scan a tag handle.
    fn scan_tag_handle(&mut self, directive: bool, start_mark: Mark, handle: &mut Vec<u8>) -> bool {
        // Check the initial '!' character.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }
        if at(&self.buffer, self.buffer_pos) != b'!' {
            self.set_scanner_tag_error(directive, start_mark, "did not find expected '!'");
            return false;
        }

        let mut s = Vec::new();

        // Copy the '!' character.
        self.read(&mut s);

        // Copy all subsequent alphabetical and numerical characters.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }
        while is_alpha(&self.buffer, self.buffer_pos) {
            self.read(&mut s);
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
        }

        // Check if the trailing character is '!' and copy it.
        if at(&self.buffer, self.buffer_pos) == b'!' {
            self.read(&mut s);
        } else {
            // It's either the '!' tag or not really a tag handle.  If it's a %TAG
            // directive, it's an error.  If it's a tag token, it must be a part of URI.
            if directive && s != b"!" {
                self.set_scanner_tag_error(directive, start_mark, "did not find expected '!'");
                return false;
            }
        }

        *handle = s;
        true
    }

    // Scan a tag.
    fn scan_tag_uri(&mut self, directive: bool, head: &[u8], start_mark: Mark, uri: &mut Vec<u8>) -> bool {
        //size_t length = head ? strlen((char *)head) : 0
        let mut s = Vec::new();
        let mut has_tag = !head.is_empty();

        // Copy the head if needed.
        //
        // Note that we don't copy the leading '!' character.
        if head.len() > 1 {
            s.extend_from_slice(&head[1..]);
        }

        // Scan the tag.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }

        // The set of characters that may appear in URI is as follows:
        //
        //      '0'-'9', 'A'-'Z', 'a'-'z', '_', '-', ';', '/', '?', ':', '@', '&',
        //      '=', '+', '$', ',', '.', '!', '~', '*', '\'', '(', ')', '[', ']',
        //      '%'.
        // [Go] TODO Convert this into more reasonable logic.
        while is_alpha(&self.buffer, self.buffer_pos)
            || matches!(
                at(&self.buffer, self.buffer_pos),
                b';' | b'/'
                    | b'?'
                    | b':'
                    | b'@'
                    | b'&'
                    | b'='
                    | b'+'
                    | b'$'
                    | b','
                    | b'.'
                    | b'!'
                    | b'~'
                    | b'*'
                    | b'\''
                    | b'('
                    | b')'
                    | b'['
                    | b']'
                    | b'%'
            )
        {
            // Check if it is a URI-escape sequence.
            if at(&self.buffer, self.buffer_pos) == b'%' {
                if !self.scan_uri_escapes(directive, start_mark, &mut s) {
                    return false;
                }
            } else {
                self.read(&mut s);
            }
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
            has_tag = true;
        }

        if !has_tag {
            self.set_scanner_tag_error(directive, start_mark, "did not find expected tag URI");
            return false;
        }
        *uri = s;
        true
    }

    // Decode an URI-escape sequence corresponding to a single UTF-8 character.
    fn scan_uri_escapes(&mut self, directive: bool, start_mark: Mark, s: &mut Vec<u8>) -> bool {
        // Decode the required number of characters.
        let mut w: usize = 1024;
        while w > 0 {
            // Check for a URI-escaped octet.
            if self.unread < 3 && !self.update_buffer(3) {
                return false;
            }

            if !(at(&self.buffer, self.buffer_pos) == b'%'
                && is_hex(&self.buffer, self.buffer_pos + 1)
                && is_hex(&self.buffer, self.buffer_pos + 2))
            {
                return self.set_scanner_tag_error(directive, start_mark, "did not find URI escaped octet");
            }

            // Get the octet.
            let octet =
                ((as_hex(&self.buffer, self.buffer_pos + 1) << 4) + as_hex(&self.buffer, self.buffer_pos + 2)) as u8;

            // If it is the leading octet, determine the length of the UTF-8 sequence.
            if w == 1024 {
                w = width(octet);
                if w == 0 {
                    return self.set_scanner_tag_error(directive, start_mark, "found an incorrect leading UTF-8 octet");
                }
            } else {
                // Check if the trailing octet is correct.
                if octet & 0xC0 != 0x80 {
                    return self.set_scanner_tag_error(
                        directive,
                        start_mark,
                        "found an incorrect trailing UTF-8 octet",
                    );
                }
            }

            // Copy the octet and move the pointers.
            s.push(octet);
            self.skip();
            self.skip();
            self.skip();
            w -= 1;
        }
        true
    }

    // Scan a block scalar.
    fn scan_block_scalar(&mut self, token: &mut Token, literal: bool) -> bool {
        // Eat the indicator '|' or '>'.
        let start_mark = self.mark;
        self.skip();

        // Scan the additional block scalar indicators.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }

        // Check for a chomping indicator.
        let (mut chomping, mut increment) = (0i64, 0i64);
        if at(&self.buffer, self.buffer_pos) == b'+' || at(&self.buffer, self.buffer_pos) == b'-' {
            // Set the chomping method and eat the indicator.
            chomping = if at(&self.buffer, self.buffer_pos) == b'+' { 1 } else { -1 };
            self.skip();

            // Check for an indentation indicator.
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
            if is_digit(&self.buffer, self.buffer_pos) {
                // Check that the indentation is greater than 0.
                if at(&self.buffer, self.buffer_pos) == b'0' {
                    self.set_scanner_error(
                        "while scanning a block scalar",
                        start_mark,
                        "found an indentation indicator equal to 0",
                    );
                    return false;
                }

                // Get the indentation level and eat the indicator.
                increment = as_digit(&self.buffer, self.buffer_pos);
                self.skip();
            }
        } else if is_digit(&self.buffer, self.buffer_pos) {
            // Do the same as above, but in the opposite order.

            if at(&self.buffer, self.buffer_pos) == b'0' {
                self.set_scanner_error(
                    "while scanning a block scalar",
                    start_mark,
                    "found an indentation indicator equal to 0",
                );
                return false;
            }
            increment = as_digit(&self.buffer, self.buffer_pos);
            self.skip();

            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
            if at(&self.buffer, self.buffer_pos) == b'+' || at(&self.buffer, self.buffer_pos) == b'-' {
                chomping = if at(&self.buffer, self.buffer_pos) == b'+' { 1 } else { -1 };
                self.skip();
            }
        }

        // Eat whitespaces and comments to the end of the line.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }
        while is_blank(&self.buffer, self.buffer_pos) {
            self.skip();
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
        }
        if at(&self.buffer, self.buffer_pos) == b'#' {
            if !self.scan_line_comment(start_mark) {
                return false;
            }
            while !is_breakz(&self.buffer, self.buffer_pos) {
                self.skip();
                if self.unread < 1 && !self.update_buffer(1) {
                    return false;
                }
            }
        }

        // Check if we are at the end of the line.
        if !is_breakz(&self.buffer, self.buffer_pos) {
            self.set_scanner_error(
                "while scanning a block scalar",
                start_mark,
                "did not find expected comment or line break",
            );
            return false;
        }

        // Eat a line break.
        if is_break(&self.buffer, self.buffer_pos) {
            if self.unread < 2 && !self.update_buffer(2) {
                return false;
            }
            self.skip_line();
        }

        let mut end_mark = self.mark;

        // Set the indentation level if it was specified.
        let mut indent: i64 = 0;
        if increment > 0 {
            indent = if self.indent >= 0 { self.indent + increment } else { increment };
        }

        // Scan the leading line breaks and determine the indentation level if needed.
        let (mut s, mut leading_break, mut trailing_breaks) = (Vec::new(), Vec::new(), Vec::new());
        if !self.scan_block_scalar_breaks(&mut indent, &mut trailing_breaks, start_mark, &mut end_mark) {
            return false;
        }

        // Scan the block scalar content.
        if self.unread < 1 && !self.update_buffer(1) {
            return false;
        }
        let mut leading_blank = false;
        while self.mark.column == indent && !is_z(&self.buffer, self.buffer_pos) {
            // We are at the beginning of a non-empty line.

            // Is it a trailing whitespace?
            let trailing_blank = is_blank(&self.buffer, self.buffer_pos);

            // Check if we need to fold the leading line break.
            if !literal && !leading_blank && !trailing_blank && !leading_break.is_empty() && leading_break[0] == b'\n' {
                // Do we need to join the lines by space?
                if trailing_breaks.is_empty() {
                    s.push(b' ');
                }
            } else {
                s.extend_from_slice(&leading_break);
            }
            leading_break.clear();

            // Append the remaining line breaks.
            s.extend_from_slice(&trailing_breaks);
            trailing_breaks.clear();

            // Is it a leading whitespace?
            leading_blank = is_blank(&self.buffer, self.buffer_pos);

            // Consume the current line.
            while !is_breakz(&self.buffer, self.buffer_pos) {
                self.read(&mut s);
                if self.unread < 1 && !self.update_buffer(1) {
                    return false;
                }
            }

            // Consume the line break.
            if self.unread < 2 && !self.update_buffer(2) {
                return false;
            }

            self.read_line(&mut leading_break);

            // Eat the following indentation spaces and line breaks.
            if !self.scan_block_scalar_breaks(&mut indent, &mut trailing_breaks, start_mark, &mut end_mark) {
                return false;
            }
        }

        // Chomp the tail.
        if chomping != -1 {
            s.extend_from_slice(&leading_break);
        }
        if chomping == 1 {
            s.extend_from_slice(&trailing_breaks);
        }

        // Create a token.
        *token = Token {
            typ: TokenType::Scalar,
            start_mark,
            end_mark,
            value: s,
            style: if literal { LITERAL_SCALAR_STYLE } else { FOLDED_SCALAR_STYLE },
            ..Token::default()
        };
        true
    }

    // Scan indentation spaces and line breaks for a block scalar.  Determine the
    // indentation level if needed.
    fn scan_block_scalar_breaks(
        &mut self,
        indent: &mut i64,
        breaks: &mut Vec<u8>,
        start_mark: Mark,
        end_mark: &mut Mark,
    ) -> bool {
        *end_mark = self.mark;

        // Eat the indentation spaces and line breaks.
        let mut max_indent: i64 = 0;
        loop {
            // Eat the indentation spaces.
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }
            while (*indent == 0 || self.mark.column < *indent) && is_space(&self.buffer, self.buffer_pos) {
                self.skip();
                if self.unread < 1 && !self.update_buffer(1) {
                    return false;
                }
            }
            if self.mark.column > max_indent {
                max_indent = self.mark.column;
            }

            // Check for a tab character messing the indentation.
            if (*indent == 0 || self.mark.column < *indent) && is_tab(&self.buffer, self.buffer_pos) {
                return self.set_scanner_error(
                    "while scanning a block scalar",
                    start_mark,
                    "found a tab character where an indentation space is expected",
                );
            }

            // Have we found a non-empty line?
            if !is_break(&self.buffer, self.buffer_pos) {
                break;
            }

            // Consume the line break.
            if self.unread < 2 && !self.update_buffer(2) {
                return false;
            }
            // [Go] Should really be returning breaks instead.
            self.read_line(breaks);
            *end_mark = self.mark;
        }

        // Determine the indentation level if needed.
        if *indent == 0 {
            *indent = max_indent;
            if *indent < self.indent + 1 {
                *indent = self.indent + 1;
            }
            if *indent < 1 {
                *indent = 1;
            }
        }
        true
    }

    // Scan a quoted scalar.
    fn scan_flow_scalar(&mut self, token: &mut Token, single: bool) -> bool {
        // Eat the left quote.
        let start_mark = self.mark;
        self.skip();

        // Consume the content of the quoted scalar.
        let (mut s, mut leading_break, mut trailing_breaks, mut whitespaces) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        loop {
            // Check that there are no document indicators at the beginning of the line.
            if self.unread < 4 && !self.update_buffer(4) {
                return false;
            }

            let (b, p) = (&self.buffer, self.buffer_pos);
            if self.mark.column == 0
                && ((at(b, p) == b'-' && at(b, p + 1) == b'-' && at(b, p + 2) == b'-')
                    || (at(b, p) == b'.' && at(b, p + 1) == b'.' && at(b, p + 2) == b'.'))
                && is_blankz(b, p + 3)
            {
                self.set_scanner_error(
                    "while scanning a quoted scalar",
                    start_mark,
                    "found unexpected document indicator",
                );
                return false;
            }

            // Check for EOF.
            if is_z(&self.buffer, self.buffer_pos) {
                self.set_scanner_error("while scanning a quoted scalar", start_mark, "found unexpected end of stream");
                return false;
            }

            // Consume non-blank characters.
            let mut leading_blanks = false;
            while !is_blankz(&self.buffer, self.buffer_pos) {
                let c = at(&self.buffer, self.buffer_pos);
                if single && c == b'\'' && at(&self.buffer, self.buffer_pos + 1) == b'\'' {
                    // Is is an escaped single quote.
                    s.push(b'\'');
                    self.skip();
                    self.skip();
                } else if single && c == b'\'' {
                    // It is a right single quote.
                    break;
                } else if !single && c == b'"' {
                    // It is a right double quote.
                    break;
                } else if !single && c == b'\\' && is_break(&self.buffer, self.buffer_pos + 1) {
                    // It is an escaped line break.
                    if self.unread < 3 && !self.update_buffer(3) {
                        return false;
                    }
                    self.skip();
                    self.skip_line();
                    leading_blanks = true;
                    break;
                } else if !single && c == b'\\' {
                    // It is an escape sequence.
                    let mut code_length = 0usize;

                    // Check the escape character.
                    match at(&self.buffer, self.buffer_pos + 1) {
                        b'0' => s.push(0),
                        b'a' => s.push(b'\x07'),
                        b'b' => s.push(b'\x08'),
                        b't' | b'\t' => s.push(b'\x09'),
                        b'n' => s.push(b'\x0A'),
                        b'v' => s.push(b'\x0B'),
                        b'f' => s.push(b'\x0C'),
                        b'r' => s.push(b'\x0D'),
                        b'e' => s.push(b'\x1B'),
                        b' ' => s.push(b'\x20'),
                        b'"' => s.push(b'"'),
                        b'\'' => s.push(b'\''),
                        b'\\' => s.push(b'\\'),
                        // NEL (#x85)
                        b'N' => s.extend_from_slice(b"\xC2\x85"),
                        // #xA0
                        b'_' => s.extend_from_slice(b"\xC2\xA0"),
                        // LS (#x2028)
                        b'L' => s.extend_from_slice(b"\xE2\x80\xA8"),
                        // PS (#x2029)
                        b'P' => s.extend_from_slice(b"\xE2\x80\xA9"),
                        b'x' => code_length = 2,
                        b'u' => code_length = 4,
                        b'U' => code_length = 8,
                        _ => {
                            self.set_scanner_error(
                                "while parsing a quoted scalar",
                                start_mark,
                                "found unknown escape character",
                            );
                            return false;
                        }
                    }

                    self.skip();
                    self.skip();

                    // Consume an arbitrary escape code.
                    if code_length > 0 {
                        let mut value: i64 = 0;

                        // Scan the character value.
                        if self.unread < code_length && !self.update_buffer(code_length) {
                            return false;
                        }
                        for k in 0..code_length {
                            if !is_hex(&self.buffer, self.buffer_pos + k) {
                                self.set_scanner_error(
                                    "while parsing a quoted scalar",
                                    start_mark,
                                    "did not find expected hexdecimal number",
                                );
                                return false;
                            }
                            value = (value << 4) + as_hex(&self.buffer, self.buffer_pos + k);
                        }

                        // Check the value and write the character.
                        if (0xD800..=0xDFFF).contains(&value) || value > 0x10FFFF {
                            self.set_scanner_error(
                                "while parsing a quoted scalar",
                                start_mark,
                                "found invalid Unicode character escape code",
                            );
                            return false;
                        }
                        if value <= 0x7F {
                            s.push(value as u8);
                        } else if value <= 0x7FF {
                            s.push((0xC0 + (value >> 6)) as u8);
                            s.push((0x80 + (value & 0x3F)) as u8);
                        } else if value <= 0xFFFF {
                            s.push((0xE0 + (value >> 12)) as u8);
                            s.push((0x80 + ((value >> 6) & 0x3F)) as u8);
                            s.push((0x80 + (value & 0x3F)) as u8);
                        } else {
                            s.push((0xF0 + (value >> 18)) as u8);
                            s.push((0x80 + ((value >> 12) & 0x3F)) as u8);
                            s.push((0x80 + ((value >> 6) & 0x3F)) as u8);
                            s.push((0x80 + (value & 0x3F)) as u8);
                        }

                        // Advance the pointer.
                        for _ in 0..code_length {
                            self.skip();
                        }
                    }
                } else {
                    // It is a non-escaped non-blank character.
                    self.read(&mut s);
                }
                if self.unread < 2 && !self.update_buffer(2) {
                    return false;
                }
            }

            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }

            // Check if we are at the end of the scalar.
            let c = at(&self.buffer, self.buffer_pos);
            if single {
                if c == b'\'' {
                    break;
                }
            } else if c == b'"' {
                break;
            }

            // Consume blank characters.
            while is_blank(&self.buffer, self.buffer_pos) || is_break(&self.buffer, self.buffer_pos) {
                if is_blank(&self.buffer, self.buffer_pos) {
                    // Consume a space or a tab character.
                    if !leading_blanks {
                        self.read(&mut whitespaces);
                    } else {
                        self.skip();
                    }
                } else {
                    if self.unread < 2 && !self.update_buffer(2) {
                        return false;
                    }

                    // Check if it is a first line break.
                    if !leading_blanks {
                        whitespaces.clear();
                        self.read_line(&mut leading_break);
                        leading_blanks = true;
                    } else {
                        self.read_line(&mut trailing_breaks);
                    }
                }
                if self.unread < 1 && !self.update_buffer(1) {
                    return false;
                }
            }

            // Join the whitespaces or fold line breaks.
            if leading_blanks {
                // Do we need to fold line breaks?
                if !leading_break.is_empty() && leading_break[0] == b'\n' {
                    if trailing_breaks.is_empty() {
                        s.push(b' ');
                    } else {
                        s.extend_from_slice(&trailing_breaks);
                    }
                } else {
                    s.extend_from_slice(&leading_break);
                    s.extend_from_slice(&trailing_breaks);
                }
                trailing_breaks.clear();
                leading_break.clear();
            } else {
                s.extend_from_slice(&whitespaces);
                whitespaces.clear();
            }
        }

        // Eat the right quote.
        self.skip();
        let end_mark = self.mark;

        // Create a token.
        *token = Token {
            typ: TokenType::Scalar,
            start_mark,
            end_mark,
            value: s,
            style: if single { SINGLE_QUOTED_SCALAR_STYLE } else { DOUBLE_QUOTED_SCALAR_STYLE },
            ..Token::default()
        };
        true
    }

    // Scan a plain scalar.
    fn scan_plain_scalar(&mut self, token: &mut Token) -> bool {
        let (mut s, mut leading_break, mut trailing_breaks, mut whitespaces) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut leading_blanks = false;
        let indent = self.indent + 1;

        let start_mark = self.mark;
        let mut end_mark = self.mark;

        // Consume the content of the plain scalar.
        loop {
            // Check for a document indicator.
            if self.unread < 4 && !self.update_buffer(4) {
                return false;
            }
            let (b, p) = (&self.buffer, self.buffer_pos);
            if self.mark.column == 0
                && ((at(b, p) == b'-' && at(b, p + 1) == b'-' && at(b, p + 2) == b'-')
                    || (at(b, p) == b'.' && at(b, p + 1) == b'.' && at(b, p + 2) == b'.'))
                && is_blankz(b, p + 3)
            {
                break;
            }

            // Check for a comment.
            if at(b, p) == b'#' {
                break;
            }

            // Consume non-blank characters.
            while !is_blankz(&self.buffer, self.buffer_pos) {
                // Check for indicators that may end a plain scalar.
                let c = at(&self.buffer, self.buffer_pos);
                if (c == b':' && is_blankz(&self.buffer, self.buffer_pos + 1))
                    || (self.flow_level > 0 && matches!(c, b',' | b'?' | b'[' | b']' | b'{' | b'}'))
                {
                    break;
                }

                // Check if we need to join whitespaces and breaks.
                if leading_blanks || !whitespaces.is_empty() {
                    if leading_blanks {
                        // Do we need to fold line breaks?
                        if leading_break[0] == b'\n' {
                            if trailing_breaks.is_empty() {
                                s.push(b' ');
                            } else {
                                s.extend_from_slice(&trailing_breaks);
                            }
                        } else {
                            s.extend_from_slice(&leading_break);
                            s.extend_from_slice(&trailing_breaks);
                        }
                        trailing_breaks.clear();
                        leading_break.clear();
                        leading_blanks = false;
                    } else {
                        s.extend_from_slice(&whitespaces);
                        whitespaces.clear();
                    }
                }

                // Copy the character.
                self.read(&mut s);

                end_mark = self.mark;
                if self.unread < 2 && !self.update_buffer(2) {
                    return false;
                }
            }

            // Is it the end?
            if !(is_blank(&self.buffer, self.buffer_pos) || is_break(&self.buffer, self.buffer_pos)) {
                break;
            }

            // Consume blank characters.
            if self.unread < 1 && !self.update_buffer(1) {
                return false;
            }

            while is_blank(&self.buffer, self.buffer_pos) || is_break(&self.buffer, self.buffer_pos) {
                if is_blank(&self.buffer, self.buffer_pos) {
                    // Check for tab characters that abuse indentation.
                    if leading_blanks && self.mark.column < indent && is_tab(&self.buffer, self.buffer_pos) {
                        self.set_scanner_error(
                            "while scanning a plain scalar",
                            start_mark,
                            "found a tab character that violates indentation",
                        );
                        return false;
                    }

                    // Consume a space or a tab character.
                    if !leading_blanks {
                        self.read(&mut whitespaces);
                    } else {
                        self.skip();
                    }
                } else {
                    if self.unread < 2 && !self.update_buffer(2) {
                        return false;
                    }

                    // Check if it is a first line break.
                    if !leading_blanks {
                        whitespaces.clear();
                        self.read_line(&mut leading_break);
                        leading_blanks = true;
                    } else {
                        self.read_line(&mut trailing_breaks);
                    }
                }
                if self.unread < 1 && !self.update_buffer(1) {
                    return false;
                }
            }

            // Check indentation level.
            if self.flow_level == 0 && self.mark.column < indent {
                break;
            }
        }

        // Create a token.
        *token = Token {
            typ: TokenType::Scalar,
            start_mark,
            end_mark,
            value: s,
            style: PLAIN_SCALAR_STYLE,
            ..Token::default()
        };

        // Note that we change the 'simple_key_allowed' flag.
        if leading_blanks {
            self.simple_key_allowed = true;
        }
        true
    }

    fn scan_line_comment(&mut self, token_mark: Mark) -> bool {
        if self.newlines > 0 {
            return true;
        }

        let mut start_mark = Mark::default();
        let mut text = Vec::new();

        let mut peek = 0usize;
        while peek < 512 {
            if self.unread < peek + 1 && !self.update_buffer(peek + 1) {
                break;
            }
            if is_blank(&self.buffer, self.buffer_pos + peek) {
                peek += 1;
                continue;
            }
            if at(&self.buffer, self.buffer_pos + peek) == b'#' {
                let seen = self.mark.index + peek as i64;
                loop {
                    if self.unread < 1 && !self.update_buffer(1) {
                        return false;
                    }
                    if is_breakz(&self.buffer, self.buffer_pos) {
                        if self.mark.index >= seen {
                            break;
                        }
                        if self.unread < 2 && !self.update_buffer(2) {
                            return false;
                        }
                        self.skip_line();
                    } else if self.mark.index >= seen {
                        if text.is_empty() {
                            start_mark = self.mark;
                        }
                        self.read(&mut text);
                    } else {
                        self.skip();
                    }
                }
            }
            break;
        }
        if !text.is_empty() {
            self.comments.push(Comment { token_mark, start_mark, line: text, ..Comment::default() });
        }
        true
    }

    fn scan_comments(&mut self, scan_mark: Mark) -> bool {
        let mut scan_mark = scan_mark;
        let mut token = self.tokens[self.tokens.len() - 1].clone();

        if token.typ == TokenType::FlowEntry && self.tokens.len() > 1 {
            token = self.tokens[self.tokens.len() - 2].clone();
        }

        let mut token_mark = token.start_mark;
        let mut start_mark = Mark::default();
        let mut next_indent = self.indent;
        if next_indent < 0 {
            next_indent = 0;
        }

        let mut recent_empty = false;
        let mut first_empty = self.newlines <= 1;

        let mut line = self.mark.line;
        let mut column = self.mark.column;

        let mut text: Vec<u8> = Vec::new();

        // The foot line is the place where a comment must start to
        // still be considered as a foot of the prior content.
        // If there's some content in the currently parsed line, then
        // the foot is the line below it.
        let mut foot_line: i64 = -1;
        if scan_mark.line > 0 {
            foot_line = self.mark.line - self.newlines as i64 + 1;
            if self.newlines == 0 && self.mark.column > 1 {
                foot_line += 1;
            }
        }

        let mut peek = 0usize;
        while peek < 512 {
            if self.unread < peek + 1 && !self.update_buffer(peek + 1) {
                break;
            }
            column += 1;
            if is_blank(&self.buffer, self.buffer_pos + peek) {
                peek += 1;
                continue;
            }
            let c = at(&self.buffer, self.buffer_pos + peek);
            let close_flow = self.flow_level > 0 && (c == b']' || c == b'}');
            if close_flow || is_breakz(&self.buffer, self.buffer_pos + peek) {
                // Got line break or terminator.
                if close_flow || !recent_empty {
                    if close_flow
                        || first_empty
                            && (start_mark.line == foot_line && token.typ != TokenType::Value
                                || start_mark.column - 1 < next_indent)
                    {
                        // This is the first empty line and there were no empty lines before,
                        // so this initial part of the comment is a foot of the prior token
                        // instead of being a head for the following one. Split it up.
                        // Alternatively, this might also be the last comment inside a flow
                        // scope, so it must be a footer.
                        if !text.is_empty() {
                            if start_mark.column - 1 < next_indent {
                                // If dedented it's unrelated to the prior token.
                                token_mark = start_mark;
                            }
                            let end = Mark { index: self.mark.index + peek as i64, line, column };
                            self.comments.push(Comment {
                                scan_mark,
                                token_mark,
                                start_mark,
                                end_mark: end,
                                foot: std::mem::take(&mut text),
                                ..Comment::default()
                            });
                            scan_mark = end;
                            token_mark = scan_mark;
                        }
                    } else if !text.is_empty() && at(&self.buffer, self.buffer_pos + peek) != 0 {
                        text.push(b'\n');
                    }
                }
                if !is_break(&self.buffer, self.buffer_pos + peek) {
                    break;
                }
                first_empty = false;
                recent_empty = true;
                column = 0;
                line += 1;
                peek += 1;
                continue;
            }

            if !text.is_empty() && (close_flow || column - 1 < next_indent && column != start_mark.column) {
                // The comment at the different indentation is a foot of the
                // preceding data rather than a head of the upcoming one.
                let end = Mark { index: self.mark.index + peek as i64, line, column };
                self.comments.push(Comment {
                    scan_mark,
                    token_mark,
                    start_mark,
                    end_mark: end,
                    foot: std::mem::take(&mut text),
                    ..Comment::default()
                });
                scan_mark = end;
                token_mark = scan_mark;
            }

            if at(&self.buffer, self.buffer_pos + peek) != b'#' {
                break;
            }

            if text.is_empty() {
                start_mark = Mark { index: self.mark.index + peek as i64, line, column };
            } else {
                text.push(b'\n');
            }

            recent_empty = false;

            // Consume until after the consumed comment line.
            let seen = self.mark.index + peek as i64;
            loop {
                if self.unread < 1 && !self.update_buffer(1) {
                    return false;
                }
                if is_breakz(&self.buffer, self.buffer_pos) {
                    if self.mark.index >= seen {
                        break;
                    }
                    if self.unread < 2 && !self.update_buffer(2) {
                        return false;
                    }
                    self.skip_line();
                } else if self.mark.index >= seen {
                    self.read(&mut text);
                } else {
                    self.skip();
                }
            }

            column = 0;
            line = self.mark.line;
            next_indent = self.indent;
            if next_indent < 0 {
                next_indent = 0;
            }
            // Go sets peek to 0 here, and its for loop's peek++ follows: the
            // next look starts at 1.
            peek = 1;
        }

        if !text.is_empty() {
            self.comments.push(Comment {
                scan_mark,
                token_mark: start_mark,
                start_mark,
                end_mark: Mark { index: self.mark.index + peek as i64 - 1, line, column },
                head: text,
                ..Comment::default()
            });
        }
        true
    }
}

// max_flow_level limits the flow_level
const MAX_FLOW_LEVEL: i64 = 10000;

// max_indents limits the indents stack size
const MAX_INDENTS: usize = 10000;

const MAX_NUMBER_LENGTH: i8 = 2;
