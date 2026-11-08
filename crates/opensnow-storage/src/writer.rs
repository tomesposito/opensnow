//! Single-table copy-on-write mutations using the vendored overwrite action.
use std::{collections::HashMap, ops::ControlFlow, sync::Arc};

use arrow::{datatypes::SchemaRef, record_batch::RecordBatch};
use datafusion::{
    common::{tree_node::{TreeNode, TreeNodeRecursion}, Column},
    dataframe::DataFrame,
    datasource::MemTable,
    logical_expr::{expr::Cast, when, Expr, Volatility},
    prelude::SessionContext,
    sql::sqlparser::{
        ast::{Ident, ObjectName, Query, Visit, Visitor},
        dialect::GenericDialect,
        keywords::Keyword,
        parser::Parser,
        tokenizer::Token,
    },
};
use futures::{stream, TryStreamExt};
use iceberg::{
    arrow::schema_to_arrow_schema,
    scan::FileScanTask,
    spec::{DataFile, DataFileFormat, FormatVersion, ManifestContentType, PartitionKey, Struct},
    table::Table,
    transaction::{ApplyTransactionAction, Transaction},
    writer::{
        base_writer::data_file_writer::DataFileWriterBuilder,
        file_writer::{
            location_generator::{DefaultFileNameGenerator, DefaultLocationGenerator},
            rolling_writer::RollingFileWriterBuilder,
            ParquetWriterBuilder,
        },
        IcebergWriter, IcebergWriterBuilder,
    },
    TableIdent,
};

use crate::{Result, StorageCatalog};

struct Mutation {
    ident: TableIdent,
    predicate: String,
    // None is DELETE; UPDATE assignments all see the original row.
    assignments: Option<HashMap<String, String>>,
}

fn identifier(ident: &Ident) -> String {
    if ident.quote_style.is_some() { ident.value.clone() } else { ident.value.to_lowercase() }
}

fn names(name: ObjectName) -> Result<Vec<String>> {
    name.0.into_iter().map(|part| part.as_ident().map(identifier)
        .ok_or_else(|| "only plain SQL identifiers are supported".into())).collect()
}

struct NoQueries;
impl Visitor for NoQueries {
    type Break = &'static str;
    fn pre_visit_query(&mut self, _: &Query) -> ControlFlow<Self::Break> {
        ControlFlow::Break("subqueries are not supported in mutations")
    }
}

fn expression(parser: &mut Parser<'_>) -> Result<String> {
    let expr = parser.parse_expr()?;
    if let ControlFlow::Break(message) = expr.visit(&mut NoQueries) {
        return Err(message.into());
    }
    Ok(expr.to_string())
}

impl Mutation {
    fn parse(sql: &str) -> Result<Option<Self>> {
        let mut parser = Parser::new(&GenericDialect).try_with_sql(sql)?;
        let update = if parser.parse_keyword(Keyword::DELETE) {
            parser.expect_keyword(Keyword::FROM)?;
            false
        } else if parser.parse_keyword(Keyword::UPDATE) {
            true
        } else {
            return Ok(None);
        };
        let table = names(parser.parse_object_name(false)?)?;
        if table.len() != 3 || table[0] != "ice" {
            return Err("mutations require exactly ice.<namespace>.<table>".into());
        }
        let ident = TableIdent::from_strs([table[1].as_str(), table[2].as_str()])?;
        let assignments = if update {
            parser.expect_keyword(Keyword::SET)?;
            let mut assignments = HashMap::new();
            loop {
                let column = identifier(&parser.parse_identifier()?);
                parser.expect_token(&Token::Eq)?;
                if assignments.insert(column, expression(&mut parser)?).is_some() {
                    return Err("duplicate SET column".into());
                }
                if !parser.consume_token(&Token::Comma) { break; }
            }
            Some(assignments)
        } else { None };
        let predicate = if parser.parse_keyword(Keyword::WHERE) {
            expression(&mut parser)?
        } else { "TRUE".into() };
        let _ = parser.consume_token(&Token::SemiColon);
        if parser.peek_token().token != Token::EOF {
            return Err(format!("unsupported mutation clause: {}", parser.peek_token()).into());
        }
        Ok(Some(Self { ident, predicate, assignments }))
    }
}

fn column(name: &str) -> Expr { Expr::Column(Column::new_unqualified(name)) }

fn row_expression(frame: &DataFrame, sql: &str) -> Result<Expr> {
    let expr = frame.parse_sql_expr(sql)?;
    expr.apply(|node| {
        let unsupported = match node {
            Expr::AggregateFunction(_) | Expr::WindowFunction(_) | Expr::Exists(_)
            | Expr::InSubquery(_) | Expr::ScalarSubquery(_) => true,
            Expr::ScalarFunction(function) => function.func.signature().volatility != Volatility::Immutable,
            _ => false,
        };
        if unsupported {
            return Err(datafusion::error::DataFusionError::Plan(
                "mutations require deterministic scalar expressions over the row".into()));
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(expr)
}

fn frame(schema: SchemaRef, batches: Vec<RecordBatch>) -> Result<DataFrame> {
    let ctx = SessionContext::new();
    Ok(ctx.read_table(Arc::new(MemTable::try_new(schema, vec![batches])?))?)
}

pub(crate) fn normalize(schema: &SchemaRef, batches: Vec<RecordBatch>) -> Result<Vec<RecordBatch>> {
    batches.into_iter().map(|batch| {
        if batch.num_columns() != schema.fields().len() {
            return Err("replacement schema must match the table".into());
        }
        for (actual, expected) in batch.schema().fields().iter().zip(schema.fields()) {
            if actual.name() != expected.name() || actual.data_type() != expected.data_type() {
                return Err("replacement column names, order, and types must match the table".into());
            }
        }
        for (array, field) in batch.columns().iter().zip(schema.fields()) {
            if !field.is_nullable() && array.null_count() != 0 {
                return Err(format!("NULL in required column {}", field.name()).into());
            }
        }
        Ok(RecordBatch::try_new(schema.clone(), batch.columns().to_vec())?)
    }).collect()
}

pub(crate) fn schema(table: &Table) -> Result<SchemaRef> {
    if table.metadata().format_version() != FormatVersion::V2
        || table.metadata().partition_specs_iter().any(|spec| !spec.is_unpartitioned()) {
        return Err("copy-on-write currently supports unpartitioned Iceberg v2 tables".into());
    }
    if table.encryption_manager().is_some() {
        return Err("copy-on-write does not yet support encrypted tables".into());
    }
    Ok(Arc::new(schema_to_arrow_schema(table.metadata().current_schema())?))
}

// FileScanTask exposes paths, not DataFile values in the pinned crate. Resolve
// the exact live objects from the same snapshot's manifests for the delete set.
pub(crate) async fn data_files(table: &Table) -> Result<HashMap<String, DataFile>> {
    let mut files = HashMap::new();
    if let Some(snapshot) = table.metadata().current_snapshot() {
        let list = table.manifest_list_reader(snapshot).load().await?;
        for manifest_file in list.entries() {
            if manifest_file.content == ManifestContentType::Deletes {
                return Err("copy-on-write does not support tables with delete files".into());
            }
            let manifest = table.manifest_reader().read(manifest_file).await?;
            for entry in manifest.entries().iter().filter(|entry| entry.is_alive()) {
                if entry.data_file().file_format() != DataFileFormat::Parquet {
                    return Err("copy-on-write currently supports Parquet data files".into());
                }
                files.insert(entry.file_path().to_owned(), entry.data_file().clone());
            }
        }
    }
    Ok(files)
}

pub(crate) async fn read_file(table: &Table, tasks: Vec<FileScanTask>) -> Result<Vec<RecordBatch>> {
    Ok(table.reader_builder().build()
        .read(Box::pin(stream::iter(tasks.into_iter().map(Ok))))?.stream()
        .try_collect().await?)
}

async fn write(table: &Table, batches: Vec<RecordBatch>) -> Result<Vec<DataFile>> {
    write_sized(table, batches, iceberg::spec::TableProperties::PROPERTY_WRITE_TARGET_FILE_SIZE_BYTES_DEFAULT).await
}

pub(crate) async fn write_sized(table: &Table, batches: Vec<RecordBatch>, target: usize) -> Result<Vec<DataFile>> {
    let parquet = ParquetWriterBuilder::from_table_properties(
        &table.metadata().table_properties(), table.current_schema_ref())?;
    let rolling = RollingFileWriterBuilder::new(
        parquet, target, table.file_io().clone(), DefaultLocationGenerator::new(table.metadata())?,
        DefaultFileNameGenerator::new(uuid::Uuid::now_v7().to_string(), None, DataFileFormat::Parquet));
    let partition = PartitionKey::new(
        table.metadata().default_partition_spec().as_ref().clone(),
        table.current_schema_ref(), Struct::empty());
    let mut writer = DataFileWriterBuilder::new(rolling).build(Some(partition)).await?;
    for batch in batches.into_iter().filter(|batch| batch.num_rows() != 0) {
        writer.write(batch).await?;
    }
    Ok(writer.close().await?)
}

impl StorageCatalog {
    /// Runs narrow DELETE/UPDATE statements through per-file copy-on-write.
    /// Other statements retain `sql` behavior. Mutations return no result batches.
    pub async fn execute(&self, sql: &str) -> Result<Vec<RecordBatch>> {
        let Some(mutation) = Mutation::parse(sql)? else { return self.sql(sql).await; };
        let table = self.catalog.load_table(&mutation.ident).await?;
        let schema = schema(&table)?;
        let empty = frame(schema.clone(), vec![RecordBatch::new_empty(schema.clone())])?;
        let predicate = row_expression(&empty, &mutation.predicate)?;
        // Validate even on empty tables and no-match writes, before creating files.
        empty.clone().filter(predicate.clone())?;
        let projection = if let Some(assignments) = &mutation.assignments {
            for name in assignments.keys() { schema.field_with_name(name)?; }
            let mut projection = Vec::new();
            for field in schema.fields() {
                let original = column(field.name());
                let value = if let Some(sql) = assignments.get(field.name()) {
                    // Cast only the assigned branch. Coercing the whole CASE can
                    // round unmatched BIGINT values through floating point.
                    let assigned = Expr::Cast(Cast::new(
                        Box::new(row_expression(&empty, sql)?), field.data_type().clone()));
                    when(predicate.clone(), assigned).otherwise(original)?
                } else { original };
                projection.push(value.alias(field.name()));
            }
            empty.clone().select(projection.clone())?.create_physical_plan().await?;
            Some(projection)
        } else { None };
        let mut files = data_files(&table).await?;
        let tasks: Vec<_> = table.scan().build()?.plan_files().await?.try_collect().await?;
        let mut by_file: HashMap<String, Vec<FileScanTask>> = HashMap::new();
        for task in tasks {
            if !task.deletes().is_empty() { return Err("delete files are not supported".into()); }
            by_file.entry(task.data_file_path().to_owned()).or_default().push(task);
        }
        let mut removed = Vec::new();
        let mut added = Vec::new();
        for (path, tasks) in by_file {
            let batches = read_file(&table, tasks).await?;
            let file_frame = frame(schema.clone(), normalize(&schema, batches)?)?;
            let matches = file_frame.clone().filter(predicate.clone())?.limit(0, Some(1))?.collect().await?;
            if matches.iter().all(|batch| batch.num_rows() == 0) { continue; }
            let replacement = if let Some(projection) = &projection {
                file_frame.select(projection.clone())?
            } else {
                // UNKNOWN does not match a SQL WHERE predicate and must survive.
                file_frame.filter(predicate.clone().is_not_true())?
            }.collect().await?;
            added.extend(write(&table, normalize(&schema, replacement)?).await?);
            removed.push(files.remove(&path).ok_or("planned file missing from current manifest")?);
        }
        if !removed.is_empty() { self.commit_overwrite(&table, removed, added).await?; }
        Ok(vec![])
    }

    /// Replaces the complete current row set in one overwrite snapshot, including
    /// an empty replacement. Input columns must match the table's names and types.
    pub async fn replace_all(&self, ident: &TableIdent, batches: Vec<RecordBatch>) -> Result<()> {
        let table = self.catalog.load_table(ident).await?;
        let schema = schema(&table)?;
        let removed = data_files(&table).await?.into_values().collect();
        let added = write(&table, normalize(&schema, batches)?).await?;
        self.commit_overwrite(&table, removed, added).await
    }

    pub(crate) async fn commit_overwrite(&self, table: &Table, removed: Vec<DataFile>, added: Vec<DataFile>) -> Result<()> {
        let tx = Transaction::new(table);
        let tx = tx.overwrite().delete_data_files(removed).add_data_files(added).apply(tx)?;
        tx.commit(self.catalog.as_ref()).await?;
        self.refresh().await
    }
}
