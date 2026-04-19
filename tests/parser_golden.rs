/// Golden tests: parse a .hmm file then serialize it back and verify
/// round-trip fidelity.
use std::fs;

// We access the library via the binary crate's modules.
// Since it's a binary crate, we import modules directly.
// For integration tests we'll replicate the parse/serialize logic.

#[path = "../src/model.rs"]
mod model;
#[path = "../src/parser.rs"]
mod parser;

#[test]
fn round_trip_sample() {
    let input = fs::read_to_string("tests/fixtures/sample.hmm").unwrap();
    let mm = parser::parse(&input);
    let output = parser::serialize_map(&mm);
    assert_eq!(output, input);
}

#[test]
fn parse_structure() {
    let input = fs::read_to_string("tests/fixtures/sample.hmm").unwrap();
    let mm = parser::parse(&input);

    // root should be "project"
    assert_eq!(mm.node(mm.root_id).title, "project");

    // root has 4 children: planning, development, testing, deployment
    let children = &mm.node(mm.root_id).children;
    assert_eq!(children.len(), 4);

    let titles: Vec<&str> = children
        .iter()
        .map(|&c| mm.node(c).title.as_str())
        .collect();
    assert_eq!(
        titles,
        vec!["planning", "development", "testing", "deployment"]
    );
}

#[test]
fn empty_file() {
    let mm = parser::parse("");
    assert_eq!(mm.node(mm.root_id).title, "root");
}

#[test]
fn single_line() {
    let input = "just a title\n";
    let mm = parser::parse(input);
    assert_eq!(mm.node(mm.root_id).title, "just a title");
    let output = parser::serialize_map(&mm);
    assert_eq!(output, input);
}
