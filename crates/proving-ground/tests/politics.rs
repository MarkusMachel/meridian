//! Phase 4 done tests: a tiny political loop and a tiny diplomacy loop.

use meridian_core::{Command, CountryId, Faction, PopGroup, ProvinceId, Rejection, ScheduleBuilder, Unit, UnitId, Vote, World};

fn c(n: u16) -> CountryId {
    CountryId(n)
}

fn world() -> World {
    let mut w = World::new(1, 3, 6);
    for p in 0..6 {
        w.set_controller_for_setup(ProvinceId(p), c(0));
    }
    for k in 0..4u32 {
        w.add_unit_for_setup(Unit { owner: c(0), location: ProvinceId(k) });
    }
    w.econ_mut().configure(2, 6);
    for r in 0..6u32 {
        w.econ_mut().stocks_of_mut(r)[0] = 100 + r as i64;
        w.econ_mut().pop.add_for_setup(PopGroup { cell: r, culture: 0, religion: 0, heads: 1000 + r, support: [250; 4] });
    }
    w.econ_mut().finance_mut(c(0)).treasury = 1001;
    w.econ_mut().finance_mut(c(0)).debt = 333;
    w.econ_mut().add_equipment(c(0), 4, 11);
    w
}

#[test]
fn a_command_outside_the_war_level_is_refused_and_levels_only_rise() {
    let rules = ScheduleBuilder::new().build().unwrap();
    let mut w = world();
    let strike = Command::HostileAct { issuer: c(0), against: c(1), required_level: 2 };
    assert!(matches!(w.tick(std::slice::from_ref(&strike), &rules).rejected[0].1, Rejection::Refused(_)));
    let r = w.tick(&[Command::SetWarLevel { issuer: c(0), against: c(1), level: 2 }, strike.clone()], &rules);
    assert!(r.rejected.is_empty());
    // the other side is still at level 0 against us
    let back = Command::HostileAct { issuer: c(1), against: c(0), required_level: 1 };
    assert_eq!(w.tick(&[back], &rules).rejected.len(), 1);
    let down = Command::SetWarLevel { issuer: c(0), against: c(1), level: 1 };
    assert_eq!(w.tick(&[down], &rules).rejected.len(), 1);
}

#[test]
fn a_law_change_passes_or_stalls_by_faction_support() {
    let mut w = world();
    w.polity_mut().set_faction(Faction { country: c(0), kind: 0, support: 600, cohesion: 1000 });
    w.polity_mut().set_faction(Faction { country: c(0), kind: 1, support: 400, cohesion: 1000 });
    assert!(!w.polity_mut().try_change_law(c(0), 1, 1, |k| k == 1, 500));
    assert!(w.polity_mut().try_change_law(c(0), 1, 1, |k| k == 0, 500));
}

#[test]
fn one_unit_defects_and_a_civil_war_split_conserves_everything() {
    let mut w = world();
    // defection: unit 3's loyalty collapses, so it changes sides
    let new = w.create_country();
    w.polity_mut().set_unit_loyalty(3, 100);
    assert!(w.polity_mut().unit_loyalty(3) < 300 && w.set_unit_owner(UnitId(3), new));
    assert_eq!(w.unit(UnitId(3)).unwrap().owner, new);

    let people = w.econ().pop.total_heads();
    let goods: i64 = w.econ().total_of_good(0);
    let money = w.econ().total_money();
    let debt: i64 = (0..4).map(|i| w.econ().finance(c(i)).debt).sum();
    let guns: u64 = (0..4).map(|i| w.econ().equipment(c(i), 4)).sum();
    let units = (0..w.unit_count()).filter(|&i| w.unit(UnitId(i)).is_some()).count();

    let rebel = w.split_country(c(0), &[ProvinceId(4), ProvinceId(5)], &[UnitId(1), UnitId(2)], 333).unwrap();
    assert_eq!(rebel.0, 4, "a new country gets a new ID; the original government keeps its own");
    assert_eq!(w.controller(ProvinceId(4)), rebel);
    assert_eq!(w.controller(ProvinceId(0)), c(0));
    assert_eq!(w.unit(UnitId(1)).unwrap().owner, rebel);
    assert_eq!(w.unit(UnitId(0)).unwrap().owner, c(0));

    assert_eq!(w.econ().pop.total_heads(), people);
    assert_eq!(w.econ().total_of_good(0), goods);
    assert_eq!(w.econ().total_money(), money);
    assert_eq!((0..5).map(|i| w.econ().finance(c(i)).debt).sum::<i64>(), debt);
    assert_eq!((0..5).map(|i| w.econ().equipment(c(i), 4)).sum::<u64>(), guns);
    assert_eq!((0..w.unit_count()).filter(|&i| w.unit(UnitId(i)).is_some()).count(), units);
    assert!(w.econ().finance(rebel).treasury > 0 && w.econ().finance(c(0)).treasury > 0);
}

#[test]
fn a_major_treaty_stalls_until_every_partys_factions_approve() {
    let rules = ScheduleBuilder::new().build().unwrap();
    let mut w = world();
    for country in [c(0), c(1)] {
        w.polity_mut().set_faction(Faction { country, kind: 0, support: 700, cohesion: 1000 });
        w.polity_mut().set_faction(Faction { country, kind: 1, support: 300, cohesion: 1000 });
    }
    let t = w.diplomacy_mut().propose_treaty(9, vec![0, 1], true, None);
    // country 1's biggest faction opposes this kind of treaty: 300 of 1000 in favor < 500
    w.polity_mut().set_opposes(c(1), 0, 9, true);
    let sign = |country| Command::SignTreaty { issuer: country, treaty: t };
    assert_eq!(w.tick(&[sign(c(0))], &rules).rejected.len(), 1, "stalls");
    w.polity_mut().set_opposes(c(1), 0, 9, false);
    assert!(w.tick(&[sign(c(0))], &rules).rejected.is_empty());
    assert!(w.diplomacy().in_force(t, w.clock().tick()));
    assert_eq!(w.tick(&[sign(c(2))], &rules).rejected.len(), 1, "not a party");
}

#[test]
fn an_alliance_call_lets_each_member_decide_and_a_un_vote_respects_a_veto() {
    let mut w = world();
    let a = w.diplomacy_mut().add_alliance(0, vec![0, 1, 2]);
    let call = w.diplomacy_mut().call_alliance(a, c(0), c(2)).unwrap();
    assert!(w.diplomacy_mut().decide(call, c(1), true));
    assert!(w.diplomacy_mut().decide(call, c(2), false));
    w.diplomacy_mut().set_council(vec![(0, true), (1, false), (2, false)]);
    let votes = [(c(0), Vote::No), (c(1), Vote::Yes), (c(2), Vote::Yes)];
    assert!(!w.diplomacy().council_vote(&votes, 2), "country 0 vetoes");
    assert!(w.diplomacy().council_vote(&[(c(0), Vote::Abstain), (c(1), Vote::Yes), (c(2), Vote::Yes)], 2));
}

#[test]
fn dry_run_checks_without_changing_anything() {
    let rules = ScheduleBuilder::new().build().unwrap();
    let w = world();
    let before = w.state_hash();
    assert!(w.dry_run(&Command::SetWarLevel { issuer: c(0), against: c(1), level: 1 }, &rules).is_ok());
    assert!(w.dry_run(&Command::SetWarLevel { issuer: c(0), against: c(0), level: 1 }, &rules).is_err());
    assert_eq!(w.state_hash(), before);
}

#[test]
fn a_polled_event_waits_for_the_countrys_choice_through_the_command_interface() {
    use meridian_core::{Condition, Effect, EventDef, EventSet, Rate};
    let def = EventDef {
        id: 1,
        country: c(0),
        trigger: Condition::Always,
        options: vec![vec![Effect::AddValue { province: ProvinceId(0), amount: 7 }]],
        cooldown: 1_000_000,
        chain: None,
        severity: 2,
    };
    let rules = ScheduleBuilder::new().events(EventSet::new(vec![def], Rate::Daily).unwrap()).build().unwrap();
    let mut w = world();
    w.tick(&[], &rules); // tick 0 is a new day: the event is raised and waits
    assert_eq!(w.event_state().pending().len(), 1);
    let choose = |issuer, option| Command::ChooseEvent { issuer, event: 1, option };
    assert_eq!(w.tick(&[choose(c(1), 0)], &rules).rejected.len(), 1, "not their event");
    assert_eq!(w.tick(&[choose(c(0), 5)], &rules).rejected.len(), 1, "no such option");
    assert!(w.tick(&[choose(c(0), 0)], &rules).rejected.is_empty());
    assert_eq!(w.value(ProvinceId(0)), 7);
    assert!(w.event_state().pending().is_empty());
}
