// Regression for gtk-rs-core b5a4071e439bef2b5eea76c3aa25e5ae84839e34.
// Run in release mode: the immutable FFI output reference can fail only
// after optimization, even when debug-mode iteration appears correct.
use glib::prelude::*;

const WORDS: [&str; 4] = ["", "alpha", "räksmörgås", "終わり"];

#[test]
fn next_returns_each_string_and_then_none() {
    let variant = WORDS.to_variant();
    let mut iter = variant.array_iter_str().unwrap();
    for word in WORDS {
        assert_eq!(iter.next(), Some(word));
    }
    assert_eq!(iter.next(), None);
}

#[test]
fn nth_returns_the_requested_string_and_advances() {
    let variant = WORDS.to_variant();
    let mut iter = variant.array_iter_str().unwrap();
    assert_eq!(iter.nth(2), Some(WORDS[2]));
    assert_eq!(iter.next(), Some(WORDS[3]));
    assert_eq!(iter.nth(0), None);
}

#[test]
fn next_back_returns_each_string_in_reverse_and_then_none() {
    let variant = WORDS.to_variant();
    let mut iter = variant.array_iter_str().unwrap();
    for word in WORDS.into_iter().rev() {
        assert_eq!(iter.next_back(), Some(word));
    }
    assert_eq!(iter.next_back(), None);
}

#[test]
fn nth_back_returns_the_requested_string_and_advances() {
    let variant = WORDS.to_variant();
    let mut iter = variant.array_iter_str().unwrap();
    assert_eq!(iter.nth_back(2), Some(WORDS[1]));
    assert_eq!(iter.next_back(), Some(WORDS[0]));
    assert_eq!(iter.nth_back(0), None);
}

#[test]
fn last_returns_the_final_string() {
    let variant = WORDS.to_variant();
    assert_eq!(variant.array_iter_str().unwrap().last(), Some(WORDS[3]));
}

#[test]
fn collect_returns_all_strings_in_order() {
    let variant = WORDS.to_variant();
    let strings: Vec<_> = variant.array_iter_str().unwrap().collect();
    assert_eq!(strings, WORDS);
}
