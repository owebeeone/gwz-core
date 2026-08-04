use serde_yaml::{Mapping, Value};
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::Marker;

use super::scalar::{parse_scalar, tagged_value};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StrictYamlDocument {
    root: Value,
}

impl StrictYamlDocument {
    pub(crate) fn root(&self) -> &Value {
        &self.root
    }

    pub(crate) fn into_root(self) -> Value {
        self.root
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StrictYamlErrorKind {
    InvalidEncoding,
    InvalidSyntax,
    MultipleDocuments,
    RootNotMapping,
    DuplicateKey,
    Anchor,
    Alias,
    MergeKey,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StrictYamlError {
    pub(crate) kind: StrictYamlErrorKind,
    pub(crate) line: Option<usize>,
    pub(crate) column: Option<usize>,
    pub(crate) detail: String,
}

impl StrictYamlError {
    fn at(kind: StrictYamlErrorKind, mark: Marker, detail: impl Into<String>) -> Self {
        Self {
            kind,
            line: Some(mark.line()),
            column: Some(mark.col()),
            detail: detail.into(),
        }
    }
}

pub(crate) fn parse_strict_yaml(bytes: &[u8]) -> Result<StrictYamlDocument, StrictYamlError> {
    let source = std::str::from_utf8(bytes).map_err(|error| StrictYamlError {
        kind: StrictYamlErrorKind::InvalidEncoding,
        line: None,
        column: None,
        detail: error.to_string(),
    })?;
    let mut builder = StrictTreeBuilder::default();
    let parse = Parser::new_from_str(source).load(&mut builder, true);
    if let Some(error) = builder.error {
        return Err(error);
    }
    if let Err(error) = parse {
        return Err(StrictYamlError::at(
            StrictYamlErrorKind::InvalidSyntax,
            *error.marker(),
            error.info(),
        ));
    }
    let root = builder.root.unwrap_or(Value::Null);
    if !matches!(root, Value::Mapping(_)) {
        return Err(match builder.document_mark.or(builder.last_mark) {
            Some(mark) => StrictYamlError::at(
                StrictYamlErrorKind::RootNotMapping,
                mark,
                "document root is not a mapping",
            ),
            None => StrictYamlError {
                kind: StrictYamlErrorKind::RootNotMapping,
                line: None,
                column: None,
                detail: "document root is not a mapping".to_owned(),
            },
        });
    }
    Ok(StrictYamlDocument { root })
}

#[derive(Default)]
struct StrictTreeBuilder {
    root: Option<Value>,
    stack: Vec<Frame>,
    documents: usize,
    document_mark: Option<Marker>,
    last_mark: Option<Marker>,
    error: Option<StrictYamlError>,
}

enum Frame {
    Sequence(Vec<Value>, Option<Tag>),
    Mapping(Mapping, Option<Node>, Option<Tag>),
}

struct Node {
    value: Value,
    merge_key: bool,
}

impl MarkedEventReceiver for StrictTreeBuilder {
    fn on_event(&mut self, event: Event, mark: Marker) {
        self.last_mark = Some(mark);
        if self.error.is_some() {
            return;
        }
        match event {
            Event::StreamStart | Event::StreamEnd | Event::Nothing | Event::DocumentEnd => {}
            Event::DocumentStart => {
                self.documents += 1;
                self.document_mark = Some(mark);
                if self.documents > 1 {
                    self.fail(
                        StrictYamlErrorKind::MultipleDocuments,
                        mark,
                        "multiple YAML documents are forbidden",
                    );
                }
            }
            Event::Alias(_) => self.fail(
                StrictYamlErrorKind::Alias,
                mark,
                "YAML aliases are forbidden",
            ),
            Event::Scalar(value, style, anchor, tag) => {
                if self.reject_anchor(anchor, mark) {
                    return;
                }
                match parse_scalar(value, style, tag) {
                    Ok(node) => self.insert(
                        Node {
                            value: node.value,
                            merge_key: node.merge_key,
                        },
                        mark,
                    ),
                    Err(detail) => self.fail(StrictYamlErrorKind::InvalidSyntax, mark, detail),
                }
            }
            Event::SequenceStart(anchor, tag) => {
                if !self.reject_anchor(anchor, mark) {
                    self.stack.push(Frame::Sequence(Vec::new(), tag));
                }
            }
            Event::SequenceEnd => match self.stack.pop() {
                Some(Frame::Sequence(values, tag)) => {
                    self.insert(
                        Node::ordinary(tagged_value(Value::Sequence(values), tag)),
                        mark,
                    );
                }
                _ => self.fail(
                    StrictYamlErrorKind::InvalidSyntax,
                    mark,
                    "unbalanced sequence",
                ),
            },
            Event::MappingStart(anchor, tag) => {
                if !self.reject_anchor(anchor, mark) {
                    self.stack.push(Frame::Mapping(Mapping::new(), None, tag));
                }
            }
            Event::MappingEnd => match self.stack.pop() {
                Some(Frame::Mapping(values, None, tag)) => {
                    self.insert(
                        Node::ordinary(tagged_value(Value::Mapping(values), tag)),
                        mark,
                    );
                }
                Some(Frame::Mapping(_, Some(_), _)) => self.fail(
                    StrictYamlErrorKind::InvalidSyntax,
                    mark,
                    "mapping key has no value",
                ),
                _ => self.fail(
                    StrictYamlErrorKind::InvalidSyntax,
                    mark,
                    "unbalanced mapping",
                ),
            },
        }
    }
}

impl StrictTreeBuilder {
    fn reject_anchor(&mut self, anchor: usize, mark: Marker) -> bool {
        if anchor == 0 {
            return false;
        }
        self.fail(
            StrictYamlErrorKind::Anchor,
            mark,
            "YAML anchors are forbidden",
        );
        true
    }

    fn insert(&mut self, node: Node, mark: Marker) {
        match self.stack.last_mut() {
            Some(Frame::Sequence(values, _)) => values.push(node.value),
            Some(Frame::Mapping(values, key, _)) => {
                if key.is_none() {
                    if node.merge_key {
                        self.fail(
                            StrictYamlErrorKind::MergeKey,
                            mark,
                            "YAML merge keys are forbidden",
                        );
                    } else {
                        *key = Some(node);
                    }
                } else {
                    let Some(key) = key.take() else {
                        self.fail(
                            StrictYamlErrorKind::InvalidSyntax,
                            mark,
                            "mapping value has no key",
                        );
                        return;
                    };
                    if values.insert(key.value, node.value).is_some() {
                        self.fail(
                            StrictYamlErrorKind::DuplicateKey,
                            mark,
                            "duplicate mapping key",
                        );
                    }
                }
            }
            None if self.root.is_none() => self.root = Some(node.value),
            None => self.fail(
                StrictYamlErrorKind::MultipleDocuments,
                mark,
                "multiple document roots are forbidden",
            ),
        }
    }

    fn fail(&mut self, kind: StrictYamlErrorKind, mark: Marker, detail: impl Into<String>) {
        self.error = Some(StrictYamlError::at(kind, mark, detail));
    }
}

impl Node {
    fn ordinary(value: Value) -> Self {
        Self {
            value,
            merge_key: false,
        }
    }
}
