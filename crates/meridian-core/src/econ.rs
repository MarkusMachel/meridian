//! Economy plumbing (Phase 4C, step 15). Tables and conserving primitives only; goods,
//! recipes, prices and costs are rules data. The engine never names a good.
//!
//! Conservation: goods change only through `apply_recipe` (inputs consumed, outputs
//! produced), construction (goods leave a stock and are spent), explicit set-up, and
//! flows (`flows.rs`). People change only through `births`, `deaths` and `transfer`.
//! Both are covered by tests.

use crate::fixed::Fixed;
use crate::flows::split_shortage;
use crate::hash::StateHasher;
use crate::ids::CountryId;

// ---------------------------------------------------------------- population groups

/// A group of people in one cell with a shared culture and religion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopGroup {
    pub cell: u32,
    pub culture: u16,
    pub religion: u8,
    pub heads: u32,
    /// Political support shares for four data-defined ideologies, thousandths each.
    pub support: [u16; 4],
}

/// Groups sorted by (cell, culture, religion). Identity survives movement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PopTable {
    groups: Vec<PopGroup>,
    /// Running totals, hashed, so conservation is checkable at any time:
    /// `total_heads == initial + born - died`.
    pub born: u64,
    pub died: u64,
}

impl PopTable {
    fn find(&self, cell: u32, culture: u16, religion: u8) -> Result<usize, usize> {
        self.groups.binary_search_by_key(&(cell, culture, religion), |g| (g.cell, g.culture, g.religion))
    }
    pub fn groups(&self) -> &[PopGroup] {
        &self.groups
    }
    pub fn len(&self) -> usize {
        self.groups.len()
    }
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
    pub fn total_heads(&self) -> u64 {
        self.groups.iter().map(|g| g.heads as u64).sum()
    }
    pub fn heads_in_cell(&self, cell: u32) -> u64 {
        let start = self.groups.partition_point(|g| g.cell < cell);
        self.groups[start..].iter().take_while(|g| g.cell == cell).map(|g| g.heads as u64).sum()
    }

    /// Set-up: add a group, or merge into an existing one (support is head-weighted).
    pub fn add_for_setup(&mut self, g: PopGroup) {
        match self.find(g.cell, g.culture, g.religion) {
            Ok(i) => {
                let old = self.groups[i];
                let total = old.heads as u64 + g.heads as u64;
                for k in 0..4 {
                    let v = (old.support[k] as u64 * old.heads as u64 + g.support[k] as u64 * g.heads as u64) / total.max(1);
                    self.groups[i].support[k] = v as u16;
                }
                self.groups[i].heads = total as u32;
            }
            Err(i) => self.groups.insert(i, g),
        }
    }

    /// Explicit births into an existing group. Returns false if there is no such group.
    pub fn births(&mut self, cell: u32, culture: u16, religion: u8, n: u32) -> bool {
        match self.find(cell, culture, religion) {
            Ok(i) => {
                self.groups[i].heads += n;
                self.born += n as u64;
                true
            }
            Err(_) => false,
        }
    }

    /// Explicit deaths; at most the group's head count die. Returns how many died.
    pub fn deaths(&mut self, cell: u32, culture: u16, religion: u8, n: u32) -> u32 {
        match self.find(cell, culture, religion) {
            Ok(i) => {
                let d = n.min(self.groups[i].heads);
                self.groups[i].heads -= d;
                self.died += d as u64;
                d
            }
            Err(_) => 0,
        }
    }

    /// Move people between cells, keeping culture, religion and support. The total never
    /// changes. Returns how many actually moved.
    pub fn transfer(&mut self, from: u32, to: u32, culture: u16, religion: u8, n: u32) -> u32 {
        let Ok(i) = self.find(from, culture, religion) else { return 0 };
        let moved = n.min(self.groups[i].heads);
        if moved == 0 || from == to {
            return 0;
        }
        let src = self.groups[i];
        self.groups[i].heads -= moved;
        self.add_for_setup(PopGroup { cell: to, heads: moved, ..src });
        moved
    }

    fn hash_into(&self, h: &mut StateHasher) {
        h.write_u32(self.groups.len() as u32);
        for g in &self.groups {
            h.write_u32(g.cell);
            h.write_u16(g.culture);
            h.write_u8(g.religion);
            h.write_u32(g.heads);
            for s in g.support {
                h.write_u16(s);
            }
        }
        h.write_u64(self.born);
        h.write_u64(self.died);
    }
}

// ---------------------------------------------------------------- buildings, stocks

/// A levelled building. One per (cell, kind).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Building {
    pub cell: u32,
    pub kind: u16,
    pub level: u8,
    /// 0 (destroyed) to 1000.
    pub condition: u16,
    pub owner: CountryId,
}

/// A recipe: per run, consume `inputs` and produce `outputs` (good, amount).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    pub inputs: Vec<(u16, i64)>,
    pub outputs: Vec<(u16, i64)>,
}

/// Run a recipe up to `runs` times against one region's stocks (indexed by good).
/// Stops at the number of runs the inputs allow. Returns the runs done.
pub fn apply_recipe(stocks: &mut [i64], recipe: &Recipe, runs: i64) -> i64 {
    let mut n = runs.max(0);
    for &(g, a) in &recipe.inputs {
        if a > 0 {
            n = n.min(stocks[g as usize] / a);
        }
    }
    for &(g, a) in &recipe.inputs {
        stocks[g as usize] -= a * n;
    }
    for &(g, a) in &recipe.outputs {
        stocks[g as usize] += a * n;
    }
    n
}

/// Share of a building's capacity that is staffed, in thousandths (0 to 1000).
pub fn staffing_permille(workers_available: i64, workers_needed: i64) -> i64 {
    if workers_needed <= 0 {
        return 1000;
    }
    (workers_available.max(0) * 1000 / workers_needed).min(1000)
}

/// New price from the supply-to-demand balance, kept inside a band.
/// `price = base * demand / supply`, clamped to `[base*lo, base*hi]`; no supply means the
/// top of the band, no demand the bottom.
pub fn banded_price(base: Fixed, supply: i64, demand: i64, lo: Fixed, hi: Fixed) -> Fixed {
    let (floor, ceil) = (base * lo, base * hi);
    if supply <= 0 {
        return if demand > 0 { ceil } else { base };
    }
    let ratio = Fixed::from_ratio(demand.max(0), supply);
    (base * ratio).max(floor).min(ceil)
}

// ---------------------------------------------------------------- construction

/// What a construction command asks for; the costs come from rules data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectTemplate {
    pub goods: Vec<(u16, i64)>,
    pub labor: i64,
    pub min_ticks: u64,
}

/// Where and what to build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectSpec {
    pub owner: CountryId,
    pub region: u32,
    pub cell: u32,
    pub kind: u16,
    pub target_level: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: u32,
    pub owner: CountryId,
    pub region: u32,
    pub cell: u32,
    pub kind: u16,
    pub target_level: u8,
    pub goods_needed: Vec<(u16, i64)>,
    pub goods_paid: Vec<i64>,
    pub labor_needed: i64,
    pub labor_paid: i64,
    pub earliest_end: u64,
}

impl Project {
    fn done(&self) -> bool {
        self.labor_paid >= self.labor_needed && self.goods_needed.iter().zip(&self.goods_paid).all(|(n, p)| *p >= n.1)
    }
}

// ---------------------------------------------------------------- production, trade, money

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionOrder {
    pub country: CountryId,
    pub model: u16,
    /// 0 means continuous.
    pub quantity: u32,
    pub priority: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TradeRoute {
    pub from_region: u32,
    pub to_region: u32,
    pub good: u16,
    pub capacity: i64,
    pub tariff_permille: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Finance {
    pub treasury: i64,
    pub debt: i64,
    /// Thousandths above the base price level.
    pub inflation: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Economy {
    pub goods: u16,
    pub regions: u32,
    /// `[region * goods + good]`.
    stocks: Vec<i64>,
    /// Fixed raw values, same layout as stocks.
    prices: Vec<i64>,
    pub pop: PopTable,
    buildings: Vec<Building>,
    projects: Vec<Project>,
    next_project: u32,
    orders: Vec<ProductionOrder>,
    /// (country, model, count), sorted.
    equipment: Vec<(u16, u16, u64)>,
    routes: Vec<TradeRoute>,
    finance: Vec<Finance>,
}

impl Economy {
    pub fn is_empty(&self) -> bool {
        *self == Economy::default()
    }

    /// Declare the shape. Clears stocks and prices.
    pub fn configure(&mut self, goods: u16, regions: u32) {
        self.goods = goods;
        self.regions = regions;
        self.stocks = vec![0; goods as usize * regions as usize];
        self.prices = vec![0; goods as usize * regions as usize];
    }

    pub fn stock(&self, region: u32, good: u16) -> i64 {
        self.stocks[region as usize * self.goods as usize + good as usize]
    }
    pub fn stocks_of(&self, region: u32) -> &[i64] {
        let g = self.goods as usize;
        &self.stocks[region as usize * g..(region as usize + 1) * g]
    }
    pub fn stocks_of_mut(&mut self, region: u32) -> &mut [i64] {
        let g = self.goods as usize;
        &mut self.stocks[region as usize * g..(region as usize + 1) * g]
    }
    pub fn all_stocks_mut(&mut self) -> &mut [i64] {
        &mut self.stocks
    }
    pub fn total_of_good(&self, good: u16) -> i64 {
        (0..self.regions).map(|r| self.stock(r, good)).sum()
    }
    pub fn price(&self, region: u32, good: u16) -> Fixed {
        Fixed::from_raw(self.prices[region as usize * self.goods as usize + good as usize])
    }
    pub fn set_price(&mut self, region: u32, good: u16, p: Fixed) {
        self.prices[region as usize * self.goods as usize + good as usize] = p.raw();
    }

    // buildings
    pub fn buildings(&self) -> &[Building] {
        &self.buildings
    }
    pub fn building(&self, cell: u32, kind: u16) -> Option<&Building> {
        self.buildings.binary_search_by_key(&(cell, kind), |b| (b.cell, b.kind)).ok().map(|i| &self.buildings[i])
    }
    pub fn set_building(&mut self, b: Building) {
        match self.buildings.binary_search_by_key(&(b.cell, b.kind), |x| (x.cell, x.kind)) {
            Ok(i) if b.level == 0 => {
                self.buildings.remove(i);
            }
            Ok(i) => self.buildings[i] = b,
            Err(i) if b.level > 0 => self.buildings.insert(i, b),
            Err(_) => {}
        }
    }
    /// Damage lowers condition; a building at zero condition loses a level and is repaired to full.
    pub fn damage_building(&mut self, cell: u32, kind: u16, amount: u16) {
        if let Some(mut b) = self.building(cell, kind).copied() {
            if amount >= b.condition {
                b.level = b.level.saturating_sub(1);
                b.condition = 1000;
            } else {
                b.condition -= amount;
            }
            self.set_building(b);
        }
    }

    // projects
    pub fn projects(&self) -> &[Project] {
        &self.projects
    }
    pub fn start_project(&mut self, spec: ProjectSpec, t: &ProjectTemplate, now: u64) -> u32 {
        let ProjectSpec { owner, region, cell, kind, target_level } = spec;
        let id = self.next_project;
        self.next_project += 1;
        self.projects.push(Project {
            id,
            owner,
            region,
            cell,
            kind,
            target_level,
            goods_needed: t.goods.clone(),
            goods_paid: vec![0; t.goods.len()],
            labor_needed: t.labor,
            labor_paid: 0,
            earliest_end: now + t.min_ticks,
        });
        id
    }

    /// Advance every project one step. `labor_by_region[r]` is the construction labor
    /// the region can spare this step. Projects in a region are served in ID order;
    /// when stocks or labor run short they are split with `split_shortage`. Finished
    /// projects raise the building level. Returns how many finished.
    pub fn advance_projects(&mut self, labor_by_region: &[i64], now: u64) -> usize {
        for region in 0..self.regions {
            let idx: Vec<usize> = (0..self.projects.len()).filter(|&i| self.projects[i].region == region).collect();
            if idx.is_empty() {
                continue;
            }
            for good in 0..self.goods {
                let demands: Vec<i64> = idx
                    .iter()
                    .map(|&i| {
                        let p = &self.projects[i];
                        p.goods_needed.iter().position(|g| g.0 == good).map_or(0, |k| p.goods_needed[k].1 - p.goods_paid[k])
                    })
                    .collect();
                if demands.iter().all(|&d| d == 0) {
                    continue;
                }
                let shares = split_shortage(self.stock(region, good), &demands);
                for (&i, &s) in idx.iter().zip(&shares) {
                    if s > 0 {
                        let k = self.projects[i].goods_needed.iter().position(|g| g.0 == good).unwrap();
                        self.projects[i].goods_paid[k] += s;
                        self.stocks[region as usize * self.goods as usize + good as usize] -= s;
                    }
                }
            }
            let labor_demands: Vec<i64> = idx.iter().map(|&i| self.projects[i].labor_needed - self.projects[i].labor_paid).collect();
            let shares = split_shortage(labor_by_region.get(region as usize).copied().unwrap_or(0), &labor_demands);
            for (&i, &s) in idx.iter().zip(&shares) {
                self.projects[i].labor_paid += s;
            }
        }
        let mut finished = 0;
        let mut i = 0;
        while i < self.projects.len() {
            if self.projects[i].done() && now >= self.projects[i].earliest_end {
                let p = self.projects.remove(i);
                let prev = self.building(p.cell, p.kind).copied();
                self.set_building(Building { cell: p.cell, kind: p.kind, level: p.target_level, condition: prev.map_or(1000, |b| b.condition), owner: p.owner });
                finished += 1;
            } else {
                i += 1;
            }
        }
        finished
    }

    // production orders and equipment
    pub fn orders(&self) -> &[ProductionOrder] {
        &self.orders
    }
    /// Replace the order for (country, model).
    pub fn set_order(&mut self, o: ProductionOrder) {
        match self.orders.binary_search_by_key(&(o.country.0, o.model), |x| (x.country.0, x.model)) {
            Ok(i) if o.quantity == 0 && o.priority == 0 => {
                self.orders.remove(i);
            }
            Ok(i) => self.orders[i] = o,
            Err(i) => self.orders.insert(i, o),
        }
    }
    pub fn equipment(&self, country: CountryId, model: u16) -> u64 {
        self.equipment.binary_search_by_key(&(country.0, model), |e| (e.0, e.1)).map_or(0, |i| self.equipment[i].2)
    }
    /// Models a country holds a stockpile of, ascending.
    pub fn models_held_by(&self, country: CountryId) -> Vec<u16> {
        self.equipment.iter().filter(|e| e.0 == country.0 && e.2 > 0).map(|e| e.1).collect()
    }
    pub fn add_equipment(&mut self, country: CountryId, model: u16, n: u64) {
        match self.equipment.binary_search_by_key(&(country.0, model), |e| (e.0, e.1)) {
            Ok(i) => self.equipment[i].2 += n,
            Err(i) => self.equipment.insert(i, (country.0, model, n)),
        }
    }
    /// Take up to `n` from the stockpile; returns what was taken.
    pub fn take_equipment(&mut self, country: CountryId, model: u16, n: u64) -> u64 {
        match self.equipment.binary_search_by_key(&(country.0, model), |e| (e.0, e.1)) {
            Ok(i) => {
                let t = n.min(self.equipment[i].2);
                self.equipment[i].2 -= t;
                t
            }
            Err(_) => 0,
        }
    }

    // trade
    pub fn routes(&self) -> &[TradeRoute] {
        &self.routes
    }
    pub fn add_route(&mut self, r: TradeRoute) {
        let key = (r.from_region, r.to_region, r.good);
        match self.routes.binary_search_by_key(&key, |x| (x.from_region, x.to_region, x.good)) {
            Ok(i) => self.routes[i] = r,
            Err(i) => self.routes.insert(i, r),
        }
    }

    // money
    pub fn finance(&self, c: CountryId) -> Finance {
        self.finance.get(c.index()).copied().unwrap_or_default()
    }
    pub fn finance_mut(&mut self, c: CountryId) -> &mut Finance {
        if self.finance.len() <= c.index() {
            self.finance.resize(c.index() + 1, Finance::default());
        }
        &mut self.finance[c.index()]
    }
    /// Move money between treasuries. Total money is unchanged. Returns false if the payer is short.
    pub fn pay(&mut self, from: CountryId, to: CountryId, amount: i64) -> bool {
        if amount < 0 || self.finance(from).treasury < amount {
            return false;
        }
        self.finance_mut(from).treasury -= amount;
        self.finance_mut(to).treasury += amount;
        true
    }
    pub fn total_money(&self) -> i64 {
        self.finance.iter().map(|f| f.treasury).sum()
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        h.write_u16(self.goods);
        h.write_u32(self.regions);
        for v in self.stocks.iter().chain(&self.prices) {
            h.write_i64(*v);
        }
        self.pop.hash_into(h);
        h.write_u32(self.buildings.len() as u32);
        for b in &self.buildings {
            h.write_u32(b.cell);
            h.write_u16(b.kind);
            h.write_u8(b.level);
            h.write_u16(b.condition);
            h.write_u16(b.owner.0);
        }
        h.write_u32(self.projects.len() as u32);
        for p in &self.projects {
            h.write_u32(p.id);
            h.write_u16(p.owner.0);
            h.write_u32(p.region);
            h.write_u32(p.cell);
            h.write_u16(p.kind);
            h.write_u8(p.target_level);
            for (g, n) in &p.goods_needed {
                h.write_u16(*g);
                h.write_i64(*n);
            }
            for v in &p.goods_paid {
                h.write_i64(*v);
            }
            h.write_i64(p.labor_needed);
            h.write_i64(p.labor_paid);
            h.write_u64(p.earliest_end);
        }
        h.write_u32(self.next_project);
        h.write_u32(self.orders.len() as u32);
        for o in &self.orders {
            h.write_u16(o.country.0);
            h.write_u16(o.model);
            h.write_u32(o.quantity);
            h.write_u8(o.priority);
        }
        h.write_u32(self.equipment.len() as u32);
        for e in &self.equipment {
            h.write_u16(e.0);
            h.write_u16(e.1);
            h.write_u64(e.2);
        }
        h.write_u32(self.routes.len() as u32);
        for r in &self.routes {
            h.write_u32(r.from_region);
            h.write_u32(r.to_region);
            h.write_u16(r.good);
            h.write_i64(r.capacity);
            h.write_u16(r.tariff_permille);
        }
        h.write_u32(self.finance.len() as u32);
        for f in &self.finance {
            h.write_i64(f.treasury);
            h.write_i64(f.debt);
            h.write_u32(f.inflation as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    fn pct(n: i64) -> Fixed {
        Fixed::from_ratio(n, 100)
    }

    #[test]
    fn people_are_conserved_except_births_and_deaths() {
        let mut t = PopTable::default();
        for cell in 0..5u32 {
            t.add_for_setup(PopGroup { cell, culture: (cell % 2) as u16, religion: 1, heads: 1000, support: [250; 4] });
        }
        let start = t.total_heads();
        let mut rng = Rng::from_seed(4);
        for _ in 0..5_000 {
            let (a, b) = (rng.below(5) as u32, rng.below(5) as u32);
            let culture = (a % 2) as u16;
            match rng.below(3) {
                0 => {
                    t.transfer(a, b, culture, 1, rng.below(400) as u32);
                }
                1 => {
                    t.births(a, culture, 1, rng.below(20) as u32);
                }
                _ => {
                    t.deaths(a, culture, 1, rng.below(30) as u32);
                }
            }
            assert_eq!(t.total_heads(), start + t.born - t.died);
        }
        // identity survives movement: refugees from a culture-0 cell stay culture 0
        let mut u = PopTable::default();
        u.add_for_setup(PopGroup { cell: 1, culture: 7, religion: 2, heads: 100, support: [100, 200, 300, 400] });
        u.transfer(1, 9, 7, 2, 40);
        let moved = u.groups().iter().find(|g| g.cell == 9).unwrap();
        assert_eq!((moved.culture, moved.religion, moved.heads, moved.support), (7, 2, 40, [100, 200, 300, 400]));
    }

    #[test]
    fn recipes_conserve_and_stop_when_inputs_run_out() {
        let r = Recipe { inputs: vec![(0, 2), (1, 1)], outputs: vec![(2, 3)] };
        let mut s = [10, 3, 0];
        assert_eq!(apply_recipe(&mut s, &r, 100), 3);
        assert_eq!(s, [4, 0, 9]);
        assert_eq!(apply_recipe(&mut s, &r, 5), 0);
    }

    #[test]
    fn staffing_and_prices() {
        assert_eq!(staffing_permille(50, 100), 500);
        assert_eq!(staffing_permille(500, 100), 1000);
        assert_eq!(staffing_permille(0, 0), 1000);
        let base = Fixed::from_int(100);
        // demand twice supply -> price x2, inside a 50%..300% band
        assert_eq!(banded_price(base, 10, 20, pct(50), pct(300)), Fixed::from_int(200));
        assert_eq!(banded_price(base, 10, 100, pct(50), pct(300)), Fixed::from_int(300));
        assert_eq!(banded_price(base, 100, 1, pct(50), pct(300)), Fixed::from_int(50));
        assert_eq!(banded_price(base, 0, 5, pct(50), pct(300)), Fixed::from_int(300));
    }

    #[test]
    fn construction_splits_scarce_goods_and_conserves() {
        let mut e = Economy::default();
        e.configure(2, 1);
        e.stocks_of_mut(0)[0] = 10; // only 10 of good 0, two projects want 10 each
        let t = ProjectTemplate { goods: vec![(0, 10)], labor: 4, min_ticks: 2 };
        e.start_project(ProjectSpec { owner: CountryId(0), region: 0, cell: 5, kind: 1, target_level: 1 }, &t, 0);
        e.start_project(ProjectSpec { owner: CountryId(0), region: 0, cell: 6, kind: 1, target_level: 1 }, &t, 0);
        let before = e.total_of_good(0);
        e.advance_projects(&[100], 0);
        let paid: i64 = e.projects().iter().map(|p| p.goods_paid[0]).sum();
        assert_eq!(paid, 10);
        assert_eq!(e.total_of_good(0) + paid, before, "goods spent are exactly what projects received");
        assert_eq!(e.projects()[0].goods_paid[0], 5);
        assert!(e.building(5, 1).is_none(), "not finished: goods short");
        e.stocks_of_mut(0)[0] = 10;
        let mut done = 0;
        for tick in 1..6 {
            done += e.advance_projects(&[100], tick);
        }
        assert_eq!(done, 2);
        assert_eq!(e.building(5, 1).unwrap().level, 1);
    }

    #[test]
    fn minimum_build_time_is_respected() {
        let mut e = Economy::default();
        e.configure(1, 1);
        e.stocks_of_mut(0)[0] = 100;
        e.start_project(ProjectSpec { owner: CountryId(0), region: 0, cell: 1, kind: 0, target_level: 2 }, &ProjectTemplate { goods: vec![(0, 5)], labor: 1, min_ticks: 10 }, 0);
        assert_eq!(e.advance_projects(&[9], 5), 0);
        assert_eq!(e.advance_projects(&[9], 10), 1);
        assert_eq!(e.building(1, 0).unwrap().level, 2);
    }

    #[test]
    fn money_moves_but_is_not_created_and_damage_lowers_levels() {
        let mut e = Economy::default();
        e.finance_mut(CountryId(0)).treasury = 100;
        assert!(e.pay(CountryId(0), CountryId(3), 60));
        assert!(!e.pay(CountryId(0), CountryId(3), 60));
        assert_eq!(e.total_money(), 100);
        e.set_building(Building { cell: 2, kind: 1, level: 2, condition: 300, owner: CountryId(0) });
        e.damage_building(2, 1, 100);
        assert_eq!(e.building(2, 1).unwrap().condition, 200);
        e.damage_building(2, 1, 250);
        assert_eq!((e.building(2, 1).unwrap().level, e.building(2, 1).unwrap().condition), (1, 1000));
        e.damage_building(2, 1, 5000);
        assert!(e.building(2, 1).is_none());
        e.add_equipment(CountryId(1), 7, 5);
        assert_eq!((e.take_equipment(CountryId(1), 7, 9), e.equipment(CountryId(1), 7)), (5, 0));
    }
}
