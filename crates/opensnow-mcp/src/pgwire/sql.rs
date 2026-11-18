mod catalog;

use std::borrow::Cow;
use std::ops::Range;

use arrow::array::UInt64Array;
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use opensnow_control::{guard_sql, ControlPlane, Principal};
use sqlparser::ast::{
    Expr, FunctionArg, FunctionArgExpr, FunctionArguments, SelectItem, SetExpr, Statement, Value,
};
use sqlparser::dialect::{GenericDialect, PostgreSqlDialect};
use sqlparser::keywords::Keyword;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Location, Token, TokenWithSpan, Tokenizer};

use super::protocol::{self, cstring, send, Error, Field, Result};
use super::{Session, Stream, VERSION};

fn syntax(error: impl ToString) -> Error {
    Error::new("42601", error.to_string())
}

/// Translate tokenizer source locations to byte offsets without rewriting SQL.
fn source_offsets(sql: &str) -> impl FnMut(Location) -> usize + '_ {
    let mut chars = sql.char_indices().peekable();
    let mut current = Location { line: 1, column: 1 };
    move |target| {
        while current != target {
            let (_, character) = chars.next().expect("token span lies within SQL source");
            if character == '\n' {
                current.line += 1;
                current.column = 1;
            } else {
                current.column += 1;
            }
        }
        chars.peek().map_or(sql.len(), |(offset, _)| *offset)
    }
}

pub(super) fn split(sql: &str) -> (Vec<&str>, Result<()>) {
    let mut tokens = Vec::new();
    let lexed = Tokenizer::new(&PostgreSqlDialect {}, sql)
        .tokenize_with_location_into_buf(&mut tokens)
        .map_err(syntax);
    let mut offset = source_offsets(sql);
    let mut depth = 0usize;
    let mut start = 0;
    let mut statements = Vec::new();
    for token in tokens {
        match token.token {
            Token::LParen | Token::LBracket => depth += 1,
            Token::RParen | Token::RBracket => depth = depth.saturating_sub(1),
            Token::SemiColon if depth == 0 => {
                let end = offset(token.span.start);
                let statement = sql[start..end].trim();
                if !statement.is_empty() {
                    statements.push(statement);
                }
                start = end + 1;
            }
            _ => (),
        }
    }
    if lexed.is_ok() && !sql[start..].trim().is_empty() {
        statements.push(sql[start..].trim());
    }
    (statements, lexed)
}

pub(super) struct Prepared {
    query: String,
    placeholders: Vec<(Range<usize>, usize)>,
    count: usize,
}

impl Prepared {
    pub fn parse(sql: &str, principal: &Principal) -> Result<Self> {
        let (statements, lexed) = split(sql);
        lexed?;
        let query = match statements.as_slice() {
            [] => "",
            [query] => *query,
            _ => return Err(syntax("expected one prepared statement")),
        };
        let tokens = tokenize(query)?;
        let mut offset = source_offsets(query);
        let mut count = 0;
        let mut placeholders = Vec::new();
        for token in &tokens {
            if let Token::Placeholder(name) = &token.token {
                let number = name
                    .strip_prefix('$')
                    .and_then(|number| number.parse::<u16>().ok())
                    .filter(|number| *number != 0)
                    .ok_or_else(Error::protocol)? as usize;
                count = count.max(number);
                let start = offset(token.span.start);
                placeholders.push((start..start + name.len(), number - 1));
            }
        }
        if classify(query, &tokens)?.is_none() {
            guard_sql(&principal.namespace, &warehouse_sql(query)?)?;
        }
        Ok(Self {
            query: query.into(),
            placeholders,
            count,
        })
    }

    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn parameter_count(&self) -> usize {
        self.count
    }

    pub fn bind(&self, params: &[Option<&str>]) -> Result<String> {
        if params.len() != self.count {
            return Err(Error::protocol());
        }
        let mut sql = String::new();
        let mut end = 0;
        for (span, index) in &self.placeholders {
            sql.push_str(&self.query[end..span.start]);
            match params[*index] {
                None => sql.push_str("NULL"),
                Some(value) if numeric(value) => {
                    // Parenthesize signed literals so a preceding minus cannot
                    // turn a negative value into a SQL comment.
                    sql.push('(');
                    sql.push_str(value);
                    sql.push(')');
                }
                Some(value) => {
                    sql.push('\'');
                    sql.push_str(&value.replace('\'', "''"));
                    sql.push('\'');
                }
            }
            if sql.len() > 16 * 1024 * 1024 {
                return Err(Error::new("54000", "bound statement is too large"));
            }
            end = span.end;
        }
        sql.push_str(&self.query[end..]);
        Ok(sql)
    }
}

fn numeric(value: &str) -> bool {
    let unsigned = value.strip_prefix(['+', '-']).unwrap_or(value);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    !(whole.is_empty() && fraction.is_empty())
        && whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit())
}

fn tokenize(sql: &str) -> Result<Vec<TokenWithSpan>> {
    Tokenizer::new(&PostgreSqlDialect {}, sql)
        .tokenize_with_location()
        .map_err(syntax)
}

/// The OpenSight planner quotes ordinary identifiers. Lowercase identifiers
/// that preserve the parsed SQL shape can use the unchanged warehouse guard.
fn warehouse_sql(sql: &str) -> Result<Cow<'_, str>> {
    // Leave comment rejection to the guard and avoid rewriting tokens from
    // SQL parser optimizer hints, whose spans refer to the hint content.
    if !sql.contains('"') || sql.contains("--") || sql.contains("/*") {
        return Ok(Cow::Borrowed(sql));
    }
    let dialect = GenericDialect {};
    let mut tokens = Tokenizer::new(&dialect, sql)
        .tokenize_with_location()
        .map_err(syntax)?;
    let mut offset = source_offsets(sql);
    let mut normalized = String::with_capacity(sql.len());
    let mut end = 0;
    for token in &mut tokens {
        let Token::Word(word) = &mut token.token else {
            continue;
        };
        if word.quote_style != Some('"') {
            continue;
        }
        let name = &word.value;
        let valid = name.len() <= 63
            && name
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_lowercase() || b == b'_')
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !valid {
            return Err(Error::new(
                "42501",
                "quoted identifier is not a lowercase warehouse identifier",
            ));
        }
        let start = offset(token.span.start);
        normalized.push_str(&sql[end..start]);
        normalized.push_str(name);
        end = start + name.len() + 2;
        // Preserve identifier interpretation for comparison with the SQL
        // that DataFusion will read after qualification.
        word.quote_style = None;
        word.keyword = Keyword::NoKeyword;
    }
    if end == 0 {
        return Ok(Cow::Borrowed(sql));
    }
    normalized.push_str(&sql[end..]);
    let intended = Parser::new(&dialect)
        .with_tokens_with_locations(tokens)
        .parse_statements()
        .map_err(syntax)?;
    let actual = Parser::parse_sql(&dialect, &normalized).map_err(syntax)?;
    if intended != actual {
        return Err(Error::new(
            "42501",
            "quoted identifier changes meaning without quotes",
        ));
    }
    Ok(Cow::Owned(normalized))
}

fn classify(sql: &str, tokens: &[TokenWithSpan]) -> Result<Option<Local>> {
    let words: Vec<_> = tokens
        .iter()
        .map(|token| &token.token)
        .filter(|token| !matches!(token, Token::Whitespace(_)))
        .collect();
    let keyword = |index: usize, name: &str| matches!(words.get(index), Some(Token::Word(word)) if word.quote_style.is_none() && word.value.eq_ignore_ascii_case(name));
    if sql.is_empty() {
        return Ok(Some(Local::Empty));
    }
    if keyword(0, "COPY") {
        return Err(Error::unsupported());
    }
    if keyword(0, "SET")
        || keyword(0, "SHOW")
        || keyword(0, "BEGIN")
        || keyword(0, "START")
        || keyword(0, "COMMIT")
        || keyword(0, "END")
        || keyword(0, "ROLLBACK")
    {
        let mut statements = Parser::parse_sql(&PostgreSqlDialect {}, sql).map_err(syntax)?;
        return match statements.pop() {
            Some(Statement::Set(_)) => Ok(Some(Local::Command("SET"))),
            Some(Statement::ShowVariable { variable }) if variable.len() == 1 => {
                Ok(Some(Local::Show(variable[0].value.clone())))
            }
            Some(Statement::StartTransaction { .. }) => Ok(Some(Local::Command("BEGIN"))),
            Some(Statement::Commit { chain: false, .. }) => Ok(Some(Local::Command("COMMIT"))),
            Some(Statement::Rollback {
                chain: false,
                savepoint: None,
            }) => Ok(Some(Local::Command("ROLLBACK"))),
            _ => Err(Error::unsupported()),
        };
    }
    let catalog = words.windows(2).any(|pair| {
        matches!(pair, [Token::Word(word), Token::Period]
            if word.value.eq_ignore_ascii_case("pg_catalog") || word.value.eq_ignore_ascii_case("information_schema"))
    });
    let catalog_relation = words.windows(2).any(|pair| {
        matches!(pair, [Token::Word(prefix), Token::Word(relation)]
            if ["from", "join"].iter().any(|name| prefix.value.eq_ignore_ascii_case(name))
                && ["pg_class", "pg_namespace", "pg_attribute"].iter().any(|name| relation.value.eq_ignore_ascii_case(name)))
    });
    if !catalog && !catalog_relation && !keyword(0, "SELECT") {
        return Ok(None);
    }
    let parsed = Parser::parse_sql(&PostgreSqlDialect {}, sql);
    if let Ok(statements) = parsed {
        if let [Statement::Query(query)] = statements.as_slice() {
            if let Some(probe) = Probe::parse(query) {
                return Ok(Some(Local::Probe(probe)));
            }
            if catalog || catalog_relation {
                return Ok(Some(Local::Catalog(catalog::Query::parse(query)?)));
            }
        }
    }
    if catalog || catalog_relation {
        Err(Error::unsupported())
    } else {
        Ok(None)
    }
}

enum Local {
    Empty,
    Command(&'static str),
    Show(String),
    Probe(Probe),
    Catalog(catalog::Query),
}

struct Probe {
    name: String,
    label: String,
    arguments: Vec<Expr>,
}

impl Probe {
    fn parse(query: &sqlparser::ast::Query) -> Option<Self> {
        let SetExpr::Select(select) = query.body.as_ref() else {
            return None;
        };
        // Comparing the rendered shape keeps clauses that change the answer
        // (WHERE, FROM, DISTINCT, LIMIT, etc.) out of the probe path.
        let [projection] = select.projection.as_slice() else {
            return None;
        };
        if query.to_string() != format!("SELECT {projection}") {
            return None;
        }
        let (expression, alias) = match projection {
            SelectItem::UnnamedExpr(expr) => (expr, None),
            SelectItem::ExprWithAlias { expr, alias } => (expr, Some(alias)),
            _ => return None,
        };
        let (name, arguments) = match expression {
            Expr::Identifier(ident)
                if ident.quote_style.is_none()
                    && ["current_user", "session_user", "user"]
                        .iter()
                        .any(|name| ident.value.eq_ignore_ascii_case(name)) =>
            {
                (ident.value.to_ascii_lowercase(), Vec::new())
            }
            Expr::Function(function) => {
                let qualified = function.name.to_string().to_ascii_lowercase();
                let name = qualified.strip_prefix("pg_catalog.").unwrap_or(&qualified);
                if matches!(function.args, FunctionArguments::None)
                    && ["current_user", "session_user", "user"].contains(&name)
                    && expression.to_string().eq_ignore_ascii_case(name)
                {
                    return Some(Self {
                        name: name.into(),
                        label: alias
                            .map(|alias| identifier_label(alias))
                            .unwrap_or_else(|| name.into()),
                        arguments: Vec::new(),
                    });
                }
                let FunctionArguments::List(list) = &function.args else {
                    return None;
                };
                if expression.to_string() != format!("{}({list})", function.name)
                    || list.duplicate_treatment.is_some()
                    || !list.clauses.is_empty()
                {
                    return None;
                }
                let expected = match name {
                    "version" | "current_database" | "current_schema" | "pg_backend_pid" => 0,
                    "set_config" => 3,
                    "format_type" => 2,
                    _ => return None,
                };
                if list.args.len() != expected {
                    return None;
                }
                let arguments = list
                    .args
                    .iter()
                    .map(|arg| match arg {
                        FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => Some(expr.clone()),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()?;
                (name.into(), arguments)
            }
            _ => return None,
        };
        let label = alias.map(identifier_label).unwrap_or_else(|| name.clone());
        Some(Self {
            name,
            label,
            arguments,
        })
    }

    fn field(&self) -> Field {
        (
            self.label.clone(),
            match self.name.as_str() {
                "pg_backend_pid" => 23,
                _ => 25,
            },
        )
    }

    fn value(&self, session: &Session) -> Result<String> {
        Ok(match self.name.as_str() {
            "version" => format!("PostgreSQL {VERSION}"),
            "current_database" => session.principal.tenant_name.clone(),
            "current_user" | "session_user" | "user" => session.principal.login.clone(),
            "current_schema" => session.principal.namespace.clone(),
            "pg_backend_pid" => session.pid.to_string(),
            "set_config" => {
                let [name, value, local] = self.arguments.as_slice() else {
                    unreachable!()
                };
                let value = string_literal(value).ok_or_else(Error::unsupported)?;
                if string_literal(name).is_none() {
                    return Err(Error::unsupported());
                }
                let local = unnest(local);
                match local {
                    Expr::Value(value) if matches!(value.value, Value::Boolean(_)) => {}
                    _ => match string_literal(local)
                        .map(str::to_ascii_lowercase)
                        .as_deref()
                    {
                        Some("true" | "t" | "false" | "f") => {}
                        _ => return Err(Error::unsupported()),
                    },
                }
                value.to_owned()
            }
            "format_type" => {
                let [oid, modifier] = self.arguments.as_slice() else {
                    unreachable!()
                };
                let oid = unnest(oid)
                    .to_string()
                    .parse()
                    .map_err(|_| Error::unsupported())?;
                let modifier = unnest(modifier).to_string();
                let modifier = if modifier.eq_ignore_ascii_case("NULL") {
                    -1
                } else {
                    modifier.parse::<i32>().map_err(|_| Error::unsupported())?
                };
                format_type(oid, modifier)?
            }
            _ => unreachable!(),
        })
    }
}

fn unnest(mut expr: &Expr) -> &Expr {
    while let Expr::Nested(inner) = expr {
        expr = inner;
    }
    expr
}

fn identifier_label(alias: &sqlparser::ast::Ident) -> String {
    if alias.quote_style.is_some() {
        alias.value.clone()
    } else {
        alias.value.to_ascii_lowercase()
    }
}

fn string_literal(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Value(value) => match &value.value {
            Value::SingleQuotedString(value) => Some(value),
            _ => None,
        },
        _ => None,
    }
}

pub(super) enum Shape {
    Known(Option<Vec<Field>>),
    WarehouseQuery,
}

pub(super) fn shape(session: &Session, sql: &str) -> Result<Shape> {
    Ok(Shape::Known(match classify(sql, &tokenize(sql)?)? {
        Some(Local::Show(name)) => {
            session.setting(&name).ok_or_else(Error::unsupported)?;
            Some(vec![(name.to_ascii_lowercase(), 25)])
        }
        Some(Local::Probe(probe)) => Some(vec![probe.field()]),
        Some(Local::Catalog(query)) => Some(query.fields()),
        Some(Local::Empty | Local::Command(_)) => None,
        None => {
            let sql = warehouse_sql(sql)?;
            match Parser::parse_sql(&GenericDialect {}, &sql)
                .map_err(syntax)?
                .as_slice()
            {
                [Statement::Query(_)] => return Ok(Shape::WarehouseQuery),
                _ => None,
            }
        }
    }))
}

pub(super) async fn execute(
    session: &mut Session,
    plane: &ControlPlane,
    sql: &str,
) -> Result<Reply> {
    let local = classify(sql, &tokenize(sql)?)?;
    if session.transaction == b'E'
        && !matches!(
            local,
            Some(Local::Command("COMMIT" | "ROLLBACK") | Local::Empty)
        )
    {
        return Err(Error::new(
            "25P02",
            "transaction block is in a failed state",
        ));
    }
    if let Some(local) = local {
        return Ok(match local {
            Local::Empty => Reply::command(""),
            Local::Command(command) => {
                let tag = match command {
                    "BEGIN" => {
                        session.transaction = b'T';
                        command
                    }
                    "COMMIT" | "ROLLBACK" => {
                        let status = session.transaction;
                        session.transaction = b'I';
                        if status == b'E' {
                            "ROLLBACK"
                        } else {
                            command
                        }
                    }
                    _ => command,
                };
                Reply::command(tag)
            }
            Local::Show(name) => Reply::scalar(
                (name.to_ascii_lowercase(), 25),
                session
                    .setting(&name)
                    .ok_or_else(Error::unsupported)?
                    .into(),
            ),
            Local::Probe(probe) => Reply::scalar(probe.field(), probe.value(session)?),
            Local::Catalog(query) => query.execute(plane, &session.principal).await?,
        });
    }
    let sql = warehouse_sql(sql)?;
    let statements = Parser::parse_sql(&GenericDialect {}, &sql).map_err(syntax)?;
    let [statement] = statements.as_slice() else {
        return Err(syntax("expected one statement"));
    };
    let result = plane
        .execute_for_principal(&session.principal, &sql, &mut None)
        .await?;
    let rows = result.outcome.rows_returned;
    let tag = match statement {
        Statement::Query(_) => {
            let fields = result
                .schema
                .as_ref()
                .map(|schema| {
                    schema
                        .fields()
                        .iter()
                        .map(|field| (field.name().clone(), arrow_oid(field.data_type())))
                        .collect()
                })
                .unwrap_or_default();
            return Ok(Reply {
                fields: Some(fields),
                rows: Rows::Arrow(result.batches),
                tag: format!("SELECT {rows}"),
            });
        }
        Statement::CreateTable(_) => "CREATE TABLE".into(),
        Statement::Drop { object_type, .. } => format!("DROP {object_type}"),
        Statement::Insert(_) => {
            let inserted = result
                .batches
                .first()
                .and_then(|batch| batch.columns().first())
                .and_then(|column| column.as_any().downcast_ref::<UInt64Array>())
                .filter(|counts| !counts.is_empty())
                .map(|counts| counts.value(0))
                .ok_or_else(|| Error::new("58000", "insert outcome did not report a row count"))?;
            format!("INSERT 0 {inserted}")
        }
        Statement::Update { .. } | Statement::Delete(_) | Statement::Merge { .. } => {
            let kind = match statement {
                Statement::Update { .. } => "UPDATE",
                Statement::Delete(_) => "DELETE",
                _ => "MERGE",
            };
            let affected = result.rows_affected.ok_or_else(|| {
                Error::new("58000", "mutation outcome did not report a row count")
            })?;
            format!("{kind} {affected}")
        }
        _ => return Err(Error::unsupported()),
    };
    Ok(Reply::command(tag))
}

pub(super) struct Reply {
    pub fields: Option<Vec<Field>>,
    rows: Rows,
    tag: String,
}

enum Rows {
    Text(Vec<Vec<Option<String>>>),
    Arrow(Vec<RecordBatch>),
}

impl Reply {
    fn command(tag: impl Into<String>) -> Self {
        Self {
            fields: None,
            rows: Rows::Text(Vec::new()),
            tag: tag.into(),
        }
    }

    fn scalar(field: Field, value: String) -> Self {
        Self::table(vec![field], vec![vec![Some(value)]])
    }

    fn table(fields: Vec<Field>, rows: Vec<Vec<Option<String>>>) -> Self {
        Self {
            fields: Some(fields),
            tag: format!("SELECT {}", rows.len()),
            rows: Rows::Text(rows),
        }
    }

    pub async fn send(&self, stream: &mut dyn Stream, start: usize, max: u32) -> Result<usize> {
        let total = match &self.rows {
            Rows::Text(rows) => rows.len(),
            Rows::Arrow(batches) => batches.iter().map(RecordBatch::num_rows).sum(),
        };
        let end = if max == 0 {
            total
        } else {
            total.min(start.saturating_add(max as usize))
        };
        match &self.rows {
            Rows::Text(rows) => {
                for row in &rows[start..end] {
                    protocol::row(stream, row.clone()).await?;
                }
            }
            Rows::Arrow(batches) => {
                let mut base = 0;
                for batch in batches {
                    let from = start.saturating_sub(base);
                    let to = end.saturating_sub(base).min(batch.num_rows());
                    if from < to {
                        let slice = batch.slice(from, to - from);
                        for row in crate::results::display_rows(&slice) {
                            protocol::row(stream, row).await?;
                        }
                    }
                    base += batch.num_rows();
                    if base >= end {
                        break;
                    }
                }
            }
        }
        if end < total {
            send(stream, b's', &[]).await?;
        } else if self.tag.is_empty() {
            send(stream, b'I', &[]).await?;
        } else {
            let mut body = Vec::new();
            cstring(&mut body, &self.tag);
            send(stream, b'C', &body).await?;
        }
        Ok(end)
    }
}

fn arrow_oid(kind: &DataType) -> u32 {
    match kind {
        DataType::Boolean => 16,
        DataType::Int8 | DataType::Int16 => 21,
        DataType::Int32 | DataType::UInt8 | DataType::UInt16 => 23,
        DataType::Int64 | DataType::UInt32 | DataType::UInt64 => 20,
        DataType::Float32 => 700,
        DataType::Float64 => 701,
        DataType::Decimal128(..) | DataType::Decimal256(..) => 1700,
        DataType::Date32 | DataType::Date64 => 1082,
        DataType::Timestamp(..) => 1114,
        DataType::Time32(..) | DataType::Time64(..) => 1083,
        DataType::Binary => 17,
        _ => 25,
    }
}

fn format_type(oid: u32, modifier: i32) -> Result<String> {
    let name = match oid {
        16 => "boolean",
        17 => "bytea",
        20 => "bigint",
        21 => "smallint",
        23 => "integer",
        25 => "text",
        26 => "oid",
        700 => "real",
        701 => "double precision",
        1082 => "date",
        1083 => "time without time zone",
        1114 => "timestamp without time zone",
        1700 => "numeric",
        _ => return Err(Error::unsupported()),
    };
    if oid == 1700 && modifier >= 4 {
        let typmod = modifier - 4;
        let precision = typmod >> 16;
        let scale = ((typmod & 0x7ff) ^ 0x400) - 0x400;
        return Ok(format!("numeric({precision},{scale})"));
    }
    if (oid == 1083 || oid == 1114) && modifier >= 0 {
        let (base, _) = name.split_once(' ').unwrap();
        return Ok(format!("{base}({modifier}) without time zone"));
    }
    Ok(name.into())
}
