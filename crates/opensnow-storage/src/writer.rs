//! Single-table copy-on-write mutations using the vendored overwrite action.
use std::{collections::HashMap, ops::ControlFlow, sync::Arc};

use arrow::{datatypes::{DataType, SchemaRef}, record_batch::RecordBatch};
use datafusion::{
    common::{tree_node::{TreeNode, TreeNodeRecursion}, Column},
    dataframe::DataFrame,
    datasource::MemTable,
    logical_expr::{expr::Cast, when, Expr, Volatility},
    prelude::SessionContext,
    sql::sqlparser::{
        ast::{
            AssignmentTarget, Expr as AstExpr, Ident, MergeAction, MergeClauseKind,
            MergeInsertKind, ObjectName, Query, Statement, TableFactor, UpdateTableFromKind,
            Visit, Visitor,
        },
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

// --- MERGE and UPDATE with FROM (issue #18) ---
//
// Both join the target to a source. The joined row set is computed with
// ordinary SELECTs over the catalog (target and source as CTEs) and then
// committed through replace_all: exactly one overwrite snapshot through
// the same atomic catalog path as every other mutation. Plain DELETE
// and UPDATE keep the per-file path below. Limits: a single source
// table or subquery, no RETURNING, matched clauses apply in written
// order, and a source row set that matches one target row more than
// once is an error rather than an arbitrary pick.

const AGGREGATE_OR_WINDOW_FNS: &[&str] = &[
    "sum", "count", "avg", "min", "max", "array_agg", "string_agg", "bool_and", "bool_or",
    "variance", "var_pop", "var_samp", "stddev", "stddev_pop", "stddev_samp", "corr",
    "covar_pop", "covar_samp", "median", "mode", "approx_count_distinct", "approx_percentile_cont",
    "rank", "dense_rank", "row_number", "percent_rank", "cume_dist", "ntile", "lag", "lead",
    "first_value", "last_value", "nth_value",
];
const NONDETERMINISTIC_FNS: &[&str] = &["random", "rand", "uuid", "gen_random_uuid"];

struct ScalarOnly;
impl Visitor for ScalarOnly {
    type Break = String;
    fn pre_visit_query(&mut self, _: &Query) -> ControlFlow<Self::Break> {
        ControlFlow::Break("subqueries are not supported in mutation expressions".into())
    }
    fn pre_visit_expr(&mut self, expr: &AstExpr) -> ControlFlow<Self::Break> {
        if let AstExpr::Function(function) = expr {
            let name = function.name.to_string().to_lowercase();
            let leaf = name.rsplit('.').next().unwrap_or(&name).trim_matches('"');
            if function.over.is_some() {
                return ControlFlow::Break("window functions are not supported in mutations".into());
            }
            if AGGREGATE_OR_WINDOW_FNS.contains(&leaf) {
                return ControlFlow::Break(format!("aggregate function {leaf} is not supported in mutations"));
            }
            if NONDETERMINISTIC_FNS.contains(&leaf) {
                return ControlFlow::Break(format!(
                    "mutations require deterministic scalar expressions; {leaf} is not deterministic"));
            }
        }
        ControlFlow::Continue(())
    }
}

/// Parses a standalone expression and rejects subqueries, aggregates,
/// windows, and nondeterministic calls. Returns the normalized text.
fn scalar_expr(text: &str) -> Result<String> {
    let mut parser = Parser::new(&GenericDialect).try_with_sql(text)?;
    let expr = parser.parse_expr()?;
    if let ControlFlow::Break(message) = expr.visit(&mut ScalarOnly) {
        return Err(message.into());
    }
    Ok(expr.to_string())
}

fn assignment_column(target: &AssignmentTarget) -> Result<String> {
    match target {
        AssignmentTarget::ColumnName(name) => {
            let parts = names(name.clone())?;
            parts.last().cloned().ok_or_else(|| "assignment target is empty".into())
        }
        AssignmentTarget::Tuple(_) => Err("tuple assignments are not supported in mutations".into()),
    }
}

fn assignment_pairs(assignments: &[datafusion::sql::sqlparser::ast::Assignment]) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for assignment in assignments {
        let column = assignment_column(&assignment.target)?;
        if out.iter().any(|(existing, _)| existing == &column) {
            return Err(format!("duplicate SET column {column}").into());
        }
        out.push((column, scalar_expr(&assignment.value.to_string())?));
    }
    Ok(out)
}

/// A table factor's alias (default: its table name) and rendered text.
fn factor_alias_render(factor: &TableFactor) -> Result<(String, String)> {
    match factor {
        TableFactor::Table { name, alias, args, .. } => {
            if args.is_some() {
                return Err("table functions are not supported as a mutation source".into());
            }
            let parts = names(name.clone())?;
            let alias = alias.as_ref().map(|a| identifier(&a.name))
                .or_else(|| parts.last().cloned())
                .ok_or("mutation source requires an alias")?;
            Ok((alias, factor.to_string()))
        }
        TableFactor::Derived { alias, .. } => {
            let alias = alias.as_ref().ok_or("a derived mutation source requires an alias")?;
            Ok((identifier(&alias.name), factor.to_string()))
        }
        _ => Err("unsupported mutation source".into()),
    }
}

/// The merge/update target: a three-part ice.<namespace>.<table> factor.
/// Returns the ident, the alias (default: table name), and the SQL name.
fn target_factor(factor: &TableFactor) -> Result<(TableIdent, String, String)> {
    let TableFactor::Table { name, alias, args, .. } = factor else {
        return Err("mutations require a plain target table".into());
    };
    if args.is_some() {
        return Err("mutations require a plain target table".into());
    }
    let parts = names(name.clone())?;
    if parts.len() != 3 || parts[0] != "ice" {
        return Err("mutations require exactly ice.<namespace>.<table>".into());
    }
    let ident = TableIdent::from_strs([parts[1].as_str(), parts[2].as_str()])?;
    let alias = alias.as_ref().map(|a| identifier(&a.name)).unwrap_or_else(|| parts[2].clone());
    Ok((ident, alias, parts.join(".")))
}

enum MatchedAction {
    Update(Vec<(String, String)>),
    Delete,
}

struct MatchedClause {
    predicate: String,
    action: MatchedAction,
}

struct InsertClause {
    predicate: String,
    columns: Vec<String>,
    values: Vec<String>,
}

struct JoinedPlan {
    ident: TableIdent,
    target_sql: String,
    target_alias: String,
    source_render: String,
    source_alias: String,
    on: String,
    matched: Vec<MatchedClause>,
    inserts: Vec<InsertClause>,
}

fn merge_plan(merge: &datafusion::sql::sqlparser::ast::Merge) -> Result<JoinedPlan> {
    if merge.output.is_some() {
        return Err("MERGE OUTPUT is not supported".into());
    }
    let (ident, target_alias, target_sql) = target_factor(&merge.table)?;
    let (source_alias, source_render) = factor_alias_render(&merge.source)?;
    let mut matched = Vec::new();
    let mut inserts = Vec::new();
    for clause in &merge.clauses {
        let predicate = match &clause.predicate {
            Some(expr) => scalar_expr(&expr.to_string())?,
            None => "TRUE".into(),
        };
        match clause.clause_kind {
            MergeClauseKind::Matched => {
                let action = match &clause.action {
                    MergeAction::Update(update) => {
                        if update.update_predicate.is_some() || update.delete_predicate.is_some() {
                            return Err("MERGE UPDATE predicates are not supported".into());
                        }
                        MatchedAction::Update(assignment_pairs(&update.assignments)?)
                    }
                    MergeAction::Delete { .. } => MatchedAction::Delete,
                    MergeAction::Insert(_) => return Err("INSERT is not a WHEN MATCHED action".into()),
                };
                matched.push(MatchedClause { predicate, action });
            }
            MergeClauseKind::NotMatched | MergeClauseKind::NotMatchedByTarget => {
                let MergeAction::Insert(insert) = &clause.action else {
                    return Err("WHEN NOT MATCHED requires an INSERT action".into());
                };
                if insert.insert_predicate.is_some() {
                    return Err("MERGE INSERT predicates are not supported".into());
                }
                let columns = insert.columns.iter().map(|name| {
                    let parts = names(name.clone())?;
                    parts.last().cloned().ok_or_else(|| "insert column is empty".into())
                }).collect::<Result<Vec<_>>>()?;
                let values = match &insert.kind {
                    MergeInsertKind::Values(values) => {
                        if values.rows.len() != 1 {
                            return Err("MERGE INSERT supports exactly one VALUES row".into());
                        }
                        values.rows[0].iter().map(|expr| scalar_expr(&expr.to_string())).collect::<Result<Vec<_>>>()?
                    }
                    MergeInsertKind::Row => return Err("MERGE INSERT ROW is not supported".into()),
                };
                if !columns.is_empty() && columns.len() != values.len() {
                    return Err("MERGE INSERT column and value counts differ".into());
                }
                inserts.push(InsertClause { predicate, columns, values });
            }
            MergeClauseKind::NotMatchedBySource => {
                return Err("WHEN NOT MATCHED BY SOURCE is not supported".into());
            }
        }
    }
    Ok(JoinedPlan {
        ident, target_sql, target_alias, source_render, source_alias,
        on: scalar_expr(&merge.on.to_string())?, matched, inserts,
    })
}

fn update_from_plan(update: &datafusion::sql::sqlparser::ast::Update) -> Result<JoinedPlan> {
    if update.returning.is_some() || !update.order_by.is_empty() || update.limit.is_some() || update.output.is_some() {
        return Err("unsupported UPDATE clause with FROM".into());
    }
    if !update.table.joins.is_empty() {
        return Err("UPDATE with joins in the target clause is not supported".into());
    }
    let (ident, target_alias, target_sql) = target_factor(&update.table.relation)?;
    let from = match &update.from {
        Some(UpdateTableFromKind::AfterSet(items)) | Some(UpdateTableFromKind::BeforeSet(items)) => items,
        None => return Err("UPDATE without FROM is not a joined mutation".into()),
    };
    if from.len() != 1 || !from[0].joins.is_empty() {
        return Err("UPDATE with FROM supports a single source table or subquery".into());
    }
    let (source_alias, source_render) = factor_alias_render(&from[0].relation)?;
    let on = match &update.selection {
        Some(expr) => scalar_expr(&expr.to_string())?,
        None => "TRUE".into(),
    };
    Ok(JoinedPlan {
        ident, target_sql, target_alias, source_render, source_alias, on,
        matched: vec![MatchedClause {
            predicate: "TRUE".into(),
            action: MatchedAction::Update(assignment_pairs(&update.assignments)?),
        }],
        inserts: Vec::new(),
    })
}

/// Returns a plan only for MERGE and for UPDATE with a FROM clause.
/// Anything else returns None and keeps its existing behavior; a MERGE
/// or UPDATE FROM that parses but uses unsupported clauses is an error.
fn joined_plan(sql: &str) -> Result<Option<JoinedPlan>> {
    let statements = match Parser::parse_sql(&GenericDialect, sql) {
        Ok(statements) => statements,
        Err(_) => return Ok(None),
    };
    if statements.len() != 1 {
        return Ok(None);
    }
    match &statements[0] {
        Statement::Merge(merge) => Ok(Some(merge_plan(merge)?)),
        Statement::Update(update) => {
            if update.from.is_none() {
                Ok(None)
            } else {
                Ok(Some(update_from_plan(update)?))
            }
        }
        _ => Ok(None),
    }
}

fn sql_type(data_type: &DataType) -> String {
    match data_type {
        DataType::Int64 => "BIGINT".into(),
        DataType::Int32 => "INT".into(),
        DataType::Int16 => "SMALLINT".into(),
        DataType::Int8 => "TINYINT".into(),
        DataType::UInt64 => "BIGINT UNSIGNED".into(),
        DataType::Float64 => "DOUBLE".into(),
        DataType::Float32 => "FLOAT".into(),
        DataType::Utf8 | DataType::LargeUtf8 => "VARCHAR".into(),
        DataType::Boolean => "BOOLEAN".into(),
        DataType::Date32 | DataType::Date64 => "DATE".into(),
        other => format!("{other}"),
    }
}

/// Casts computed batches to the target schema. DataFusion promotes
/// some types in CASE and CAST expressions (VARCHAR results come back
/// as Utf8View), and replace_all rightly requires exact types.
fn coerce_to_schema(schema: &SchemaRef, batches: Vec<RecordBatch>) -> Result<Vec<RecordBatch>> {
    batches.into_iter().map(|batch| {
        if batch.num_columns() != schema.fields().len() {
            return Err("computed row set does not match the target table's columns".into());
        }
        let mut columns = Vec::new();
        for (column, field) in batch.columns().iter().zip(schema.fields()) {
            if column.data_type() == field.data_type() {
                columns.push(column.clone());
            } else {
                columns.push(arrow::compute::kernels::cast::cast(column, field.data_type())?);
            }
        }
        Ok(RecordBatch::try_new(schema.clone(), columns)?)
    }).collect()
}

fn count_cell(batches: &[RecordBatch]) -> Result<i64> {
    use arrow::array::Array;
    let batch = batches.first().ok_or("count query returned no batch")?;
    let array = batch.column(0).as_any().downcast_ref::<arrow::array::Int64Array>()
        .ok_or("count query did not return an integer")?;
    if array.is_null(0) {
        Ok(0)
    } else {
        Ok(array.value(0))
    }
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
    // Name the actual cause. A v3 table (the provider creates one for a
    // nanosecond TIMESTAMP column) is not a partitioning problem, and the
    // old combined message sent Gate 1 debugging at the wrong suspect.
    if table.metadata().format_version() != FormatVersion::V2 {
        let version = match table.metadata().format_version() {
            FormatVersion::V1 => "v1",
            FormatVersion::V2 => "v2",
            FormatVersion::V3 => "v3",
        };
        return Err(format!(
            "copy-on-write supports Iceberg format version v2 tables only; this table is format version {version}"
        ).into());
    }
    if table.metadata().partition_specs_iter().any(|spec| !spec.is_unpartitioned()) {
        return Err("copy-on-write currently supports unpartitioned tables; this table is partitioned".into());
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
    /// Runs a MERGE or an UPDATE with FROM through the joined mutation
    /// path: the new row set is computed with SELECTs over the catalog
    /// and committed by replace_all as exactly one overwrite snapshot.
    /// A merge or update that applies to no row creates no snapshot.
    async fn execute_joined(&self, plan: JoinedPlan) -> Result<Vec<RecordBatch>> {
        let table = self.catalog.load_table(&plan.ident).await?;
        let schema = schema(&table)?;
        let fields: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
        for clause in &plan.matched {
            if let MatchedAction::Update(pairs) = &clause.action {
                for (column, _) in pairs {
                    schema.field_with_name(column)?;
                }
            }
        }
        for insert in &plan.inserts {
            let columns = if insert.columns.is_empty() { &fields } else { &insert.columns };
            if columns.len() != insert.values.len() {
                return Err("MERGE INSERT column and value counts differ".into());
            }
            for column in columns {
                schema.field_with_name(column)?;
            }
        }
        let ta = &plan.target_alias;
        let sa = &plan.source_alias;
        let ctes = format!(
            "WITH __opensnow_tgt AS (SELECT *, 1 AS __os_tgt FROM {}), \
             __opensnow_src AS (SELECT *, 1 AS __os_src FROM {})",
            plan.target_sql, plan.source_render);
        let join = format!(
            "FROM __opensnow_tgt AS {ta} LEFT JOIN __opensnow_src AS {sa} ON ({})", plan.on);
        let target_rows = count_cell(&self.sql(
            &format!("SELECT COUNT(*) FROM {}", plan.target_sql)).await?)?;
        let joined_rows = count_cell(&self.sql(
            &format!("{ctes} SELECT COUNT(*) {join}")).await?)?;
        if joined_rows > target_rows {
            return Err("mutation source matches a target row more than once".into());
        }
        let applied = if plan.matched.is_empty() {
            0
        } else {
            let any = plan.matched.iter().map(|c| format!("({})", c.predicate))
                .collect::<Vec<_>>().join(" OR ");
            count_cell(&self.sql(&format!(
                "{ctes} SELECT COUNT(*) {join} WHERE {sa}.__os_src IS NOT NULL AND ({any})")).await?)?
        };
        let insert_any = plan.inserts.iter().map(|c| format!("({})", c.predicate))
            .collect::<Vec<_>>().join(" OR ");
        let insert_count = if plan.inserts.is_empty() {
            0
        } else {
            count_cell(&self.sql(&format!(
                "{ctes} SELECT COUNT(*) FROM __opensnow_src AS {sa} \
                 LEFT JOIN __opensnow_tgt AS {ta} ON ({}) \
                 WHERE {ta}.__os_tgt IS NULL AND ({insert_any})", plan.on)).await?)?
        };
        if applied == 0 && insert_count == 0 {
            return Ok(vec![]);
        }
        let mut projections = Vec::new();
        for field in schema.fields() {
            let name = field.name();
            let mut case = format!("CASE WHEN {sa}.__os_src IS NULL THEN {ta}.\"{name}\"");
            for clause in &plan.matched {
                if let MatchedAction::Update(pairs) = &clause.action {
                    if let Some((_, expr)) = pairs.iter().find(|(column, _)| column == name) {
                        case.push_str(&format!(
                            " WHEN ({}) THEN CAST({expr} AS {})",
                            clause.predicate, sql_type(field.data_type())));
                    }
                }
            }
            case.push_str(&format!(" ELSE {ta}.\"{name}\" END AS \"{name}\""));
            projections.push(case);
        }
        let mut drop_case = format!("CASE WHEN {sa}.__os_src IS NULL THEN 0");
        for clause in &plan.matched {
            let value = if matches!(clause.action, MatchedAction::Delete) { 1 } else { 0 };
            drop_case.push_str(&format!(" WHEN ({}) THEN {value}", clause.predicate));
        }
        drop_case.push_str(" ELSE 0 END");
        let outer = fields.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(", ");
        let target_side = format!(
            "{ctes} SELECT {outer} FROM (SELECT {}, {drop_case} AS __os_drop {join}) AS __os_m \
             WHERE __os_drop = 0", projections.join(", "));
        let mut batches = self.sql(&target_side).await?;
        if insert_count > 0 {
            let mut projections = Vec::new();
            for field in schema.fields() {
                let name = field.name();
                let mut case = String::from("CASE");
                for insert in &plan.inserts {
                    let columns = if insert.columns.is_empty() { &fields } else { &insert.columns };
                    let branch = match columns.iter().position(|c| c == name) {
                        Some(index) => format!(
                            "CAST({} AS {})", insert.values[index], sql_type(field.data_type())),
                        None => format!("CAST(NULL AS {})", sql_type(field.data_type())),
                    };
                    case.push_str(&format!(" WHEN ({}) THEN {branch}", insert.predicate));
                }
                case.push_str(&format!(
                    " ELSE CAST(NULL AS {}) END AS \"{name}\"", sql_type(field.data_type())));
                projections.push(case);
            }
            let insert_sql = format!(
                "{ctes} SELECT {} FROM __opensnow_src AS {sa} \
                 LEFT JOIN __opensnow_tgt AS {ta} ON ({}) \
                 WHERE {ta}.__os_tgt IS NULL AND ({insert_any})",
                projections.join(", "), plan.on);
            batches.extend(self.sql(&insert_sql).await?);
        }
        let batches = coerce_to_schema(&schema, batches)?;
        self.replace_all(&plan.ident, batches).await?;
        Ok(vec![])
    }

    /// Runs narrow DELETE/UPDATE statements through per-file copy-on-write.
    /// Other statements retain `sql` behavior. Mutations return no result batches.
    pub async fn execute(&self, sql: &str) -> Result<Vec<RecordBatch>> {
        if let Some(plan) = joined_plan(sql)? {
            return self.execute_joined(plan).await;
        }
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
