//! Value lists written out, or generated from end points like a `.env` vector.
use std::fmt;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{ArrayBuilder, ObjectBuilder, OneOfBuilder, Schema, Type};

/// Most values one generated grid may hold.
pub const MAX_GRID_COUNT: usize = 1_000_000;

/// A number, an array of numbers, or `{"from", "to", "count", "unit"}`.
///
/// A generated grid expands exactly as a `.env` vector given by its two end
/// points: `from + i * ((to - from) / (count - 1))`, computed in `unit` and
/// then converted to metres. `count` may be omitted where the model has an
/// automatic count, such as BELLHOP launch angles.
#[derive(Clone, Debug, PartialEq)]
pub enum Grid {
    Values(Vec<f64>),
    Generated {
        from: f64,
        to: f64,
        count: Option<usize>,
        unit: GridUnit,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GridUnit {
    #[default]
    M,
    Km,
}

impl utoipa::PartialSchema for Grid {
    fn schema() -> RefOr<Schema> {
        let number = || ObjectBuilder::new().schema_type(Type::Number);
        OneOfBuilder::new()
            .description(Some(
                "A number, an array of numbers, or a grid generated from its end points",
            ))
            .item(number())
            .item(ArrayBuilder::new().items(number()))
            .item(
                ObjectBuilder::new()
                    .property("from", number())
                    .property("to", number())
                    .property("count", ObjectBuilder::new().schema_type(Type::Integer))
                    .property("unit", GridUnit::schema())
                    .required("from")
                    .required("to"),
            )
            .into()
    }
}

impl ToSchema for Grid {}

impl GridUnit {
    fn scale(self) -> f64 {
        match self {
            Self::M => 1.0,
            Self::Km => 1000.0,
        }
    }
}

impl Grid {
    /// Expands the grid. `automatic` supplies the count when it is omitted.
    ///
    /// # Errors
    ///
    /// Returns a message when the count is missing or out of range.
    pub fn expand(&self, automatic: Option<usize>) -> Result<Vec<f64>, String> {
        match self {
            Self::Values(values) => Ok(values.clone()),
            &Self::Generated {
                from,
                to,
                count,
                unit,
            } => {
                let count = count
                    .or(automatic)
                    .ok_or_else(|| "`count` is required".to_owned())?;
                if count == 0 || count > MAX_GRID_COUNT {
                    return Err(format!("`count` must be in 1..={MAX_GRID_COUNT}"));
                }
                #[allow(clippy::float_cmp)]
                if count == 1 && from != to {
                    return Err("a single-value grid needs `from` equal to `to`".into());
                }
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
                    count: Some(values.len()),
                    unit,
                };
                if candidate
                    .expand(None)
                    .is_ok_and(|expanded| same_bits(&expanded, values))
                {
                    return candidate;
                }
            }
        }
        Self::Values(values.to_vec())
    }
}

/// The reference subtabulation: two end points, then `first + i * delta`.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn linspace(first: f64, last: f64, count: usize) -> Vec<f64> {
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
                if let Some(count) = count {
                    map.serialize_entry("count", count)?;
                }
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
                    count,
                    unit: unit.unwrap_or_default(),
                })
            }
        }

        deserializer.deserialize_any(GridVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::{Grid, GridUnit};

    #[test]
    fn generated_grids_match_the_reference_subtabulation() {
        let grid: Grid =
            serde_json::from_str(r#"{"from": 0, "to": 100, "count": 501, "unit": "km"}"#).unwrap();
        let values = grid.expand(None).unwrap();
        assert_eq!(values.len(), 501);
        // The `.env` reader computes kilometres, then scales: keep its rounding.
        assert_eq!(values[3], 0.600_000_000_000_000_1 * 1000.0);
        assert_eq!(Grid::describe(&values), grid);
    }

    #[test]
    fn scalars_lists_and_inexact_values_stay_explicit() {
        assert_eq!(
            serde_json::from_str::<Grid>("1000").unwrap(),
            Grid::Values(vec![1000.0])
        );
        let values = [0.0, 1.0, 2.5];
        assert_eq!(Grid::describe(&values), Grid::Values(values.to_vec()));
        assert_eq!(
            Grid::describe(&[0.0, 25.0, 50.0]),
            Grid::Generated {
                from: 0.0,
                to: 50.0,
                count: Some(3),
                unit: GridUnit::M
            }
        );
    }

    #[test]
    fn rejects_unknown_keys_and_bad_counts() {
        assert!(serde_json::from_str::<Grid>(r#"{"from": 0, "to": 1, "step": 2}"#).is_err());
        let grid: Grid = serde_json::from_str(r#"{"from": 0, "to": 1}"#).unwrap();
        assert!(grid.expand(None).is_err());
        assert_eq!(grid.expand(Some(3)).unwrap(), [0.0, 0.5, 1.0]);
        let grid: Grid = serde_json::from_str(r#"{"from": 0, "to": 1, "count": 0}"#).unwrap();
        assert!(grid.expand(None).is_err());
    }
}
