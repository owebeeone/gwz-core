use serde_yaml::{Mapping, Value};

use super::super::raw_yaml::{StrictYamlErrorKind, parse_strict_yaml};

fn parsed(yaml: &str) -> Value {
    parse_strict_yaml(yaml.as_bytes()).unwrap().into_root()
}

fn rejected(yaml: &str, kind: StrictYamlErrorKind) {
    let error = parse_strict_yaml(yaml.as_bytes()).unwrap_err();
    assert_eq!(error.kind, kind, "{yaml}");
    assert!(error.line.is_some(), "{yaml}: {error:?}");
    assert!(error.column.is_some(), "{yaml}: {error:?}");
}

#[test]
fn strict_tree_matches_released_scalar_and_container_semantics() {
    let corpus = [
        "null_value: null\nbool_value: TRUE\nstring_value: 'true'\n",
        "signed: -42\nunsigned: 18446744073709551615\nleading_zero: 01\n",
        "hex: 0x2A\noctal: 0o52\nbinary: 0b101010\n",
        "float: 3.25\ninfinity: .inf\nnan: .nan\n",
        "quoted: \"x: &y *z <<\"\nliteral: |\n  &anchor *alias <<\n",
        "nested: [{key: value}, [one, two]]\nempty_map: {}\nempty_seq: []\n",
        "tagged_string: !!str 42\ntagged_int: !!int 0x2A\ncustom: !future 7\n",
        "quoted_int: !!int '42'\nquoted_bool: !!bool \"true\"\nquoted_float: !!float '3.25'\nquoted_null: !!null '~'\n",
    ];
    for yaml in corpus {
        let expected: Value = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed(yaml), expected, "{yaml}");
    }
}

#[test]
fn duplicate_keys_are_rejected_at_every_depth() {
    for yaml in [
        "schema: first\nschema: second\n",
        "baseline:\n  lock: first\n  lock: second\n",
        "participants:\n  member:\n    path: a\n    path: b\n",
        "future:\n  - {key: first, key: second}\n",
    ] {
        rejected(yaml, StrictYamlErrorKind::DuplicateKey);
    }
}

#[test]
fn every_anchor_and_alias_form_is_rejected_even_when_unused() {
    for yaml in [
        "value: &scalar text\n",
        "value: &sequence [one, two]\n",
        "value: &mapping {key: value}\n",
        "base: &base {key: value}\ncopy: *base\n",
    ] {
        let error = parse_strict_yaml(yaml.as_bytes()).unwrap_err();
        assert!(
            matches!(
                error.kind,
                StrictYamlErrorKind::Anchor | StrictYamlErrorKind::Alias
            ),
            "{yaml}: {error:?}"
        );
    }
}

#[test]
fn semantic_merge_keys_reject_but_quoted_text_remains_ordinary() {
    rejected(
        "base: {key: value}\nvalue: {<<: {key: other}}\n",
        StrictYamlErrorKind::MergeKey,
    );
    rejected(
        "base: {key: value}\nvalue: {!!merge '<<': {key: other}}\n",
        StrictYamlErrorKind::MergeKey,
    );

    let mut expected_inner = Mapping::new();
    expected_inner.insert(Value::String("<<".into()), Value::String("ordinary".into()));
    let mut expected = Mapping::new();
    expected.insert(
        Value::String("value".into()),
        Value::Mapping(expected_inner),
    );
    assert_eq!(
        parsed("value: {\"<<\": ordinary}\n"),
        Value::Mapping(expected)
    );
}

#[test]
fn document_and_encoding_shape_fail_closed() {
    assert_eq!(
        parse_strict_yaml(&[0xff]).unwrap_err().kind,
        StrictYamlErrorKind::InvalidEncoding
    );
    rejected(
        "mapping: [unterminated\n",
        StrictYamlErrorKind::InvalidSyntax,
    );
    rejected("", StrictYamlErrorKind::RootNotMapping);
    rejected("- sequence\n", StrictYamlErrorKind::RootNotMapping);
    rejected("scalar\n", StrictYamlErrorKind::RootNotMapping);
    rejected(
        "---\nfirst: value\n---\nsecond: value\n",
        StrictYamlErrorKind::MultipleDocuments,
    );
}
