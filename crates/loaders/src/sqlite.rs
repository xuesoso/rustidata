//! SQLite loader: an index sheet of tables; each table loads fully via a
//! single query (rows realized into a ColumnarStore — SQLite's own file is
//! the out-of-core backing, queries are fast).

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use rustidata_core::column::Column;
use rustidata_core::sheet::{Sheet, SheetKind};
use rustidata_core::store::ColumnarStore;
use rustidata_core::value::{VType, Value};

fn sql_cell(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Int(i),
        ValueRef::Real(f) => Value::Float(f),
        ValueRef::Text(t) => Value::Str(String::from_utf8_lossy(t).into_owned().into()),
        ValueRef::Blob(b) => Value::Str(format!("<blob {} bytes>", b.len()).into()),
    }
}

/// Quote an identifier for SQL, doubling embedded quotes. Table names come
/// from the (potentially malicious) database file itself, so this must be
/// applied to every interpolated identifier.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn vtype_from_decl(decl: Option<&str>) -> VType {
    let Some(d) = decl else { return VType::Any };
    let d = d.to_ascii_uppercase();
    if d.contains("INT") {
        VType::Int
    } else if d.contains("REAL") || d.contains("FLOA") || d.contains("DOUB") || d.contains("NUM") || d.contains("DEC") {
        VType::Float
    } else if d.contains("DATE") || d.contains("TIME") {
        VType::Date
    } else {
        VType::Any
    }
}

/// The index sheet: one row per table/view.
pub fn load(path: &Path) -> Result<Sheet> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut stmt = conn.prepare(
        "SELECT name, type FROM sqlite_master WHERE type IN ('table','view') ORDER BY name",
    )?;
    let tables: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<_, _>>()?;

    let mut rows = Vec::new();
    for (name, ttype) in &tables {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {}", quote_ident(name)), [], |r| {
                r.get(0)
            })
            .unwrap_or(-1);
        rows.push(vec![
            Value::Str(name.as_str().into()),
            Value::Str(ttype.as_str().into()),
            Value::Int(count),
        ]);
    }

    let store = ColumnarStore::from_rows(3, rows);
    let columns = vec![
        Column::source("table", 0),
        Column::source("type", 1),
        Column::source("rows", 2).with_type(VType::Int),
    ];
    let mut sheet = Sheet::new(crate::sheet_name(path), Arc::new(store), columns);
    sheet.kind = SheetKind::Dir; // index-like: Enter loads the table
    sheet.source_path = Some(path.to_path_buf());
    Ok(sheet)
}

/// Load one table from a SQLite database as a sheet.
pub fn load_table(path: &Path, table: &str) -> Result<Sheet> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;

    // schema for types
    let mut decls: Vec<(String, Option<String>)> = Vec::new();
    {
        let mut stmt = conn.prepare(&format!("PRAGMA table_xinfo({})", quote_ident(table)))?;
        let mut qrows = stmt.query([])?;
        while let Some(r) = qrows.next()? {
            let name: String = r.get(1)?;
            let decl: Option<String> = r.get(2)?;
            decls.push((name, decl));
        }
    }

    let mut stmt = conn.prepare(&format!("SELECT * FROM {}", quote_ident(table)))?;
    let ncols = stmt.column_count();
    let col_names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut rows = Vec::new();
    let mut qrows = stmt.query([])?;
    while let Some(r) = qrows.next()? {
        let mut row = Vec::with_capacity(ncols);
        for i in 0..ncols {
            row.push(sql_cell(r.get_ref(i)?));
        }
        rows.push(row);
    }

    let store = ColumnarStore::from_rows(ncols, rows);
    let columns: Vec<Column> = col_names
        .into_iter()
        .enumerate()
        .map(|(i, n)| {
            let vtype = decls
                .iter()
                .find(|(dn, _)| dn == &n)
                .map(|(_, d)| vtype_from_decl(d.as_deref()))
                .unwrap_or(VType::Any);
            Column::source(n, i).with_type(vtype)
        })
        .collect();
    Ok(Sheet::new(table, Arc::new(store), columns))
}

/// Save a sheet as a table in a SQLite database (visidata save_sqlite parity).
pub fn save_sqlite(sheet: &Sheet, path: &Path) -> Result<()> {
    let conn = Connection::open(path)?;
    let vis = sheet.visible_cols();
    let cols_sql: Vec<String> = vis
        .iter()
        .map(|&c| {
            let col = &sheet.columns[c];
            let sql_type = match col.vtype {
                VType::Int | VType::Len => "INTEGER",
                VType::Float | VType::Currency | VType::FloatSI => "REAL",
                _ => "TEXT",
            };
            format!("{} {}", quote_ident(&col.name), sql_type)
        })
        .collect();
    let table = quote_ident(&sheet.name);
    conn.execute(&format!("DROP TABLE IF EXISTS {}", table), [])?;
    conn.execute(
        &format!("CREATE TABLE {} ({})", table, cols_sql.join(", ")),
        [],
    )?;
    let placeholders: Vec<String> = (1..=vis.len()).map(|i| format!("?{}", i)).collect();
    let mut stmt = conn.prepare(&format!(
        "INSERT INTO {} VALUES ({})",
        table,
        placeholders.join(", ")
    ))?;
    for rid in sheet.iter_rowids() {
        let params: Vec<rusqlite::types::Value> = vis
            .iter()
            .map(|&c| match sheet.columns[c].typed_value(sheet, rid) {
                Value::Null => rusqlite::types::Value::Null,
                Value::Bool(b) => rusqlite::types::Value::Integer(b as i64),
                Value::Int(i) => rusqlite::types::Value::Integer(i),
                Value::Float(f) => rusqlite::types::Value::Real(f),
                v => rusqlite::types::Value::Text(v.to_display().to_string()),
            })
            .collect();
        stmt.execute(rusqlite::params_from_iter(params))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "CREATE TABLE people (name TEXT, age INTEGER, score REAL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO people VALUES ('alice', 30, 1.5), ('bob', 25, 2.5)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn index_and_table() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.db");
        make_db(&p);

        let idx = load(&p).unwrap();
        assert_eq!(idx.nrows(), 1);
        assert_eq!(idx.typed_at(0, 0), Value::Str("people".into()));
        assert_eq!(idx.typed_at(0, 2), Value::Int(2));

        let t = load_table(&p, "people").unwrap();
        assert_eq!(t.nrows(), 2);
        assert_eq!(t.columns[1].vtype, VType::Int);
        assert_eq!(t.typed_at(0, 0), Value::Str("alice".into()));
        assert_eq!(t.typed_at(1, 1), Value::Int(25));
    }

    #[test]
    fn hostile_table_names_are_quoted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("evil.db");
        let conn = Connection::open(&p).unwrap();
        // table name containing a quote and SQL metacharacters
        conn.execute(r#"CREATE TABLE "ev""il; DROP TABLE x--" (a TEXT)"#, [])
            .unwrap();
        conn.execute(r#"INSERT INTO "ev""il; DROP TABLE x--" VALUES ('ok')"#, [])
            .unwrap();
        drop(conn);

        let idx = load(&p).unwrap();
        assert_eq!(idx.nrows(), 1);
        assert_eq!(idx.typed_at(0, 2), Value::Int(1)); // count query survived

        let name = idx.typed_at(0, 0).to_display().to_string();
        let t = load_table(&p, &name).unwrap();
        assert_eq!(t.typed_at(0, 0), Value::Str("ok".into()));
    }

    #[test]
    fn save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        make_db(&src);
        let t = load_table(&src, "people").unwrap();

        let dst = dir.path().join("dst.db");
        save_sqlite(&t, &dst).unwrap();
        let t2 = load_table(&dst, "people").unwrap();
        assert_eq!(t2.nrows(), 2);
        assert_eq!(t2.typed_at(0, 1), Value::Int(30));
    }
}
