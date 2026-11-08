//! Phase 2 done tests: loading, one test per error class, all-errors-at-once.
use meridian_data::bake::{bake_scenario, unbake_scenario};
use meridian_data::{load_scenario_file, validate_scenario_file, DataError, ErrorKind};
use std::path::PathBuf;

fn fx(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn errs(name: &str) -> Vec<DataError> {
    load_scenario_file(&fx(name)).expect_err("fixture should fail")
}

#[test]
fn sample_scenario_loads_and_hash_is_stable() {
    let h1 = load_scenario_file(&fx("valid.ron")).unwrap().state_hash();
    let h2 = load_scenario_file(&fx("valid.ron")).unwrap().state_hash();
    assert_eq!(h1, h2);
    assert_eq!(h1, GOLDEN_SAMPLE_HASH);
    // The shipped sample is the same file as the fixture.
    let shipped = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/scenarios/sample.ron");
    assert_eq!(load_scenario_file(&shipped).unwrap().state_hash(), h1);
}
const GOLDEN_SAMPLE_HASH: u64 = 6068662019443810327;

fn assert_names_file_and_item(e: &DataError, file: &str) {
    assert!(e.file.ends_with(file), "error should name the file: {e}");
    assert!(!e.item.is_empty() && !e.problem.is_empty(), "error should name item and problem: {e}");
}

#[test]
fn malformed_file() {
    let e = errs("malformed.ron");
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].kind, ErrorKind::Malformed);
    assert_names_file_and_item(&e[0], "malformed.ron");
    assert!(e[0].item.contains("line 5"), "{}", e[0]);
}

#[test]
fn missing_field() {
    let e = errs("missing_field.ron");
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].kind, ErrorKind::MissingField);
    assert!(e[0].problem.contains("controller"), "{}", e[0]);
    assert_names_file_and_item(&e[0], "missing_field.ron");
}

#[test]
fn wrong_version() {
    let e = errs("wrong_version.ron");
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].kind, ErrorKind::WrongVersion);
    assert!(e[0].problem.contains("99"));
}

#[test]
fn duplicate_id() {
    let e = errs("duplicate_id.ron");
    assert!(e.iter().any(|x| x.kind == ErrorKind::DuplicateId && x.item.contains("countries")), "{e:?}");
}

#[test]
fn unknown_reference() {
    let e = errs("unknown_reference.ron");
    let refs: Vec<_> = e.iter().filter(|x| x.kind == ErrorKind::UnknownReference).collect();
    assert_eq!(refs.len(), 2, "{e:?}"); // owner 7 and location 40
    assert!(refs.iter().all(|x| x.item.contains("units[2]")));
}

#[test]
fn missing_file_is_an_io_error() {
    let e = load_scenario_file(&fx("does_not_exist.ron")).unwrap_err();
    assert_eq!(e[0].kind, ErrorKind::Io);
}

#[test]
fn validator_reports_every_error_in_one_pass() {
    let e = validate_scenario_file(&fx("many_errors.ron")).unwrap_err();
    let kinds: Vec<_> = e.iter().map(|x| x.kind).collect();
    // unit owner + unit location + site condition + 2 provinces with controller 9
    assert_eq!(e.len(), 5, "{e:#?}");
    assert_eq!(kinds.iter().filter(|k| **k == ErrorKind::UnknownReference).count(), 4);
    assert_eq!(kinds.iter().filter(|k| **k == ErrorKind::BadValue).count(), 1);
}

#[test]
fn bake_round_trip_and_corruption() {
    let s = validate_scenario_file(&fx("valid.ron")).unwrap();
    let bytes = bake_scenario(&s);
    assert_eq!(bake_scenario(&s), bytes, "baking must be deterministic");
    assert_eq!(unbake_scenario("t", &bytes).unwrap(), s);

    let mut flipped = bytes.clone();
    *flipped.last_mut().unwrap() ^= 1;
    assert_eq!(unbake_scenario("t", &flipped).unwrap_err().kind, ErrorKind::BadBinary);
    assert_eq!(unbake_scenario("t", &bytes[..bytes.len() - 3]).unwrap_err().kind, ErrorKind::BadBinary);
    assert_eq!(unbake_scenario("t", b"nope").unwrap_err().kind, ErrorKind::BadBinary);
    let mut v = bytes.clone();
    v[4] = 2;
    assert_eq!(unbake_scenario("t", &v).unwrap_err().kind, ErrorKind::WrongVersion);
}
