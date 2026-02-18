use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "diagnostic_reports", schema_name = "flexi")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub trace_id: String, // UUID v7
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: String, // Partition Key / RLS Scope
    pub error_code: String,
    pub context: Json, // DiagnosticContext
    pub suggestion: Option<String>,
    pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
