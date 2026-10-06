//! Quicksort with a median of three, insertion sort under 16 elements.

pub fn sort(a: &mut [u32]) {
    let mut a = a;
    // Recurse into the smaller part, loop on the larger: the stack stays O(log n).
    while a.len() >= 16 {
        let p = partition(a);
        let (l, r) = a.split_at_mut(p);
        let r = &mut r[1..];
        if l.len() < r.len() {
            sort(l);
            a = r;
        } else {
            sort(r);
            a = l;
        }
    }
    for i in 1..a.len() {
        let mut j = i;
        while j > 0 && a[j - 1] > a[j] {
            a.swap(j - 1, j);
            j -= 1;
        }
    }
}

/// Lomuto partition around the median of first, middle and last; returns the pivot's place.
fn partition(a: &mut [u32]) -> usize {
    let (lo, mid, hi) = (0, a.len() / 2, a.len() - 1);
    if a[mid] < a[lo] {
        a.swap(mid, lo);
    }
    if a[hi] < a[lo] {
        a.swap(hi, lo);
    }
    if a[hi] < a[mid] {
        a.swap(hi, mid);
    }
    a.swap(mid, hi);
    let pivot = a[hi];
    let mut store = 0;
    for i in 0..hi {
        if a[i] < pivot {
            a.swap(i, store);
            store += 1;
        }
    }
    a.swap(store, hi);
    store
}
