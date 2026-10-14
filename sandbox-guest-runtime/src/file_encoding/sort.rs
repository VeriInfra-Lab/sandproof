// Copyright 2022 The Go Authors. All rights reserved.
// Port of Go 1.26.8 slices.SortFunc's pdqsort and helpers. BSD terms in
// ../multipart/GO-LICENSE. Stable sort/first-maximum would change which coding
// wins equal-quality ties in long fixed-envd headers. Keep exact permutations.
use super::Item;

fn less(a: Item, b: Item) -> bool {
    // cmp.Compare(b.quality, a.quality): NaN compares below every non-NaN,
    // including -Inf; two NaNs compare equal, as do positive/negative zero.
    a.quality > b.quality || b.quality.is_nan() && !a.quality.is_nan()
}
pub(super) fn sort(data: &mut [Item]) {
    let length = data.len();
    pdqsort(data, 0, length, usize::BITS - length.leading_zeros());
}
fn insertion(data: &mut [Item], a: usize, b: usize) {
    for i in a + 1..b {
        let mut j = i;
        while j > a && less(data[j], data[j - 1]) {
            data.swap(j, j - 1);
            j -= 1;
        }
    }
}
fn sift_down(data: &mut [Item], mut root: usize, hi: usize, first: usize) {
    loop {
        let mut child = 2 * root + 1;
        if child >= hi {
            return;
        }
        if child + 1 < hi && less(data[first + child], data[first + child + 1]) {
            child += 1;
        }
        if !less(data[first + root], data[first + child]) {
            return;
        }
        data.swap(first + root, first + child);
        root = child;
    }
}
fn heap(data: &mut [Item], a: usize, b: usize) {
    let hi = b - a;
    for i in (0..=(hi - 1) / 2).rev() {
        sift_down(data, i, hi, a);
    }
    for i in (0..hi).rev() {
        data.swap(a, a + i);
        sift_down(data, 0, i, a);
    }
}
fn pdqsort(data: &mut [Item], mut a: usize, mut b: usize, mut limit: u32) {
    let mut balanced = true;
    let mut partitioned = true;
    loop {
        let length = b - a;
        if length <= 12 {
            insertion(data, a, b);
            return;
        }
        if limit == 0 {
            heap(data, a, b);
            return;
        }
        if !balanced {
            break_patterns(data, a, b);
            limit -= 1;
        }
        let (mut pivot, mut hint) = choose_pivot(data, a, b);
        if hint == Hint::Decreasing {
            data[a..b].reverse();
            pivot = b - 1 - (pivot - a);
            hint = Hint::Increasing;
        }
        if balanced && partitioned && hint == Hint::Increasing && partial_insertion(data, a, b) {
            return;
        }
        if a > 0 && !less(data[a - 1], data[pivot]) {
            a = partition_equal(data, a, b, pivot);
            continue;
        }
        let (mid, already) = partition(data, a, b, pivot);
        partitioned = already;
        let left = mid - a;
        let right = b - mid;
        let threshold = length / 8;
        if left < right {
            balanced = left >= threshold;
            pdqsort(data, a, mid, limit);
            a = mid + 1;
        } else {
            balanced = right >= threshold;
            pdqsort(data, mid + 1, b, limit);
            b = mid;
        }
    }
}
fn partition(data: &mut [Item], a: usize, b: usize, pivot: usize) -> (usize, bool) {
    data.swap(a, pivot);
    let mut i = a + 1;
    let mut j = b - 1;
    while i <= j && less(data[i], data[a]) {
        i += 1;
    }
    while i <= j && !less(data[j], data[a]) {
        j -= 1;
    }
    if i > j {
        data.swap(j, a);
        return (j, true);
    }
    data.swap(i, j);
    i += 1;
    j -= 1;
    loop {
        while i <= j && less(data[i], data[a]) {
            i += 1;
        }
        while i <= j && !less(data[j], data[a]) {
            j -= 1;
        }
        if i > j {
            break;
        }
        data.swap(i, j);
        i += 1;
        j -= 1;
    }
    data.swap(j, a);
    (j, false)
}
fn partition_equal(data: &mut [Item], a: usize, b: usize, pivot: usize) -> usize {
    data.swap(a, pivot);
    let mut i = a + 1;
    let mut j = b - 1;
    loop {
        while i <= j && !less(data[a], data[i]) {
            i += 1;
        }
        while i <= j && less(data[a], data[j]) {
            j -= 1;
        }
        if i > j {
            break;
        }
        data.swap(i, j);
        i += 1;
        j -= 1;
    }
    i
}
fn partial_insertion(data: &mut [Item], a: usize, b: usize) -> bool {
    let mut i = a + 1;
    for _ in 0..5 {
        while i < b && !less(data[i], data[i - 1]) {
            i += 1;
        }
        if i == b {
            return true;
        }
        if b - a < 50 {
            return false;
        }
        data.swap(i, i - 1);
        if i - a >= 2 {
            // The pinned Go loop intentionally stops at 1, not a + 1.
            for j in (1..i).rev() {
                if !less(data[j], data[j - 1]) {
                    break;
                }
                data.swap(j, j - 1);
            }
        }
        if b - i >= 2 {
            for j in i + 1..b {
                if !less(data[j], data[j - 1]) {
                    break;
                }
                data.swap(j, j - 1);
            }
        }
    }
    false
}
fn break_patterns(data: &mut [Item], a: usize, b: usize) {
    let length = b - a;
    if length >= 8 {
        let mut random = length as u64;
        let modulus = 1_usize << (usize::BITS - length.leading_zeros());
        let center = a + (length / 4) * 2;
        for index in center - 1..=center + 1 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let mut other = usize::try_from(random & (modulus - 1) as u64).expect("masked index");
            if other >= length {
                other -= length;
            }
            data.swap(index, a + other);
        }
    }
}
#[derive(PartialEq, Eq)]
enum Hint {
    Unknown,
    Increasing,
    Decreasing,
}
fn choose_pivot(data: &[Item], a: usize, b: usize) -> (usize, Hint) {
    let length = b - a;
    let mut swaps = 0;
    let mut first = a + length / 4;
    let mut middle = a + length / 4 * 2;
    let mut last = a + length / 4 * 3;
    if length >= 8 {
        if length >= 50 {
            first = median(data, first - 1, first, first + 1, &mut swaps);
            middle = median(data, middle - 1, middle, middle + 1, &mut swaps);
            last = median(data, last - 1, last, last + 1, &mut swaps);
        }
        middle = median(data, first, middle, last, &mut swaps);
    }
    (
        middle,
        match swaps {
            0 => Hint::Increasing,
            12 => Hint::Decreasing,
            _ => Hint::Unknown,
        },
    )
}
fn order2(data: &[Item], a: usize, b: usize, swaps: &mut usize) -> (usize, usize) {
    if less(data[b], data[a]) {
        *swaps += 1;
        (b, a)
    } else {
        (a, b)
    }
}
fn median(data: &[Item], a: usize, b: usize, c: usize, swaps: &mut usize) -> usize {
    let (a, b) = order2(data, a, b, swaps);
    let (b, _) = order2(data, b, c, swaps);
    order2(data, a, b, swaps).1
}
