use opensnow_control::{ControlPlane, Principal};
use sqlparser::ast::{
    BinaryOperator, Expr, GroupByExpr, JoinConstraint, JoinOperator, ObjectName, OrderByKind,
    SelectItem, SetExpr, TableFactor, UnaryOperator, Value,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use super::{string_literal, Error, Field, Reply, Result};

// The unfiltered psql 16 \dt query. A tenant namespace is visible on this
// connection. There are no system, TOAST, partitioned or access-method rows.
const PSQL_DT: &str = "SELECT n.nspname AS \"Schema\", c.relname AS \"Name\",
  CASE c.relkind WHEN 'r' THEN 'table' WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized view'
    WHEN 'i' THEN 'index' WHEN 'S' THEN 'sequence' WHEN 't' THEN 'TOAST table'
    WHEN 'f' THEN 'foreign table' WHEN 'p' THEN 'partitioned table'
    WHEN 'I' THEN 'partitioned index' END AS \"Type\",
  pg_catalog.pg_get_userbyid(c.relowner) AS \"Owner\"
  FROM pg_catalog.pg_class c
  LEFT JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
  LEFT JOIN pg_catalog.pg_am am ON am.oid = c.relam
  WHERE c.relkind IN ('r','p','') AND n.nspname <> 'pg_catalog'
    AND n.nspname !~ '^pg_toast' AND n.nspname <> 'information_schema'
    AND pg_catalog.pg_table_is_visible(c.oid)
  ORDER BY 1,2";

#[derive(Clone, Copy)]
enum Source {
    Tables,
    Columns,
    Class,
    Attribute,
    Psql,
}

type Row = Vec<Option<String>>;

pub(super) struct Query {
    source: Source,
    projection: Vec<(usize, String)>,
    filters: Vec<Filter>,
    order: Vec<(usize, bool)>,
}

struct Relation<'a> {
    name: &'a ObjectName,
    alias: Option<&'a str>,
}

impl<'a> Relation<'a> {
    fn new(factor: &'a TableFactor) -> Result<Self> {
        let TableFactor::Table { name, alias, .. } = factor else {
            return Err(Error::unsupported());
        };
        if let Some(alias) = alias {
            if !alias.columns.is_empty() {
                return Err(Error::unsupported());
            }
        }
        let simple = match alias {
            Some(alias) => [format!("{name} {alias}"), format!("{name} AS {alias}")]
                .contains(&factor.to_string()),
            None => name.to_string() == factor.to_string(),
        };
        if !simple {
            return Err(Error::unsupported());
        }
        Ok(Self {
            name,
            alias: alias.as_ref().map(|alias| alias.name.value.as_str()),
        })
    }

    fn is(&self, schema: &str, name: &str) -> bool {
        match self.name.0.as_slice() {
            [part] => schema == "pg_catalog" && part.to_string().eq_ignore_ascii_case(name),
            [prefix, part] => identifier_matches(prefix, schema) && identifier_matches(part, name),
            _ => false,
        }
    }

    fn qualifier(&self) -> String {
        self.alias
            .map(str::to_string)
            .unwrap_or_else(|| self.name.0.last().unwrap().to_string())
    }
}

fn identifier_matches(ident: &impl ToString, name: &str) -> bool {
    let rendered = ident.to_string();
    rendered.eq_ignore_ascii_case(name) || rendered == format!("\"{name}\"")
}

impl Source {
    fn columns(self) -> &'static [(&'static str, u32)] {
        match self {
            Self::Tables => &[
                ("table_catalog", 25),
                ("table_schema", 25),
                ("table_name", 25),
                ("table_type", 25),
            ],
            Self::Columns => &[
                ("table_schema", 25),
                ("table_name", 25),
                ("column_name", 25),
                ("ordinal_position", 23),
                ("data_type", 25),
                ("is_nullable", 25),
            ],
            Self::Class => &[
                ("relname", 25),
                ("relkind", 25),
                ("nspname", 25),
                ("oid", 26),
            ],
            Self::Attribute => &[
                ("attname", 25),
                ("atttypid", 26),
                ("attnum", 21),
                ("attrelid", 26),
                ("attisdropped", 16),
            ],
            Self::Psql => &[("Schema", 25), ("Name", 25), ("Type", 25), ("Owner", 25)],
        }
    }
}

impl Query {
    pub fn parse(query: &sqlparser::ast::Query) -> Result<Self> {
        let SetExpr::Select(select) = query.body.as_ref() else {
            return Err(Error::unsupported());
        };
        if select.projection.len() == 4 && select.from.len() == 1 && select.from[0].joins.len() == 2
        {
            let standard = Parser::parse_sql(&PostgreSqlDialect {}, PSQL_DT).unwrap();
            if query.to_string() == standard[0].to_string() {
                return Ok(Self {
                    source: Source::Psql,
                    projection: standard_projection(Source::Psql),
                    filters: Vec::new(),
                    order: vec![(0, true), (1, true)],
                });
            }
        }
        let [from] = select.from.as_slice() else {
            return Err(Error::unsupported());
        };
        if query.with.is_some()
            || query.limit_clause.is_some()
            || query.fetch.is_some()
            || !query.locks.is_empty()
            || query.for_clause.is_some()
            || query.settings.is_some()
            || query.format_clause.is_some()
            || !query.pipe_operators.is_empty()
            || select.distinct.is_some()
            || select.top.is_some()
            || select.into.is_some()
            || !select.optimizer_hints.is_empty()
            || select.select_modifiers.is_some()
            || select.exclude.is_some()
            || !select.lateral_views.is_empty()
            || select.prewhere.is_some()
            || !select.connect_by.is_empty()
            || !select.cluster_by.is_empty()
            || !select.distribute_by.is_empty()
            || !select.sort_by.is_empty()
            || select.having.is_some()
            || !select.named_window.is_empty()
            || select.qualify.is_some()
            || select.value_table_mode.is_some()
            || !matches!(&select.group_by, GroupByExpr::Expressions(expr, modifiers)
                if expr.is_empty() && modifiers.is_empty())
        {
            return Err(Error::unsupported());
        }
        let relation = Relation::new(&from.relation)?;
        let source = if relation.is("information_schema", "tables") {
            Source::Tables
        } else if relation.is("information_schema", "columns") {
            Source::Columns
        } else if relation.is("pg_catalog", "pg_class") {
            Source::Class
        } else if relation.is("pg_catalog", "pg_attribute") {
            Source::Attribute
        } else {
            return Err(Error::unsupported());
        };
        let base = relation.qualifier();
        let namespace = match from.joins.as_slice() {
            [] => None,
            [join] if matches!(source, Source::Class) => {
                let namespace = Relation::new(&join.relation)?;
                if !namespace.is("pg_catalog", "pg_namespace") {
                    return Err(Error::unsupported());
                }
                let (JoinOperator::Join(JoinConstraint::On(condition))
                | JoinOperator::Inner(JoinConstraint::On(condition))
                | JoinOperator::Left(JoinConstraint::On(condition))
                | JoinOperator::LeftOuter(JoinConstraint::On(condition))) = &join.join_operator
                else {
                    return Err(Error::unsupported());
                };
                let namespace = namespace.qualifier();
                let condition = condition.to_string().to_ascii_lowercase();
                let lhs = format!("{base}.relnamespace").to_ascii_lowercase();
                let rhs = format!("{namespace}.oid").to_ascii_lowercase();
                if condition != format!("{lhs} = {rhs}") && condition != format!("{rhs} = {lhs}") {
                    return Err(Error::unsupported());
                }
                Some(namespace)
            }
            _ => return Err(Error::unsupported()),
        };
        let column = |expr: &Expr| {
            let (qualifier, name) = match expr {
                Expr::Identifier(name) => (None, name),
                Expr::CompoundIdentifier(parts) => {
                    let [relation, name] = parts.as_slice() else {
                        return Err(Error::unsupported());
                    };
                    (Some(relation.value.as_str()), name)
                }
                _ => return Err(Error::unsupported()),
            };
            let owner =
                if matches!(source, Source::Class) && name.value.eq_ignore_ascii_case("nspname") {
                    namespace.as_deref()
                } else {
                    Some(base.as_str())
                };
            if let Some(qualifier) = qualifier {
                if !owner.is_some_and(|owner| owner.eq_ignore_ascii_case(qualifier)) {
                    return Err(Error::unsupported());
                }
            } else if namespace.is_some() && name.value.eq_ignore_ascii_case("oid") {
                return Err(Error::unsupported());
            }
            source
                .columns()
                .iter()
                .position(|(column, _)| identifier_matches(name, column))
                .ok_or_else(Error::unsupported)
        };
        let projection = match select.projection.as_slice() {
            [item @ SelectItem::Wildcard(_)] if item.to_string() == "*" => {
                standard_projection(source)
            }
            fields => fields
                .iter()
                .map(|field| match field {
                    SelectItem::UnnamedExpr(expr) => {
                        let index = column(expr)?;
                        Ok((index, source.columns()[index].0.into()))
                    }
                    SelectItem::ExprWithAlias { expr, alias } => {
                        let name = if alias.quote_style.is_some() {
                            alias.value.clone()
                        } else {
                            alias.value.to_ascii_lowercase()
                        };
                        Ok((column(expr)?, name))
                    }
                    _ => Err(Error::unsupported()),
                })
                .collect::<Result<Vec<_>>>()?,
        };
        let mut filters = Vec::new();
        if let Some(predicate) = &select.selection {
            add_filters(predicate, &column, &mut filters)?;
        }
        let order = match &query.order_by {
            None => Vec::new(),
            Some(order) => {
                let OrderByKind::Expressions(items) = &order.kind else {
                    return Err(Error::unsupported());
                };
                if order.interpolate.is_some() {
                    return Err(Error::unsupported());
                }
                items
                    .iter()
                    .map(|item| {
                        if item.with_fill.is_some() {
                            return Err(Error::unsupported());
                        }
                        let index = if let Ok(position) = item.expr.to_string().parse::<usize>() {
                            position
                                .checked_sub(1)
                                .and_then(|position| projection.get(position))
                                .map(|(index, _)| *index)
                                .ok_or_else(Error::unsupported)?
                        } else if let Some((index, _)) = projection
                            .iter()
                            .find(|(_, name)| identifier_matches(&item.expr, name))
                        {
                            *index
                        } else {
                            column(&item.expr)?
                        };
                        Ok((index, item.options.asc.unwrap_or(true)))
                    })
                    .collect::<Result<Vec<_>>>()?
            }
        };
        Ok(Self {
            source,
            projection,
            filters,
            order,
        })
    }

    pub fn fields(&self) -> Vec<Field> {
        self.projection
            .iter()
            .map(|(index, name)| (name.clone(), self.source.columns()[*index].1))
            .collect()
    }

    pub async fn execute(self, plane: &ControlPlane, principal: &Principal) -> Result<Reply> {
        let tables = plane.list_tables_for_principal(principal).await?;
        let mut rows = Vec::new();
        for (index, table) in tables.into_iter().enumerate() {
            let namespace = &principal.namespace;
            let oid = (16384 + index).to_string();
            match self.source {
                Source::Tables => rows.push(text([
                    &principal.tenant_name,
                    namespace,
                    &table,
                    "BASE TABLE",
                ])),
                Source::Class => rows.push(text([&table, "r", namespace, &oid])),
                Source::Psql => rows.push(vec![
                    Some(namespace.clone()),
                    Some(table),
                    Some("table".into()),
                    None,
                ]),
                Source::Columns | Source::Attribute => {
                    let (schema_index, table_index) = (0, 1);
                    let matches_table = match self.source {
                        Source::Columns => self.filters.iter().all(|filter| match filter.column {
                            index if index == schema_index => {
                                filter.matches(Some(namespace), principal)
                            }
                            index if index == table_index => {
                                filter.matches(Some(&table), principal)
                            }
                            _ => true,
                        }),
                        _ => self.filters.iter().all(|filter| {
                            filter.column != 3 || filter.matches(Some(&oid), principal)
                        }),
                    };
                    if !matches_table {
                        continue;
                    }
                    let columns = plane.describe_table_fields(principal, &table).await?;
                    for (index, (column, required)) in columns.into_iter().enumerate() {
                        let position = (index + 1).to_string();
                        rows.push(match self.source {
                            Source::Columns => text([
                                namespace,
                                &table,
                                &column.name,
                                &position,
                                &column.data_type.to_ascii_lowercase(),
                                if required { "NO" } else { "YES" },
                            ]),
                            _ => text([
                                &column.name,
                                &iceberg_oid(&column.data_type).to_string(),
                                &position,
                                &oid,
                                "false",
                            ]),
                        });
                    }
                }
            }
        }
        if self
            .filters
            .iter()
            .any(|filter| matches!(filter.value, Operand::Parameter))
        {
            return Err(Error::protocol());
        }
        rows.retain(|row| {
            self.filters
                .iter()
                .all(|filter| filter.matches(row[filter.column].as_deref(), principal))
        });
        rows.sort_by(|left, right| {
            for (column, asc) in &self.order {
                let left = left[*column].as_deref();
                let right = right[*column].as_deref();
                let comparison = if matches!(self.source.columns()[*column].1, 21 | 23 | 26) {
                    left.and_then(|value| value.parse::<i64>().ok())
                        .cmp(&right.and_then(|value| value.parse::<i64>().ok()))
                } else {
                    left.cmp(&right)
                };
                if !comparison.is_eq() {
                    return if *asc {
                        comparison
                    } else {
                        comparison.reverse()
                    };
                }
            }
            std::cmp::Ordering::Equal
        });
        Ok(Reply::table(
            self.fields(),
            rows.into_iter()
                .map(|row| {
                    self.projection
                        .iter()
                        .map(|(index, _)| row[*index].clone())
                        .collect()
                })
                .collect(),
        ))
    }
}

fn standard_projection(source: Source) -> Vec<(usize, String)> {
    source
        .columns()
        .iter()
        .enumerate()
        .map(|(index, (name, _))| (index, (*name).into()))
        .collect()
}

fn text<const N: usize>(values: [&str; N]) -> Row {
    values
        .into_iter()
        .map(|value| Some(value.to_owned()))
        .collect()
}

fn iceberg_oid(kind: &str) -> u32 {
    match kind.to_ascii_lowercase().as_str() {
        "boolean" => 16,
        "int" => 23,
        "bigint" => 20,
        "float" => 700,
        "double" => 701,
        "date" => 1082,
        "time" => 1083,
        "timestamp" | "timestamp with time zone" | "timestamp_ns" | "timestamptz_ns" => 1114,
        "binary" => 17,
        kind if kind.starts_with("decimal(") => 1700,
        _ => 25,
    }
}

enum Operand {
    Text(String),
    Number(i64),
    Null,
    Schema,
    Database,
    Parameter,
}

enum Comparison {
    Equal,
    NotEqual,
    Greater,
    Like,
}

struct Filter {
    column: usize,
    comparison: Comparison,
    value: Operand,
}

impl Filter {
    fn matches(&self, actual: Option<&str>, principal: &Principal) -> bool {
        let Some(actual) = actual else {
            return false;
        };
        let expected = match &self.value {
            Operand::Text(value) => value,
            Operand::Number(expected) => {
                return actual
                    .parse::<i64>()
                    .is_ok_and(|actual| match self.comparison {
                        Comparison::Equal => actual == *expected,
                        Comparison::NotEqual => actual != *expected,
                        Comparison::Greater => actual > *expected,
                        Comparison::Like => false,
                    });
            }
            Operand::Schema => &principal.namespace,
            Operand::Database => &principal.tenant_name,
            Operand::Null | Operand::Parameter => return false,
        };
        match self.comparison {
            Comparison::Equal => actual == expected,
            Comparison::NotEqual => actual != expected,
            Comparison::Like => like(actual, expected),
            Comparison::Greater => actual > expected.as_str(),
        }
    }
}

fn add_filters(
    expr: &Expr,
    column: &impl Fn(&Expr) -> Result<usize>,
    filters: &mut Vec<Filter>,
) -> Result<()> {
    let (left, comparison, right) = match expr {
        Expr::Nested(expr) => return add_filters(expr, column, filters),
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            add_filters(left, column, filters)?;
            return add_filters(right, column, filters);
        }
        Expr::BinaryOp { left, op, right } => (
            left,
            match op {
                BinaryOperator::Eq => Comparison::Equal,
                BinaryOperator::NotEq => Comparison::NotEqual,
                BinaryOperator::Gt => Comparison::Greater,
                _ => return Err(Error::unsupported()),
            },
            right,
        ),
        Expr::Like {
            expr,
            pattern,
            negated: false,
            any: false,
            escape_char: None,
        } => (expr, Comparison::Like, pattern),
        Expr::UnaryOp {
            op: UnaryOperator::Not,
            expr,
        } if expr.to_string().eq_ignore_ascii_case("attisdropped") => {
            filters.push(Filter {
                column: column(expr)?,
                comparison: Comparison::Equal,
                value: Operand::Text("false".into()),
            });
            return Ok(());
        }
        _ => return Err(Error::unsupported()),
    };
    let value = operand(right)?;
    filters.push(Filter {
        column: column(left)?,
        comparison,
        value,
    });
    Ok(())
}

fn operand(expr: &Expr) -> Result<Operand> {
    if let Expr::Nested(expr) = expr {
        return operand(expr);
    }
    if let Some(value) = string_literal(expr) {
        return Ok(Operand::Text(value.into()));
    }
    if let Expr::Value(value) = expr {
        return match &value.value {
            Value::Placeholder(_) => Ok(Operand::Parameter),
            Value::Number(number, _) => number
                .parse()
                .map(Operand::Number)
                .map_err(|_| Error::unsupported()),
            Value::Boolean(value) => Ok(Operand::Text(value.to_string())),
            Value::Null => Ok(Operand::Null),
            _ => Err(Error::unsupported()),
        };
    }
    match expr.to_string().to_ascii_lowercase().as_str() {
        "current_schema()" => Ok(Operand::Schema),
        "current_database()" => Ok(Operand::Database),
        _ => expr
            .to_string()
            .parse()
            .map(Operand::Number)
            .map_err(|_| Error::unsupported()),
    }
}

/// Match SQL LIKE with its default backslash escape.
fn like(text: &str, pattern: &str) -> bool {
    let text: Vec<_> = text.chars().collect();
    let mut matches = vec![false; text.len() + 1];
    matches[0] = true;
    let mut pattern = pattern.chars();
    while let Some(character) = pattern.next() {
        match character {
            '%' => {
                for i in 1..matches.len() {
                    matches[i] |= matches[i - 1];
                }
            }
            character => {
                let wildcard = character == '_';
                let literal = if character == '\\' {
                    let Some(escaped) = pattern.next() else {
                        return false;
                    };
                    escaped
                } else {
                    character
                };
                for i in (1..matches.len()).rev() {
                    matches[i] = matches[i - 1] && (wildcard || text[i - 1] == literal);
                }
                matches[0] = false;
            }
        }
    }
    matches[text.len()]
}
