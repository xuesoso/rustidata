//! Parquet and Arrow IPC loaders, wrapping RecordBatches directly (no
//! transcoding) behind the RowSource trait.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use arrow::array::ArrayRef;
use arrow::datatypes::DataType;
use arrow::record_batch::{RecordBatch, RecordBatchReader};
use rustidata_core::column::Column;
use rustidata_core::sheet::Sheet;
use rustidata_core::store::{Progress, RawCell, RowId, RowSource};
use rustidata_core::value::{VType, Value};

use crate::FileType;

/// RowSource over a set of RecordBatches.
pub struct ArrowStore {
    batches: Vec<RecordBatch>,
    /// cumulative row counts for batch lookup
    offsets: Vec<u64>,
    nrows: u64,
    ncols: usize,
}

impl ArrowStore {
    pub fn new(batches: Vec<RecordBatch>) -> Self {
        let mut offsets = Vec::with_capacity(batches.len() + 1);
        let mut total = 0u64;
        offsets.push(0);
        for b in &batches {
            total += b.num_rows() as u64;
            offsets.push(total);
        }
        let ncols = batches.first().map(|b| b.num_columns()).unwrap_or(0);
        ArrowStore {
            batches,
            offsets,
            nrows: total,
            ncols,
        }
    }

    fn locate(&self, row: RowId) -> Option<(usize, usize)> {
        if row >= self.nrows {
            return None;
        }
        let bi = match self.offsets.binary_search(&row) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        Some((bi, (row - self.offsets[bi]) as usize))
    }
}

/// Downcast an ArrayRef to a concrete array type. Arrow guarantees the
/// data_type matches the concrete array, but a malformed file must degrade to
/// Null rather than panic the viewer.
macro_rules! dc {
    ($arr:expr, $t:ty) => {
        match $arr.as_any().downcast_ref::<$t>() {
            Some(a) => a,
            None => return Value::Null,
        }
    };
}

fn arrow_cell(arr: &ArrayRef, i: usize) -> Value {
    use arrow::array::*;
    if arr.is_null(i) {
        return Value::Null;
    }
    match arr.data_type() {
        DataType::Boolean => Value::Bool(dc!(arr, BooleanArray).value(i)),
        DataType::Int8 => Value::Int(dc!(arr, Int8Array).value(i) as i64),
        DataType::Int16 => Value::Int(dc!(arr, Int16Array).value(i) as i64),
        DataType::Int32 => Value::Int(dc!(arr, Int32Array).value(i) as i64),
        DataType::Int64 => Value::Int(dc!(arr, Int64Array).value(i)),
        DataType::UInt8 => Value::Int(dc!(arr, UInt8Array).value(i) as i64),
        DataType::UInt16 => Value::Int(dc!(arr, UInt16Array).value(i) as i64),
        DataType::UInt32 => Value::Int(dc!(arr, UInt32Array).value(i) as i64),
        DataType::UInt64 => Value::Int(dc!(arr, UInt64Array).value(i) as i64),
        DataType::Float32 => Value::Float(dc!(arr, Float32Array).value(i) as f64),
        DataType::Float64 => Value::Float(dc!(arr, Float64Array).value(i)),
        DataType::Utf8 => Value::Str(dc!(arr, StringArray).value(i).into()),
        DataType::LargeUtf8 => Value::Str(dc!(arr, LargeStringArray).value(i).into()),
        DataType::Date32 => {
            let days = dc!(arr, Date32Array).value(i) as i64;
            Value::Date(days.saturating_mul(86_400_000_000))
        }
        DataType::Date64 => {
            let ms = dc!(arr, Date64Array).value(i);
            Value::Date(ms.saturating_mul(1000))
        }
        DataType::Timestamp(unit, _) => {
            use arrow::datatypes::TimeUnit;
            let v = match unit {
                TimeUnit::Second => dc!(arr, TimestampSecondArray).value(i).saturating_mul(1_000_000),
                TimeUnit::Millisecond => dc!(arr, TimestampMillisecondArray).value(i).saturating_mul(1000),
                TimeUnit::Microsecond => dc!(arr, TimestampMicrosecondArray).value(i),
                TimeUnit::Nanosecond => dc!(arr, TimestampNanosecondArray).value(i) / 1000,
            };
            Value::Date(v)
        }
        _ => {
            // fall back to arrow's display formatting
            match arrow::util::display::array_value_to_string(arr, i) {
                Ok(s) => Value::Str(s.into()),
                Err(_) => Value::Null,
            }
        }
    }
}

impl RowSource for ArrowStore {
    fn len(&self) -> u64 {
        self.nrows
    }

    fn loading(&self) -> Option<Progress> {
        None
    }

    fn ncols(&self) -> usize {
        self.ncols
    }

    fn raw(&self, row: RowId, col: usize) -> RawCell {
        match self.locate(row) {
            Some((bi, ri)) => {
                let batch = &self.batches[bi];
                if col >= batch.num_columns() {
                    return RawCell::Missing;
                }
                RawCell::Owned(arrow_cell(batch.column(col), ri))
            }
            None => RawCell::Missing,
        }
    }
}

fn vtype_for(dt: &DataType) -> VType {
    match dt {
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64 => VType::Int,
        DataType::Float16 | DataType::Float32 | DataType::Float64 => VType::Float,
        DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _) => VType::Date,
        _ => VType::Any,
    }
}

pub fn load(path: &Path, ft: FileType) -> Result<Sheet> {
    let file = std::fs::File::open(path)?;
    let (schema, batches) = match ft {
        FileType::Parquet => {
            let reader =
                parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)?
                    .with_batch_size(64 * 1024)
                    .build()?;
            let schema = reader.schema();
            let batches: Vec<RecordBatch> = reader.collect::<std::result::Result<_, _>>()?;
            (schema, batches)
        }
        _ => {
            // Arrow IPC: try file format, then stream format
            match arrow::ipc::reader::FileReader::try_new(file.try_clone()?, None) {
                Ok(reader) => {
                    let schema = reader.schema();
                    let batches: Vec<RecordBatch> =
                        reader.collect::<std::result::Result<_, _>>()?;
                    (schema, batches)
                }
                Err(_) => {
                    let file = std::fs::File::open(path)?;
                    let reader = arrow::ipc::reader::StreamReader::try_new(file, None)?;
                    let schema = reader.schema();
                    let batches: Vec<RecordBatch> =
                        reader.collect::<std::result::Result<_, _>>()?;
                    (schema, batches)
                }
            }
        }
    };

    let columns: Vec<Column> = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(i, f)| Column::source(f.name().clone(), i).with_type(vtype_for(f.data_type())))
        .collect();
    let store = ArrowStore::new(batches);
    Ok(Sheet::new(crate::sheet_name(path), Arc::new(store), columns))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Int64Array, StringArray};
    use arrow::datatypes::{Field, Schema};

    fn sample_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec![Some("a"), None, Some("c")])),
            ],
        )
        .unwrap()
    }

    #[test]
    fn arrow_store_access() {
        let store = ArrowStore::new(vec![sample_batch(), sample_batch()]);
        assert_eq!(store.len(), 6);
        assert_eq!(store.raw(0, 0), RawCell::Owned(Value::Int(1)));
        assert_eq!(store.raw(1, 1), RawCell::Owned(Value::Null));
        assert_eq!(store.raw(5, 1), RawCell::Owned(Value::Str("c".into())));
        assert_eq!(store.raw(6, 0), RawCell::Missing);
    }

    #[test]
    fn parquet_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.parquet");
        let batch = sample_batch();
        let file = std::fs::File::create(&p).unwrap();
        let mut writer =
            parquet::arrow::ArrowWriter::try_new(file, batch.schema(), None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let sheet = load(&p, FileType::Parquet).unwrap();
        assert_eq!(sheet.nrows(), 3);
        assert_eq!(sheet.columns[0].vtype, VType::Int);
        assert_eq!(sheet.typed_at(2, 1), Value::Str("c".into()));
    }
}
