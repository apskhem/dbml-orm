use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::{Write, Result, Error, ErrorKind};
use std::{path::Path, fs};

use crate::ast::*;
use crate::generator::{Codegen, TargetLang, Block};
use crate::parser::*;

use inflector::Inflector;

mod traits;
use traits::*;

const NAME: &'static str = env!("CARGO_PKG_NAME");
const VERSION: &'static str = env!("CARGO_PKG_VERSION");

#[derive(Debug, PartialEq, Clone)]
pub enum Target {
  SeaORMPostgreSQL
}

#[derive(Debug, PartialEq, Clone)]
pub struct Config  {
  in_path: OsString,
  out_path: Option<OsString>,
  target: Target
}

impl Config {
  pub fn new(in_path: impl AsRef<Path>, target: Target) -> Self {
    Self {
      in_path: in_path.as_ref().into(),
      out_path: None,
      target
    }
  }

  pub fn set_out_path(mut self, path: impl AsRef<Path>) -> Self {
    self.out_path = Some(path.as_ref().into());

    self
  }

  pub fn transpile(&self) -> Result<()> {
    let raw_in = fs::read_to_string(&self.in_path)?;

    let out_ast = parse(&raw_in).unwrap_or_else(|e| panic!("{}", e));

    let sem_ast = out_ast.into_semantic();
    
    let result = transpile(sem_ast, &self.target).unwrap_or_else(|e| panic!("{}", e));

    let out_path = if let Some(out_path) = self.out_path.clone() {
      out_path
    } else {
      env::var_os("OUT_DIR")
        .ok_or_else(|| {
          Error::new(ErrorKind::Other, "OUT_DIR environment variable is not set")
        })?
    };

    File::create(out_path)?.write_all(result.as_bytes())?;

    Ok(())
  }
}

fn transpile(ast: schema::SematicSchemaBlock, target: &Target) -> Result<String> {
  match target {
    Target::SeaORMPostgreSQL => transpile_sea_orm_postgresql(ast)
  }
}

fn transpile_sea_orm_postgresql(ast: schema::SematicSchemaBlock) -> Result<String> {
  let codegen = Codegen::new(TargetLang::Rust)
    .line(format!("//! Generated by {NAME} {VERSION}"))
    .line_skip(1)
    .line("use sea_orm::entity::prelude::*;");

  let codegen = ast.tables.into_iter().fold(codegen, |acc, table| {
    let table::TableBlock {
      ident: table::TableIdent {
        name,
        schema,
        ..
      },
      fields,
      indexes,
      ..
    } = table;

    let table_block = Block::new(2, Some("pub struct Model"));
    let rel_block = Block::new(2, Some("pub enum Relation"));

    let table_block = fields.into_iter().fold(table_block,|acc, field| {
      let mut out_fields = vec![];

      if field.col_settings.is_pk {
        out_fields.push("primary_key")
      } else if field.col_settings.is_nullable {
        out_fields.push("nullable")
      }

      let field_rust_type = field.col_type.to_rust_sea_orm_type();
      let field_string = if field.col_settings.is_nullable {
        format!("Option<{}>", field_rust_type)
      } else {
        field_rust_type
      };
      
      acc
        .line_cond(!out_fields.is_empty(), format!("#[sea_orm({})]", out_fields.join(", ")))
        .line(format!("pub {}: {},", field.col_name, field_string))
    });

    let mod_block = Block::new(1, Some(format!("pub mod {}", name)))
      .line("use sea_orm::entity::prelude::*;")
      .line_skip(1)
      .line(format!("#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]"))
      .line(format!(r#"#[sea_orm(table_name = "{}", schema_name = "{}")]"#, name, schema.unwrap_or_else(|| "public".into())))
      .block(table_block)
      .line_skip(1)
      .line("#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]")
      .block(rel_block)
      .line_skip(1)
      .line("impl ActiveModelBehavior for ActiveModel {}");

    acc
      .line_skip(1)
      .block(mod_block)
  });

  let codegen = ast.enums.into_iter().fold(codegen, |acc, r#enum| {
    let enums::EnumBlock {
      ident: enums::EnumIdent {
        name,
        schema,
        ..
      },
      values,
      ..
    } = r#enum;

    let enum_block = Block::new(1, Some(format!("pub enum {}", name.to_pascal_case())));

    let enum_block = values.into_iter().enumerate().fold(enum_block,|acc, (i, value)| {
      acc
        .line(format!("#[sea_orm(num_value = {})]", i))
        .line(format!("{},", value.value.to_pascal_case()))
    });

    acc
      .line_skip(1)
      .line("#[derive(Debug, PartialEq, EnumIter, DeriveActiveEnum)]")
      .line(
        format!(r#"#[sea_orm(rs_type = "i32", db_type = "Integer", enum_name = "{}", schema_name = "{}")]"#, name, schema.unwrap_or("public".into()))
      )
      .block(enum_block)
  });

  Ok(codegen.to_string())
}
