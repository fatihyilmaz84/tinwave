//! Keeps `SignalKind` and the payload field lists in sync with `docs/guides/signal-catalog.md`.
//!
//! The "fixture" is the catalog itself: the markdown file is embedded at compile time and parsed
//! here, so there is no second hand-typed list to drift. If someone adds, renames, re-tiers or
//! re-classes a signal in the catalog (or changes a payload field list) without updating
//! `tinwave-signals`, this test fails; the same holds the other way round.
//!
//! What is read from the catalog:
//! - every row of a table whose header starts with `Signal`: the names in the first column, the
//!   tier (`T0`..`T3`, `derived`, `meta`) and the payload column (field names are the lowercase
//!   backticked words; a backticked `A|B|C` after a field lists that field's enum values);
//! - the sentence naming the storage class **C** signals;
//! - the "Common enums" bullets for `InputSource` and `Section`.
//!
//! Documented mappings between catalog and code (everything else must match exactly):
//! - `pos_ds` in a payload is the event's own `pos_ds` column, not a packed field.
//! - A quantized field keeps its catalog name in `FieldSpec::catalog` (`gesture_ms` is stored as
//!   50 ms steps, `listened_ms` as deciseconds); `SearchNoResult.query` is stored as a hash.
//! - `SignalDropped` (tier `meta`) is in-memory diagnostics and has no kind.
//! - Derived signals have no tier of their own in the catalog; the code assigns T0 (the tier of
//!   their inputs).

// Test code: a failed unwrap is the failure report. clippy.toml only exempts `#[test]` bodies, not
// helpers in integration-test crates.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};

use tinwave_core::Tier;
use tinwave_signals::*;

const CATALOG: &str = include_str!("../../../docs/guides/signal-catalog.md");

/// Words in the payload column that look like field names but are types.
const TYPE_WORDS: &[&str] = &["u8", "u16", "u32"];

#[derive(Debug, Clone)]
struct Row {
    names: Vec<String>,
    tier: String,
    /// Catalog field names, in catalog order, without `pos_ds`.
    fields: BTreeSet<String>,
    /// Enum values listed for a field, e.g. `queue_origin` -> {Manual, Radio, ...}.
    enum_values: BTreeMap<String, BTreeSet<String>>,
}

/// Splits a markdown table row into cells; `|` inside a backtick span does not split
/// (the catalog writes enum lists as `` `A|B|C` ``).
fn split_cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut in_code = false;
    for ch in line.chars() {
        match ch {
            '`' => {
                in_code = !in_code;
                current.push(ch);
            }
            '|' if !in_code => {
                cells.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    cells.push(current.trim().to_owned());
    cells
}

/// The contents of every backtick span in `text`, in order.
fn code_spans(text: &str) -> Vec<String> {
    text.split('`')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, span)| span.to_owned())
        .collect()
}

fn is_field_name(span: &str) -> bool {
    let mut chars = span.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && span
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !TYPE_WORDS.contains(&span)
}

fn parse_rows() -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    let mut in_signal_table = false;
    for line in CATALOG.lines() {
        if !line.trim_start().starts_with('|') {
            in_signal_table = false;
            continue;
        }
        let cells = split_cells(line);
        if cells.first().map(String::as_str) == Some("Signal") {
            in_signal_table = true;
            continue;
        }
        if !in_signal_table
            || cells
                .iter()
                .all(|c| c.chars().all(|ch| ch == '-' || ch == ':'))
        {
            continue;
        }
        assert!(cells.len() >= 5, "signal row with too few cells: {line}");

        let names = code_spans(&cells[0]);
        let payload = &cells[3];
        let mut fields = BTreeSet::new();
        let mut enum_values: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

        if payload.trim_start().starts_with("same") {
            // "same as above" / "same": start from the previous row's payload.
            let previous = rows.last().expect("'same' row without a previous row");
            fields = previous.fields.clone();
            enum_values = previous.enum_values.clone();
        }
        let mut last_field: Option<String> = None;
        for span in code_spans(payload) {
            if is_field_name(&span) {
                fields.insert(span.clone());
                last_field = Some(span);
            } else if TYPE_WORDS.contains(&span.as_str()) {
                continue;
            } else if let Some(field) = &last_field {
                let values = enum_values.entry(field.clone()).or_default();
                values.extend(span.split('|').map(|v| v.trim().to_owned()));
            }
        }
        fields.remove("pos_ds");

        rows.push(Row {
            names,
            tier: cells[1].clone(),
            fields,
            enum_values,
        });
    }
    rows
}

/// Names the catalog declares as storage class **C**.
fn counter_names() -> BTreeSet<String> {
    let line = CATALOG
        .lines()
        .find(|l| l.contains("storage class **C**"))
        .expect("catalog names the storage class C signals");
    let before = line
        .split("are storage class")
        .next()
        .expect("split always yields one item");
    code_spans(before).into_iter().collect()
}

fn kinds_by_name() -> BTreeMap<&'static str, SignalKind> {
    SignalKind::ALL.iter().map(|k| (k.name(), *k)).collect()
}

fn catalog_names(rows: &[Row]) -> BTreeSet<String> {
    rows.iter()
        .filter(|r| r.tier != "meta")
        .flat_map(|r| r.names.iter().cloned())
        .collect()
}

#[test]
fn parser_finds_the_catalog() {
    let rows = parse_rows();
    let names = catalog_names(&rows);
    assert!(names.len() >= 40, "parsed only {} signals", names.len());
    for anchor in [
        "VolumeChanged",
        "SeekBack",
        "SearchNoResult",
        "ExplanationOpened",
    ] {
        assert!(names.contains(anchor), "{anchor} not parsed");
    }
    assert_eq!(
        counter_names(),
        ["Impression", "ArtworkHover", "PageDwell"]
            .map(str::to_owned)
            .into_iter()
            .collect()
    );
}

#[test]
fn every_catalog_signal_has_a_kind_and_every_kind_is_in_the_catalog() {
    let rows = parse_rows();
    let in_catalog = catalog_names(&rows);
    let in_code: BTreeSet<String> = SignalKind::ALL
        .iter()
        .map(|k| k.name().to_owned())
        .collect();
    assert_eq!(
        in_catalog.difference(&in_code).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "catalog signals without a SignalKind"
    );
    assert_eq!(
        in_code.difference(&in_catalog).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "SignalKind variants that are not in the catalog"
    );
}

#[test]
fn meta_signals_have_no_kind() {
    let rows = parse_rows();
    let meta: Vec<&str> = rows
        .iter()
        .filter(|r| r.tier == "meta")
        .flat_map(|r| r.names.iter().map(String::as_str))
        .collect();
    assert_eq!(meta, ["SignalDropped"]);
    assert!(!kinds_by_name().contains_key("SignalDropped"));
}

#[test]
fn storage_class_and_tier_match_the_catalog() {
    let rows = parse_rows();
    let counters = counter_names();
    let kinds = kinds_by_name();
    for row in rows.iter().filter(|r| r.tier != "meta") {
        for name in &row.names {
            let kind = kinds[name.as_str()];
            let expected_class = if row.tier == "derived" {
                StorageClass::Derived
            } else if counters.contains(name) {
                StorageClass::Counter
            } else {
                StorageClass::Event
            };
            assert_eq!(
                kind.storage_class(),
                expected_class,
                "{name}: storage class"
            );

            if row.tier == "derived" {
                assert_eq!(kind.tier(), Tier::T0, "{name}: derived signals use T0");
            } else {
                let number: u8 = row
                    .tier
                    .trim_start_matches('T')
                    .parse()
                    .unwrap_or_else(|_| panic!("{name}: unreadable tier {:?}", row.tier));
                assert_eq!(kind.tier(), Tier::try_from(number).unwrap(), "{name}: tier");
            }
        }
    }
}

#[test]
fn payload_fields_match_the_catalog() {
    let rows = parse_rows();
    let kinds = kinds_by_name();
    for row in rows.iter().filter(|r| r.tier != "meta") {
        let mut union = BTreeSet::new();
        for name in &row.names {
            let kind = kinds[name.as_str()];
            let code_fields: BTreeSet<String> = kind
                .payload_fields()
                .iter()
                .map(|f| f.catalog.to_owned())
                .collect();
            if kind.is_event() {
                assert!(
                    code_fields.is_subset(&row.fields),
                    "{name}: payload stores {:?}, which the catalog row does not list ({:?})",
                    code_fields.difference(&row.fields).collect::<Vec<_>>(),
                    row.fields
                );
                union.extend(code_fields);
            } else {
                assert!(
                    code_fields.is_empty(),
                    "{name}: only raw events have a packed payload"
                );
            }
        }
        // Rows covering several kinds (`Paused / Resumed`) may split the fields between them,
        // but together they must cover the whole catalog payload.
        if row.names.iter().all(|n| kinds[n.as_str()].is_event()) {
            assert_eq!(
                union, row.fields,
                "{:?}: catalog payload fields not covered by the kinds' packed payloads",
                row.names
            );
        }
    }
}

#[test]
fn payload_field_names_are_unique_per_kind() {
    for kind in SignalKind::events() {
        let names: BTreeSet<_> = kind.payload_fields().iter().map(|f| f.name).collect();
        let catalog: BTreeSet<_> = kind.payload_fields().iter().map(|f| f.catalog).collect();
        assert_eq!(names.len(), kind.payload_fields().len(), "{kind}");
        assert_eq!(catalog.len(), kind.payload_fields().len(), "{kind}");
    }
}

fn names_of<T: Copy>(all: &[T], name: impl Fn(T) -> &'static str) -> BTreeSet<String> {
    all.iter().map(|v| name(*v).to_owned()).collect()
}

#[test]
fn enum_values_listed_in_payload_columns_match_the_code() {
    let rows = parse_rows();
    let expected: BTreeMap<&str, BTreeSet<String>> = BTreeMap::from([
        (
            "queue_origin",
            names_of(QueueOrigin::ALL, QueueOrigin::name),
        ),
        ("origin", names_of(PlayOrigin::ALL, PlayOrigin::name)),
        ("class", names_of(OutputClass::ALL, OutputClass::name)),
        (
            "transport",
            names_of(OutputTransport::ALL, OutputTransport::name),
        ),
    ]);
    let mut checked = BTreeSet::new();
    for row in &rows {
        for (field, values) in &row.enum_values {
            if let Some(code_values) = expected.get(field.as_str()) {
                assert_eq!(values, code_values, "{:?}.{field}", row.names);
                checked.insert(field.clone());
            }
        }
    }
    assert_eq!(
        checked,
        expected.keys().map(|k| (*k).to_owned()).collect(),
        "every enum with a value list in the catalog must be checked"
    );
}

#[test]
fn common_enums_match_the_code() {
    let rows = parse_rows();

    let bullet = |name: &str| -> BTreeSet<String> {
        let line = CATALOG
            .lines()
            .find(|l| l.starts_with(&format!("- `{name}`")))
            .unwrap_or_else(|| panic!("no Common enums bullet for {name}"));
        let list = code_spans(line)
            .into_iter()
            .find(|s| s.contains('|'))
            .unwrap_or_else(|| panic!("no value list in the {name} bullet"));
        list.split('|')
            .map(|v| v.trim().split('(').next().unwrap_or("").trim().to_owned())
            .collect()
    };

    // InputSource: the bullet, plus values that payload columns add (`System` on
    // SystemVolumeChanged, ADR 0004).
    let mut input = bullet("InputSource");
    for row in &rows {
        if let Some(extra) = row.enum_values.get("input") {
            input.extend(extra.iter().cloned());
        }
    }
    assert_eq!(
        input,
        InputSource::NAMES
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>()
    );

    assert_eq!(
        bullet("Section"),
        names_of(SectionKind::ALL, SectionKind::name)
    );
}
