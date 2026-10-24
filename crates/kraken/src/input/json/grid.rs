//! Value lists written out, or generated from end points like a `.env` vector.
use std::fmt;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Most values one generated grid may hold.
pub const MAX_GRID_COUNT: usize = 1_000_000;

/// A number, an array of numbers, or `{"from", "to", "count", "unit"}`.
///
/// A generated grid expands exactly as a `.env` vector given by its two end
/// points: `from + i * ((to - from) / (count - 1))`, computed in `unit` and
/// then converted to metres. Depths expand in single precision, as KRAKEN's
/// `ReadSzRz` does.
#[derive(Clone, Debug, PartialEq)]
pub enum Grid {
    Values(Vec<f64>),
    Generated {
        from: f64,
        to: f64,
        count: usize,
        unit: GridUnit,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GridUnit {
    #[default]
    M,
    Km,
}

impl GridUnit {
    fn scale(self) -> f64 {
        match self {
            Self::M => 1.0,
            Self::Km => 1000.0,
        }
    }
}

impl Grid {
    /// Expands the grid in double precision.
    ///
    /// # Errors
    ///
    /// Returns a message when the count is out of range.
    pub fn expand(&self) -> Result<Vec<f64>, String> {
        match self {
            Self::Values(values) => Ok(values.clone()),
            &Self::Generated {
                from,
                to,
                count,
                unit,
            } => {
                check_count(from, to, count)?;
                let mut values = linspace(from, to, count);
                if unit != GridUnit::M {
                    for value in &mut values {
                        *value *= unit.scale();
                    }
                }
                Ok(values)
            }
        }
    }

    /// Expands a depth grid in single precision, like the Toolbox: every
    /// value is rounded to `f32`, and generated grids step in `f32`.
    ///
    /// # Errors
    ///
    /// Returns a message when the count is out of range or a unit is given.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub fn expand_single(&self) -> Result<Vec<f64>, String> {
        match self {
            Self::Values(values) => Ok(values.iter().map(|&v| f64::from(v as f32)).collect()),
            &Self::Generated {
                from,
                to,
                count,
                unit,
            } => {
                if unit != GridUnit::M {
                    return Err("depths are always in metres".into());
                }
                check_count(from, to, count)?;
                let (start, end) = (from as f32, to as f32);
                Ok(match count {
                    1 => vec![f64::from(start)],
                    2 => vec![f64::from(start), f64::from(end)],
                    _ => {
                        let step = (end - start) / (count - 1) as f32;
                        (0..count)
                            .map(|index| f64::from(start + index as f32 * step))
                            .collect()
                    }
                })
            }
        }
    }

    /// The shortest exact description of `values`: a generated grid when one
    /// reproduces every value bit for bit, otherwise the values themselves.
    #[must_use]
    pub fn describe(values: &[f64]) -> Self {
        if let [first, .., last] = *values
            && values.len() >= 3
        {
            for unit in [GridUnit::M, GridUnit::Km] {
                let candidate = Self::Generated {
                    from: first / unit.scale(),
                    to: last / unit.scale(),
                    count: values.len(),
                    unit,
                };
                if candidate
                    .expand()
                    .is_ok_and(|expanded| same_bits(&expanded, values))
                {
                    return candidate;
                }
            }
        }
        Self::Values(values.to_vec())
    }

    /// Describes single-precision depths, after `adjust` (the interface snap).
    /// Values exact in `f32` are written with their shortest `f32` spelling.
    #[must_use]
    pub fn describe_single(values: &[f64], adjust: impl Fn(&mut [f64])) -> Self {
        let spelled: Vec<f64> = values.iter().map(|&value| single_spelling(value)).collect();
        if let [first, .., last] = spelled[..]
            && values.len() >= 3
        {
            let candidate = Self::Generated {
                from: first,
                to: last,
                count: values.len(),
                unit: GridUnit::M,
            };
            if let Ok(mut expanded) = candidate.expand_single() {
                adjust(&mut expanded);
                if same_bits(&expanded, values) {
                    return candidate;
                }
            }
        }
        Self::Values(spelled)
    }
}

/// The shortest decimal that reads back as the same `f32`, if `value` is one.
#[allow(clippy::cast_possible_truncation)]
fn single_spelling(value: f64) -> f64 {
    let single = value as f32;
    if f64::from(single).to_bits() == value.to_bits() {
        single.to_string().parse().unwrap_or(value)
    } else {
        value
    }
}

#[allow(clippy::float_cmp)]
fn check_count(from: f64, to: f64, count: usize) -> Result<(), String> {
    if count == 0 || count > MAX_GRID_COUNT {
        return Err(format!("`count` must be in 1..={MAX_GRID_COUNT}"));
    }
    if count == 1 && from != to {
        return Err("a single-value grid needs `from` equal to `to`".into());
    }
    Ok(())
}

/// The reference subtabulation: two end points, then `first + i * delta`.
#[allow(clippy::cast_precision_loss)]
fn linspace(first: f64, last: f64, count: usize) -> Vec<f64> {
    match count {
        0 => Vec::new(),
        1 => vec![first],
        2 => vec![first, last],
        _ => {
            let delta = (last - first) / (count - 1) as f64;
            (0..count)
                .map(|index| first + index as f64 * delta)
                .collect()
        }
    }
}

pub(crate) fn same_bits(left: &[f64], right: &[f64]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.to_bits() == right.to_bits())
}

impl Serialize for Grid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Values(values) => values.serialize(serializer),
            Self::Generated {
                from,
                to,
                count,
                unit,
            } => {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("from", from)?;
                map.serialize_entry("to", to)?;
                map.serialize_entry("count", count)?;
                if *unit != GridUnit::M {
                    map.serialize_entry("unit", unit)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Grid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct GridVisitor;

        impl<'de> Visitor<'de> for GridVisitor {
            type Value = Grid;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .write_str("a number, an array of numbers, or {\"from\", \"to\", \"count\"}")
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Grid, E> {
                Ok(Grid::Values(vec![value]))
            }

            #[allow(clippy::cast_precision_loss)]
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Grid, E> {
                Ok(Grid::Values(vec![value as f64]))
            }

            #[allow(clippy::cast_precision_loss)]
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Grid, E> {
                Ok(Grid::Values(vec![value as f64]))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Grid, A::Error> {
                let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(4096));
                while let Some(value) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(Grid::Values(values))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Grid, A::Error> {
                let (mut from, mut to, mut count, mut unit) = (None, None, None, None);
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "from" if from.is_none() => from = Some(map.next_value()?),
                        "to" if to.is_none() => to = Some(map.next_value()?),
                        "count" if count.is_none() => count = Some(map.next_value()?),
                        "unit" if unit.is_none() => unit = Some(map.next_value()?),
                        "from" | "to" | "count" | "unit" => {
                            return Err(de::Error::custom(format!("duplicate field `{key}`")));
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &["from", "to", "count", "unit"],
                            ));
                        }
                    }
                }
                Ok(Grid::Generated {
                    from: from.ok_or_else(|| de::Error::missing_field("from"))?,
                    to: to.ok_or_else(|| de::Error::missing_field("to"))?,
                    count: count.ok_or_else(|| de::Error::missing_field("count"))?,
                    unit: unit.unwrap_or_default(),
                })
            }
        }

        deserializer.deserialize_any(GridVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::Grid;

    #[test]
    fn single_precision_depths_match_read_sz_rz_and_spell_shortest() {
        let grid: Grid = serde_json::from_str(r#"{"from": 0, "to": 5100, "count": 1000}"#).unwrap();
        let depths = grid.expand_single().unwrap();
        assert_eq!(depths[1], f64::from(5100.0_f32 / 999.0));
        assert_eq!(Grid::describe_single(&depths, |_| ()), grid);
        let depths = Grid::Values(vec![0.1, 25.0]).expand_single().unwrap();
        assert_eq!(depths[0], f64::from(0.1_f32));
        assert_eq!(
            serde_json::to_string(&Grid::describe_single(&depths, |_| ())).unwrap(),
            "[0.1,25.0]"
        );
    }
}
