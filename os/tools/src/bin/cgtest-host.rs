//! Host half of the code-generation self-test: prints what the az030 must print.
extern crate alloc;
#[path = "../../../tests/cgtest/src/tests.rs"]
mod tests;

fn main() {
    tests::run(&mut tests::Out(&mut |s| print!("{s}")));
}
