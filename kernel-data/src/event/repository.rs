use crate::connection::{RawConnection, TenantScoped};
use crate::event::entities_causality_seq;
use crate::event::entities_entity_seq;
use crate::event::entities_outbox;
use kernel_core::event::{EventEnvelope, EventError, OrderMode};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ActiveModelTrait, DbErr, EntityTrait, Set};
use uuid::Uuid;
use std::convert::TryFrom;

pub struct EventRepository;

impl EventRepository {
    /// Creates an event and inserts it into the outbox within the given transaction.
    /// Returns the fully formed EventEnvelope with the assigned sequence number.
    pub async fn create_event(
        db: &TenantScoped<RawConnection>,
        event_id: Uuid,
        event_type: String,
        order_mode: OrderMode,
        payload: serde_json::Value,
    ) -> Result<EventEnvelope, EventError> {
        let tenant_id = &db.tenant_id;
        let now = chrono::Utc::now();

        // 1. Generate Sequence
        // 1. Generate Sequence
        let (seq, metadata_mode_str) = match &order_mode {
            OrderMode::Entity { entity_id, .. } => {
                let seq = Self::next_entity_seq(db, tenant_id.as_str(), *entity_id)
                    .await
                    .map_err(|e| {
                        EventError::Store(format!("failed to generate entity seq: {}", e))
                    })?;
                (seq, "entity")
            }
            OrderMode::Causality { key, .. } => {
                let seq = Self::next_causality_seq(db, tenant_id.as_str(), key)
                    .await
                    .map_err(|e| {
                        EventError::Store(format!("failed to generate causality seq: {}", e))
                    })?;
                (seq, "causality")
            }
        };

        // 2. Prepare OrderMode with seq
        // 2. Prepare OrderMode with seq
        // Safety: seq is generated from i64 in DB (BIGINT), we assumes it fits in u64 if non-negative.
        // Logic ensures strictly positive seq from next_*_seq.
        let seq_u64 = u64::try_from(seq).map_err(|e| {
            EventError::Store(format!("generated sequence {} is invalid for u64: {}", seq, e))
        })?;

        let final_order_mode = match order_mode {
            OrderMode::Entity { entity_id, .. } => OrderMode::Entity {
                entity_id,
                seq: Some(seq_u64),
            },
            OrderMode::Causality { key, .. } => OrderMode::Causality {
                key,
                seq: Some(seq_u64),
            },
        };

        // 3. Insert into Outbox
        let outbox_model = entities_outbox::ActiveModel {
            event_id: Set(event_id),
            tenant_id: Set(tenant_id.as_str().to_string()),
            event_type: Set(event_type.clone()),
            order_mode: Set(metadata_mode_str.to_string()),
            entity_id: Set(match &final_order_mode {
                OrderMode::Entity { entity_id, .. } => Some(*entity_id),
                _ => None,
            }),
            entity_seq: Set(match &final_order_mode {
                OrderMode::Entity { .. } => Some(seq),
                _ => None,
            }),
            causality_key: Set(match &final_order_mode {
                OrderMode::Causality { key, .. } => Some(key.clone()),
                _ => None,
            }),
            causality_seq: Set(match &final_order_mode {
                OrderMode::Causality { .. } => Some(seq),
                _ => None,
            }),
            payload: Set(payload.clone()),
            created_at: Set(now.into()), // Use captured time
            published_at: Set(None),
        };

        outbox_model
            .insert(&db.inner.txn)
            .await
            .map_err(|e| EventError::Store(format!("failed to insert into outbox: {}", e)))?;

        // 4. Return Envelope
        Ok(EventEnvelope {
                event_id,
                tenant_id: tenant_id.clone(),
                event_type,
                order_mode: final_order_mode,
                payload,
                created_at: now, // Use captured time
            })
    }

    async fn next_entity_seq(
        db: &TenantScoped<RawConnection>,
        tenant_id: &str,
        entity_id: Uuid,
    ) -> Result<i64, DbErr> {
        let on_conflict = OnConflict::columns([
            entities_entity_seq::Column::TenantId,
            entities_entity_seq::Column::EntityId,
        ])
        .value(
            entities_entity_seq::Column::LastSeq,
            Expr::col(entities_entity_seq::Column::LastSeq).add(1),
        )
        .to_owned();

        let model = entities_entity_seq::Entity::insert(entities_entity_seq::ActiveModel {
            tenant_id: Set(tenant_id.to_string()),
            entity_id: Set(entity_id),
            last_seq: Set(1),
        })
        .on_conflict(on_conflict)
        .exec_with_returning(&db.inner.txn)
        .await?;

        Ok(model.last_seq)
    }

    async fn next_causality_seq(
        db: &TenantScoped<RawConnection>,
        tenant_id: &str,
        key: &str,
    ) -> Result<i64, DbErr> {
        let on_conflict = OnConflict::columns([
            entities_causality_seq::Column::TenantId,
            entities_causality_seq::Column::CausalityKey,
        ])
        .value(
            entities_causality_seq::Column::LastSeq,
            Expr::col(entities_causality_seq::Column::LastSeq).add(1),
        )
        .to_owned();

        let model = entities_causality_seq::Entity::insert(entities_causality_seq::ActiveModel {
            tenant_id: Set(tenant_id.to_string()),
            causality_key: Set(key.to_string()),
            last_seq: Set(1),
        })
        .on_conflict(on_conflict)
        .exec_with_returning(&db.inner.txn)
        .await?;

        Ok(model.last_seq)
    }
}
