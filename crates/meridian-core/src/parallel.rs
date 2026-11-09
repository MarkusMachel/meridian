//! Deterministic parallel helper (Phase 4A, step 6).
//!
//! The rule: a parallel step may only write data that no other thread touches, and
//! each element's new value may depend only on that element's own index and old value
//! (plus read-only data). Then the result is the same for any thread count, because
//! no thread can see another's writes. Anything that draws random numbers, appends to
//! a shared list or sums into a shared counter is NOT allowed here; do it
//! single-threaded, or collect per-chunk results and merge them in chunk order.
//!
//! Rust note for C# readers: `split_at_mut` / `chunks_mut` hand out non-overlapping
//! `&mut` slices, and the compiler proves they are disjoint. `thread::scope` lets the
//! threads borrow local data and guarantees they all finish before it returns.

use std::thread;

/// Call `f(first_index, chunk)` on contiguous pieces of `data`, using up to `threads`
/// threads. `f` must follow the rule above. With `threads <= 1` it runs inline.
pub fn for_each_chunk_mut<T, F>(data: &mut [T], threads: usize, f: F)
where
    T: Send,
    F: Fn(usize, &mut [T]) + Sync,
{
    let threads = threads.max(1);
    if threads == 1 || data.len() < 2 {
        f(0, data);
        return;
    }
    let chunk = data.len().div_ceil(threads);
    thread::scope(|s| {
        for (i, part) in data.chunks_mut(chunk).enumerate() {
            let f = &f;
            s.spawn(move || f(i * chunk, part));
        }
    });
}

/// Like `for_each_chunk_mut`, but each chunk returns a value; results come back in
/// chunk order (not completion order), so merging them is deterministic.
pub fn map_chunks_mut<T, R, F>(data: &mut [T], threads: usize, f: F) -> Vec<R>
where
    T: Send,
    R: Send,
    F: Fn(usize, &mut [T]) -> R + Sync,
{
    let threads = threads.max(1);
    if threads == 1 || data.len() < 2 {
        return vec![f(0, data)];
    }
    let chunk = data.len().div_ceil(threads);
    thread::scope(|s| {
        let handles: Vec<_> = data
            .chunks_mut(chunk)
            .enumerate()
            .map(|(i, part)| {
                let f = &f;
                s.spawn(move || f(i * chunk, part))
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("parallel chunk panicked")).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Signals;
    use crate::world::{Ruleset, World};

    /// Daily: every province's value becomes a function of its index and old value.
    struct Parallel(usize);
    impl Ruleset for Parallel {
        fn run(&self, world: &mut World, s: Signals) {
            if s.new_day {
                for_each_chunk_mut(world.values_mut(), self.0, |first, part| {
                    for (k, v) in part.iter_mut().enumerate() {
                        let i = (first + k) as i64;
                        *v = (*v * 31 + i * i + 7) % 1_000_003;
                    }
                });
            }
        }
    }

    #[test]
    fn any_thread_count_gives_the_single_threaded_hash_over_a_year() {
        let run = |threads: usize| {
            let mut w = World::new(5, 2, 1000);
            for _ in 0..8760 {
                w.tick(&[], &Parallel(threads));
            }
            w.state_hash()
        };
        let one = run(1);
        for t in [2, 3, 4, 7, 16] {
            assert_eq!(run(t), one, "{t} threads differ from 1 thread");
        }
    }

    #[test]
    fn chunk_results_come_back_in_chunk_order() {
        let mut data: Vec<u32> = (0..100).collect();
        let sums = map_chunks_mut(&mut data, 4, |first, part| (first, part.iter().sum::<u32>()));
        let firsts: Vec<usize> = sums.iter().map(|s| s.0).collect();
        assert_eq!(firsts, vec![0, 25, 50, 75]);
        assert_eq!(sums.iter().map(|s| s.1).sum::<u32>(), 4950);
    }
}
