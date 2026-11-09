use super::*;
use crate::management::workspace_id;
use meerkat_protocol::{
    AgentRef, GetWorkspaceRequest, ManagementEventKind, Timer, TimerRepeat, TimerRequest,
};
use meerkat_storage::{NewTimer, TimerRecord, TimerRepeatRecord};

const MINUTE: i64 = 60_000;
/// Shortest gap between two firings of one timer.
const SHORTEST_MINUTES: i64 = 5;
const PROMPT_LIMIT: usize = 2_000;

fn zone(name: &str) -> Result<jiff::tz::TimeZone, RuntimeError> {
    jiff::tz::TimeZone::get(name)
        .map_err(|_| RuntimeError::Application(format!("Unknown timezone: {name}")))
}

/// The zone timers use when the Agent names none.
pub(crate) fn local_zone() -> String {
    jiff::tz::TimeZone::system()
        .iana_name()
        .unwrap_or("UTC")
        .to_owned()
}

fn cron(expression: &str) -> Result<croner::Cron, RuntimeError> {
    if expression.split_whitespace().count() != 5 {
        return Err(RuntimeError::Application(
            "cron needs five fields: minute hour day month weekday".into(),
        ));
    }
    expression
        .parse::<croner::Cron>()
        .map_err(|error| RuntimeError::Application(format!("Invalid cron: {error}")))
}

/// When a timer fires after `after`, or None once it is spent.
pub(crate) fn next_fire(
    repeat: &TimerRepeatRecord,
    timezone: &str,
    after: i64,
) -> Result<Option<i64>, RuntimeError> {
    match repeat {
        TimerRepeatRecord::Once => Ok(None),
        TimerRepeatRecord::Every { minutes } => {
            Ok(Some(after.saturating_add(i64::from(*minutes) * MINUTE)))
        }
        TimerRepeatRecord::Cron { expression } => {
            let start = jiff::Timestamp::from_millisecond(after)
                .map_err(|error| RuntimeError::Application(error.to_string()))?
                .to_zoned(zone(timezone)?);
            let next = cron(expression)?
                .find_next_occurrence(&start, false)
                .map_err(|error| RuntimeError::Application(format!("Invalid cron: {error}")))?;
            Ok(Some(next.timestamp().as_millisecond()))
        }
    }
}

/// A point in time the Agent wrote: with an offset, or as wall time in `timezone`.
fn instant(text: &str, timezone: &str) -> Result<i64, RuntimeError> {
    if let Ok(timestamp) = text.parse::<jiff::Timestamp>() {
        return Ok(timestamp.as_millisecond());
    }
    let wall = text.parse::<jiff::civil::DateTime>().map_err(|_| {
        RuntimeError::Application(format!(
            "Invalid time: {text}. Use YYYY-MM-DDTHH:MM in the timer's timezone."
        ))
    })?;
    Ok(wall
        .to_zoned(zone(timezone)?)
        .map_err(|error| RuntimeError::Application(error.to_string()))?
        .timestamp()
        .as_millisecond())
}

/// Wall time the Agent reads back, in the timer's zone.
fn wall(at: i64, timezone: &str) -> String {
    jiff::Timestamp::from_millisecond(at)
        .ok()
        .zip(jiff::tz::TimeZone::get(timezone).ok())
        .map(|(timestamp, zone)| {
            timestamp
                .to_zoned(zone)
                .strftime("%Y-%m-%d %H:%M %a")
                .to_string()
        })
        .unwrap_or_else(|| at.to_string())
}

/// A timer from `timer_set` arguments: cron, or every N minutes, or one time.
fn parse_set(
    object: &serde_json::Map<String, serde_json::Value>,
    now: i64,
) -> Result<(TimerRepeatRecord, String, i64), RuntimeError> {
    let timezone = match object.get("timezone").and_then(serde_json::Value::as_str) {
        Some(name) if !name.is_empty() => {
            zone(name)?;
            name.to_owned()
        }
        _ => local_zone(),
    };
    let at = object
        .get("at")
        .and_then(serde_json::Value::as_str)
        .map(|text| instant(text, &timezone))
        .transpose()?;
    let after = object.get("in_minutes").and_then(serde_json::Value::as_i64);
    if at.is_some() && after.is_some() {
        return Err(RuntimeError::Application(
            "Use either at or in_minutes, not both.".into(),
        ));
    }
    if after.is_some_and(|minutes| minutes < 1) {
        return Err(RuntimeError::Application(
            "in_minutes must be at least 1.".into(),
        ));
    }
    let first = at.or(after.map(|minutes| now + minutes * MINUTE));
    if first.is_some_and(|at| at <= now) {
        return Err(RuntimeError::Application(format!(
            "That time has passed; it is now {}.",
            wall(now, &timezone)
        )));
    }
    let every = object
        .get("every_minutes")
        .and_then(serde_json::Value::as_i64);
    let expression = object.get("cron").and_then(serde_json::Value::as_str);
    let (repeat, next) = match (expression, every, first) {
        (Some(_), Some(_), _) => {
            return Err(RuntimeError::Application(
                "Use either cron or every_minutes, not both.".into(),
            ));
        }
        (Some(_), None, Some(_)) => {
            return Err(RuntimeError::Application(
                "cron sets its own times; leave out at and in_minutes.".into(),
            ));
        }
        (Some(expression), None, None) => {
            let repeat = TimerRepeatRecord::Cron {
                expression: expression.trim().to_owned(),
            };
            let next = next_fire(&repeat, &timezone, now)?.unwrap_or(now);
            let after = next_fire(&repeat, &timezone, next)?.unwrap_or(i64::MAX);
            if after - next < SHORTEST_MINUTES * MINUTE {
                return Err(RuntimeError::Application(format!(
                    "Timers fire at most every {SHORTEST_MINUTES} minutes."
                )));
            }
            (repeat, next)
        }
        (None, Some(minutes), first) => {
            if !(SHORTEST_MINUTES..=366 * 24 * 60).contains(&minutes) {
                return Err(RuntimeError::Application(format!(
                    "every_minutes must be between {SHORTEST_MINUTES} and 527040."
                )));
            }
            (
                TimerRepeatRecord::Every {
                    minutes: minutes as u32,
                },
                first.unwrap_or(now + minutes * MINUTE),
            )
        }
        (None, None, Some(first)) => (TimerRepeatRecord::Once, first),
        (None, None, None) => {
            return Err(RuntimeError::Application(
                "Say when: at, in_minutes, every_minutes or cron.".into(),
            ));
        }
    };
    Ok((repeat, timezone, next))
}

fn timer_json(timer: &TimerRecord) -> serde_json::Value {
    serde_json::json!({
        "timer_id": timer.timer_id,
        "prompt": timer.prompt,
        "next_fire_at": wall(timer.next_fire_at, &timer.timezone),
        "repeat": timer.repeat,
        "timezone": timer.timezone,
        "channel_id": timer.channel_id.as_str(),
        "chat_id": timer.provider_chat_id,
        "chat_name": timer.chat_name,
    })
}

fn timer_dto(timer: TimerRecord) -> Timer {
    Timer {
        timer_id: timer.timer_id,
        agent: Some(AgentRef {
            workspace_id: timer.workspace_id.to_string(),
            agent_id: timer.agent_id.to_string(),
        }),
        prompt: timer.prompt,
        repeat: match timer.repeat {
            TimerRepeatRecord::Once => TimerRepeat::Once,
            TimerRepeatRecord::Every { minutes } => TimerRepeat::Every { minutes },
            TimerRepeatRecord::Cron { expression } => TimerRepeat::Cron { expression },
        },
        timezone: timer.timezone,
        next_fire_at: timer.next_fire_at,
        last_fired_at: timer.last_fired_at,
        chat_name: timer.chat_name,
        created_at: timer.created_at,
    }
}

impl RuntimeApplication {
    /// The Agent's `timer_*` tools; None for any other tool.
    pub(crate) async fn invoke_timer_tool(
        &self,
        name: &str,
        validated: &ValidatedRunContext,
        arguments: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        let workspace = &validated.workspace_id;
        let agent = &validated.agent_id;
        let database = self.runtime()?.database().clone();
        let now = now_millis();
        let result = match name {
            "timer_set" => {
                let object = arguments.as_object().ok_or_else(|| {
                    RuntimeError::Application("tool arguments must be an object".into())
                })?;
                let text = |key: &str| {
                    object
                        .get(key)
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| RuntimeError::Application(format!("{key} required")))
                };
                let prompt = text("prompt")?;
                if prompt.chars().count() > PROMPT_LIMIT {
                    return Err(RuntimeError::Application(format!(
                        "prompt is limited to {PROMPT_LIMIT} characters."
                    )));
                }
                let channel_id = ChannelId::new(text("channel_id")?)
                    .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
                let (repeat, timezone, next_fire_at) = parse_set(object, now)?;
                let timer = database
                    .create_timer(
                        &NewTimer {
                            workspace_id: workspace.clone(),
                            agent_id: agent.clone(),
                            channel_id,
                            provider_chat_id: text("chat_id")?.to_owned(),
                            prompt: prompt.to_owned(),
                            repeat,
                            timezone,
                            next_fire_at,
                        },
                        now,
                    )
                    .await
                    .map_err(|error| match error {
                        StorageError::Invalid(_) => RuntimeError::Application(
                            "You can only set timers in conversations you belong to.".into(),
                        ),
                        StorageError::Conflict(_) => RuntimeError::Application(format!(
                            "You already have {} timers; cancel one first.",
                            meerkat_storage::TIMER_LIMIT
                        )),
                        error => error.into(),
                    })?;
                self.timers_changed(workspace, agent);
                timer_json(&timer)
            }
            "timer_list" => {
                let timers = database.list_timers(workspace, Some(agent)).await?;
                serde_json::json!({
                    "now": wall(now, &local_zone()),
                    "timers": timers.iter().map(timer_json).collect::<Vec<_>>(),
                })
            }
            "timer_cancel" => {
                let timer_id = arguments
                    .get("timer_id")
                    .and_then(serde_json::Value::as_i64)
                    .ok_or_else(|| RuntimeError::Application("timer_id required".into()))?;
                let timer = database
                    .cancel_timer(workspace, Some(agent), timer_id, now)
                    .await?
                    .ok_or_else(|| {
                        RuntimeError::Application("You have no active timer with that ID.".into())
                    })?;
                self.timers_changed(workspace, agent);
                serde_json::json!({ "cancelled": timer.timer_id })
            }
            _ => return Ok(None),
        };
        Ok(Some(result))
    }

    pub async fn list_timers(
        &self,
        request: GetWorkspaceRequest,
    ) -> Result<Vec<Timer>, RuntimeError> {
        let workspace = workspace_id(request.workspace)?;
        let database = self.runtime()?.database().clone();
        database.workspace(&workspace).await?;
        Ok(database
            .list_timers(&workspace, None)
            .await?
            .into_iter()
            .map(timer_dto)
            .collect())
    }

    pub async fn cancel_timer(&self, request: TimerRequest) -> Result<Timer, RuntimeError> {
        let workspace = workspace_id(request.workspace)?;
        let timer = self
            .runtime()?
            .database()
            .cancel_timer(&workspace, None, request.timer_id, now_millis())
            .await?
            .ok_or_else(|| RuntimeError::Application("unknown timer".into()))?;
        self.timers_changed(&workspace, &timer.agent_id);
        Ok(timer_dto(timer))
    }

    /// Undo a cancel. A repeating timer that missed its time moves on to the next one.
    pub async fn restore_timer(&self, request: TimerRequest) -> Result<Timer, RuntimeError> {
        let workspace = workspace_id(request.workspace)?;
        let database = self.runtime()?.database().clone();
        let now = now_millis();
        let timer = database
            .cancelled_timer(&workspace, request.timer_id)
            .await?
            .ok_or_else(|| RuntimeError::Application("unknown timer".into()))?;
        let mut next = timer.next_fire_at;
        while next <= now {
            match next_fire(&timer.repeat, &timer.timezone, next.max(now - MINUTE))? {
                Some(later) => next = later,
                None => break,
            }
        }
        let timer = database
            .restore_timer(&workspace, request.timer_id, next, now)
            .await?
            .ok_or_else(|| RuntimeError::Application("unknown timer".into()))?;
        self.timers_changed(&workspace, &timer.agent_id);
        Ok(timer_dto(timer))
    }

    /// Fire the Workspace's due timers into their Agents' inboxes; returns when the next is due.
    /// A repeating timer that missed several times while the app was closed fires once.
    pub(crate) async fn fire_due_timers(
        &self,
        runtime: &Arc<Runtime>,
        workspace: &WorkspaceId,
    ) -> Result<Option<i64>, RuntimeError> {
        let database = runtime.database();
        let now = now_millis();
        for timer in database.due_timers(workspace, now).await? {
            let next = match next_fire(&timer.repeat, &timer.timezone, now) {
                Ok(next) => next,
                Err(error) => {
                    log::error!(
                        "[runtime] timer={} schedule invalid: {error}",
                        timer.timer_id
                    );
                    None
                }
            };
            if database
                .fire_timer(timer.timer_id, timer.next_fire_at, next, now)
                .await?
            {
                self.timers_changed(workspace, &timer.agent_id);
            }
        }
        Ok(database.next_timer_at(workspace).await?)
    }

    fn timers_changed(&self, workspace: &WorkspaceId, agent: &AgentId) {
        self.emit_management(
            ManagementEventKind::TimersChanged,
            workspace,
            Some(agent),
            now_millis(),
        );
        if let Ok(runtime) = self.runtime() {
            runtime.database().wake();
        }
    }
}
