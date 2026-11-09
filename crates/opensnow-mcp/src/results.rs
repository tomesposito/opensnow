//! Arrow batches to a plain JSON shape agents can read: column names
//! and types, rows as strings (null stays null), and a row count.
//! Values are display strings, not typed JSON. Both local and hosted
//! paths use this one encoding, so a result reads the same either way.

use arrow::array::Array;
use arrow::compute::cast_with_options;
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use arrow::util::display::array_value_to_string;

pub fn batches_to_json(batches: &[RecordBatch]) -> serde_json::Value {
    let mut columns: Vec<serde_json::Value> = Vec::new();
    let mut rows: Vec<serde_json::Value> = Vec::new();
    if let Some(first) = batches.first() {
        let schema = first.schema();
        for field in schema.fields() {
            columns.push(serde_json::json!({"name": field.name(), "type": field.data_type().to_string()}));
        }
    }
    for batch in batches {
        let casted: Vec<_> = batch.columns().iter().map(|column| {
            if column.data_type() == &DataType::Utf8 {
                None
            } else {
                cast_with_options(column, &DataType::Utf8, &Default::default()).ok()
            }
        }).collect();
        for row in 0..batch.num_rows() {
            let mut values = Vec::new();
            for (index, column) in batch.columns().iter().enumerate() {
                if column.is_null(row) {
                    values.push(serde_json::Value::Null);
                    continue;
                }
                let text = match &casted[index] {
                    Some(array) => array_value_to_string(array, row).unwrap_or_else(|_| "null".into()),
                    None => array_value_to_string(column, row).unwrap_or_else(|_| "null".into()),
                };
                values.push(serde_json::Value::String(text));
            }
            rows.push(serde_json::Value::Array(values));
        }
    }
    serde_json::json!({"columns": columns, "rows": rows, "row_count": rows.len()})
}
