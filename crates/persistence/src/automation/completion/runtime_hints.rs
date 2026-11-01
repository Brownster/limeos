//! Query-only bounded hints. Admission reloads all authority and current work.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationOwnerPage {
    pub owners: Vec<String>,
    pub next: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservationWorkHint {
    Queued(ObservationJob),
    Schedule(Schedule),
}

impl Store {
    pub fn automation_observation_owner_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<ObservationOwnerPage> {
        if !(1..=MAX_TICK_SCHEDULES).contains(&limit)
            || after.is_some_and(|v| !limeos_domain::identifier(v))
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        let mut statement = self.conn.prepare("SELECT DISTINCT owner FROM automation_schedules WHERE owner>? ORDER BY owner LIMIT ?").map_err(durable)?;
        let mut rows = statement
            .query(params![after.unwrap_or(""), limit as i64 + 1])
            .map_err(durable)?;
        let mut owners = Vec::new();
        let mut next = None;
        while let Some(row) = rows.next().map_err(durable)? {
            if owners.len() == limit {
                next = owners.last().cloned();
                break;
            }
            let owner = row
                .get_ref(0)
                .map_err(durable)?
                .as_str()
                .map_err(|_| Error(ErrorCode::StateNotDurable))?;
            if !limeos_domain::identifier(owner) {
                return Err(Error(ErrorCode::StateNotDurable));
            }
            owners.push(owner.into());
        }
        Ok(ObservationOwnerPage { owners, next })
    }

    pub fn automation_observation_owner_work(
        &self,
        owner: &str,
        after: Option<&str>,
    ) -> Result<Option<ObservationWorkHint>> {
        if !limeos_domain::identifier(owner) || after.is_some_and(|v| !limeos_domain::opaque_id(v))
        {
            return Err(Error(ErrorCode::InvalidInput));
        }
        // Match the completion admission's oldest-per-owner ordering exactly.
        let mut statement = self.conn.prepare("SELECT j.id,j.intent FROM jobs j JOIN automation_occurrences o ON o.job=j.id WHERE j.principal=? AND j.state='queued' AND json_extract(j.intent,'$.operation')='automation_observe_health' ORDER BY o.created,o.due,j.id LIMIT 1").map_err(durable)?;
        let mut rows = statement.query([owner]).map_err(durable)?;
        if let Some(row) = rows.next().map_err(durable)? {
            let id = row
                .get_ref(0)
                .map_err(durable)?
                .as_str()
                .map_err(|_| Error(ErrorCode::StateNotDurable))?;
            let intent = row
                .get_ref(1)
                .map_err(durable)?
                .as_str()
                .map_err(|_| Error(ErrorCode::StateNotDurable))?;
            return Ok(Some(ObservationWorkHint::Queued(queued_hint(id, intent)?)));
        }
        let id = next_schedule(&self.conn, owner, after)?;
        let id = if id.is_none() && after.is_some() {
            next_schedule(&self.conn, owner, None)?
        } else {
            id
        };
        id.map(|id| {
            load(&self.conn, &id)?
                .map(|r| ObservationWorkHint::Schedule(r.schedule))
                .ok_or(Error(ErrorCode::StateNotDurable))
        })
        .transpose()
    }
}

fn next_schedule(conn: &Connection, owner: &str, after: Option<&str>) -> Result<Option<String>> {
    let mut statement = conn
        .prepare("SELECT id FROM automation_schedules WHERE owner=? AND id>? ORDER BY id LIMIT 1")
        .map_err(durable)?;
    let mut rows = statement
        .query(params![owner, after.unwrap_or("")])
        .map_err(durable)?;
    let Some(row) = rows.next().map_err(durable)? else {
        return Ok(None);
    };
    let id = row
        .get_ref(0)
        .map_err(durable)?
        .as_str()
        .map_err(|_| Error(ErrorCode::StateNotDurable))?;
    if !limeos_domain::opaque_id(id) {
        return Err(Error(ErrorCode::StateNotDurable));
    }
    Ok(Some(id.into()))
}

#[cfg(test)]
mod tests;
