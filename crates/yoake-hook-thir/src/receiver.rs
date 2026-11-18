use crate::model::{Borrow, ReceiverAdjustment};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NodeKind {
    #[default]
    Other,
    Scope,
    Borrow,
    Deref,
    Call,
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub kind: NodeKind,
    pub borrow: Borrow,
    pub children: Vec<usize>,
    pub call_args: Vec<usize>,
    pub from_hir_call: bool,
    pub callee: Option<String>,
}

impl Default for Borrow {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Default)]
pub struct ReceiverTree {
    pub nodes: Vec<Node>,
}

impl ReceiverTree {
    pub fn node(&mut self) -> usize {
        self.nodes.push(Node {
            from_hir_call: true,
            ..Node::default()
        });
        self.nodes.len() - 1
    }

    pub fn child(&mut self, parent: usize, child: usize) {
        self.nodes[parent].children.push(child);
    }

    pub fn adjustment(&self, root: Option<usize>) -> ReceiverAdjustment {
        let mut result = ReceiverAdjustment {
            borrow: Borrow::None,
            derefs: 0,
        };
        let mut current = root;
        let mut through_deref = false;
        while let Some(index) = current {
            let node = &self.nodes[index];
            match node.kind {
                NodeKind::Scope => {
                    current = node.children.first().copied();
                }
                NodeKind::Borrow => {
                    if result.borrow == Borrow::None {
                        result.borrow = node.borrow;
                    }
                    through_deref = false;
                    current = node.children.first().copied();
                }
                NodeKind::Deref => {
                    result.derefs += 1;
                    through_deref = true;
                    current = node.children.first().copied();
                }
                NodeKind::Call
                    if !node.from_hir_call
                        && node.callee.as_deref().is_some_and(is_overloaded_deref) =>
                {
                    if !through_deref {
                        result.derefs += 1;
                    }
                    through_deref = false;
                    current = node.call_args.first().copied();
                }
                _ => break,
            }
        }
        result
    }
}

fn is_overloaded_deref(path: &str) -> bool {
    path.ends_with("::Deref::deref") || path.ends_with("::DerefMut::deref_mut")
}
