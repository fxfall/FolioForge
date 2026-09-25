//! The single authoritative natural-order comparator used for comic paths.

use std::cmp::Ordering;

use unicode_normalization::UnicodeNormalization;

/// Compare names by Unicode NFC + lowercase text, treating ASCII digit runs
/// numerically without integer parsing or overflow. Numeric ties prefer the
/// longer zero-padded digit run (`02` before `2`) to match the pinned KCC
/// ordering probe; remaining ties use normalized then raw Unicode scalar
/// order, so the result is deterministic and total.
pub fn natural_order_cmp(left: &str, right: &str) -> Ordering {
    let left_folded = folded_chars(left);
    let right_folded = folded_chars(right);
    let (mut left_index, mut right_index) = (0, 0);

    while left_index < left_folded.len() && right_index < right_folded.len() {
        let left_char = left_folded[left_index];
        let right_char = right_folded[right_index];
        if left_char.is_ascii_digit() && right_char.is_ascii_digit() {
            let left_end = digit_run_end(&left_folded, left_index);
            let right_end = digit_run_end(&right_folded, right_index);
            let left_run = &left_folded[left_index..left_end];
            let right_run = &right_folded[right_index..right_end];
            let magnitude_order = compare_digit_runs(left_run, right_run);
            if magnitude_order != Ordering::Equal {
                return magnitude_order;
            }
            left_index = left_end;
            right_index = right_end;
            continue;
        }

        match left_char.cmp(&right_char) {
            Ordering::Equal => {
                left_index += 1;
                right_index += 1;
            }
            different => return different,
        }
    }

    match left_folded.len().cmp(&right_folded.len()) {
        Ordering::Equal => normalized_tie_break(left, right),
        different => different,
    }
}

fn folded_chars(value: &str) -> Vec<char> {
    value.nfc().flat_map(char::to_lowercase).collect::<Vec<_>>()
}

fn digit_run_end(chars: &[char], start: usize) -> usize {
    chars[start..]
        .iter()
        .position(|value| !value.is_ascii_digit())
        .map_or(chars.len(), |offset| start + offset)
}

fn compare_digit_runs(left: &[char], right: &[char]) -> Ordering {
    let left_significant = significant_digits(left);
    let right_significant = significant_digits(right);
    left_significant
        .len()
        .cmp(&right_significant.len())
        .then_with(|| left_significant.cmp(right_significant))
        .then_with(|| right.len().cmp(&left.len()))
}

fn significant_digits(digits: &[char]) -> &[char] {
    let first_non_zero = digits
        .iter()
        .position(|value| *value != '0')
        .unwrap_or(digits.len());
    &digits[first_non_zero..]
}

fn normalized_tie_break(left: &str, right: &str) -> Ordering {
    let left_normalized = left.nfc().collect::<String>();
    let right_normalized = right.nfc().collect::<String>();
    left_normalized
        .cmp(&right_normalized)
        .then_with(|| left.cmp(right))
}
