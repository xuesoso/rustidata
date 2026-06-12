//! Rhai expression engine: derived columns, select-by-expression,
//! edit-by-expression.
//!
//! Column names are injected as scope variables (sanitized to identifiers, as
//! visidata does with clean_to_id). Helper functions mirror common Python
//! idioms: str(), int(), float(), len().

use std::sync::{Arc, OnceLock};

use rhai::{Dynamic, Engine, Scope, AST};

use crate::sheet::Sheet;
use crate::store::RowId;
use crate::value::Value;

fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut e = Engine::new();
        e.set_max_expr_depths(64, 64);
        e.set_max_operations(1_000_000);
        // python-ish helpers
        e.register_fn("str", |v: Dynamic| v.to_string());
        e.register_fn("int", |v: Dynamic| -> Dynamic {
            if let Ok(i) = v.as_int() {
                return Dynamic::from(i);
            }
            if let Ok(f) = v.as_float() {
                return Dynamic::from(f as i64);
            }
            if let Some(s) = v.read_lock::<rhai::ImmutableString>() {
                if let Some(i) = crate::value::parse_int_str(&s) {
                    return Dynamic::from(i);
                }
            }
            Dynamic::UNIT
        });
        e.register_fn("float", |v: Dynamic| -> Dynamic {
            if let Ok(f) = v.as_float() {
                return Dynamic::from(f);
            }
            if let Ok(i) = v.as_int() {
                return Dynamic::from(i as f64);
            }
            if let Some(s) = v.read_lock::<rhai::ImmutableString>() {
                if let Some(f) = crate::value::parse_float_str(&s) {
                    return Dynamic::from(f);
                }
            }
            Dynamic::UNIT
        });
        e
    })
}

/// Compile an expression once; the AST is shared and Send+Sync (rhai "sync").
pub fn compile(src: &str) -> Result<Arc<AST>, String> {
    engine()
        .compile_expression(src)
        .map(Arc::new)
        .map_err(|e| e.to_string())
}

/// Sanitize a column name into a valid identifier (visidata's clean_to_id).
pub fn clean_to_id(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if c.is_alphanumeric() || c == '_' {
            if i == 0 && c.is_ascii_digit() {
                out.push('_');
            }
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

pub fn value_to_dynamic(v: &Value) -> Dynamic {
    match v {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => Dynamic::from(*b),
        Value::Int(i) => Dynamic::from(*i),
        Value::Float(f) => Dynamic::from(*f),
        Value::Date(us) => Dynamic::from(*us),
        Value::Str(s) => Dynamic::from(s.to_string()),
        Value::Error(_) => Dynamic::UNIT,
    }
}

pub fn dynamic_to_value(d: Dynamic) -> Value {
    if d.is_unit() {
        return Value::Null;
    }
    if let Ok(b) = d.as_bool() {
        return Value::Bool(b);
    }
    if let Ok(i) = d.as_int() {
        return Value::Int(i);
    }
    if let Ok(f) = d.as_float() {
        return Value::Float(f);
    }
    Value::Str(d.to_string().into())
}

/// Build the scope for one row: every non-hidden column's typed value under
/// its sanitized name, plus `row` (the 0-based view-independent row id).
fn scope_for<'a>(sheet: &Sheet, row: RowId, skip_ast: &AST) -> Scope<'a> {
    let mut scope = Scope::new();
    scope.push("row", row as i64);
    for col in &sheet.columns {
        if let crate::column::ColumnKind::Expr { ast, .. } = &col.kind {
            // avoid self-recursion: don't bind the column being evaluated
            if std::ptr::eq(ast.as_ref(), skip_ast) {
                continue;
            }
        }
        let v = col.typed_value(sheet, row);
        scope.push_dynamic(clean_to_id(&col.name), value_to_dynamic(&v));
    }
    scope
}

/// Evaluate a compiled expression for one row of a sheet.
pub fn eval_row(sheet: &Sheet, row: RowId, ast: &AST) -> Value {
    let mut scope = scope_for(sheet, row, ast);
    match engine().eval_ast_with_scope::<Dynamic>(&mut scope, ast) {
        Ok(d) => dynamic_to_value(d),
        Err(e) => Value::Error(Arc::from(e.to_string().as_str())),
    }
}

/// Evaluate an expression as a row predicate (select-by-expr).
pub fn eval_predicate(sheet: &Sheet, row: RowId, ast: &AST) -> bool {
    match eval_row(sheet, row, ast) {
        Value::Bool(b) => b,
        Value::Null | Value::Error(_) => false,
        Value::Int(i) => i != 0,
        Value::Float(f) => f != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::Date(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_ids() {
        assert_eq!(clean_to_id("First Name"), "First_Name");
        assert_eq!(clean_to_id("1abc"), "_1abc");
        assert_eq!(clean_to_id("price ($)"), "price____");
    }

    #[test]
    fn compile_and_convert() {
        let ast = compile("1 + 2").unwrap();
        let mut scope = Scope::new();
        let d = engine()
            .eval_ast_with_scope::<Dynamic>(&mut scope, &ast)
            .unwrap();
        assert_eq!(dynamic_to_value(d), Value::Int(3));
    }

    #[test]
    fn bad_expression_errors() {
        assert!(compile("1 +").is_err());
    }
}
