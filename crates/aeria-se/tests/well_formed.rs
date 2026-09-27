use aeria_se::bytes::{self, Expr, Node};

const VECTORS: &str = include_str!("fixtures/well_formed.vectors.txt");
const GOLDEN: &str = include_str!("fixtures/macro_text.golden.txt");

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
        .collect()
}

fn vectors() -> impl Iterator<Item = (bool, &'static str, Vec<u8>)> {
    VECTORS
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split('\t');
            let expected = match parts.next() {
                Some("yes") => true,
                Some("no") => false,
                other => panic!("unexpected answer {other:?}"),
            };
            let name = parts.next().expect("name");
            (expected, name, hex(parts.next().unwrap_or_default()))
        })
}

fn has_raw(nodes: &[Node]) -> bool {
    fn expr_has_raw(expr: &Expr) -> bool {
        match expr {
            Expr::Raw(_) => true,
            Expr::Str(nodes) => has_raw(nodes),
            Expr::Param(_, operand) => expr_has_raw(operand),
            Expr::Compare(_, left, right) => expr_has_raw(left) || expr_has_raw(right),
            Expr::Int(_) | Expr::Nullary(_) => false,
        }
    }
    nodes.iter().any(|node| match node {
        Node::Raw(_) => true,
        Node::Macro(macro_node) => macro_node.args.iter().any(expr_has_raw),
        Node::Text(_) => false,
    })
}

#[test]
fn vectors_give_the_expected_answer() {
    for (expected, name, bytes) in vectors() {
        assert_eq!(bytes::is_well_formed(&bytes), expected, "{name}");
    }
}

#[test]
fn every_golden_vector_is_a_well_formed_vector() {
    let names: Vec<&str> = vectors()
        .filter(|(expected, ..)| *expected)
        .map(|(_, name, _)| name)
        .collect();
    for line in GOLDEN
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
    {
        let name = line.split('\t').next().expect("name");
        assert!(
            names.contains(&format!("golden_{name}").as_str()),
            "golden vector {name} is missing from well_formed.vectors.txt"
        );
    }
}

#[test]
fn strings_read_without_raw_bytes_are_well_formed() {
    for (_, name, bytes) in vectors() {
        let nodes = bytes::decode(&bytes);
        if !bytes.is_empty() && !has_raw(&nodes) && name != "expression_depth_257" {
            assert!(bytes::is_well_formed(&bytes), "{name}");
        }
    }
}
