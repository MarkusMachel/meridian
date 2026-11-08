//! Loader and validator. `validate_*` reports ALL errors in one pass.
//! `load_*` builds a world only when validation found nothing, so a bad file
//! never produces a partly built world.

use crate::error::{DataError, ErrorKind};
use crate::schema::*;
use meridian_core::{CountryId, ProvinceId, Site, SiteKind, Unit, World};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

/// Just the version, read first so a wrong-version file gets a version error
/// instead of a confusing field error.
#[derive(Deserialize)]
struct Header {
    version: u32,
}

fn read_file(path: &Path) -> Result<(String, String), Vec<DataError>> {
    let name = path.display().to_string();
    match std::fs::read_to_string(path) {
        Ok(t) => Ok((name, t)),
        Err(e) => Err(vec![DataError::new(&name, "-", ErrorKind::Io, e.to_string())]),
    }
}

pub fn validate_scenario_file(path: &Path) -> Result<Scenario, Vec<DataError>> {
    let (name, text) = read_file(path)?;
    validate_scenario_str(&name, &text)
}

pub fn load_scenario_file(path: &Path) -> Result<World, Vec<DataError>> {
    let (name, text) = read_file(path)?;
    load_scenario_str(&name, &text)
}

pub fn load_scenario_str(file: &str, text: &str) -> Result<World, Vec<DataError>> {
    let scenario = validate_scenario_str(file, text)?;
    Ok(build_world(&scenario))
}

/// Parse and check a scenario. Ok means every reference resolves.
pub fn validate_scenario_str(file: &str, text: &str) -> Result<Scenario, Vec<DataError>> {
    // 1. Version first.
    match ron::from_str::<Header>(text) {
        Ok(h) if h.version != SCENARIO_VERSION => {
            return Err(vec![DataError::new(
                file,
                "version",
                ErrorKind::WrongVersion,
                format!("file is version {}, this build reads version {}", h.version, SCENARIO_VERSION),
            )]);
        }
        Ok(_) => {}
        Err(e) => return Err(vec![ron_error(file, &e)]),
    }
    // 2. Full parse.
    let scenario: Scenario = ron::from_str(text).map_err(|e| vec![ron_error(file, &e)])?;
    // 3. Semantic checks, collecting everything.
    let errors = check(file, &scenario);
    if errors.is_empty() { Ok(scenario) } else { Err(errors) }
}

fn ron_error(file: &str, e: &ron::error::SpannedError) -> DataError {
    let at = format!("line {}, column {}", e.span.start.line, e.span.start.col);
    match &e.code {
        ron::Error::MissingStructField { field, outer } => DataError::new(
            file,
            format!("{} ({at})", outer.as_deref().unwrap_or("-")),
            ErrorKind::MissingField,
            format!("missing field `{field}`"),
        ),
        other => DataError::new(file, at, ErrorKind::Malformed, other.to_string()),
    }
}

/// Check ids of one table: unique, then contiguous from 0.
fn check_ids(file: &str, table: &str, ids: &[u32], errors: &mut Vec<DataError>) -> HashSet<u32> {
    let mut seen = HashSet::new();
    for (i, id) in ids.iter().enumerate() {
        if !seen.insert(*id) {
            errors.push(DataError::new(
                file,
                format!("{table}[{i}] (id {id})"),
                ErrorKind::DuplicateId,
                format!("id {id} is used more than once in `{table}`"),
            ));
        }
    }
    // Contiguity: ids 0..n-1 each appear. Skipped when duplicates exist,
    // because the duplicate error already explains the problem.
    let n = seen.len() as u32;
    if seen.len() != ids.len() {
        return seen;
    }
    for expected in 0..n {
        if !seen.contains(&expected) {
            errors.push(DataError::new(
                file,
                table,
                ErrorKind::NonContiguousId,
                format!("ids must run 0..{} with no gaps; id {expected} is missing", n - 1),
            ));
            break;
        }
    }
    seen
}

fn check(file: &str, s: &Scenario) -> Vec<DataError> {
    let mut e = Vec::new();

    if s.countries.is_empty() {
        e.push(DataError::new(file, "countries", ErrorKind::BadValue, "need at least one country"));
    }
    if s.countries.len() > u16::MAX as usize {
        e.push(DataError::new(file, "countries", ErrorKind::BadValue, "too many countries (max 65535)"));
    }
    if s.provinces.is_empty() {
        e.push(DataError::new(file, "provinces", ErrorKind::BadValue, "need at least one province"));
    }

    let country_ids = check_ids(file, "countries", &s.countries.iter().map(|c| c.id).collect::<Vec<_>>(), &mut e);
    let province_ids = check_ids(file, "provinces", &s.provinces.iter().map(|p| p.id).collect::<Vec<_>>(), &mut e);
    check_ids(file, "units", &s.units.iter().map(|u| u.id).collect::<Vec<_>>(), &mut e);
    check_ids(file, "sites", &s.sites.iter().map(|x| x.id).collect::<Vec<_>>(), &mut e);

    let unknown = |e: &mut Vec<DataError>, item: String, field: &str, target: &str, id: u32| {
        e.push(DataError::new(file, item, ErrorKind::UnknownReference, format!("`{field}` refers to {target} {id}, which does not exist")));
    };

    for (i, p) in s.provinces.iter().enumerate() {
        if !country_ids.contains(&p.controller) {
            unknown(&mut e, format!("provinces[{i}] (id {}, \"{}\")", p.id, p.name), "controller", "country", p.controller);
        }
    }
    for (i, u) in s.units.iter().enumerate() {
        let item = format!("units[{i}] (id {})", u.id);
        if !country_ids.contains(&u.owner) {
            unknown(&mut e, item.clone(), "owner", "country", u.owner);
        }
        if !province_ids.contains(&u.location) {
            unknown(&mut e, item, "location", "province", u.location);
        }
    }
    for (i, x) in s.sites.iter().enumerate() {
        let item = format!("sites[{i}] (id {}, \"{}\")", x.id, x.name);
        if !country_ids.contains(&x.owner) {
            unknown(&mut e, item.clone(), "owner", "country", x.owner);
        }
        if !province_ids.contains(&x.province) {
            unknown(&mut e, item.clone(), "province", "province", x.province);
        }
        if x.condition > 100 {
            e.push(DataError::new(file, item.clone(), ErrorKind::BadValue, format!("condition {} is above 100", x.condition)));
        }
        if i16::try_from(x.built_year).is_err() {
            e.push(DataError::new(file, item.clone(), ErrorKind::BadValue, format!("built_year {} out of range", x.built_year)));
        }
        if let Some(d) = x.destroyed_year {
            if i16::try_from(d).is_err() {
                e.push(DataError::new(file, item.clone(), ErrorKind::BadValue, format!("destroyed_year {d} out of range")));
            } else if d < x.built_year {
                e.push(DataError::new(file, item, ErrorKind::BadValue, format!("destroyed_year {d} is before built_year {}", x.built_year)));
            }
        }
    }
    e
}

/// Only call on a scenario that passed `check`.
fn build_world(s: &Scenario) -> World {
    let mut w = World::new(s.seed, s.countries.len() as u16, s.provinces.len() as u32);
    for p in &s.provinces {
        w.set_controller_for_setup(ProvinceId(p.id), CountryId(p.controller as u16));
    }
    let mut units: Vec<&UnitDef> = s.units.iter().collect();
    units.sort_by_key(|u| u.id);
    for u in units {
        w.add_unit_for_setup(Unit { owner: CountryId(u.owner as u16), location: ProvinceId(u.location) });
    }
    let mut sites: Vec<&SiteDef> = s.sites.iter().collect();
    sites.sort_by_key(|x| x.id);
    for x in sites {
        w.add_site_for_setup(Site {
            kind: match x.kind {
                SiteKindDef::City => SiteKind::City,
                SiteKindDef::Port => SiteKind::Port,
                SiteKindDef::Airfield => SiteKind::Airfield,
                SiteKindDef::Crossing => SiteKind::Crossing,
                SiteKindDef::Industry => SiteKind::Industry,
                SiteKindDef::Base => SiteKind::Base,
            },
            province: ProvinceId(x.province),
            owner: CountryId(x.owner as u16),
            condition: x.condition as u8,
            built_year: x.built_year as i16,
            destroyed_year: x.destroyed_year.map(|d| d as i16),
        });
    }
    w
}
