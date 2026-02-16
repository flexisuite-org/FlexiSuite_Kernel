use kernel_core::event::{EventEnvelope, OrderMode, EventError};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, EntityTrait,
    Set, DbErr,
};
use sea_orm::sea_query::{Expr, OnConflict};
use uuid::Uuid;
use crate::event::entities_entity_seq;
use crate::event::entities_causality_seq;
use crate::event::entities_outbox;

pub struct EventRepository;

impl EventRepository {
    /// Creates an event and inserts it into the outbox within the given transaction.
    /// Returns the fully formed EventEnvelope with the assigned sequence number.
    pub async fn create_event<C>(
        db: &C,
        event_id: Uuid,
        order_mode: OrderMode,
        payload: serde_json::Value,
    ) -> Result<EventEnvelope, EventError>
    where
        C: ConnectionTrait,
    {
        // 1. Generate Sequence
        let (seq, metadata_mode_str) = match &order_mode {
            OrderMode::Entity { entity_id, .. } => {
                let seq = Self::next_entity_seq(db, *entity_id).await
                    .map_err(|e| EventError::Store(format!("failed to generate entity seq: {}", e)))?;
                (seq, "entity")
            }
            OrderMode::Causality { key, .. } => {
                let seq = Self::next_causality_seq(db, key).await
                    .map_err(|e| EventError::Store(format!("failed to generate causality seq: {}", e)))?;
                (seq, "causality")
            }
        };

        // 2. Prepare OrderMode with seq
        let final_order_mode = match order_mode {
            OrderMode::Entity { entity_id, .. } => OrderMode::Entity {
                entity_id,
                seq: Some(seq as u64),
            },
            OrderMode::Causality { key, .. } => OrderMode::Causality {
                key,
                seq: Some(seq as u64),
            },
        };

        // 3. Insert into Outbox
        let outbox_model = entities_outbox::ActiveModel {
            event_id: Set(event_id),
            order_mode: Set(metadata_mode_str.to_string()),
            entity_id: Set(match &final_order_mode {
                OrderMode::Entity { entity_id, .. } => Some(*entity_id),
                _ => None,
            }),
            entity_seq: Set(match &final_order_mode {
                OrderMode::Entity { seq, .. } => seq.map(|s| s as i64),
                _ => None,
            }),
            causality_key: Set(match &final_order_mode {
                OrderMode::Causality { key, .. } => Some(key.clone()),
                _ => None,
            }),
            causality_seq: Set(match &final_order_mode {
                OrderMode::Causality { seq, .. } => seq.map(|s| s as i64),
                _ => None,
            }),
            payload: Set(payload.clone()),
            created_at: Set(chrono::Utc::now().into()), // Or use DB default
            published_at: Set(None),
        };

        outbox_model.insert(db).await
            .map_err(|e| EventError::Store(format!("failed to insert into outbox: {}", e)))?;

        // 4. Return Envelope
        Ok(EventEnvelope {
            event_id,
            order_mode: final_order_mode,
            payload,
            created_at: chrono::Utc::now(), // Approximate, strictly should use DB time or pass in time
        })
    }

    async fn next_entity_seq<C>(db: &C, entity_id: Uuid) -> Result<i64, DbErr>
    where
        C: ConnectionTrait,
    {
        // Use raw SQL for atomic upsert with returning, as SeaORM's ON CONFLICT support for returning might vary by backend/version
        // ensuring maximum compatibility and explicit control.
        // Actually, SeaORM 1.1 supports this well.

        let on_conflict = OnConflict::column(entities_entity_seq::Column::EntityId)
            .update_column(entities_entity_seq::Column::LastSeq)
            .value(
                entities_entity_seq::Column::LastSeq,
                Expr::col(entities_entity_seq::Column::LastSeq).add(1)
            )
            .to_owned();

        let model = entities_entity_seq::Entity::insert(
            entities_entity_seq::ActiveModel {
                entity_id: Set(entity_id),
                last_seq: Set(1),
            }
        )
        .on_conflict(on_conflict)
        .exec_with_returning(db)
        .await?;

        Ok(model.last_seq)
    }

    async fn next_causality_seq<C>(db: &C, key: &str) -> Result<i64, DbErr>
    where
        C: ConnectionTrait,
    {
        let on_conflict = OnConflict::column(entities_causality_seq::Column::CausalityKey)
            .update_column(entities_causality_seq::Column::LastSeq)
            .value(
                entities_causality_seq::Column::LastSeq,
                Expr::col(entities_causality_seq::Column::LastSeq).add(1)
            )
            .to_owned();

        let model = entities_causality_seq::Entity::insert(
            entities_causality_seq::ActiveModel {
                causality_key: Set(key.to_string()),
                last_seq: Set(1),
            }
        )
        .on_conflict(on_conflict)
        .exec_with_returning(db)
        .await?;

        Ok(model.last_seq)
    }
}
