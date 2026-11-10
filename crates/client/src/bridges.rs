//! Ties the bridges the map draws to the bridges the simulation knows.
//!
//! The drawing finds a bridge wherever a road or railway line crosses a river line
//! (`Renderer::bridge_sites`). The simulation has a bridge on a river edge between two cells that
//! a road or railway crosses (`Map::bridge_edges`, and the world's bridge table for its
//! condition). The two come from the same roads and rivers but are not the same list, so each
//! drawn bridge is tied to the nearest simulation bridge, if one is near enough.

/// A drawn bridge belongs to a simulation bridge at most this far away (the middle of the two
/// cells' centres). Cells are about 15 km across.
pub const LINK_REACH_M: i64 = 25_000;

/// For each drawn bridge position, the index of the nearest simulation bridge edge, or `None`.
/// `edges` are (cell, cell) pairs; `cell_x`, `cell_y` the cell centres in world metres.
pub fn link(sites: &[[i32; 2]], edges: &[(u32, u32)], cell_x: &[i32], cell_y: &[i32]) -> Vec<Option<u32>> {
    // the middle of each edge, sorted by x so a site only looks at edges in its strip
    let mut mids: Vec<(i64, i64, u32)> = edges
        .iter()
        .enumerate()
        .map(|(i, &(a, b))| {
            let (a, b) = (a as usize, b as usize);
            ((cell_x[a] as i64 + cell_x[b] as i64) / 2, (cell_y[a] as i64 + cell_y[b] as i64) / 2, i as u32)
        })
        .collect();
    mids.sort_unstable();
    sites
        .iter()
        .map(|s| {
            let (x, y) = (s[0] as i64, s[1] as i64);
            let start = mids.partition_point(|m| m.0 < x - LINK_REACH_M);
            mids[start..]
                .iter()
                .take_while(|m| m.0 <= x + LINK_REACH_M)
                .map(|m| ((m.0 - x).pow(2) + (m.1 - y).pow(2), m.2))
                .filter(|&(d2, _)| d2 <= LINK_REACH_M * LINK_REACH_M)
                .min()
                .map(|(_, i)| i)
        })
        .collect()
}

/// The condition to draw each bridge site with: that of its simulation bridge, or 1000 (whole)
/// for a site with none. `bridges` is the simulation's table, (cell, cell, condition) ascending.
pub fn conditions(links: &[Option<u32>], edges: &[(u32, u32)], bridges: &[(u32, u32, u16)]) -> Vec<u16> {
    links
        .iter()
        .map(|l| match l {
            None => 1000,
            Some(i) => {
                let key = edges[*i as usize];
                bridges.binary_search_by_key(&key, |b| (b.0, b.1)).map_or(1000, |k| bridges[k].2)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawn_bridges_take_the_condition_of_the_nearest_simulation_bridge() {
        // four cells in a row, 15 km apart; simulation bridges on edges 0-1 and 2-3
        let (cx, cy) = ([0, 15_000, 30_000, 45_000], [0, 0, 0, 0]);
        let edges = [(0, 1), (2, 3)];
        // drawn bridges: near the first edge, near the second, and one 200 km away
        let sites = [[9_000, 3_000], [36_000, -4_000], [200_000, 0]];
        let links = link(&sites, &edges, &cx, &cy);
        assert_eq!(links, vec![Some(0), Some(1), None]);
        // the second bridge has been blown; the first is damaged; the far one is not the simulation's
        let table = [(0, 1, 400), (2, 3, 0)];
        assert_eq!(conditions(&links, &edges, &table), vec![400, 0, 1000]);
        // an edge with no entry in the table (a map with flags but an unseeded world) counts as whole
        assert_eq!(conditions(&links, &edges, &[]), vec![1000, 1000, 1000]);
    }
}
