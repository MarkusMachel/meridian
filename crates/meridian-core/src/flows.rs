//! Shared flow primitives (Phase 4A, step 10): shortage splitting, double-buffered
//! flows and multi-source supply propagation. All integer math, no floats. Each has
//! a conservation test: nothing is created or lost except where documented.
//!
//! Known limit of version 1: propagation has no per-link capacity yet (the design
//! notes start with one shared capacity per link). Only distance and the sources'
//! amounts limit supply.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Split `available` among claimants whose needs are `demands`.
///
/// - If everyone can be served, each gets exactly their demand.
/// - Otherwise each gets `available * demand / total_demand` rounded down, and the
///   leftover units (fewer than the number of claimants) go one each to claimants in
///   index order (so by ID if the slice is ordered by ID), skipping zero demands.
///
/// The shares always sum to `min(available, total_demand)` and never exceed a demand.
/// Negative inputs count as zero.
pub fn split_shortage(available: i64, demands: &[i64]) -> Vec<i64> {
    let available = available.max(0);
    let total: i128 = demands.iter().map(|&d| d.max(0) as i128).sum();
    if available as i128 >= total {
        return demands.iter().map(|&d| d.max(0)).collect();
    }
    let mut shares: Vec<i64> = demands.iter().map(|&d| ((available as i128 * d.max(0) as i128) / total) as i64).collect();
    let mut left = available - shares.iter().sum::<i64>();
    for (s, &d) in shares.iter_mut().zip(demands) {
        if left == 0 {
            break;
        }
        if d > 0 {
            *s += 1;
            left -= 1;
        }
    }
    debug_assert_eq!(left, 0);
    shares
}

/// A request to move goods between two stocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flow {
    pub from: u32,
    pub to: u32,
    pub amount: i64,
}

/// Apply all flows at once, computed from the start-of-step stocks so that the order
/// of the requests cannot change the outcome. If a source is asked for more than it
/// holds, its stock is split among its requests with `split_shortage` (requests
/// ordered by destination, then amount). The total over all stocks never changes.
/// Returns the amount actually moved per request, in the order the requests were given.
pub fn apply_flows(stock: &mut [i64], flows: &[Flow]) -> Vec<i64> {
    // Canonical order, independent of how the caller listed the requests.
    let mut order: Vec<usize> = (0..flows.len()).collect();
    order.sort_by_key(|&i| (flows[i].from, flows[i].to, flows[i].amount, i));
    let mut moved = vec![0i64; flows.len()];
    let mut start = 0;
    while start < order.len() {
        let from = flows[order[start]].from;
        let end = start + order[start..].iter().take_while(|&&i| flows[i].from == from).count();
        let group = &order[start..end];
        let demands: Vec<i64> = group.iter().map(|&i| flows[i].amount).collect();
        let shares = split_shortage(stock[from as usize], &demands);
        for (&i, &s) in group.iter().zip(&shares) {
            moved[i] = s;
        }
        start = end;
    }
    // Apply after everything has been decided.
    for (f, &m) in flows.iter().zip(&moved) {
        stock[f.from as usize] -= m;
        stock[f.to as usize] += m;
    }
    moved
}

/// An undirected graph in compressed form, with an integer cost per edge.
#[derive(Clone, Debug)]
pub struct CostGraph {
    offsets: Vec<u32>,
    /// (neighbor, cost)
    edges: Vec<(u32, u32)>,
}

impl CostGraph {
    pub fn from_edges(nodes: u32, edges: &[(u32, u32, u32)]) -> CostGraph {
        let mut lists: Vec<Vec<(u32, u32)>> = vec![Vec::new(); nodes as usize];
        for &(a, b, c) in edges {
            lists[a as usize].push((b, c));
            lists[b as usize].push((a, c));
        }
        let mut offsets = vec![0u32];
        let mut flat = Vec::new();
        for l in &mut lists {
            l.sort_unstable();
            flat.extend_from_slice(l);
            offsets.push(flat.len() as u32);
        }
        CostGraph { offsets, edges: flat }
    }
    pub fn node_count(&self) -> u32 {
        (self.offsets.len() - 1) as u32
    }
    fn neighbors(&self, n: u32) -> &[(u32, u32)] {
        &self.edges[self.offsets[n as usize] as usize..self.offsets[n as usize + 1] as usize]
    }
}

/// A supply source: a node holding `amount` units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub node: u32,
    pub amount: i64,
}

pub struct SupplyResult {
    /// Units delivered to each node.
    pub delivered: Vec<i64>,
    /// Units each source still holds after delivering (same order as the input).
    pub left_over: Vec<i64>,
}

/// Spread supply from several sources over a graph.
///
/// 1. Every node within `max_cost` of some source is assigned to its cheapest source;
///    equal cost goes to the source with the lower index.
/// 2. Each source splits its amount among its assigned nodes' demands with
///    `split_shortage`, nodes in ID order.
///
/// Conservation: `sum(delivered) + sum(left_over) == sum(source amounts)`, and no node
/// receives more than its demand.
pub fn propagate_supply(graph: &CostGraph, sources: &[Source], demand: &[i64], max_cost: u32) -> SupplyResult {
    let n = graph.node_count() as usize;
    // best[node] = (cost, source index); Dijkstra with a total order on (cost, source, node).
    let mut best: Vec<Option<(u32, u32)>> = vec![None; n];
    let mut heap = BinaryHeap::new();
    for (si, s) in sources.iter().enumerate() {
        let cand = (0u32, si as u32);
        if best[s.node as usize].is_none_or(|b| cand < b) {
            best[s.node as usize] = Some(cand);
            heap.push(Reverse((0u32, si as u32, s.node)));
        }
    }
    while let Some(Reverse((cost, si, node))) = heap.pop() {
        if best[node as usize] != Some((cost, si)) {
            continue;
        }
        for &(nb, c) in graph.neighbors(node) {
            let nc = cost.saturating_add(c);
            if nc > max_cost {
                continue;
            }
            let cand = (nc, si);
            if best[nb as usize].is_none_or(|b| cand < b) {
                best[nb as usize] = Some(cand);
                heap.push(Reverse((nc, si, nb)));
            }
        }
    }
    let mut delivered = vec![0i64; n];
    let mut left_over = Vec::with_capacity(sources.len());
    for (si, s) in sources.iter().enumerate() {
        let nodes: Vec<usize> = (0..n).filter(|&v| best[v].is_some_and(|b| b.1 == si as u32)).collect();
        let demands: Vec<i64> = nodes.iter().map(|&v| demand[v]).collect();
        let shares = split_shortage(s.amount, &demands);
        for (&v, &sh) in nodes.iter().zip(&shares) {
            delivered[v] = sh;
        }
        left_over.push(s.amount.max(0) - shares.iter().sum::<i64>());
    }
    SupplyResult { delivered, left_over }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    #[test]
    fn shortage_is_proportional_with_remainders_by_index() {
        assert_eq!(split_shortage(100, &[10, 20]), vec![10, 20]);
        // 10 units among 1:1:1 -> 3 each plus the spare unit to the first claimant
        assert_eq!(split_shortage(10, &[5, 5, 5]), vec![4, 3, 3]);
        assert_eq!(split_shortage(0, &[5, 5]), vec![0, 0]);
        assert_eq!(split_shortage(7, &[0, 4, 0, 4]), vec![0, 4, 0, 3]);
    }

    #[test]
    fn shortage_conserves_and_respects_demands_for_random_inputs() {
        let mut rng = Rng::from_seed(3);
        for _ in 0..5_000 {
            let n = 1 + rng.below(12) as usize;
            let demands: Vec<i64> = (0..n).map(|_| rng.below(1000) as i64).collect();
            let avail = rng.below(5000) as i64;
            let shares = split_shortage(avail, &demands);
            let total: i64 = demands.iter().sum();
            assert_eq!(shares.iter().sum::<i64>(), avail.min(total));
            assert!(shares.iter().zip(&demands).all(|(s, d)| s <= d && *s >= 0));
        }
    }

    #[test]
    fn flows_conserve_and_ignore_request_order() {
        let mut rng = Rng::from_seed(9);
        for _ in 0..2_000 {
            let n = 2 + rng.below(8) as usize;
            let stock: Vec<i64> = (0..n).map(|_| rng.below(100) as i64).collect();
            let flows: Vec<Flow> = (0..rng.below(20))
                .map(|_| Flow { from: rng.below(n as u64) as u32, to: rng.below(n as u64) as u32, amount: rng.below(80) as i64 })
                .collect();
            let mut a = stock.clone();
            apply_flows(&mut a, &flows);
            assert_eq!(a.iter().sum::<i64>(), stock.iter().sum::<i64>(), "total changed");
            assert!(a.iter().all(|&x| x >= 0), "negative stock");
            let mut reversed = flows.clone();
            reversed.reverse();
            let mut b = stock.clone();
            apply_flows(&mut b, &reversed);
            assert_eq!(a, b, "request order changed the result");
        }
    }

    #[test]
    fn a_chain_of_flows_moves_only_start_of_step_stock() {
        // 0 -> 1 -> 2 in one step: node 1 starts empty, so it cannot pass on what it receives.
        let mut s = vec![10, 0, 0];
        apply_flows(&mut s, &[Flow { from: 0, to: 1, amount: 10 }, Flow { from: 1, to: 2, amount: 10 }]);
        assert_eq!(s, vec![0, 10, 0]);
    }

    #[test]
    fn supply_goes_to_nearest_source_and_conserves() {
        // line 0-1-2-3-4, sources at 0 and 4 (amount 10 each), everyone wants 4
        let g = CostGraph::from_edges(5, &[(0, 1, 1), (1, 2, 1), (2, 3, 1), (3, 4, 1)]);
        let srcs = [Source { node: 0, amount: 10 }, Source { node: 4, amount: 10 }];
        let r = propagate_supply(&g, &srcs, &[4, 4, 4, 4, 4], 10);
        // node 2 is equidistant: lower source index (0) wins. Source 0 serves 0,1,2 (12 wanted, 10 held).
        assert_eq!(r.delivered, vec![4, 3, 3, 4, 4]);
        assert_eq!(r.left_over, vec![0, 2]);
        // out of range gets nothing
        let r = propagate_supply(&g, &srcs[..1], &[4, 4, 4, 4, 4], 2);
        assert_eq!(r.delivered, vec![4, 3, 3, 0, 0]); // 10 units split 4:4:4 -> 4,3,3
    }

    #[test]
    fn supply_conservation_on_random_graphs() {
        let mut rng = Rng::from_seed(21);
        for _ in 0..300 {
            let n = 3 + rng.below(30) as u32;
            let edges: Vec<(u32, u32, u32)> = (0..n * 2).map(|_| (rng.below(n as u64) as u32, rng.below(n as u64) as u32, 1 + rng.below(5) as u32)).collect();
            let g = CostGraph::from_edges(n, &edges);
            let srcs: Vec<Source> = (0..1 + rng.below(4)).map(|_| Source { node: rng.below(n as u64) as u32, amount: rng.below(200) as i64 }).collect();
            let demand: Vec<i64> = (0..n).map(|_| rng.below(30) as i64).collect();
            let r = propagate_supply(&g, &srcs, &demand, 8);
            let total_in: i64 = srcs.iter().map(|s| s.amount).sum();
            assert_eq!(r.delivered.iter().sum::<i64>() + r.left_over.iter().sum::<i64>(), total_in);
            assert!(r.delivered.iter().zip(&demand).all(|(d, w)| d <= w));
        }
    }
}
