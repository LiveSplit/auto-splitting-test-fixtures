//! Finds the patches where a measurement changes, across every release of
//! the minors given, with as few measurements as a bisect allows.
//!
//! Patches inside one minor come out in order, so a change inside a minor
//! stays changed, and the first and the last patch of the minor tell
//! whether anything moved in between. A range whose ends differ splits in
//! the middle until the two patches on either side of the move sit next to
//! each other. The last patch of one minor is also held against the first
//! patch of the next, which catches a change that only lands in a later
//! minor or that a backport takes back.

use std::{collections::HashMap, fmt::Display, io};

/// One boundary: the first release with the new measurement, and the last
/// release with the old one.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Boundary {
    pub(crate) last_old: String,
    pub(crate) first_new: String,
}

/// Runs the bisect over `releases`, in order, with `minor_starts` saying
/// which indexes begin a minor. `measure` gives a release's measurement,
/// or `None` where nothing could be measured, and is called at most once
/// per release. `report` hears about every measurement as it lands.
pub(crate) fn boundaries<M: PartialEq + Clone + Display>(
    releases: &[String],
    minor_starts: &[usize],
    mut measure: impl FnMut(&str) -> io::Result<Option<M>>,
    mut report: impl FnMut(&str, &str),
) -> io::Result<Vec<Boundary>> {
    let mut seen: HashMap<usize, Option<M>> = HashMap::new();
    let mut at = |index: usize, seen: &mut HashMap<usize, Option<M>>| -> io::Result<Option<M>> {
        if let Some(found) = seen.get(&index) {
            return Ok(found.clone());
        }
        let found = measure(&releases[index])?;
        report(
            &releases[index],
            &found
                .as_ref()
                .map_or("nothing measured".to_string(), |m| m.to_string()),
        );
        seen.insert(index, found.clone());
        Ok(found)
    };

    // The edges: the first and the last patch of every minor.
    let mut edges = Vec::new();
    for (i, &start) in minor_starts.iter().enumerate() {
        let end = minor_starts.get(i + 1).map_or(releases.len(), |&next| next) - 1;
        if start <= end {
            edges.push(start);
            if end != start {
                edges.push(end);
            }
        }
    }
    let mut known = Vec::new();
    for &edge in &edges {
        if at(edge, &mut seen)?.is_some() {
            known.push(edge);
        }
    }

    let mut found = Vec::new();
    let mut ranges: Vec<(usize, usize)> = known.windows(2).map(|pair| (pair[0], pair[1])).collect();
    while let Some((low, high)) = ranges.pop() {
        let (at_low, at_high) = (at(low, &mut seen)?, at(high, &mut seen)?);
        if at_low == at_high {
            continue;
        }
        if high == low + 1 {
            found.push((low, high));
            continue;
        }
        // The measurable patch nearest the middle. A range with none left
        // in between is reported as it is.
        let middle = (low + high) / 2;
        let mut split = None;
        for step in 0..high - low {
            for index in [middle.wrapping_sub(step), middle + step] {
                if low < index && index < high && at(index, &mut seen)?.is_some() {
                    split = Some(index);
                    break;
                }
            }
            if split.is_some() {
                break;
            }
        }
        match split {
            Some(middle) => {
                ranges.push((middle, high));
                ranges.push((low, middle));
            }
            None => found.push((low, high)),
        }
    }
    found.sort();
    Ok(found
        .into_iter()
        .map(|(low, high)| Boundary {
            last_old: releases[low].clone(),
            first_new: releases[high].clone(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn releases(minors: &[&[&str]]) -> (Vec<String>, Vec<usize>) {
        let mut all = Vec::new();
        let mut starts = Vec::new();
        for minor in minors {
            starts.push(all.len());
            all.extend(minor.iter().map(|s| s.to_string()));
        }
        (all, starts)
    }

    // Measures by a table, counting the calls, so a test can say both what
    // the bisect found and how much it cost.
    fn run(minors: &[&[&str]], table: &[(&str, Option<u32>)]) -> (Vec<Boundary>, usize) {
        let (all, starts) = releases(minors);
        let calls = RefCell::new(0);
        let found = boundaries(
            &all,
            &starts,
            |release| {
                *calls.borrow_mut() += 1;
                Ok(table
                    .iter()
                    .find(|(r, _)| *r == release)
                    .and_then(|(_, m)| *m))
            },
            |_, _| {},
        )
        .unwrap();
        (found, calls.into_inner())
    }

    #[test]
    fn steady_minor_costs_two_measurements() {
        let (found, calls) = run(
            &[&["1.0.0f1", "1.0.1f1", "1.0.2f1", "1.0.3f1"]],
            &[
                ("1.0.0f1", Some(1)),
                ("1.0.1f1", Some(1)),
                ("1.0.2f1", Some(1)),
                ("1.0.3f1", Some(1)),
            ],
        );
        assert!(found.is_empty());
        assert_eq!(calls, 2);
    }

    #[test]
    fn finds_move_inside_minor() {
        let (found, _) = run(
            &[&["1.0.0f1", "1.0.1f1", "1.0.2f1", "1.0.3f1", "1.0.4f1"]],
            &[
                ("1.0.0f1", Some(1)),
                ("1.0.1f1", Some(1)),
                ("1.0.2f1", Some(1)),
                ("1.0.3f1", Some(2)),
                ("1.0.4f1", Some(2)),
            ],
        );
        assert_eq!(
            found,
            vec![Boundary {
                last_old: "1.0.2f1".into(),
                first_new: "1.0.3f1".into()
            }]
        );
    }

    #[test]
    fn finds_move_between_minors() {
        let (found, calls) = run(
            &[&["1.0.0f1", "1.0.9f1"], &["1.1.0f1", "1.1.5f1"]],
            &[
                ("1.0.0f1", Some(1)),
                ("1.0.9f1", Some(1)),
                ("1.1.0f1", Some(2)),
                ("1.1.5f1", Some(2)),
            ],
        );
        assert_eq!(
            found,
            vec![Boundary {
                last_old: "1.0.9f1".into(),
                first_new: "1.1.0f1".into()
            }]
        );
        assert_eq!(calls, 4);
    }

    #[test]
    fn reverted_change_gives_two_boundaries() {
        let (found, _) = run(
            &[&["1.0.0f1", "1.0.1f1", "1.0.2f1"], &["1.1.0f1"]],
            &[
                ("1.0.0f1", Some(1)),
                ("1.0.1f1", Some(2)),
                ("1.0.2f1", Some(2)),
                ("1.1.0f1", Some(1)),
            ],
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].first_new, "1.0.1f1");
        assert_eq!(found[1].first_new, "1.1.0f1");
    }

    #[test]
    fn skips_patch_that_cannot_be_measured() {
        let (found, _) = run(
            &[&["1.0.0f1", "1.0.1f1", "1.0.2f1", "1.0.3f1"]],
            &[
                ("1.0.0f1", Some(1)),
                ("1.0.1f1", None),
                ("1.0.2f1", Some(2)),
                ("1.0.3f1", Some(2)),
            ],
        );
        assert_eq!(
            found,
            vec![Boundary {
                last_old: "1.0.0f1".into(),
                first_new: "1.0.2f1".into()
            }]
        );
    }
}
