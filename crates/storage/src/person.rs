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
    pub avatar: Option<lankide_core::ResourceRef>,
    pub position: String,
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

    pub fn list_human_team_member_views(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<HumanTeamMemberView>> {
        let connection = self.read_conn()?;
        let mut statement = connection.prepare("SELECT person_id,team_role,name,avatar_json,position FROM people WHERE workspace_id=?1 AND team_role IN('member','lead') ORDER BY created_at,person_id")?;
        Ok(statement
            .query_map([workspace_id.as_str()], |row| {
                let role: String = row.get(1)?;
                let avatar: Option<String> = row.get(3)?;
                Ok(HumanTeamMemberView {
                    person_id: person_id(row.get(0)?)?,
                    name: row.get(2)?,
                    team_role: TeamRole::parse(&role)
                        .ok_or_else(|| rusqlite::Error::InvalidQuery)?,
                    avatar: avatar
                        .map(|v| {
                            serde_json::from_str(&v).map_err(|e| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    3,
                                    rusqlite::types::Type::Text,
                                    Box::new(e),
                                )
                            })
                        })
                        .transpose()?,
                    position: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Enrollment, enterprise validation and first-human election share one transaction.
    pub fn join_human_team(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
        tenant_id: Option<&str>,
        name: Option<&str>,
        avatar: Option<&lankide_core::ResourceRef>,
        now: i64,
    ) -> Result<PersonRecord> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let tenant: Option<String> = transaction.query_row("SELECT tenant_id FROM people WHERE workspace_id=?1 AND team_role IN('member','lead') AND tenant_id IS NOT NULL LIMIT 1", [workspace_id.as_str()], |r| r.get(0)).optional()?;
        if tenant.as_deref().is_some_and(|v| Some(v) != tenant_id) {
            return Err(StorageError::Conflict("team_enterprise".into()));
        }
        let has_member: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM people WHERE workspace_id=?1 AND team_role IN('member','lead'))", [workspace_id.as_str()], |r| r.get(0))?;
        let role = if has_member { "member" } else { "lead" };
        transaction.execute("UPDATE people SET team_role=CASE WHEN team_role='external' THEN ?1 ELSE team_role END, position=CASE WHEN team_role='external' AND ?1='lead' THEN 'Team Leader' ELSE position END,name=COALESCE(?2,name),avatar_json=COALESCE(?3,avatar_json),tenant_id=?4,updated_at=?5 WHERE workspace_id=?6 AND person_id=?7", params![role,name,avatar.map(serde_json::to_string).transpose()?,tenant_id,now,workspace_id.as_str(),person_id.as_str()])?;
        let person = load_person_by_id(&transaction, workspace_id, person_id)?
            .ok_or_else(|| StorageError::Invalid("unknown person".into()))?;
        transaction.commit()?;
        Ok(person)
    }

    /// Atomically reserve a team-wide sync attempt before any network request.
    pub fn claim_human_profile_sync(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
        force: bool,
        now: i64,
    ) -> Result<bool> {
        Ok(self.conn()?.execute("UPDATE people SET last_profile_sync_at=?3 WHERE workspace_id=?1 AND person_id=?2 AND team_role IN('member','lead') AND (?4 OR last_profile_sync_at IS NULL OR last_profile_sync_at<=?3-3600000)", params![workspace_id.as_str(),person_id.as_str(),now,force])? == 1)
    }

    pub fn human_profile(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
    ) -> Result<(Option<String>, Option<lankide_core::ResourceRef>)> {
        let (name, avatar): (Option<String>, Option<String>) = self.read_conn()?.query_row("SELECT name,avatar_json FROM people WHERE workspace_id=?1 AND person_id=?2 AND team_role IN('member','lead')", params![workspace_id.as_str(),person_id.as_str()], |r| Ok((r.get(0)?,r.get(1)?)))?;
        Ok((name, avatar.map(|v| serde_json::from_str(&v)).transpose()?))
    }

    pub fn update_human_profile(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
        name: Option<&str>,
        avatar: Option<&lankide_core::ResourceRef>,
        now: i64,
    ) -> Result<bool> {
        let connection = self.conn()?;
        let changed = connection.execute("UPDATE people SET name=COALESCE(?3,name),avatar_json=COALESCE(?4,avatar_json),updated_at=?5 WHERE workspace_id=?1 AND person_id=?2 AND team_role IN('member','lead') AND ((?3 IS NOT NULL AND name IS NOT ?3) OR (?4 IS NOT NULL AND avatar_json IS NOT ?4))", params![workspace_id.as_str(),person_id.as_str(),name,avatar.map(serde_json::to_string).transpose()?,now])?;
        if changed == 0 {
            let exists: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM people WHERE workspace_id=?1 AND person_id=?2 AND team_role IN('member','lead'))", params![workspace_id.as_str(),person_id.as_str()], |r| r.get(0))?;
            if !exists {
                return Err(StorageError::Invalid("unknown team member".into()));
            }
        }
        Ok(changed == 1)
    }

    pub fn update_human_position(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
        position: &str,
        now: i64,
    ) -> Result<()> {
        if position.chars().count() > 120 {
            return Err(StorageError::Invalid("position too long".into()));
        }
        let connection = self.conn()?;
        let changed = connection.execute("UPDATE people SET position=?1,updated_at=?2 WHERE workspace_id=?3 AND person_id=?4 AND team_role IN('member','lead')",params![position.trim(),now,workspace_id.as_str(),person_id.as_str()])?;
        if changed != 1 {
            return Err(StorageError::Invalid("unknown team member".into()));
        }
        Ok(())
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
