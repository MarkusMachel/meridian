//! Phase 4 done test: the tiny economy conserves goods and money, and drafting lowers output.

use meridian_core::{apply_recipe, staffing_permille, CountryId, PopGroup, Rate, Recipe, ScheduleBuilder, Signals, System, World};

const FOOD: usize = 0;
const GOODS: usize = 1;

fn recipe() -> Recipe {
    // 2 food + labor -> 1 consumer good
    Recipe { inputs: vec![(FOOD as u16, 2)], outputs: vec![(GOODS as u16, 1)] }
}

/// Daily: each region runs the recipe up to 100 times, scaled by how well it is staffed.
fn production(w: &mut World, _: Signals) {
    for r in 0..w.econ().regions {
        let workers = w.econ().pop.heads_in_cell(r) as i64;
        let runs = 100 * staffing_permille(workers, 1000) / 1000;
        apply_recipe(w.econ_mut().stocks_of_mut(r), &recipe(), runs);
    }
}

/// Daily: country 0 pays a flat tax to country 1 (money moves, never appears).
fn tax(w: &mut World, _: Signals) {
    w.econ_mut().pay(CountryId(0), CountryId(1), 10);
}

fn world() -> World {
    let mut w = World::new(1, 2, 2);
    let e = w.econ_mut();
    e.configure(2, 2);
    for r in 0..2u32 {
        e.stocks_of_mut(r)[FOOD] = 1_000_000;
        e.pop.add_for_setup(PopGroup { cell: r, culture: 0, religion: 0, heads: 1000, support: [250; 4] });
    }
    e.finance_mut(CountryId(0)).treasury = 100_000;
    w
}

fn run(draft_at_day: Option<u64>) -> World {
    let rules = ScheduleBuilder::new()
        .system(System { name: "production", rate: Rate::Daily, after: &[], run: production })
        .system(System { name: "tax", rate: Rate::Daily, after: &["production"], run: tax })
        .build()
        .unwrap();
    let mut w = world();
    for t in 0..30 * 24 {
        if draft_at_day.is_some_and(|d| t == d * 24) {
            // drafting: half of region 0's people leave civilian work (counted as deaths from the economy's view)
            w.econ_mut().pop.deaths(0, 0, 0, 500);
        }
        w.tick(&[], &rules);
    }
    w
}

#[test]
fn goods_and_money_are_conserved() {
    let w = run(None);
    let e = w.econ();
    let made = e.total_of_good(GOODS as u16);
    let used = 2_000_000 - e.total_of_good(FOOD as u16);
    assert_eq!(used, made * 2, "every consumer good cost exactly 2 food");
    assert_eq!(made, 2 * 30 * 100, "fully staffed: 100 runs a day in both regions");
    assert_eq!(e.total_money(), 100_000, "tax moves money, never creates it");
    assert_eq!(e.finance(CountryId(1)).treasury, 300);
}

#[test]
fn drafting_lowers_output() {
    let free = run(None).econ().total_of_good(GOODS as u16);
    let drafted = run(Some(10));
    assert!(drafted.econ().total_of_good(GOODS as u16) < free);
    // days 0..10 full (2*100 a day), days 10..30 region 0 at 50%: 100 + 50 per day
    assert_eq!(drafted.econ().total_of_good(GOODS as u16), 10 * 200 + 20 * 150);
}
