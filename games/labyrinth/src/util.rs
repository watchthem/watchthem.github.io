//! Small shared helpers.
//!
//! `sort_by_key` is the crate's one sort. Every `slice::sort_by` call site monomorphizes
//! its own copy of std's stable sort — ~3KB of wasm each, 67KB of the shared bundle
//! across labyrinth's 22 sites. `sort_by_key` sorts `(key, index)` pairs through a
//! single instantiation instead and then permutes the slice into place, so a call site
//! costs only its key closure and a small permutation loop.

/// Stable sort of `v` by `key`, ascending in `f32::total_cmp` order. Negate the key for
/// descending (far-to-near painter's order); integer and bool keys cast exactly (below
/// 2^24).
pub fn sort_by_key<T>(v: &mut [T], key: impl Fn(&T) -> f32) {
    let mut order: Vec<(f32, u32)> = v
        .iter()
        .enumerate()
        .map(|(i, x)| (key(x), i as u32))
        .collect();
    sort_pairs(&mut order);
    let mut src: Vec<u32> = order.into_iter().map(|(_, i)| i).collect();
    // Gather in place, one cycle at a time: slot `p` takes the element from `src[p]`.
    // A finished slot is marked `src[p] == p`.
    for i in 0..src.len() {
        let mut p = i;
        loop {
            let s = src[p] as usize;
            src[p] = p as u32;
            if s == i {
                break;
            }
            v.swap(p, s);
            p = s;
        }
    }
}

/// The single real sort. The index tiebreak makes the unstable (smaller) sort stable.
#[inline(never)]
fn sort_pairs(o: &mut [(f32, u32)]) {
    o.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_std_stable_sort() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        for n in 0..200 {
            let v: Vec<(f32, usize)> = (0..n)
                .map(|i| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    ((seed % 7) as f32 - 3.0, i)
                })
                .collect();
            let mut want = v.clone();
            want.sort_by(|a, b| b.0.total_cmp(&a.0));
            let mut got = v;
            sort_by_key(&mut got, |x| -x.0);
            assert_eq!(got, want, "n={n}");
        }
    }
}
