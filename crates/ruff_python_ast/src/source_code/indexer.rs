//! Struct used to index source code, to enable efficient lookup of tokens that
//! are omitted from the AST (e.g., commented lines).

use crate::source_code::Locator;
use ruff_text_size::{TextRange, TextSize};
use rustpython_parser::lexer::LexResult;
use rustpython_parser::Tok;

pub struct Indexer {
    /// Stores the ranges of comments sorted by their range in increasing order.
    comments: Vec<TextRange>,
    /// Stores the start offset of continuation lines.
    continuation_lines: Vec<TextSize>,
    string_ranges: Vec<TextRange>,
}

impl Indexer {
    pub fn from_tokens(tokens: &[LexResult], locator: &Locator) -> Self {
        let mut commented_lines = Vec::new();
        let mut continuation_lines = Vec::new();
        let mut string_ranges = Vec::new();
        // Token, end
        let mut prev_end = TextSize::default();
        let mut prev_token: Option<&Tok> = None;
        let mut line_start = TextSize::default();

        for (start, tok, end) in tokens.iter().flatten() {
            let trivia = &locator.contents()[TextRange::new(prev_end, *start)];

            for (index, text) in trivia.match_indices(['\n', '\r']) {
                if text == "\r" && trivia.as_bytes().get(index + 1) == Some(&b'\n') {
                    continue;
                }

                if let Some(prev_token) = prev_token {
                    if !matches!(
                        prev_token,
                        Tok::Newline | Tok::NonLogicalNewline | Tok::Comment(..),
                    ) {
                        continuation_lines.push(line_start);
                    }
                }

                line_start = prev_end + TextSize::try_from(index + 1).unwrap();
            }

            match tok {
                Tok::Comment(..) => {
                    commented_lines.push(TextRange::new(*start, *end));
                }
                Tok::Newline | Tok::NonLogicalNewline => {
                    line_start = *end;
                }
                Tok::String {
                    triple_quoted: true,
                    ..
                } => string_ranges.push(TextRange::new(*start, *end)),
                _ => {}
            }

            prev_token = Some(tok);
            prev_end = *end;
        }
        Self {
            comments: commented_lines,
            continuation_lines,
            string_ranges,
        }
    }

    /// Returns the byte offset ranges of comments
    pub fn comment_ranges(&self) -> &[TextRange] {
        &self.comments
    }

    /// Returns the line start positions of continuations (backslash).
    pub fn continuation_line_starts(&self) -> &[TextSize] {
        &self.continuation_lines
    }

    /// Return a slice of all ranges that include a triple-quoted string.
    pub fn string_ranges(&self) -> &[TextRange] {
        &self.string_ranges
    }

    pub fn is_continuation(&self, offset: TextSize, locator: &Locator) -> bool {
        let line_start = locator.line_start(offset);
        self.continuation_lines.binary_search(&line_start).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use ruff_text_size::{TextRange, TextSize};
    use rustpython_parser::lexer::LexResult;
    use rustpython_parser::{lexer, Mode};

    use crate::source_code::{Indexer, Locator};

    #[test]
    fn continuation() {
        let contents = r#"x = 1"#;
        let lxr: Vec<LexResult> = lexer::lex(contents, Mode::Module).collect();
        let indexer = Indexer::from_tokens(&lxr, &Locator::new(contents));
        assert_eq!(indexer.continuation_line_starts(), &[]);

        let contents = r#"
        # Hello, world!

        x = 1

        y = 2
        "#
        .trim();

        let lxr: Vec<LexResult> = lexer::lex(contents, Mode::Module).collect();
        let indexer = Indexer::from_tokens(&lxr, &Locator::new(contents));
        assert_eq!(indexer.continuation_line_starts(), &[]);

        let contents = r#"
x = \
    1

if True:
    z = \
        \
        2

(
    "abc" # Foo
    "def" \
    "ghi"
)
"#
        .trim();
        let lxr: Vec<LexResult> = lexer::lex(contents, Mode::Module).collect();
        let indexer = Indexer::from_tokens(&lxr.as_slice(), &Locator::new(contents));
        assert_eq!(
            indexer.continuation_line_starts(),
            [
                // row 1
                TextSize::from(0),
                // row 5
                TextSize::from(22),
                // row 6
                TextSize::from(32),
                // row 11
                TextSize::from(71),
            ]
        );

        let contents = r#"
x = 1; import sys
import os

if True:
    x = 1; import sys
    import os

if True:
    x = 1; \
        import os

x = 1; \
import os
"#
        .trim();
        let lxr: Vec<LexResult> = lexer::lex(contents, Mode::Module).collect();
        let indexer = Indexer::from_tokens(&lxr.as_slice(), &Locator::new(contents));
        assert_eq!(
            indexer.continuation_line_starts(),
            [
                // row 9
                TextSize::from(84),
                // row 12
                TextSize::from(116)
            ]
        );
    }

    #[test]
    fn string_ranges() {
        let contents = r#""this is a single-quoted string""#;
        let lxr: Vec<LexResult> = lexer::lex(contents, Mode::Module).collect();
        let indexer = Indexer::from_tokens(&lxr.as_slice(), &Locator::new(contents));
        assert_eq!(indexer.string_ranges(), []);

        let contents = r#"
            """
            this is a multiline string
            """
            "#;
        let lxr: Vec<LexResult> = lexer::lex(contents, Mode::Module).collect();
        let indexer = Indexer::from_tokens(&lxr.as_slice(), &Locator::new(contents));
        assert_eq!(
            indexer.string_ranges(),
            [TextRange::new(TextSize::from(13), TextSize::from(71))]
        );

        let contents = r#"
            """
            '''this is a multiline string with multiple delimiter types'''
            """
            "#;
        let lxr: Vec<LexResult> = lexer::lex(contents, Mode::Module).collect();
        let indexer = Indexer::from_tokens(&lxr.as_slice(), &Locator::new(contents));
        assert_eq!(
            indexer.string_ranges(),
            [TextRange::new(TextSize::from(13), TextSize::from(107))]
        );

        let contents = r#"
            """
            this is one
            multiline string
            """
            """
            and this is
            another
            """
            "#;
        let lxr: Vec<LexResult> = lexer::lex(contents, Mode::Module).collect();
        let indexer = Indexer::from_tokens(&lxr.as_slice(), &Locator::new(contents));
        assert_eq!(
            indexer.string_ranges(),
            &[
                TextRange::new(TextSize::from(13), TextSize::from(85)),
                TextRange::new(TextSize::from(98), TextSize::from(161))
            ]
        );
    }
}
