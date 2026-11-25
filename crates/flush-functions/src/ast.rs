//! AST for function-mode scripts (mirrors `runtime/ast.ts`).

/// Plain text node between `$` calls.
#[derive(Debug, Clone, PartialEq)]
pub struct TextNode {
    pub value: String,
    /// Char offset inside `Program::source`.
    pub offset: usize,
}

/// `$name[arg;arg]` call node. `args` holds one node list per `;`-separated
/// argument (still unevaluated — evaluation is lazy).
#[derive(Debug, Clone, PartialEq)]
pub struct CallNode {
    pub name: String,
    pub args: Vec<Vec<Node>>,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Text(TextNode),
    Call(CallNode),
}

impl Node {
    pub fn offset(&self) -> usize {
        match self {
            Node::Text(n) => n.offset,
            Node::Call(n) => n.offset,
        }
    }
}

/// A compiled script.
#[derive(Debug, Clone)]
pub struct Program {
    pub source: String,
    pub file: String,
    pub nodes: Vec<Node>,
}
