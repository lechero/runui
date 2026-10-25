//! parserc.go: the tokens read into events, the comments the scanner kept
//! given to the events they belong to.

use super::h::*;

// The parser implements the following grammar:
//
// stream               ::= STREAM-START implicit_document? explicit_document* STREAM-END
// implicit_document    ::= block_node DOCUMENT-END*
// explicit_document    ::= DIRECTIVE* DOCUMENT-START block_node? DOCUMENT-END*
// block_node_or_indentless_sequence    ::=
//                          ALIAS
//                          | properties (block_content | indentless_block_sequence)?
//                          | block_content
//                          | indentless_block_sequence
// block_node           ::= ALIAS
//                          | properties block_content?
//                          | block_content
// flow_node            ::= ALIAS
//                          | properties flow_content?
//                          | flow_content
// properties           ::= TAG ANCHOR? | ANCHOR TAG?
// block_content        ::= block_collection | flow_collection | SCALAR
// flow_content         ::= flow_collection | SCALAR
// block_collection     ::= block_sequence | block_mapping
// flow_collection      ::= flow_sequence | flow_mapping
// block_sequence       ::= BLOCK-SEQUENCE-START (BLOCK-ENTRY block_node?)* BLOCK-END
// indentless_sequence  ::= (BLOCK-ENTRY block_node?)+
// block_mapping        ::= BLOCK-MAPPING_START
//                          ((KEY block_node_or_indentless_sequence?)?
//                          (VALUE block_node_or_indentless_sequence?)?)*
//                          BLOCK-END
// flow_sequence        ::= FLOW-SEQUENCE-START
//                          (flow_sequence_entry FLOW-ENTRY)*
//                          flow_sequence_entry?
//                          FLOW-SEQUENCE-END
// flow_sequence_entry  ::= flow_node | KEY flow_node? (VALUE flow_node?)?
// flow_mapping         ::= FLOW-MAPPING-START
//                          (flow_mapping_entry FLOW-ENTRY)*
//                          flow_mapping_entry?
//                          FLOW-MAPPING-END
// flow_mapping_entry   ::= flow_node | KEY flow_node? (VALUE flow_node?)?

fn default_tag_directives() -> [TagDirective; 2] {
    [
        TagDirective { handle: b"!".to_vec(), prefix: b"!".to_vec() },
        TagDirective { handle: b"!!".to_vec(), prefix: b"tag:yaml.org,2002:".to_vec() },
    ]
}

impl Parser {
    // Peek the next token in the token queue: a copy of it.
    fn peek_token(&mut self) -> Option<Token> {
        if self.token_available || self.fetch_more_tokens() {
            let token = self.tokens[self.tokens_head].clone();
            self.unfold_comments(&token);
            return Some(token);
        }
        None
    }

    // unfold_comments walks through the comments queue and joins all
    // comments behind the position of the provided token into the respective
    // top-level comment slices in the parser.
    fn unfold_comments(&mut self, token: &Token) {
        while self.comments_head < self.comments.len()
            && token.start_mark.index >= self.comments[self.comments_head].token_mark.index
        {
            let comment = &self.comments[self.comments_head];
            if !comment.head.is_empty() {
                if token.typ == TokenType::BlockEnd {
                    // No heads on ends, so keep comment.head for a follow up token.
                    break;
                }
                if !self.head_comment.is_empty() {
                    self.head_comment.push(b'\n');
                }
                self.head_comment.extend_from_slice(&comment.head);
            }
            if !comment.foot.is_empty() {
                if !self.foot_comment.is_empty() {
                    self.foot_comment.push(b'\n');
                }
                self.foot_comment.extend_from_slice(&comment.foot);
            }
            if !comment.line.is_empty() {
                if !self.line_comment.is_empty() {
                    self.line_comment.push(b'\n');
                }
                self.line_comment.extend_from_slice(&comment.line);
            }
            self.comments[self.comments_head] = Comment::default();
            self.comments_head += 1;
        }
    }

    // Remove the next token from the queue (must be called after peek_token).
    fn skip_token(&mut self) {
        self.token_available = false;
        self.tokens_parsed += 1;
        self.stream_end_produced = self.tokens[self.tokens_head].typ == TokenType::StreamEnd;
        self.tokens_head += 1;
    }

    /// parse is the next event.
    pub(super) fn parse(&mut self, event: &mut Event) -> bool {
        // Erase the event object.
        *event = Event::default();

        // No events after the end of the stream or error.
        if self.stream_end_produced || self.error != ErrorType::None || self.state == ParserState::End {
            return true;
        }

        // Generate the next event.
        self.state_machine(event)
    }

    // Set parser error.
    fn set_parser_error(&mut self, problem: &str, problem_mark: Mark) -> bool {
        self.error = ErrorType::Parser;
        self.problem = problem.to_string();
        self.problem_mark = problem_mark;
        false
    }

    fn set_parser_error_context(
        &mut self,
        context: &str,
        context_mark: Mark,
        problem: &str,
        problem_mark: Mark,
    ) -> bool {
        self.error = ErrorType::Parser;
        self.context = context.to_string();
        self.context_mark = context_mark;
        self.problem = problem.to_string();
        self.problem_mark = problem_mark;
        false
    }

    fn pop_state(&mut self) {
        self.state = self.states.pop().unwrap_or_default();
    }

    // State dispatcher.
    fn state_machine(&mut self, event: &mut Event) -> bool {
        match self.state {
            ParserState::StreamStart => self.parse_stream_start(event),
            ParserState::ImplicitDocumentStart => self.parse_document_start(event, true),
            ParserState::DocumentStart => self.parse_document_start(event, false),
            ParserState::DocumentContent => self.parse_document_content(event),
            ParserState::DocumentEnd => self.parse_document_end(event),
            ParserState::BlockNode => self.parse_node(event, true, false),
            ParserState::BlockNodeOrIndentlessSequence => self.parse_node(event, true, true),
            ParserState::FlowNode => self.parse_node(event, false, false),
            ParserState::BlockSequenceFirstEntry => self.parse_block_sequence_entry(event, true),
            ParserState::BlockSequenceEntry => self.parse_block_sequence_entry(event, false),
            ParserState::IndentlessSequenceEntry => self.parse_indentless_sequence_entry(event),
            ParserState::BlockMappingFirstKey => self.parse_block_mapping_key(event, true),
            ParserState::BlockMappingKey => self.parse_block_mapping_key(event, false),
            ParserState::BlockMappingValue => self.parse_block_mapping_value(event),
            ParserState::FlowSequenceFirstEntry => self.parse_flow_sequence_entry(event, true),
            ParserState::FlowSequenceEntry => self.parse_flow_sequence_entry(event, false),
            ParserState::FlowSequenceEntryMappingKey => self.parse_flow_sequence_entry_mapping_key(event),
            ParserState::FlowSequenceEntryMappingValue => self.parse_flow_sequence_entry_mapping_value(event),
            ParserState::FlowSequenceEntryMappingEnd => self.parse_flow_sequence_entry_mapping_end(event),
            ParserState::FlowMappingFirstKey => self.parse_flow_mapping_key(event, true),
            ParserState::FlowMappingKey => self.parse_flow_mapping_key(event, false),
            ParserState::FlowMappingValue => self.parse_flow_mapping_value(event, false),
            ParserState::FlowMappingEmptyValue => self.parse_flow_mapping_value(event, true),
            ParserState::End => panic!("invalid parser state"),
        }
    }

    // Parse the production:
    //
    //	stream   ::= STREAM-START implicit_document? explicit_document* STREAM-END
    //	             ************
    fn parse_stream_start(&mut self, event: &mut Event) -> bool {
        let Some(token) = self.peek_token() else {
            return false;
        };
        if token.typ != TokenType::StreamStart {
            return self.set_parser_error("did not find expected <stream-start>", token.start_mark);
        }
        self.state = ParserState::ImplicitDocumentStart;
        *event = Event {
            typ: EventType::StreamStart,
            start_mark: token.start_mark,
            end_mark: token.end_mark,
            encoding: token.encoding,
            ..Event::default()
        };
        self.skip_token();
        true
    }

    // Parse the productions:
    //
    //	implicit_document    ::= block_node DOCUMENT-END*
    //	                         *
    //	explicit_document    ::= DIRECTIVE* DOCUMENT-START block_node? DOCUMENT-END*
    //	                         *************************
    fn parse_document_start(&mut self, event: &mut Event, implicit: bool) -> bool {
        let Some(mut token) = self.peek_token() else {
            return false;
        };

        // Parse extra document end indicators.
        if !implicit {
            while token.typ == TokenType::DocumentEnd {
                self.skip_token();
                let Some(t) = self.peek_token() else {
                    return false;
                };
                token = t;
            }
        }

        if implicit
            && token.typ != TokenType::VersionDirective
            && token.typ != TokenType::TagDirective
            && token.typ != TokenType::DocumentStart
            && token.typ != TokenType::StreamEnd
        {
            // Parse an implicit document.
            if !self.process_directives(None, None) {
                return false;
            }
            self.states.push(ParserState::DocumentEnd);
            self.state = ParserState::BlockNode;

            let mut head_comment = Vec::new();
            if !self.head_comment.is_empty() {
                // [Go] Scan the header comment backwards, and if an empty line is found, break
                //      the header so the part before the last empty line goes into the
                //      document header, while the bottom of it goes into a follow up event.
                let mut i = self.head_comment.len() - 1;
                while i > 0 {
                    if self.head_comment[i] == b'\n' {
                        if i == self.head_comment.len() - 1 {
                            head_comment = self.head_comment[..i].to_vec();
                            self.head_comment = self.head_comment[i + 1..].to_vec();
                            break;
                        } else if self.head_comment[i - 1] == b'\n' {
                            head_comment = self.head_comment[..i - 1].to_vec();
                            self.head_comment = self.head_comment[i + 1..].to_vec();
                            break;
                        }
                    }
                    i -= 1;
                }
            }

            *event = Event {
                typ: EventType::DocumentStart,
                start_mark: token.start_mark,
                end_mark: token.end_mark,
                head_comment,
                ..Event::default()
            };
        } else if token.typ != TokenType::StreamEnd {
            // Parse an explicit document.
            let mut version_directive = None;
            let mut tag_directives = Vec::new();
            let start_mark = token.start_mark;
            if !self.process_directives(Some(&mut version_directive), Some(&mut tag_directives)) {
                return false;
            }
            let Some(token) = self.peek_token() else {
                return false;
            };
            if token.typ != TokenType::DocumentStart {
                self.set_parser_error("did not find expected <document start>", token.start_mark);
                return false;
            }
            self.states.push(ParserState::DocumentEnd);
            self.state = ParserState::DocumentContent;
            let end_mark = token.end_mark;

            *event = Event {
                typ: EventType::DocumentStart,
                start_mark,
                end_mark,
                version_directive,
                tag_directives,
                implicit: false,
                ..Event::default()
            };
            self.skip_token();
        } else {
            // Parse the stream end.
            self.state = ParserState::End;
            *event = Event {
                typ: EventType::StreamEnd,
                start_mark: token.start_mark,
                end_mark: token.end_mark,
                ..Event::default()
            };
            self.skip_token();
        }

        true
    }

    // Parse the productions:
    //
    //	explicit_document    ::= DIRECTIVE* DOCUMENT-START block_node? DOCUMENT-END*
    //	                                                   ***********
    fn parse_document_content(&mut self, event: &mut Event) -> bool {
        let Some(token) = self.peek_token() else {
            return false;
        };

        if token.typ == TokenType::VersionDirective
            || token.typ == TokenType::TagDirective
            || token.typ == TokenType::DocumentStart
            || token.typ == TokenType::DocumentEnd
            || token.typ == TokenType::StreamEnd
        {
            self.pop_state();
            return self.process_empty_scalar(event, token.start_mark);
        }
        self.parse_node(event, true, false)
    }

    // Parse the productions:
    //
    //	implicit_document    ::= block_node DOCUMENT-END*
    //	                                    *************
    //	explicit_document    ::= DIRECTIVE* DOCUMENT-START block_node? DOCUMENT-END*
    fn parse_document_end(&mut self, event: &mut Event) -> bool {
        let Some(token) = self.peek_token() else {
            return false;
        };

        let start_mark = token.start_mark;
        let mut end_mark = token.start_mark;

        let mut implicit = true;
        if token.typ == TokenType::DocumentEnd {
            end_mark = token.end_mark;
            self.skip_token();
            implicit = false;
        }

        self.tag_directives.clear();

        self.state = ParserState::DocumentStart;
        *event = Event { typ: EventType::DocumentEnd, start_mark, end_mark, implicit, ..Event::default() };
        self.set_event_comments(event);
        if !event.head_comment.is_empty() && event.foot_comment.is_empty() {
            event.foot_comment = std::mem::take(&mut event.head_comment);
        }
        true
    }

    fn set_event_comments(&mut self, event: &mut Event) {
        event.head_comment = std::mem::take(&mut self.head_comment);
        event.line_comment = std::mem::take(&mut self.line_comment);
        event.foot_comment = std::mem::take(&mut self.foot_comment);
        self.tail_comment.clear();
        self.stem_comment.clear();
    }

    // Parse the productions:
    //
    //	block_node_or_indentless_sequence    ::=
    //	                         ALIAS
    //	                         *****
    //	                         | properties (block_content | indentless_block_sequence)?
    //	                           **********  *
    //	                         | block_content | indentless_block_sequence
    //	                           *
    //	block_node           ::= ALIAS
    //	                         *****
    //	                         | properties block_content?
    //	                           ********** *
    //	                         | block_content
    //	                           *
    //	flow_node            ::= ALIAS
    //	                         *****
    //	                         | properties flow_content?
    //	                           ********** *
    //	                         | flow_content
    //	                           *
    //	properties           ::= TAG ANCHOR? | ANCHOR TAG?
    //	                         *************************
    //	block_content        ::= block_collection | flow_collection | SCALAR
    //	                                                              ******
    //	flow_content         ::= flow_collection | SCALAR
    //	                                           ******
    fn parse_node(&mut self, event: &mut Event, block: bool, indentless_sequence: bool) -> bool {
        let Some(mut token) = self.peek_token() else {
            return false;
        };

        if token.typ == TokenType::Alias {
            self.pop_state();
            *event = Event {
                typ: EventType::Alias,
                start_mark: token.start_mark,
                end_mark: token.end_mark,
                anchor: Some(token.value.clone()),
                ..Event::default()
            };
            self.set_event_comments(event);
            self.skip_token();
            return true;
        }

        let mut start_mark = token.start_mark;
        let mut end_mark = token.start_mark;

        let mut tag_token = false;
        let (mut tag_handle, mut tag_suffix, mut anchor): (Vec<u8>, Vec<u8>, Option<Vec<u8>>) =
            (Vec::new(), Vec::new(), None);
        let mut tag_mark = Mark::default();
        if token.typ == TokenType::Anchor {
            anchor = Some(token.value.clone());
            start_mark = token.start_mark;
            end_mark = token.end_mark;
            self.skip_token();
            let Some(t) = self.peek_token() else {
                return false;
            };
            token = t;
            if token.typ == TokenType::Tag {
                tag_token = true;
                tag_handle = token.value.clone();
                tag_suffix = token.suffix.clone();
                tag_mark = token.start_mark;
                end_mark = token.end_mark;
                self.skip_token();
                let Some(t) = self.peek_token() else {
                    return false;
                };
                token = t;
            }
        } else if token.typ == TokenType::Tag {
            tag_token = true;
            tag_handle = token.value.clone();
            tag_suffix = token.suffix.clone();
            start_mark = token.start_mark;
            tag_mark = token.start_mark;
            end_mark = token.end_mark;
            self.skip_token();
            let Some(t) = self.peek_token() else {
                return false;
            };
            token = t;
            if token.typ == TokenType::Anchor {
                anchor = Some(token.value.clone());
                end_mark = token.end_mark;
                self.skip_token();
                let Some(t) = self.peek_token() else {
                    return false;
                };
                token = t;
            }
        }

        let mut tag: Vec<u8> = Vec::new();
        if tag_token {
            if tag_handle.is_empty() {
                tag = std::mem::take(&mut tag_suffix);
            } else {
                for td in &self.tag_directives {
                    if td.handle == tag_handle {
                        tag = td.prefix.clone();
                        tag.extend_from_slice(&tag_suffix);
                        break;
                    }
                }
                if tag.is_empty() {
                    self.set_parser_error_context(
                        "while parsing a node",
                        start_mark,
                        "found undefined tag handle",
                        tag_mark,
                    );
                    return false;
                }
            }
        }

        let implicit = tag.is_empty();
        if indentless_sequence && token.typ == TokenType::BlockEntry {
            end_mark = token.end_mark;
            self.state = ParserState::IndentlessSequenceEntry;
            *event = Event {
                typ: EventType::SequenceStart,
                start_mark,
                end_mark,
                anchor,
                tag,
                implicit,
                style: BLOCK_SEQUENCE_STYLE,
                ..Event::default()
            };
            return true;
        }
        if token.typ == TokenType::Scalar {
            let (mut plain_implicit, mut quoted_implicit) = (false, false);
            end_mark = token.end_mark;
            if (tag.is_empty() && token.style == PLAIN_SCALAR_STYLE) || (tag.len() == 1 && tag[0] == b'!') {
                plain_implicit = true;
            } else if tag.is_empty() {
                quoted_implicit = true;
            }
            self.pop_state();

            *event = Event {
                typ: EventType::Scalar,
                start_mark,
                end_mark,
                anchor,
                tag,
                value: token.value.clone(),
                implicit: plain_implicit,
                quoted_implicit,
                style: token.style,
                ..Event::default()
            };
            self.set_event_comments(event);
            self.skip_token();
            return true;
        }
        if token.typ == TokenType::FlowSequenceStart {
            // [Go] Some of the events below can be merged as they differ only on style.
            end_mark = token.end_mark;
            self.state = ParserState::FlowSequenceFirstEntry;
            *event = Event {
                typ: EventType::SequenceStart,
                start_mark,
                end_mark,
                anchor,
                tag,
                implicit,
                style: FLOW_SEQUENCE_STYLE,
                ..Event::default()
            };
            self.set_event_comments(event);
            return true;
        }
        if token.typ == TokenType::FlowMappingStart {
            end_mark = token.end_mark;
            self.state = ParserState::FlowMappingFirstKey;
            *event = Event {
                typ: EventType::MappingStart,
                start_mark,
                end_mark,
                anchor,
                tag,
                implicit,
                style: FLOW_MAPPING_STYLE,
                ..Event::default()
            };
            self.set_event_comments(event);
            return true;
        }
        if block && token.typ == TokenType::BlockSequenceStart {
            end_mark = token.end_mark;
            self.state = ParserState::BlockSequenceFirstEntry;
            *event = Event {
                typ: EventType::SequenceStart,
                start_mark,
                end_mark,
                anchor,
                tag,
                implicit,
                style: BLOCK_SEQUENCE_STYLE,
                ..Event::default()
            };
            if !self.stem_comment.is_empty() {
                event.head_comment = std::mem::take(&mut self.stem_comment);
            }
            return true;
        }
        if block && token.typ == TokenType::BlockMappingStart {
            end_mark = token.end_mark;
            self.state = ParserState::BlockMappingFirstKey;
            *event = Event {
                typ: EventType::MappingStart,
                start_mark,
                end_mark,
                anchor,
                tag,
                implicit,
                style: BLOCK_MAPPING_STYLE,
                ..Event::default()
            };
            if !self.stem_comment.is_empty() {
                event.head_comment = std::mem::take(&mut self.stem_comment);
            }
            return true;
        }
        if anchor.as_ref().is_some_and(|a| !a.is_empty()) || !tag.is_empty() {
            self.pop_state();

            *event = Event {
                typ: EventType::Scalar,
                start_mark,
                end_mark,
                anchor,
                tag,
                implicit,
                quoted_implicit: false,
                style: PLAIN_SCALAR_STYLE,
                ..Event::default()
            };
            return true;
        }

        let context = if block { "while parsing a block node" } else { "while parsing a flow node" };
        self.set_parser_error_context(context, start_mark, "did not find expected node content", token.start_mark);
        false
    }

    // Parse the productions:
    //
    //	block_sequence ::= BLOCK-SEQUENCE-START (BLOCK-ENTRY block_node?)* BLOCK-END
    //	                   ********************  *********** *             *********
    fn parse_block_sequence_entry(&mut self, event: &mut Event, first: bool) -> bool {
        if first {
            let Some(token) = self.peek_token() else {
                return false;
            };
            self.marks.push(token.start_mark);
            self.skip_token();
        }

        let Some(token) = self.peek_token() else {
            return false;
        };

        if token.typ == TokenType::BlockEntry {
            let mark = token.end_mark;
            let prior_head_len = self.head_comment.len();
            self.skip_token();
            self.split_stem_comment(prior_head_len);
            let Some(token) = self.peek_token() else {
                return false;
            };
            if token.typ != TokenType::BlockEntry && token.typ != TokenType::BlockEnd {
                self.states.push(ParserState::BlockSequenceEntry);
                return self.parse_node(event, true, false);
            } else {
                self.state = ParserState::BlockSequenceEntry;
                return self.process_empty_scalar(event, mark);
            }
        }
        if token.typ == TokenType::BlockEnd {
            self.pop_state();
            self.marks.pop();

            *event = Event {
                typ: EventType::SequenceEnd,
                start_mark: token.start_mark,
                end_mark: token.end_mark,
                ..Event::default()
            };

            self.skip_token();
            return true;
        }

        let context_mark = self.marks.pop().unwrap_or_default();
        self.set_parser_error_context(
            "while parsing a block collection",
            context_mark,
            "did not find expected '-' indicator",
            token.start_mark,
        )
    }

    // Parse the productions:
    //
    //	indentless_sequence  ::= (BLOCK-ENTRY block_node?)+
    //	                          *********** *
    fn parse_indentless_sequence_entry(&mut self, event: &mut Event) -> bool {
        let Some(token) = self.peek_token() else {
            return false;
        };

        if token.typ == TokenType::BlockEntry {
            let mark = token.end_mark;
            let prior_head_len = self.head_comment.len();
            self.skip_token();
            self.split_stem_comment(prior_head_len);
            let Some(token) = self.peek_token() else {
                return false;
            };
            if token.typ != TokenType::BlockEntry
                && token.typ != TokenType::Key
                && token.typ != TokenType::Value
                && token.typ != TokenType::BlockEnd
            {
                self.states.push(ParserState::IndentlessSequenceEntry);
                return self.parse_node(event, true, false);
            }
            self.state = ParserState::IndentlessSequenceEntry;
            return self.process_empty_scalar(event, mark);
        }
        self.pop_state();

        *event = Event {
            typ: EventType::SequenceEnd,
            start_mark: token.start_mark,
            end_mark: token.start_mark, // [Go] Shouldn't this be token.end_mark?
            ..Event::default()
        };
        true
    }

    // Split stem comment from head comment.
    //
    // When a sequence or map is found under a sequence entry, the former head comment
    // is assigned to the underlying sequence or map as a whole, not the individual
    // sequence or map entry as would be expected otherwise. To handle this case the
    // previous head comment is moved aside as the stem comment.
    fn split_stem_comment(&mut self, stem_len: usize) {
        if stem_len == 0 {
            return;
        }

        let Some(token) = self.peek_token() else {
            return;
        };
        if token.typ != TokenType::BlockSequenceStart && token.typ != TokenType::BlockMappingStart {
            return;
        }

        self.stem_comment = self.head_comment[..stem_len].to_vec();
        if self.head_comment.len() == stem_len {
            self.head_comment = Vec::new();
        } else {
            // Copy suffix to prevent very strange bugs if someone ever appends
            // further bytes to the prefix in the stem_comment slice above.
            self.head_comment = self.head_comment[stem_len + 1..].to_vec();
        }
    }

    // Parse the productions:
    //
    //	block_mapping        ::= BLOCK-MAPPING_START
    //	                         *******************
    //	                         ((KEY block_node_or_indentless_sequence?)?
    //	                           *** *
    //	                         (VALUE block_node_or_indentless_sequence?)?)*
    //
    //	                         BLOCK-END
    //	                         *********
    fn parse_block_mapping_key(&mut self, event: &mut Event, first: bool) -> bool {
        if first {
            let Some(token) = self.peek_token() else {
                return false;
            };
            self.marks.push(token.start_mark);
            self.skip_token();
        }

        let Some(token) = self.peek_token() else {
            return false;
        };

        // [Go] A tail comment was left from the prior mapping value processed. Emit an event
        //      as it needs to be processed with that value and not the following key.
        if !self.tail_comment.is_empty() {
            *event = Event {
                typ: EventType::TailComment,
                start_mark: token.start_mark,
                end_mark: token.end_mark,
                foot_comment: std::mem::take(&mut self.tail_comment),
                ..Event::default()
            };
            return true;
        }

        if token.typ == TokenType::Key {
            let mark = token.end_mark;
            self.skip_token();
            let Some(token) = self.peek_token() else {
                return false;
            };
            if token.typ != TokenType::Key && token.typ != TokenType::Value && token.typ != TokenType::BlockEnd {
                self.states.push(ParserState::BlockMappingValue);
                return self.parse_node(event, true, true);
            } else {
                self.state = ParserState::BlockMappingValue;
                return self.process_empty_scalar(event, mark);
            }
        } else if token.typ == TokenType::BlockEnd {
            self.pop_state();
            self.marks.pop();
            *event = Event {
                typ: EventType::MappingEnd,
                start_mark: token.start_mark,
                end_mark: token.end_mark,
                ..Event::default()
            };
            self.set_event_comments(event);
            self.skip_token();
            return true;
        }

        let context_mark = self.marks.pop().unwrap_or_default();
        self.set_parser_error_context(
            "while parsing a block mapping",
            context_mark,
            "did not find expected key",
            token.start_mark,
        )
    }

    // Parse the productions:
    //
    //	block_mapping        ::= BLOCK-MAPPING_START
    //
    //	                          ((KEY block_node_or_indentless_sequence?)?
    //
    //	                          (VALUE block_node_or_indentless_sequence?)?)*
    //	                           ***** *
    //	                          BLOCK-END
    fn parse_block_mapping_value(&mut self, event: &mut Event) -> bool {
        let Some(token) = self.peek_token() else {
            return false;
        };
        if token.typ == TokenType::Value {
            let mark = token.end_mark;
            self.skip_token();
            let Some(token) = self.peek_token() else {
                return false;
            };
            if token.typ != TokenType::Key && token.typ != TokenType::Value && token.typ != TokenType::BlockEnd {
                self.states.push(ParserState::BlockMappingKey);
                return self.parse_node(event, true, true);
            }
            self.state = ParserState::BlockMappingKey;
            return self.process_empty_scalar(event, mark);
        }
        self.state = ParserState::BlockMappingKey;
        self.process_empty_scalar(event, token.start_mark)
    }

    // Parse the productions:
    //
    //	flow_sequence        ::= FLOW-SEQUENCE-START
    //	                         *******************
    //	                         (flow_sequence_entry FLOW-ENTRY)*
    //	                          *                   **********
    //	                         flow_sequence_entry?
    //	                         *
    //	                         FLOW-SEQUENCE-END
    //	                         *****************
    //	flow_sequence_entry  ::= flow_node | KEY flow_node? (VALUE flow_node?)?
    //	                         *
    fn parse_flow_sequence_entry(&mut self, event: &mut Event, first: bool) -> bool {
        if first {
            let Some(token) = self.peek_token() else {
                return false;
            };
            self.marks.push(token.start_mark);
            self.skip_token();
        }
        let Some(mut token) = self.peek_token() else {
            return false;
        };
        if token.typ != TokenType::FlowSequenceEnd {
            if !first {
                if token.typ == TokenType::FlowEntry {
                    self.skip_token();
                    let Some(t) = self.peek_token() else {
                        return false;
                    };
                    token = t;
                } else {
                    let context_mark = self.marks.pop().unwrap_or_default();
                    return self.set_parser_error_context(
                        "while parsing a flow sequence",
                        context_mark,
                        "did not find expected ',' or ']'",
                        token.start_mark,
                    );
                }
            }

            if token.typ == TokenType::Key {
                self.state = ParserState::FlowSequenceEntryMappingKey;
                *event = Event {
                    typ: EventType::MappingStart,
                    start_mark: token.start_mark,
                    end_mark: token.end_mark,
                    implicit: true,
                    style: FLOW_MAPPING_STYLE,
                    ..Event::default()
                };
                self.skip_token();
                return true;
            } else if token.typ != TokenType::FlowSequenceEnd {
                self.states.push(ParserState::FlowSequenceEntry);
                return self.parse_node(event, false, false);
            }
        }

        self.pop_state();
        self.marks.pop();

        *event = Event {
            typ: EventType::SequenceEnd,
            start_mark: token.start_mark,
            end_mark: token.end_mark,
            ..Event::default()
        };
        self.set_event_comments(event);

        self.skip_token();
        true
    }

    // Parse the productions:
    //
    //	flow_sequence_entry  ::= flow_node | KEY flow_node? (VALUE flow_node?)?
    //	                                     *** *
    fn parse_flow_sequence_entry_mapping_key(&mut self, event: &mut Event) -> bool {
        let Some(token) = self.peek_token() else {
            return false;
        };
        if token.typ != TokenType::Value && token.typ != TokenType::FlowEntry && token.typ != TokenType::FlowSequenceEnd
        {
            self.states.push(ParserState::FlowSequenceEntryMappingValue);
            return self.parse_node(event, false, false);
        }
        let mark = token.end_mark;
        self.skip_token();
        self.state = ParserState::FlowSequenceEntryMappingValue;
        self.process_empty_scalar(event, mark)
    }

    // Parse the productions:
    //
    //	flow_sequence_entry  ::= flow_node | KEY flow_node? (VALUE flow_node?)?
    //	                                                     ***** *
    fn parse_flow_sequence_entry_mapping_value(&mut self, event: &mut Event) -> bool {
        let Some(token) = self.peek_token() else {
            return false;
        };
        if token.typ == TokenType::Value {
            self.skip_token();
            let Some(token) = self.peek_token() else {
                return false;
            };
            if token.typ != TokenType::FlowEntry && token.typ != TokenType::FlowSequenceEnd {
                self.states.push(ParserState::FlowSequenceEntryMappingEnd);
                return self.parse_node(event, false, false);
            }
        }
        self.state = ParserState::FlowSequenceEntryMappingEnd;
        // [Go] the token here is the VALUE one, the inner token shadowing it
        self.process_empty_scalar(event, token.start_mark)
    }

    // Parse the productions:
    //
    //	flow_sequence_entry  ::= flow_node | KEY flow_node? (VALUE flow_node?)?
    //	                                                                     *
    fn parse_flow_sequence_entry_mapping_end(&mut self, event: &mut Event) -> bool {
        let Some(token) = self.peek_token() else {
            return false;
        };
        self.state = ParserState::FlowSequenceEntry;
        *event = Event {
            typ: EventType::MappingEnd,
            start_mark: token.start_mark,
            end_mark: token.start_mark, // [Go] Shouldn't this be end_mark?
            ..Event::default()
        };
        true
    }

    // Parse the productions:
    //
    //	flow_mapping         ::= FLOW-MAPPING-START
    //	                         ******************
    //	                         (flow_mapping_entry FLOW-ENTRY)*
    //	                          *                  **********
    //	                         flow_mapping_entry?
    //	                         ******************
    //	                         FLOW-MAPPING-END
    //	                         ****************
    //	flow_mapping_entry   ::= flow_node | KEY flow_node? (VALUE flow_node?)?
    //	                         *           *** *
    fn parse_flow_mapping_key(&mut self, event: &mut Event, first: bool) -> bool {
        if first {
            let token = self.peek_token().unwrap_or_default();
            self.marks.push(token.start_mark);
            self.skip_token();
        }

        let Some(mut token) = self.peek_token() else {
            return false;
        };

        if token.typ != TokenType::FlowMappingEnd {
            if !first {
                if token.typ == TokenType::FlowEntry {
                    self.skip_token();
                    let Some(t) = self.peek_token() else {
                        return false;
                    };
                    token = t;
                } else {
                    let context_mark = self.marks.pop().unwrap_or_default();
                    return self.set_parser_error_context(
                        "while parsing a flow mapping",
                        context_mark,
                        "did not find expected ',' or '}'",
                        token.start_mark,
                    );
                }
            }

            if token.typ == TokenType::Key {
                self.skip_token();
                let Some(token) = self.peek_token() else {
                    return false;
                };
                if token.typ != TokenType::Value
                    && token.typ != TokenType::FlowEntry
                    && token.typ != TokenType::FlowMappingEnd
                {
                    self.states.push(ParserState::FlowMappingValue);
                    return self.parse_node(event, false, false);
                } else {
                    self.state = ParserState::FlowMappingValue;
                    return self.process_empty_scalar(event, token.start_mark);
                }
            } else if token.typ != TokenType::FlowMappingEnd {
                self.states.push(ParserState::FlowMappingEmptyValue);
                return self.parse_node(event, false, false);
            }
        }

        self.pop_state();
        self.marks.pop();
        *event = Event {
            typ: EventType::MappingEnd,
            start_mark: token.start_mark,
            end_mark: token.end_mark,
            ..Event::default()
        };
        self.set_event_comments(event);
        self.skip_token();
        true
    }

    // Parse the productions:
    //
    //	flow_mapping_entry   ::= flow_node | KEY flow_node? (VALUE flow_node?)?
    //	                                  *                  ***** *
    fn parse_flow_mapping_value(&mut self, event: &mut Event, empty: bool) -> bool {
        let Some(mut token) = self.peek_token() else {
            return false;
        };
        if empty {
            self.state = ParserState::FlowMappingKey;
            return self.process_empty_scalar(event, token.start_mark);
        }
        if token.typ == TokenType::Value {
            self.skip_token();
            let Some(t) = self.peek_token() else {
                return false;
            };
            token = t;
            if token.typ != TokenType::FlowEntry && token.typ != TokenType::FlowMappingEnd {
                self.states.push(ParserState::FlowMappingKey);
                return self.parse_node(event, false, false);
            }
        }
        self.state = ParserState::FlowMappingKey;
        self.process_empty_scalar(event, token.start_mark)
    }

    // Generate an empty scalar event.
    fn process_empty_scalar(&mut self, event: &mut Event, mark: Mark) -> bool {
        *event = Event {
            typ: EventType::Scalar,
            start_mark: mark,
            end_mark: mark,
            value: Vec::new(), // Empty
            implicit: true,
            style: PLAIN_SCALAR_STYLE,
            ..Event::default()
        };
        true
    }

    // Parse directives.
    fn process_directives(
        &mut self,
        version_directive_ref: Option<&mut Option<VersionDirective>>,
        tag_directives_ref: Option<&mut Vec<TagDirective>>,
    ) -> bool {
        let mut version_directive: Option<VersionDirective> = None;
        let mut tag_directives: Vec<TagDirective> = Vec::new();

        let Some(mut token) = self.peek_token() else {
            return false;
        };

        while token.typ == TokenType::VersionDirective || token.typ == TokenType::TagDirective {
            if token.typ == TokenType::VersionDirective {
                if version_directive.is_some() {
                    self.set_parser_error("found duplicate %YAML directive", token.start_mark);
                    return false;
                }
                if token.major != 1 || token.minor != 1 {
                    self.set_parser_error("found incompatible YAML document", token.start_mark);
                    return false;
                }
                version_directive = Some(VersionDirective { major: token.major, minor: token.minor });
            } else if token.typ == TokenType::TagDirective {
                let value = TagDirective { handle: token.value.clone(), prefix: token.prefix.clone() };
                if !self.append_tag_directive(value.clone(), false, token.start_mark) {
                    return false;
                }
                tag_directives.push(value);
            }

            self.skip_token();
            let Some(t) = self.peek_token() else {
                return false;
            };
            token = t;
        }

        for td in default_tag_directives() {
            if !self.append_tag_directive(td, true, token.start_mark) {
                return false;
            }
        }

        if let Some(r) = version_directive_ref {
            *r = version_directive;
        }
        if let Some(r) = tag_directives_ref {
            *r = tag_directives;
        }
        true
    }

    // Append a tag directive to the directives stack.
    fn append_tag_directive(&mut self, value: TagDirective, allow_duplicates: bool, mark: Mark) -> bool {
        for td in &self.tag_directives {
            if value.handle == td.handle {
                if allow_duplicates {
                    return true;
                }
                return self.set_parser_error("found duplicate %TAG directive", mark);
            }
        }

        // [Go] I suspect the copy is unnecessary. This was likely done
        // because there was no way to track ownership of the data.
        self.tag_directives.push(value);
        true
    }
}
