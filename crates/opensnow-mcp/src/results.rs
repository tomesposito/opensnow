//! Arrow batches to a plain JSON shape agents can read: column names
//! and types, rows as strings (null stays null), and a row count.
//! Values are display strings, not typed JSON. Both local and hosted
//! paths use this one encoding, so a result reads the same either way.

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
            columns.push(
                serde_json::json!({"name": field.name(), "type": field.data_type().to_string()}),
            );
        }
    }
    for batch in batches {
        rows.extend(display_rows(batch).map(|row| serde_json::json!(row)));
    }
    serde_json::json!({"columns": columns, "rows": rows, "row_count": rows.len()})
}

pub(crate) fn display_rows(batch: &RecordBatch) -> impl Iterator<Item = Vec<Option<String>>> + '_ {
    let casted: Vec<_> = batch
        .columns()
        .iter()
        .map(|column| {
            if column.data_type() == &DataType::Utf8 {
                None
            } else {
                cast_with_options(column, &DataType::Utf8, &Default::default()).ok()
            }
        })
        .collect();
    let nulls: Vec<_> = batch
        .columns()
        .iter()
        .map(|column| column.logical_nulls())
        .collect();
    (0..batch.num_rows()).map(move |row| {
        batch
            .columns()
            .iter()
            .enumerate()
            .map(|(index, column)| {
                if nulls[index]
                    .as_ref()
                    .is_some_and(|nulls| nulls.is_null(row))
                {
                    return None;
                }
                let array = casted[index].as_ref().unwrap_or(column);
                Some(array_value_to_string(array, row).unwrap_or_else(|_| "null".into()))
            })
            .collect()
    })
}
