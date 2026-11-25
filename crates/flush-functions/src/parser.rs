//! Script parser (port of `runtime/parser.ts`).
//!
//! Grammar: text interleaved with `$name[arg;arg;...]` calls. `\` escapes
//! `\`, `$`, `;`, `[` and `]`. Inside an argument, a literal `[...]` pair is
//! skipped. Nesting is limited to 64 levels and sources to 100_000 chars.

use crate::ast::{CallNode, Node, Program, TextNode};
use crate::errors::ScriptError;

const SOURCE_LIMIT: usize = 100_000;
const DEPTH_LIMIT: usize = 64;

fn name_start(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_lowercase() || c == '_'
}

/// Parse `source` into a `Program`. `file` names the script in errors.
pub fn parse(source: &str, file: &str) -> Result<Program, ScriptError> {
    let chars: Vec<char> = source.chars().collect();
    if chars.len() > SOURCE_LIMIT {
        return Err(ScriptError::new(
            "SOURCE_LIMIT",
            "Script exceeds 100000 characters.",
            source,
            0,
            file,
        ));
    }
    let mut p = Parser {
        chars: &chars,
        source,
        file,
        pos: 0,
    };
    let nodes = p.sequence(false, 0)?;
    Ok(Program {
        source: source.to_string(),
        file: file.to_string(),
        nodes,
    })
}

struct Parser<'a> {
    chars: &'a [char],
    source: &'a str,
    file: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn fail<T>(&self, message: impl Into<String>, offset: usize) -> Result<T, ScriptError> {
        Err(ScriptError::new(
            "SYNTAX",
            message,
            self.source,
            offset,
            self.file,
        ))
    }

    fn at(&self, index: usize) -> Option<char> {
        self.chars.get(index).copied()
    }

    fn sequence(&mut self, argument: bool, depth: usize) -> Result<Vec<Node>, ScriptError> {
        if depth > DEPTH_LIMIT {
            return self.fail("Nesting exceeds 64 levels.", self.pos);
        }
        let mut nodes: Vec<Node> = Vec::new();
        let mut value = String::new();
        // `chunk` marks where scanning resumes; `node_start` is the
        // TextNode offset (start of the text run, before any escapes).
        let mut chunk = self.pos;
        let mut node_start = self.pos;

        macro_rules! flush {
            () => {
                if value.is_empty() {
                    node_start = chunk;
                }
                value.push_str(&self.chars[chunk..self.pos].iter().collect::<String>());
                if !value.is_empty() {
                    nodes.push(Node::Text(TextNode {
                        value: std::mem::take(&mut value),
                        offset: node_start,
                    }));
                }
                chunk = self.pos;
                let _ = chunk;
            };
        }

        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if argument && (c == ';' || c == ']') {
                break;
            }
            if !argument && c == ']' {
                return self.fail("Unexpected closing bracket.", self.pos);
            }
            if c == '\\' && matches!(self.at(self.pos + 1), Some('\\' | '$' | ';' | '[' | ']')) {
                if value.is_empty() {
                    node_start = chunk;
                }
                value.push_str(&self.chars[chunk..self.pos].iter().collect::<String>());
                value.push(self.chars[self.pos + 1]);
                self.pos += 2;
                chunk = self.pos;
                continue;
            }
            if c == '$' && self.at(self.pos + 1).is_some_and(name_start) {
                flush!();
                let offset = self.pos;
                self.pos += 1;
                let name_offset = self.pos;
                self.pos += 1;
                while self
                    .at(self.pos)
                    .is_some_and(|c| name_start(c) || c.is_ascii_digit())
                {
                    self.pos += 1;
                }
                let name: String = self.chars[name_offset..self.pos].iter().collect();
                let mut args: Vec<Vec<Node>> = Vec::new();
                if self.at(self.pos) == Some('[') {
                    self.pos += 1;
                    if self.at(self.pos) == Some(']') {
                        self.pos += 1;
                    } else {
                        loop {
                            args.push(self.sequence(true, depth + 1)?);
                            match self.at(self.pos) {
                                Some(']') => {
                                    self.pos += 1;
                                    break;
                                }
                                Some(';') => {
                                    self.pos += 1;
                                }
                                _ => return self.fail(format!("Missing ] for ${name}."), offset),
                            }
                        }
                    }
                }
                nodes.push(Node::Call(CallNode { name, args, offset }));
                chunk = self.pos;
            } else if c == '[' {
                if !argument {
                    return self.fail("Literal bracket outside an argument.", self.pos);
                }
                let mut level = 0i32;
                let mut end = self.pos;
                while end < self.chars.len() {
                    let current = self.chars[end];
                    if current == '\\' {
                        end += 1;
                    } else if current == '[' {
                        level += 1;
                    } else if current == ']' {
                        level -= 1;
                        if level == 0 {
                            break;
                        }
                    }
                    end += 1;
                }
                if level != 0 {
                    return self.fail("Unclosed literal bracket.", self.pos);
                }
                self.pos = end + 1;
            } else {
                self.pos += 1;
                if self.pos - chunk >= 16 {
                    // Fast-forward to the next delimiter, like the JS regex scan.
                    while self.pos < self.chars.len()
                        && !matches!(self.chars[self.pos], '\\' | '$' | '[' | ']' | ';')
                    {
                        self.pos += 1;
                    }
                }
            }
        }
        flush!();
        let _ = chunk;
        Ok(nodes)
    }
}
