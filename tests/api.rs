#[test]
fn fmt() {
    supercilex_tests::fmt();
}

#[test]
fn clippy() {
    supercilex_tests::clippy();
}

#[test]
fn api() {
    supercilex_tests::api();
}

#[test]
fn readme() {
    trycmd::TestCases::new().case("README.md");
}
