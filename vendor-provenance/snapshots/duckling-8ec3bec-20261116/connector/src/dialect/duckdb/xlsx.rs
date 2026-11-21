use anyhow::{Result, anyhow};
use calamine::{Data, Reader, open_workbook_auto};
use duckdb::Connection;
use duckdb::core::{DataChunkHandle, Inserter, LogicalTypeHandle, LogicalTypeId};
use duckdb::vtab::{BindInfo, InitInfo, VTab, TableFunctionInfo};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColumnKind {
  Boolean,
  Bigint,
  Double,
  Varchar,
}

pub struct XlsxBind {
  names: Vec<String>,
  kinds: Vec<ColumnKind>,
  rows: Vec<Vec<Data>>,
}

pub struct XlsxInit {
  next_row: AtomicUsize,
}

pub struct ReadXlsx;

pub fn register(conn: &Connection) -> duckdb::Result<()> {
  conn.register_table_function::<ReadXlsx>("read_xlsx")
}

fn column_letters(mut index: usize) -> String {
  let mut out = Vec::new();
  loop {
    out.push((b'A' + (index % 26) as u8) as char);
    if index < 26 {
      break;
    }
    index = index / 26 - 1;
  }
  out.iter().rev().collect()
}

fn cell_text(cell: &Data) -> String {
  match cell {
    Data::Empty => String::new(),
    other => other.to_string(),
  }
}

fn infer_kind(rows: &[Vec<Data>], column: usize) -> ColumnKind {
  let mut ints = false;
  let mut floats = false;
  let mut bools = false;
  let mut other = false;
  for row in rows {
    match row.get(column) {
      None | Some(Data::Empty) => {}
      Some(Data::Int(_)) => ints = true,
      Some(Data::Float(_)) => floats = true,
      Some(Data::Bool(_)) => bools = true,
      Some(_) => other = true,
    }
  }
  match (ints, floats, bools, other) {
    (_, _, _, true) => ColumnKind::Varchar,
    (false, false, true, false) => ColumnKind::Boolean,
    (_, _, true, false) => ColumnKind::Varchar,
    (true, false, false, false) => ColumnKind::Bigint,
    (_, true, false, false) => ColumnKind::Double,
    _ => ColumnKind::Varchar,
  }
}

fn config_string(bind: &BindInfo, name: &str) -> Option<String> {
  bind
    .get_config_option(name)
    .filter(|value| !value.is_null())
    .map(|value| value.to_string())
}

fn resolve_path(bind: &BindInfo, raw: &str) -> Result<PathBuf> {
  if raw.contains("://") {
    return Err(anyhow!("read_xlsx only reads local files: {raw}"));
  }
  let mut path = PathBuf::from(raw);
  if path.is_relative()
    && let Some(search) = config_string(bind, "file_search_path")
    && let Some(base) = search.split(',').map(str::trim).find(|part| !part.is_empty())
  {
    path = Path::new(base).join(path);
  }
  let resolved = path
    .canonicalize()
    .map_err(|error| anyhow!("cannot open xlsx file {raw}: {error}"))?;

  let external_access = bind
    .get_config_option("enable_external_access")
    .map(|value| value.to_bool())
    .unwrap_or(true);
  if !external_access {
    let allowed = bind
      .get_config_option("allowed_directories")
      .and_then(|value| value.to_list())
      .unwrap_or_default();
    let permitted = allowed.iter().any(|entry| {
      let entry = entry.to_string();
      if entry.contains("://") {
        return false;
      }
      let root = Path::new(&entry)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&entry));
      resolved.starts_with(root)
    });
    if !permitted {
      return Err(anyhow!(
        "Permission Error: Cannot access file \"{raw}\" - file system operations are disabled by configuration"
      ));
    }
  }
  Ok(resolved)
}

fn named_bool(bind: &BindInfo, name: &str, default: bool) -> bool {
  bind
    .get_named_parameter(name)
    .filter(|value| !value.is_null())
    .map(|value| value.to_bool())
    .unwrap_or(default)
}

fn logical(kind: ColumnKind) -> LogicalTypeHandle {
  LogicalTypeHandle::from(match kind {
    ColumnKind::Boolean => LogicalTypeId::Boolean,
    ColumnKind::Bigint => LogicalTypeId::Bigint,
    ColumnKind::Double => LogicalTypeId::Double,
    ColumnKind::Varchar => LogicalTypeId::Varchar,
  })
}

impl VTab for ReadXlsx {
  type InitData = XlsxInit;
  type BindData = XlsxBind;

  fn bind(bind: &BindInfo) -> Result<Self::BindData, Box<dyn std::error::Error>> {
    let raw = bind.get_parameter(0).to_string();
    let path = resolve_path(bind, &raw)?;
    let header = named_bool(bind, "header", true);
    let all_varchar = named_bool(bind, "all_varchar", false);

    let mut workbook = open_workbook_auto(&path)
      .map_err(|error| anyhow!("cannot read xlsx file {raw}: {error}"))?;
    let range = match bind
      .get_named_parameter("sheet")
      .filter(|value| !value.is_null())
      .map(|value| value.to_string())
    {
      Some(sheet) => workbook
        .worksheet_range(&sheet)
        .map_err(|error| anyhow!("cannot read sheet {sheet}: {error}"))?,
      None => workbook
        .worksheet_range_at(0)
        .ok_or_else(|| anyhow!("workbook has no sheets"))?
        .map_err(|error| anyhow!("cannot read first sheet: {error}"))?,
    };

    let mut rows: Vec<Vec<Data>> = range.rows().map(|row| row.to_vec()).collect();
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let names: Vec<String> = if header && !rows.is_empty() {
      let first = rows.remove(0);
      (0..width)
        .map(|index| {
          let text = first.get(index).map(cell_text).unwrap_or_default();
          if text.is_empty() {
            column_letters(index)
          } else {
            text
          }
        })
        .collect()
    } else {
      (0..width).map(column_letters).collect()
    };
    if names.is_empty() {
      return Err(anyhow!("xlsx sheet in {raw} has no columns").into());
    }
    let kinds: Vec<ColumnKind> = (0..width)
      .map(|index| {
        if all_varchar {
          ColumnKind::Varchar
        } else {
          infer_kind(&rows, index)
        }
      })
      .collect();
    for (name, kind) in names.iter().zip(&kinds) {
      bind.add_result_column(name, logical(*kind));
    }
    Ok(XlsxBind { names, kinds, rows })
  }

  fn init(_init: &InitInfo) -> Result<Self::InitData, Box<dyn std::error::Error>> {
    Ok(XlsxInit {
      next_row: AtomicUsize::new(0),
    })
  }

  fn func(
    func: &TableFunctionInfo<Self>,
    output: &mut DataChunkHandle,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let bind = func.get_bind_data();
    let init = func.get_init_data();
    let capacity = output.flat_vector(0).capacity();
    let start = init.next_row.fetch_add(capacity, Ordering::SeqCst);
    if start >= bind.rows.len() {
      output.set_len(0);
      return Ok(());
    }
    let end = (start + capacity).min(bind.rows.len());
    let count = end - start;
    for (column, kind) in bind.kinds.iter().enumerate() {
      let mut vector = output.flat_vector(column);
      for offset in 0..count {
        let cell = bind.rows[start + offset].get(column).unwrap_or(&Data::Empty);
        if matches!(cell, Data::Empty) {
          vector.set_null(offset);
          continue;
        }
        match (kind, cell) {
          (ColumnKind::Boolean, Data::Bool(value)) => unsafe {
            vector.as_mut_slice::<bool>()[offset] = *value;
          },
          (ColumnKind::Bigint, Data::Int(value)) => unsafe {
            vector.as_mut_slice::<i64>()[offset] = *value;
          },
          (ColumnKind::Double, Data::Int(value)) => unsafe {
            vector.as_mut_slice::<f64>()[offset] = *value as f64;
          },
          (ColumnKind::Double, Data::Float(value)) => unsafe {
            vector.as_mut_slice::<f64>()[offset] = *value;
          },
          (ColumnKind::Varchar, other) => vector.insert(offset, other.to_string().as_str()),
          _ => vector.set_null(offset),
        }
      }
    }
    output.set_len(count);
    Ok(())
  }

  fn parameters() -> Option<Vec<LogicalTypeHandle>> {
    Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
  }

  fn named_parameters() -> Option<Vec<(String, LogicalTypeHandle)>> {
    Some(vec![
      ("header".to_string(), LogicalTypeHandle::from(LogicalTypeId::Boolean)),
      ("sheet".to_string(), LogicalTypeHandle::from(LogicalTypeId::Varchar)),
      ("all_varchar".to_string(), LogicalTypeHandle::from(LogicalTypeId::Boolean)),
      ("ignore_errors".to_string(), LogicalTypeHandle::from(LogicalTypeId::Boolean)),
    ])
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sample(dir: &Path) -> PathBuf {
    let path = dir.join("sample.xlsx");
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.write_string(0, 0, "id").unwrap();
    sheet.write_string(0, 1, "score").unwrap();
    sheet.write_string(0, 2, "label").unwrap();
    sheet.write_number(1, 0, 1).unwrap();
    sheet.write_number(1, 1, 2.5).unwrap();
    sheet.write_string(1, 2, "x, \"y\"").unwrap();
    sheet.write_number(2, 0, 2).unwrap();
    sheet.write_number(2, 1, 3.5).unwrap();
    sheet.write_string(2, 2, "z").unwrap();
    workbook.save(&path).unwrap();
    path
  }

  fn sealed(dir: &Path) -> Connection {
    let config = duckdb::Config::default()
      .with("autoinstall_known_extensions", "false")
      .and_then(|config| config.with("autoload_known_extensions", "false"))
      .unwrap();
    let conn = Connection::open_in_memory_with_flags(config).unwrap();
    register(&conn).unwrap();
    conn
      .execute_batch(&format!(
        "SET allowed_directories = ['{}']; SET enable_external_access = false; SET lock_configuration = true;",
        dir.display()
      ))
      .unwrap();
    conn
  }

  fn temp() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("duckling_vtab_xlsx_{}", nanoid::nanoid!(8)));
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
  }

  #[test]
  fn typed_columns_and_aggregates_on_a_sealed_handle() {
    let dir = temp();
    let path = sample(&dir);
    let conn = sealed(&dir);
    let sql = format!(
      "SELECT sum(score), min(id), string_agg(label, '|' ORDER BY id) FROM read_xlsx('{}')",
      path.display()
    );
    let (sum, min, labels): (f64, i64, String) = conn
      .query_row(&sql, [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
      .unwrap();
    assert_eq!(sum, 6.0);
    assert_eq!(min, 1);
    assert_eq!(labels, "x, \"y\"|z");
  }

  #[test]
  fn named_parameters_header_all_varchar_and_sheet() {
    let dir = temp();
    let path = sample(&dir);
    let conn = sealed(&dir);
    let rows: i64 = conn
      .query_row(
        &format!("SELECT count(*) FROM read_xlsx('{}', header=false)", path.display()),
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(rows, 3);
    let first: String = conn
      .query_row(
        &format!("SELECT \"A\" FROM read_xlsx('{}', header=false) LIMIT 1", path.display()),
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(first, "id");
    let kind: String = conn
      .query_row(
        &format!(
          "SELECT typeof(id) FROM read_xlsx('{}', all_varchar=true) LIMIT 1",
          path.display()
        ),
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(kind, "VARCHAR");
    assert!(
      conn
        .prepare(&format!("SELECT * FROM read_xlsx('{}', sheet='missing')", path.display()))
        .is_err()
    );
  }

  #[test]
  fn paths_outside_the_allowed_directories_are_rejected() {
    let allowed = temp();
    let other = temp();
    let path = sample(&other);
    let conn = sealed(&allowed);
    let error = conn
      .prepare(&format!("SELECT * FROM read_xlsx('{}')", path.display()))
      .unwrap_err()
      .to_string();
    assert!(error.contains("Permission Error"), "{error}");
    let url = conn
      .prepare("SELECT * FROM read_xlsx('https://example.com/a.xlsx')")
      .unwrap_err()
      .to_string();
    assert!(url.contains("local files"), "{url}");
  }

  #[test]
  fn extensions_still_cannot_be_installed_after_sealing() {
    let dir = temp();
    let conn = sealed(&dir);
    assert!(conn.execute_batch("INSTALL excel; LOAD excel;").is_err());
  }

  #[test]
  fn column_letters_follow_spreadsheet_order() {
    assert_eq!(column_letters(0), "A");
    assert_eq!(column_letters(25), "Z");
    assert_eq!(column_letters(26), "AA");
    assert_eq!(column_letters(27), "AB");
  }
}
