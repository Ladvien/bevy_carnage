//! **The one canonical sort.** Every fold in this crate whose input order is not already a property
//! of the geometry passes through [`sort_total_by_key_at`].
//!
//! It lived in `bake.rs` while it had exactly one call site: the vertex-soup sort, whose input is an
//! ECS query. The carnage layer added more folds that must come back in the same order on every run —
//! wound extraction over a bond graph, chief among them — and the choice at that point is one shared
//! checked sort or several unchecked ones. A second copy would drift, and the copy that drifted would
//! be the one whose ties nobody thought to check.
//!
//! There is deliberately nothing else in this module. It is not a "utils" file; it is the sort.

/// **Sort by a key that must be a TOTAL order — and prove it, don't assert it in a comment.**
///
/// The site that motivated it is the one place in this crate whose input is an ECS query — which is
/// exactly where a runtime check earns its keep, because query order is not stable across `App`
/// instances. A comment asserting the key is total cannot fail; this can, and it is what caught the
/// vertex-soup order bug `bake::seed_from_path`'s doc records.
///
/// Under `debug_assertions` or the `strict-order` feature it **panics naming the call site and the
/// duplicated key** the moment a tie occurs. A release build pays nothing.
pub(crate) fn sort_total_by_key_at<T, K, F>(site: &'static str, v: &mut [T], mut f: F)
where
    K: Ord + std::fmt::Debug,
    F: FnMut(&T) -> K,
{
    v.sort_unstable_by_key(&mut f);
    #[cfg(any(debug_assertions, feature = "strict-order"))]
    {
        for w in v.windows(2) {
            let (a, b) = (f(&w[0]), f(&w[1]));
            assert!(
                a != b,
                "{site}: sort key is NOT a total order — two elements produced {a:?}. \
                 `sort_unstable` then resolves them by input order, which for an ECS query is not \
                 stable across `App` instances. Widen the key, or use a canonical whole-value sort."
            );
        }
    }
    #[cfg(not(any(debug_assertions, feature = "strict-order")))]
    let _ = site;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_total_key_sorts_and_does_not_trip_the_check() {
        let mut v = vec![3u32, 1, 2];
        sort_total_by_key_at("order::tests", &mut v, |x| *x);
        assert_eq!(v, vec![1, 2, 3]);
    }

    /// **The check is the reason this function exists**, so it is worth a test that it actually
    /// fires. Only under the configurations that compile it in — a release build without
    /// `strict-order` deliberately pays nothing and so cannot fail here.
    #[test]
    #[cfg(any(debug_assertions, feature = "strict-order"))]
    #[should_panic(expected = "sort key is NOT a total order")]
    fn a_duplicated_key_panics_naming_the_site() {
        let mut v = vec![(1u32, 'a'), (1u32, 'b')];
        sort_total_by_key_at("order::tests::duplicated", &mut v, |x| x.0);
    }
}
