use std::collections::BTreeMap;
use std::fmt;

use crate::Value;

const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_bytes: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WireValue {
    Unit,
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Sequence(ValueTree),
    Object(ValueTree),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValueTree {
    pub root: u32,
    pub nodes: Vec<ValueNode>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecordEntry {
    pub key: String,
    pub node: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ValueNode {
    Unit,
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Sequence(Vec<u32>),
    Object(Vec<RecordEntry>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    NodeLimit,
    DepthLimit,
    ByteLimit,
    InvalidIndex,
    RepeatedNode,
    UnreachableNode,
    RootKind,
    DuplicateKey,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid injection value: {self:?}")
    }
}

impl std::error::Error for Error {}

struct Budget {
    limits: Limits,
    nodes: usize,
    bytes: usize,
}

impl Budget {
    fn new(limits: Limits) -> Self {
        Self {
            limits,
            nodes: 0,
            bytes: 0,
        }
    }

    fn room(&self, nodes: usize) -> Result<(), Error> {
        match self.nodes.checked_add(nodes) {
            Some(total) if total <= self.limits.max_nodes => Ok(()),
            _ => Err(Error::NodeLimit),
        }
    }

    fn visit(&mut self, depth: usize) -> Result<(), Error> {
        if depth > self.limits.max_depth || depth > MAX_DEPTH {
            return Err(Error::DepthLimit);
        }
        self.room(1)?;
        self.nodes += 1;
        Ok(())
    }

    fn bytes(&mut self, count: usize) -> Result<(), Error> {
        match self.bytes.checked_add(count) {
            Some(total) if total <= self.limits.max_bytes => {
                self.bytes = total;
                Ok(())
            }
            _ => Err(Error::ByteLimit),
        }
    }
}

pub struct Encoder {
    budget: Budget,
}

impl Encoder {
    pub fn new(limits: Limits) -> Self {
        Self {
            budget: Budget::new(limits),
        }
    }

    pub fn encode(&mut self, value: &Value) -> Result<WireValue, Error> {
        match value {
            Value::List(_) | Value::Record(_) => {
                let mut nodes = Vec::new();
                let root = encode_node(value, 1, &mut self.budget, &mut nodes)?;
                let tree = ValueTree { root, nodes };
                Ok(match value {
                    Value::List(_) => WireValue::Sequence(tree),
                    Value::Record(_) => WireValue::Object(tree),
                    _ => unreachable!(),
                })
            }
            _ => {
                self.budget.visit(1)?;
                Ok(match value {
                    Value::Unit => WireValue::Unit,
                    Value::Boolean(value) => WireValue::Boolean(*value),
                    Value::Signed(value) => WireValue::Signed(*value),
                    Value::Unsigned(value) => WireValue::Unsigned(*value),
                    Value::Float(value) => WireValue::Float(*value),
                    Value::Text(value) => {
                        self.budget.bytes(value.len())?;
                        WireValue::Text(value.clone())
                    }
                    Value::Bytes(value) => {
                        self.budget.bytes(value.len())?;
                        WireValue::Bytes(value.clone())
                    }
                    _ => unreachable!(),
                })
            }
        }
    }
}

fn encode_node(
    value: &Value,
    depth: usize,
    budget: &mut Budget,
    nodes: &mut Vec<ValueNode>,
) -> Result<u32, Error> {
    budget.visit(depth)?;
    let node = match value {
        Value::Unit => ValueNode::Unit,
        Value::Boolean(value) => ValueNode::Boolean(*value),
        Value::Signed(value) => ValueNode::Signed(*value),
        Value::Unsigned(value) => ValueNode::Unsigned(*value),
        Value::Float(value) => ValueNode::Float(*value),
        Value::Text(value) => {
            budget.bytes(value.len())?;
            ValueNode::Text(value.clone())
        }
        Value::Bytes(value) => {
            budget.bytes(value.len())?;
            ValueNode::Bytes(value.clone())
        }
        Value::List(values) => {
            budget.room(values.len())?;
            let mut children = Vec::with_capacity(values.len());
            for value in values {
                children.push(encode_node(value, depth + 1, budget, nodes)?);
            }
            ValueNode::Sequence(children)
        }
        Value::Record(values) => {
            budget.room(values.len())?;
            let mut children = Vec::with_capacity(values.len());
            for (key, value) in values {
                budget.bytes(key.len())?;
                let node = encode_node(value, depth + 1, budget, nodes)?;
                children.push(RecordEntry {
                    key: key.clone(),
                    node,
                });
            }
            ValueNode::Object(children)
        }
    };
    let index = u32::try_from(nodes.len()).map_err(|_| Error::NodeLimit)?;
    nodes.push(node);
    Ok(index)
}

pub struct Decoder {
    budget: Budget,
}

impl Decoder {
    pub fn new(limits: Limits) -> Self {
        Self {
            budget: Budget::new(limits),
        }
    }

    pub fn decode(&mut self, value: &WireValue) -> Result<Value, Error> {
        match value {
            WireValue::Sequence(tree) | WireValue::Object(tree) => {
                self.budget.room(tree.nodes.len())?;
                let root = tree.nodes.get(tree.root as usize).ok_or(Error::InvalidIndex)?;
                if !matches!(
                    (value, root),
                    (WireValue::Sequence(_), ValueNode::Sequence(_))
                        | (WireValue::Object(_), ValueNode::Object(_))
                ) {
                    return Err(Error::RootKind);
                }
                let mut seen = vec![false; tree.nodes.len()];
                let result = decode_node(tree, tree.root, 1, &mut self.budget, &mut seen)?;
                if seen.iter().any(|visited| !visited) {
                    return Err(Error::UnreachableNode);
                }
                Ok(result)
            }
            _ => {
                self.budget.visit(1)?;
                Ok(match value {
                    WireValue::Unit => Value::Unit,
                    WireValue::Boolean(value) => Value::Boolean(*value),
                    WireValue::Signed(value) => Value::Signed(*value),
                    WireValue::Unsigned(value) => Value::Unsigned(*value),
                    WireValue::Float(value) => Value::Float(*value),
                    WireValue::Text(value) => {
                        self.budget.bytes(value.len())?;
                        Value::Text(value.clone())
                    }
                    WireValue::Bytes(value) => {
                        self.budget.bytes(value.len())?;
                        Value::Bytes(value.clone())
                    }
                    _ => unreachable!(),
                })
            }
        }
    }
}

fn decode_node(
    tree: &ValueTree,
    index: u32,
    depth: usize,
    budget: &mut Budget,
    seen: &mut [bool],
) -> Result<Value, Error> {
    let index = index as usize;
    let node = tree.nodes.get(index).ok_or(Error::InvalidIndex)?;
    if seen[index] {
        return Err(Error::RepeatedNode);
    }
    budget.visit(depth)?;
    seen[index] = true;
    Ok(match node {
        ValueNode::Unit => Value::Unit,
        ValueNode::Boolean(value) => Value::Boolean(*value),
        ValueNode::Signed(value) => Value::Signed(*value),
        ValueNode::Unsigned(value) => Value::Unsigned(*value),
        ValueNode::Float(value) => Value::Float(*value),
        ValueNode::Text(value) => {
            budget.bytes(value.len())?;
            Value::Text(value.clone())
        }
        ValueNode::Bytes(value) => {
            budget.bytes(value.len())?;
            Value::Bytes(value.clone())
        }
        ValueNode::Sequence(children) => {
            budget.room(children.len())?;
            let mut values = Vec::with_capacity(children.len());
            for child in children {
                values.push(decode_node(tree, *child, depth + 1, budget, seen)?);
            }
            Value::List(values)
        }
        ValueNode::Object(children) => {
            budget.room(children.len())?;
            let mut values = BTreeMap::new();
            for child in children {
                budget.bytes(child.key.len())?;
                let value = decode_node(tree, child.node, depth + 1, budget, seen)?;
                if values.insert(child.key.clone(), value).is_some() {
                    return Err(Error::DuplicateKey);
                }
            }
            Value::Record(values)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits {
            max_nodes: 32,
            max_depth: 8,
            max_bytes: 64,
        }
    }

    #[test]
    fn values_round_trip() {
        let values = [
            Value::Unit,
            Value::Boolean(true),
            Value::Signed(-3),
            Value::Unsigned(7),
            Value::Float(1.5),
            Value::Text("text".to_owned()),
            Value::Bytes(vec![1, 2]),
            Value::List(vec![Value::Record(BTreeMap::from([(
                "key".to_owned(),
                Value::List(vec![Value::Unsigned(42)]),
            )]))]),
            Value::Record(BTreeMap::new()),
        ];
        let mut encoder = Encoder::new(limits());
        let mut decoder = Decoder::new(limits());
        for value in values {
            let wire = encoder.encode(&value).unwrap();
            assert_eq!(decoder.decode(&wire).unwrap(), value);
        }
    }

    #[test]
    fn rejects_invalid_or_non_tree_graphs() {
        let cases = [
            (vec![ValueNode::Sequence(vec![1])], 0, Error::InvalidIndex),
            (vec![ValueNode::Sequence(vec![0])], 0, Error::RepeatedNode),
            (
                vec![ValueNode::Sequence(vec![1, 1]), ValueNode::Unit],
                0,
                Error::RepeatedNode,
            ),
            (
                vec![ValueNode::Sequence(vec![]), ValueNode::Unit],
                0,
                Error::UnreachableNode,
            ),
            (vec![ValueNode::Unit], 0, Error::RootKind),
        ];
        for (nodes, root, error) in cases {
            let wire = WireValue::Sequence(ValueTree { root, nodes });
            assert_eq!(Decoder::new(limits()).decode(&wire), Err(error));
        }
        let duplicate = WireValue::Object(ValueTree {
            root: 0,
            nodes: vec![
                ValueNode::Object(vec![
                    RecordEntry {
                        key: "x".to_owned(),
                        node: 1,
                    },
                    RecordEntry {
                        key: "x".to_owned(),
                        node: 2,
                    },
                ]),
                ValueNode::Unit,
                ValueNode::Unit,
            ],
        });
        assert_eq!(
            Decoder::new(limits()).decode(&duplicate),
            Err(Error::DuplicateKey)
        );
    }

    #[test]
    fn enforces_cumulative_limits_in_both_directions() {
        let limits = Limits {
            max_nodes: 3,
            max_depth: 2,
            max_bytes: 3,
        };
        let mut encoder = Encoder::new(limits);
        assert!(encoder.encode(&Value::Text("ab".to_owned())).is_ok());
        assert_eq!(
            encoder.encode(&Value::Text("cd".to_owned())),
            Err(Error::ByteLimit)
        );

        let mut decoder = Decoder::new(limits);
        assert!(decoder.decode(&WireValue::Text("ab".to_owned())).is_ok());
        assert_eq!(
            decoder.decode(&WireValue::Text("cd".to_owned())),
            Err(Error::ByteLimit)
        );

        let mut encoder = Encoder::new(limits);
        assert!(encoder.encode(&Value::Unit).is_ok());
        assert!(encoder.encode(&Value::Boolean(true)).is_ok());
        assert!(encoder.encode(&Value::Signed(1)).is_ok());
        assert_eq!(encoder.encode(&Value::Unit), Err(Error::NodeLimit));

        let too_deep = Value::List(vec![Value::List(vec![Value::Unit])]);
        assert_eq!(
            Encoder::new(limits).encode(&too_deep),
            Err(Error::DepthLimit)
        );
        let tree = WireValue::Sequence(ValueTree {
            root: 0,
            nodes: vec![
                ValueNode::Sequence(vec![1]),
                ValueNode::Sequence(vec![2]),
                ValueNode::Unit,
            ],
        });
        assert_eq!(Decoder::new(limits).decode(&tree), Err(Error::DepthLimit));
    }
}
