use crate::error::conv;
use crate::workspace::{pid, wid};
use crate::{Result, Storage, StorageError};
use lankide_core::{PersonId, ProviderId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use lankide_core::TeamRole;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PersonRecord {
    pub workspace_id: WorkspaceId,
    pub person_id: PersonId,
    pub team_role: TeamRole,
    pub provider_instance_id: ProviderId,
    pub external_id: String,
    pub provider_identity_state: Value,
    pub provider_state_revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HumanTeamMemberView {
    pub person_id: PersonId,
    pub name: Option<String>,
    pub team_role: TeamRole,
}

fn person_id(s: String) -> rusqlite::Result<PersonId> {
    PersonId::new(s).map_err(conv)
}

fn map_person(row: &rusqlite::Row<'_>) -> rusqlite::Result<PersonRecord> {
    let role = row.get::<_, String>(2)?;
    let provider_state = row.get::<_, String>(5)?;
    Ok(PersonRecord {
        workspace_id: wid(row.get(0)?)?,
        person_id: person_id(row.get(1)?)?,
        team_role: TeamRole::parse(&role)
            .ok_or_else(|| StorageError::Invalid(format!("team role {role}")))
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        provider_instance_id: pid(row.get(3)?)?,
        external_id: row.get(4)?,
        provider_identity_state: serde_json::from_str(&provider_state).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        provider_state_revision: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn load_person_by_external(
    connection: &Connection,
    workspace_id: &WorkspaceId,
    provider_instance_id: &ProviderId,
    external_id: &str,
) -> Result<Option<PersonRecord>> {
    Ok(connection.query_row(
        "SELECT workspace_id,person_id,team_role,provider_instance_id,external_id,provider_identity_state_json,provider_state_revision,created_at,updated_at FROM people WHERE workspace_id=?1 AND provider_instance_id=?2 AND external_id=?3",
        params![workspace_id.as_str(), provider_instance_id.as_str(), external_id],
        map_person,
    ).optional()?)
}

fn load_person_by_id(
    connection: &Connection,
    workspace_id: &WorkspaceId,
    person_id: &PersonId,
) -> Result<Option<PersonRecord>> {
    Ok(connection.query_row(
        "SELECT workspace_id,person_id,team_role,provider_instance_id,external_id,provider_identity_state_json,provider_state_revision,created_at,updated_at FROM people WHERE workspace_id=?1 AND person_id=?2",
        params![workspace_id.as_str(), person_id.as_str()],
        map_person,
    ).optional()?)
}

impl Storage {
    pub fn get_or_create_person(
        &self,
        workspace_id: &WorkspaceId,
        provider_instance_id: &ProviderId,
        external_id: &str,
        now: i64,
    ) -> Result<PersonRecord> {
        let external_id = external_id.trim();
        if external_id.is_empty() {
            return Err(StorageError::Invalid("empty external person id".into()));
        }
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        if let Some(person) = load_person_by_external(
            &transaction,
            workspace_id,
            provider_instance_id,
            external_id,
        )? {
            transaction.commit()?;
            return Ok(person);
        }
        let person_id = loop {
            let candidate = uuid::Uuid::new_v4().simple().to_string()[..6].to_owned();
            let occupied: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM people WHERE workspace_id=?1 AND person_id=?2 UNION ALL SELECT 1 FROM agents WHERE workspace_id=?1 AND agent_id=?2)",
                params![workspace_id.as_str(), candidate],
                |row| row.get(0),
            )?;
            if !occupied {
                break PersonId::new(candidate)
                    .map_err(|error| StorageError::Invalid(error.to_string()))?;
            }
        };
        transaction.execute(
            "INSERT INTO people(workspace_id,person_id,team_role,provider_instance_id,external_id,provider_identity_state_json,provider_state_revision,created_at,updated_at) VALUES(?1,?2,'external',?3,?4,'{}',0,?5,?5)",
            params![workspace_id.as_str(), person_id.as_str(), provider_instance_id.as_str(), external_id, now],
        )?;
        let person = load_person_by_id(&transaction, workspace_id, &person_id)?
            .ok_or_else(|| StorageError::Invalid("person disappeared".into()))?;
        transaction.commit()?;
        Ok(person)
    }

    pub fn person(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
    ) -> Result<Option<PersonRecord>> {
        load_person_by_id(&*self.read_conn()?, workspace_id, person_id)
    }

    pub fn list_human_team_members(&self, workspace_id: &WorkspaceId) -> Result<Vec<PersonRecord>> {
        let connection = self.read_conn()?;
        let mut statement = connection.prepare(
            "SELECT workspace_id,person_id,team_role,provider_instance_id,external_id,provider_identity_state_json,provider_state_revision,created_at,updated_at FROM people WHERE workspace_id=?1 AND team_role IN('member','lead') ORDER BY person_id",
        )?;
        Ok(statement
            .query_map([workspace_id.as_str()], map_person)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Return the Agent-facing human roster. Display names come from accepted
    /// messages, so the people identity record remains limited to stable IDs
    /// and Provider-owned identity state.
    pub fn list_human_team_member_views(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<HumanTeamMemberView>> {
        let connection = self.read_conn()?;
        let mut statement = connection.prepare(
            "SELECT p.person_id,p.team_role,(SELECT NULLIF(TRIM(m.sender_name),'') FROM messages m JOIN chats c ON c.chat_id=m.chat_id WHERE c.workspace_id=p.workspace_id AND m.sender_kind='human' AND m.sender_id=p.person_id AND NULLIF(TRIM(m.sender_name),'') IS NOT NULL ORDER BY m.occurred_at DESC,m.message_id DESC LIMIT 1) FROM people p WHERE p.workspace_id=?1 AND p.team_role IN('member','lead') ORDER BY p.person_id",
        )?;
        Ok(statement
            .query_map([workspace_id.as_str()], |row| {
                let role = row.get::<_, String>(1)?;
                Ok(HumanTeamMemberView {
                    person_id: person_id(row.get(0)?)?,
                    name: row.get(2)?,
                    team_role: TeamRole::parse(&role)
                        .ok_or_else(|| StorageError::Invalid(format!("team role {role}")))
                        .map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                1,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn compare_and_set_person_state(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
        expected_revision: i64,
        provider_identity_state: &Value,
        now: i64,
    ) -> Result<PersonRecord> {
        if !provider_identity_state.is_object() {
            return Err(StorageError::Invalid(
                "person provider identity state must be an object".into(),
            ));
        }
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE people SET provider_identity_state_json=?1,provider_state_revision=provider_state_revision+1,updated_at=?2 WHERE workspace_id=?3 AND person_id=?4 AND provider_state_revision=?5",
            params![serde_json::to_string(provider_identity_state)?, now, workspace_id.as_str(), person_id.as_str(), expected_revision],
        )?;
        if changed != 1 {
            let exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM people WHERE workspace_id=?1 AND person_id=?2)",
                params![workspace_id.as_str(), person_id.as_str()],
                |row| row.get(0),
            )?;
            return if exists {
                Err(StorageError::Conflict("stale person identity state".into()))
            } else {
                Err(StorageError::Invalid("unknown person".into()))
            };
        }
        load_person_by_id(&*connection, workspace_id, person_id)?
            .ok_or_else(|| StorageError::Invalid("person disappeared".into()))
    }

    pub fn set_person_team_role(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
        role: TeamRole,
        now: i64,
    ) -> Result<PersonRecord> {
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE people SET team_role=?1,updated_at=?2 WHERE workspace_id=?3 AND person_id=?4",
            params![
                role.as_str(),
                now,
                workspace_id.as_str(),
                person_id.as_str()
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::Invalid("unknown person".into()));
        }
        load_person_by_id(&*connection, workspace_id, person_id)?
            .ok_or_else(|| StorageError::Invalid("person disappeared".into()))
    }

    pub fn initialize_team_lead(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
        now: i64,
    ) -> Result<PersonRecord> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let current_lead: Option<String> = transaction
            .query_row(
                "SELECT person_id FROM people WHERE workspace_id=?1 AND team_role='lead'",
                [workspace_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        match current_lead {
            Some(current) if current != person_id.as_str() => {
                return Err(StorageError::Conflict(
                    "workspace already has a team lead".into(),
                ));
            }
            Some(_) => {}
            None => {
                let changed = transaction.execute(
                    "UPDATE people SET team_role='lead',updated_at=?1 WHERE workspace_id=?2 AND person_id=?3",
                    params![now, workspace_id.as_str(), person_id.as_str()],
                )?;
                if changed != 1 {
                    return Err(StorageError::Invalid("unknown person".into()));
                }
            }
        }
        let person = load_person_by_id(&transaction, workspace_id, person_id)?
            .ok_or_else(|| StorageError::Invalid("person disappeared".into()))?;
        transaction.commit()?;
        Ok(person)
    }

    pub fn update_team_member(
        &self,
        workspace_id: &WorkspaceId,
        actor: &PersonId,
        target: &PersonId,
        role: TeamRole,
        now: i64,
    ) -> Result<PersonRecord> {
        if role == TeamRole::External {
            return Err(StorageError::Invalid("unsupported team role update".into()));
        }
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let actor_role: Option<String> = transaction
            .query_row(
                "SELECT team_role FROM people WHERE workspace_id=?1 AND person_id=?2",
                params![workspace_id.as_str(), actor.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if actor_role.as_deref() != Some("lead") {
            return Err(StorageError::Conflict("team lead required".into()));
        }
        if role == TeamRole::Lead {
            transaction.execute(
                "UPDATE people SET team_role='member',updated_at=?1 WHERE workspace_id=?2 AND team_role='lead'",
                params![now, workspace_id.as_str()],
            )?;
        }
        let changed = transaction.execute(
            "UPDATE people SET team_role=?1,updated_at=?2 WHERE workspace_id=?3 AND person_id=?4",
            params![role.as_str(), now, workspace_id.as_str(), target.as_str()],
        )?;
        if changed != 1 {
            return Err(StorageError::Invalid("unknown person".into()));
        }
        let person = load_person_by_id(&transaction, workspace_id, target)?
            .ok_or_else(|| StorageError::Invalid("person disappeared".into()))?;
        transaction.commit()?;
        Ok(person)
    }
}
