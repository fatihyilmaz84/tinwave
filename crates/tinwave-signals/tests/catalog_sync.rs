//! Keeps `SignalKind` and the payload field lists in sync with `docs/guides/signal-catalog.md`.
//!
//! The "fixture" is the catalog itself: the markdown file is embedded at compile time and parsed
//! here, so there is no second hand-typed list to drift. If someone adds, renames, re-tiers or
//! re-classes a signal in the catalog (or changes a payload field list) without updating
//! `tinwave-signals`, this test fails; the same holds the other way round.
//!
//! What is read from the catalog:
//! - every row of a table whose header starts with `Signal`. Columns are found by header name
//!   (`Signal`, `Tier`, `Payload`), so an extra column cannot shift them. From a row: the names,
//!   the tier (`T0`..`T3`, `derived`, `meta`) and the payload column (field names are the
//!   lowercase backticked words; a backticked `A|B|C` after a field lists that field's enum
//!   values, which must be compared with the code or the test fails);
//! - the sentence naming the storage class **C** signals;
//! - the "Common enums" bullets for `InputSource` and `Section`.
//!
//! Documented mappings between catalog and code (everything else must match exactly):
//! - `pos_ds` in a payload is the event's own `pos_ds` column, not a packed field.
//! - A quantized field keeps its catalog name in `FieldSpec::catalog` (`gesture_ms` is stored as
//!   50 ms steps, `listened_ms` as deciseconds); `SearchNoResult.query` is stored as a hash.
//! - `SignalDropped` (tier `meta`) is in-memory diagnostics and has no kind.
//! - Derived signals have no tier of their own in the catalog; they take the highest tier among
//!   their inputs (`SignalKind::derived_inputs`), so `PerceivedLevel` is T1 and `RateSession` T0.

// Test code: a failed unwrap is the failure report. clippy.toml only exempts `#[test]` bodies, not
// helpers in integration-test crates.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};

use tinwave_core::Tier;
use tinwave_signals::*;

const CATALOG: &str = include_str!("../../../docs/guides/signal-catalog.md");

/// Words in the payload column that look like field names but are types.
const TYPE_WORDS: &[&str] = &["bool", "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64"];

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

/// Column positions of a signal table, looked up by header name so a new column (for example
/// "bytes/day") cannot shift the ones we read.
struct Columns {
    count: usize,
    signal: usize,
    tier: usize,
    payload: usize,
}

impl Columns {
    fn from_header(header: &[String]) -> Self {
        let find = |name: &str| {
            let mut matches = header.iter().enumerate().filter(|(_, h)| *h == name);
            let (index, _) = matches.next().unwrap_or_else(|| {
                panic!("signal table header has no {name:?} column: {header:?}")
            });
            assert!(matches.next().is_none(), "duplicate {name:?} column");
            index
        };
        Self {
            count: header.len(),
            signal: find("Signal"),
            tier: find("Tier"),
            payload: find("Payload"),
        }
    }
}

fn parse_rows() -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    let mut columns: Option<Columns> = None;
    for line in CATALOG.lines() {
        if !line.trim_start().starts_with('|') {
            columns = None;
            continue;
        }
        let cells = split_cells(line);
        if cells.first().map(String::as_str) == Some("Signal") {
            columns = Some(Columns::from_header(&cells));
            continue;
        }
        let Some(columns) = &columns else { continue };
        if cells
            .iter()
            .all(|c| c.chars().all(|ch| ch == '-' || ch == ':'))
        {
            continue;
        }
        assert_eq!(
            cells.len(),
            columns.count,
            "signal row does not have one cell per header column: {line}"
        );

        let names = code_spans(&cells[columns.signal]);
        let payload = &cells[columns.payload];
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
            tier: cells[columns.tier].clone(),
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
                // The catalog gives no tier for derived rows; the rule is "highest tier among
                // the inputs" (`SignalKind::derived_inputs`).
                let highest = kind.derived_inputs().iter().map(|i| i.tier()).max();
                assert_eq!(Some(kind.tier()), highest, "{name}: derived tier");
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

/// Rows that cover several kinds but have a single payload column, where the catalog does not say
/// which kind carries which field. This is the documented reading of those rows:
/// `Paused` has no `pause_ms` ("on resume"), and a session's `start_origin` is known at the
/// start while `tracks` and `minutes` only exist at the end.
///
/// Every other multi-kind row (`QueuedNext / QueuedLater`, `PlaylistAdded / PlaylistRemoved`,
/// `Favorited / Unfavorited`) is checked strictly: **each** kind must carry **every** field the
/// row lists. A kind listed here must still cover the row together with its siblings.
const SPLIT_ROWS: &[(&str, &[&str])] = &[
    ("Paused", &["input"]),
    ("Resumed", &["pause_ms", "input"]),
    ("SessionStarted", &["start_origin"]),
    ("SessionEnded", &["tracks", "minutes"]),
];

fn split_fields(name: &str) -> Option<BTreeSet<String>> {
    SPLIT_ROWS
        .iter()
        .find(|(kind, _)| *kind == name)
        .map(|(_, fields)| fields.iter().map(|f| (*f).to_owned()).collect())
}

#[test]
fn payload_fields_match_the_catalog() {
    let rows = parse_rows();
    let kinds = kinds_by_name();
    let mut used_splits = BTreeSet::new();
    for row in rows.iter().filter(|r| r.tier != "meta") {
        let split_names: Vec<&String> = row
            .names
            .iter()
            .filter(|n| split_fields(n).is_some())
            .collect();
        if !split_names.is_empty() {
            assert_eq!(
                split_names.len(),
                row.names.len(),
                "{:?}: a row is either fully listed in SPLIT_ROWS or not at all",
                row.names
            );
        }
        let mut union = BTreeSet::new();
        for name in &row.names {
            let kind = kinds[name.as_str()];
            let code_fields: BTreeSet<String> = kind
                .payload_fields()
                .iter()
                .map(|f| f.catalog.to_owned())
                .collect();
            if !kind.is_event() {
                assert!(
                    code_fields.is_empty(),
                    "{name}: only raw events have a packed payload"
                );
                continue;
            }
            // Expected catalog fields of this kind: its SPLIT_ROWS entry, otherwise everything the
            // row lists.
            let expected = match split_fields(name) {
                Some(split) => {
                    used_splits.insert(name.clone());
                    assert!(
                        split.is_subset(&row.fields),
                        "{name}: SPLIT_ROWS lists fields the catalog row does not: {:?}",
                        split.difference(&row.fields).collect::<Vec<_>>()
                    );
                    split
                }
                None => row.fields.clone(),
            };
            assert_eq!(
                code_fields, expected,
                "{name}: packed payload fields differ from the catalog row {:?}",
                row.fields
            );
            union.extend(code_fields);
        }
        if row.names.iter().all(|n| kinds[n.as_str()].is_event()) {
            assert_eq!(
                union, row.fields,
                "{:?}: catalog payload fields not covered by the kinds' packed payloads",
                row.names
            );
        }
    }
    let all_splits: BTreeSet<String> = SPLIT_ROWS.iter().map(|(k, _)| (*k).to_owned()).collect();
    assert_eq!(used_splits, all_splits, "stale SPLIT_ROWS entry");
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

/// The code enum behind a payload field, for catalog value lists `field (`A|B|C`)`.
///
/// Every enum-valued payload field is mapped here even when the catalog does not list its values
/// yet, so the moment a list appears it is compared with the code. `input` is the one field
/// handled elsewhere (`common_enums_match_the_code` merges its values with the Common enums
/// bullet). A value list for any other field fails the test instead of being skipped.
fn enum_names_for(row: &Row, field: &str) -> Option<BTreeSet<String>> {
    Some(match field {
        "queue_origin" => names_of(QueueOrigin::ALL, QueueOrigin::name),
        "origin" | "start_origin" => names_of(PlayOrigin::ALL, PlayOrigin::name),
        "class" | "endpoint_class" => names_of(OutputClass::ALL, OutputClass::name),
        "transport" => names_of(OutputTransport::ALL, OutputTransport::name),
        "mode" => names_of(RepeatMode::ALL, RepeatMode::name),
        "from_view" | "view" => names_of(View::ALL, View::name),
        "playlist_kind" => names_of(PlaylistKind::ALL, PlaylistKind::name),
        "scope" => names_of(SearchScope::ALL, SearchScope::name),
        "entity_kind" => names_of(EntityKind::ALL, EntityKind::name),
        "state" => names_of(WindowStateKind::ALL, WindowStateKind::name),
        // `kind` is also a field of the in-memory `SignalDropped`; only RadioFeedback's is an enum.
        "kind" if row.names.iter().any(|n| n == "RadioFeedback") => {
            names_of(RadioFeedbackKind::ALL, RadioFeedbackKind::name)
        }
        _ => return None,
    })
}

#[test]
fn enum_values_listed_in_payload_columns_match_the_code() {
    let rows = parse_rows();
    let mut checked = BTreeSet::new();
    for row in rows.iter().filter(|r| r.tier != "meta") {
        for (field, values) in &row.enum_values {
            if field == "input" {
                continue;
            }
            let code_values = enum_names_for(row, field).unwrap_or_else(|| {
                panic!(
                    "{:?}.{field}: the catalog lists values {values:?} for a field this test \
                     does not know; map it in enum_names_for",
                    row.names
                )
            });
            assert_eq!(values, &code_values, "{:?}.{field}", row.names);
            checked.insert(field.clone());
        }
    }
    // Value lists the catalog has today; if one is dropped the parser is probably broken.
    for field in ["queue_origin", "origin", "class", "transport"] {
        assert!(checked.contains(field), "no value list found for {field}");
    }
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
