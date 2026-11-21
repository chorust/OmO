use crate::cancel::CancelToken;
use crate::dialect::Connection;
#[cfg(test)]
use crate::dialect::duckdb::duckdb_sync::DuckDbSyncConnection;
use crate::dialect::duckdb::session::NativeSession;
use crate::utils::{FunctionMeta, Metadata, RawArrowData, TreeNode};
use async_trait::async_trait;
use regex::Regex;
use std::sync::{Arc, OnceLock};

pub mod bootstrap;
pub mod duckdb_sync;
pub mod session;
pub mod xlsx;

#[derive(Debug, Clone)]
pub struct DuckDbConnection {
  pub path: String,
  pub cwd: Option<String>,
  session: Arc<NativeSession>,
}

impl Default for DuckDbConnection {
  fn default() -> Self {
    Self::new(":memory:".to_string(), None)
  }
}

impl DuckDbConnection {
  pub fn new(path: String, cwd: Option<String>) -> Self {
    let session = Arc::new(NativeSession::new_local(Some(path.clone()), cwd.clone()));
    Self { path, cwd, session }
  }

  async fn run_query(
    &self,
    query_sql: String,
    original_sql: String,
    count_sql: Option<String>,
    cancel: Option<&CancelToken>,
  ) -> anyhow::Result<RawArrowData> {
    self
      .session
      .run(cancel, move |connection, token| {
        token.check()?;
        let mut result = match duckdb_sync::query_fallible(&connection.inner, &query_sql) {
          Ok(result) => result,
          Err(error) => {
            token.check()?;
            return Err(error);
          }
        };
        token.check()?;
        if let Some(count_sql) = count_sql {
          match connection
            .inner
            .query_row(&count_sql, [], |row| row.get::<_, usize>(0))
          {
            Ok(total) => result.total = total,
            Err(_) => token.check()?,
          }
        }
        token.check()?;
        result.sql = Some(original_sql);
        Ok(result)
      })
      .await
  }
}

fn fn_call_re() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]*\([^)]*\)$").unwrap())
}

/// Check if `table` is a DuckDB table function call like `read_parquet('path')`.
pub fn is_file_function(table: &str) -> bool {
  let t = table.trim().to_lowercase();
  (t.starts_with("read_parquet(")
    || t.starts_with("read_csv(")
    || t.starts_with("read_json(")
    || t.starts_with("read_xlsx("))
    && t.ends_with(')')
}

#[async_trait]
impl Connection for DuckDbConnection {
  fn dialect(&self) -> &'static str {
    "duckdb"
  }

  fn native_session(&self) -> Option<Arc<NativeSession>> {
    Some(Arc::clone(&self.session))
  }

  async fn get_db(&self) -> anyhow::Result<TreeNode> {
    self
      .session
      .run(None, |connection, token| {
        token.check()?;
        connection.get_db()
      })
      .await
  }

  async fn list_databases(&self) -> anyhow::Result<Vec<String>> {
    let path = self.path.clone();
    let name = std::path::Path::new(&path)
      .file_stem()
      .map(|n| n.to_string_lossy().to_string())
      .unwrap_or_else(|| path.clone());
    Ok(vec![name])
  }

  async fn query(&self, sql: &str, _limit: usize, _offset: usize) -> anyhow::Result<RawArrowData> {
    self
      .run_query(sql.to_string(), sql.to_string(), None, None)
      .await
  }

  async fn query_cancellable(
    &self,
    sql: &str,
    _limit: usize,
    _offset: usize,
    cancel: Option<&CancelToken>,
  ) -> anyhow::Result<RawArrowData> {
    self
      .run_query(sql.to_string(), sql.to_string(), None, cancel)
      .await
  }

  async fn query_count(&self, sql: &str) -> anyhow::Result<usize> {
    let sql = sql.to_string();
    self
      .session
      .run(None, move |connection, token| {
        token.check()?;
        let result = connection
          .inner
          .query_row(&sql, [], |row| row.get::<_, usize>(0));
        token.check()?;
        Ok(result?)
      })
      .await
  }

  async fn paging_query(
    &self,
    sql: &str,
    limit: Option<usize>,
    offset: Option<usize>,
  ) -> anyhow::Result<RawArrowData> {
    self
      .paging_query_cancellable(sql, limit, offset, None)
      .await
  }

  async fn paging_query_cancellable(
    &self,
    sql: &str,
    limit: Option<usize>,
    offset: Option<usize>,
    cancel: Option<&CancelToken>,
  ) -> anyhow::Result<RawArrowData> {
    let dialect = self.dialect();
    let statement = crate::dialect::ast::first_stmt(dialect, sql);
    let query_sql = statement
      .as_ref()
      .and_then(|statement| crate::dialect::ast::limit_stmt(dialect, statement, limit, offset))
      .unwrap_or_else(|| sql.to_string());
    let count_sql = statement
      .as_ref()
      .and_then(|statement| crate::dialect::ast::count_stmt(dialect, statement));
    self
      .run_query(query_sql, sql.to_string(), count_sql, cancel)
      .await
  }

  async fn query_table(
    &self,
    table: &str,
    limit: usize,
    offset: usize,
    where_: &str,
    order_by: &str,
    select_extras: &str,
  ) -> anyhow::Result<RawArrowData> {
    self
      .query_table_cancellable(table, limit, offset, where_, order_by, select_extras, None)
      .await
  }

  async fn query_table_cancellable(
    &self,
    table: &str,
    limit: usize,
    offset: usize,
    where_: &str,
    order_by: &str,
    select_extras: &str,
    cancel: Option<&CancelToken>,
  ) -> anyhow::Result<RawArrowData> {
    let original_sql = self._table_query_sql(table, where_, order_by, select_extras);
    let mut query_sql = original_sql.clone();
    if limit != 0 {
      query_sql = format!("{query_sql} limit {limit}");
    }
    if offset != 0 {
      query_sql = format!("{query_sql} offset {offset}");
    }
    let count_sql = self._table_count_sql(table, where_);
    self
      .run_query(query_sql, original_sql, Some(count_sql), cancel)
      .await
  }

  async fn show_schema(&self, schema: &str) -> anyhow::Result<RawArrowData> {
    let schema = schema.to_string();
    self
      .session
      .run(None, move |connection, token| {
        token.check()?;
        let batch = connection.show_schema(&schema)?;
        token.check()?;
        Ok(RawArrowData::from_batch(batch))
      })
      .await
  }

  async fn show_column(&self, schema: Option<&str>, table: &str) -> anyhow::Result<RawArrowData> {
    if is_file_function(table) {
      let sql = format!("DESCRIBE SELECT * FROM {table}");
      log::info!("show columns (file): {}", &sql);
      return self.query(&sql, 0, 0).await;
    }
    let (db, tbl) = if schema.is_none() && table.contains('.') {
      let parts: Vec<&str> = table.splitn(2, '.').collect();
      (parts[0], parts[1])
    } else {
      ("", table)
    };
    let sql = format!(
      "select * from information_schema.columns where table_schema='{db}' and table_name='{tbl}'"
    );
    log::info!("show columns: {}", &sql);
    self.query(&sql, 0, 0).await
  }

  async fn all_columns(&self) -> anyhow::Result<Vec<Metadata>> {
    self
      .session
      .run(None, |connection, token| {
        token.check()?;
        let columns = connection.all_columns()?;
        token.check()?;
        Ok(columns)
      })
      .await
  }

  async fn functions(&self) -> anyhow::Result<Vec<FunctionMeta>> {
    self
      .session
      .run(None, |connection, token| {
        token.check()?;
        let functions = connection.functions()?;
        token.check()?;
        Ok(functions)
      })
      .await
  }

  async fn drop_table(&self, schema: Option<&str>, table: &str) -> anyhow::Result<String> {
    let (db, tbl) = if schema.is_none() && table.contains('.') {
      let parts: Vec<&str> = table.splitn(2, '.').collect();
      (parts[0], parts[1])
    } else {
      ("", table)
    };

    let table_name = if db.is_empty() {
      tbl.to_string()
    } else {
      format!("{db}.{tbl}")
    };
    self
      .session
      .run(None, move |connection, token| {
        token.check()?;
        connection.drop_table(&table_name)?;
        token.check()?;
        Ok(String::new())
      })
      .await
  }

  async fn table_row_count(&self, table: &str, r#where: &str) -> anyhow::Result<usize> {
    let sql = self._table_count_sql(table, r#where);
    self
      .session
      .run(None, move |connection, token| {
        token.check()?;
        let result = connection
          .inner
          .query_row(&sql, [], |row| row.get::<_, usize>(0));
        token.check()?;
        Ok(result?)
      })
      .await
  }

  fn normalize(&self, name: &str) -> String {
    if name.contains(' ') {
      format!("\"{name}\"")
    } else {
      name.to_string()
    }
  }

  async fn export(
    &self,
    sql: &str,
    file: &str,
    format: &str,
    options: &crate::utils::ExportOptions,
    cancel: Option<&crate::cancel::CancelToken>,
  ) -> anyhow::Result<()> {
    let sql = sql.to_string();
    let file = file.to_string();
    let format = format.to_string();
    let options = options.clone();
    self
      .session
      .run(cancel, move |connection, token| {
        token.check()?;
        connection.export(&sql, &file, &format, &options)?;
        token.check()?;
        Ok(())
      })
      .await
  }
  fn start_quote(&self) -> &'static str {
    "\""
  }
  fn end_quote(&self) -> &'static str {
    "\""
  }
  fn validator(&self, id: &str) -> bool {
    if id.is_empty() {
      return false;
    }

    if id.starts_with("'") && id.ends_with("'") {
      return true;
    }

    if fn_call_re().is_match(id) {
      return true;
    }

    let mut chars = id.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
      return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
  }
}

#[tokio::test]
#[ignore = "requires a local duckdb file"]
async fn test_duckdb() {
  use arrow::util::pretty::print_batches;
  let conn = DuckDbSyncConnection::new(
    Some("D:/data_obs/product_nice_l1/deepseek.db".to_string()),
    None,
  )
  .unwrap();
  let res = conn.query("SELECT extension_name, installed, description FROM duckdb_extensions()");
  let (_, batch) = res.unwrap();
  let _ = print_batches(&[batch]);
}

#[cfg(test)]
mod file_function_tests {
  use super::is_file_function;

  #[test]
  fn test_is_file_function() {
    assert!(is_file_function("read_parquet('data.parquet')"));
    assert!(is_file_function("read_csv('data.csv')"));
    assert!(is_file_function("read_json('data.json')"));
    assert!(is_file_function("read_xlsx('data.xlsx')"));
    assert!(is_file_function("read_parquet('D:/path/to/file.parquet')"));
    assert!(is_file_function("  read_csv('file.csv')  "));

    assert!(!is_file_function("my_table"));
    assert!(!is_file_function("read_parquet"));
    assert!(!is_file_function("schema.read_parquet('f.parquet')"));
    assert!(!is_file_function("SELECT read_parquet('f.parquet')"));
  }
}
