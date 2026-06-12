//! Aggregators (visidata parity: count/min/max/sum/avg/median/mode/stdev/distinct).

use std::collections::HashMap;

use crate::value::{value_cmp, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Aggregator {
    Count,
    Min,
    Max,
    Sum,
    Avg,
    Median,
    Mode,
    Stdev,
    Distinct,
}

impl Aggregator {
    pub fn name(&self) -> &'static str {
        match self {
            Aggregator::Count => "count",
            Aggregator::Min => "min",
            Aggregator::Max => "max",
            Aggregator::Sum => "sum",
            Aggregator::Avg => "avg",
            Aggregator::Median => "median",
            Aggregator::Mode => "mode",
            Aggregator::Stdev => "stdev",
            Aggregator::Distinct => "distinct",
        }
    }

    pub fn from_name(s: &str) -> Option<Aggregator> {
        Some(match s {
            "count" => Aggregator::Count,
            "min" => Aggregator::Min,
            "max" => Aggregator::Max,
            "sum" => Aggregator::Sum,
            "avg" | "mean" => Aggregator::Avg,
            "median" => Aggregator::Median,
            "mode" => Aggregator::Mode,
            "stdev" => Aggregator::Stdev,
            "distinct" => Aggregator::Distinct,
            _ => return None,
        })
    }

    pub fn all() -> &'static [Aggregator] {
        &[
            Aggregator::Count,
            Aggregator::Min,
            Aggregator::Max,
            Aggregator::Sum,
            Aggregator::Avg,
            Aggregator::Median,
            Aggregator::Mode,
            Aggregator::Stdev,
            Aggregator::Distinct,
        ]
    }

    /// Apply to a set of values. Nulls and errors are skipped (visidata
    /// behavior), except Count which counts non-null values.
    pub fn apply(&self, values: &[Value]) -> Value {
        let valid: Vec<&Value> = values
            .iter()
            .filter(|v| !v.is_null() && !v.is_error())
            .collect();
        match self {
            Aggregator::Count => Value::Int(valid.len() as i64),
            Aggregator::Distinct => {
                let mut seen = std::collections::HashSet::new();
                for v in &valid {
                    seen.insert(v.to_display());
                }
                Value::Int(seen.len() as i64)
            }
            Aggregator::Min => valid
                .iter()
                .min_by(|a, b| value_cmp(a, b))
                .map(|v| (*v).clone())
                .unwrap_or(Value::Null),
            Aggregator::Max => valid
                .iter()
                .max_by(|a, b| value_cmp(a, b))
                .map(|v| (*v).clone())
                .unwrap_or(Value::Null),
            Aggregator::Mode => {
                let mut counts: HashMap<compact_str::CompactString, (usize, &Value)> =
                    HashMap::new();
                for v in &valid {
                    let e = counts.entry(v.to_display()).or_insert((0, v));
                    e.0 += 1;
                }
                counts
                    .into_values()
                    .max_by_key(|(n, _)| *n)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Null)
            }
            Aggregator::Sum => {
                let nums = numeric(&valid);
                if nums.is_empty() {
                    return Value::Null;
                }
                if valid.iter().all(|v| matches!(v, Value::Int(_))) {
                    Value::Int(nums.iter().map(|f| *f as i64).sum())
                } else {
                    Value::Float(nums.iter().sum())
                }
            }
            Aggregator::Avg => {
                let nums = numeric(&valid);
                if nums.is_empty() {
                    return Value::Null;
                }
                Value::Float(nums.iter().sum::<f64>() / nums.len() as f64)
            }
            Aggregator::Median => {
                let mut nums = numeric(&valid);
                if nums.is_empty() {
                    return Value::Null;
                }
                nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let n = nums.len();
                if n % 2 == 1 {
                    Value::Float(nums[n / 2])
                } else {
                    Value::Float((nums[n / 2 - 1] + nums[n / 2]) / 2.0)
                }
            }
            Aggregator::Stdev => {
                let nums = numeric(&valid);
                if nums.len() < 2 {
                    return Value::Null;
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let var = nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
                    / (nums.len() - 1) as f64;
                Value::Float(var.sqrt())
            }
        }
    }
}

fn numeric(values: &[&Value]) -> Vec<f64> {
    values.iter().filter_map(|v| v.as_f64()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals() -> Vec<Value> {
        vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Null,
            Value::Int(2),
        ]
    }

    #[test]
    fn basics() {
        let v = vals();
        assert_eq!(Aggregator::Count.apply(&v), Value::Int(4));
        assert_eq!(Aggregator::Sum.apply(&v), Value::Int(8));
        assert_eq!(Aggregator::Avg.apply(&v), Value::Float(2.0));
        assert_eq!(Aggregator::Min.apply(&v), Value::Int(1));
        assert_eq!(Aggregator::Max.apply(&v), Value::Int(3));
        assert_eq!(Aggregator::Mode.apply(&v), Value::Int(2));
        assert_eq!(Aggregator::Distinct.apply(&v), Value::Int(3));
        assert_eq!(Aggregator::Median.apply(&v), Value::Float(2.0));
    }

    #[test]
    fn stdev() {
        let v = vec![Value::Float(2.0), Value::Float(4.0), Value::Float(4.0), Value::Float(4.0), Value::Float(5.0), Value::Float(5.0), Value::Float(7.0), Value::Float(9.0)];
        if let Value::Float(s) = Aggregator::Stdev.apply(&v) {
            assert!((s - 2.138).abs() < 0.01);
        } else {
            panic!("expected float");
        }
    }

    #[test]
    fn empty_input() {
        assert_eq!(Aggregator::Sum.apply(&[]), Value::Null);
        assert_eq!(Aggregator::Count.apply(&[]), Value::Int(0));
    }
}
