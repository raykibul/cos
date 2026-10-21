//! Your limits on the chief for one run (its preset and approval cap), what
//! the chief has used of them, and the audit trail of answers given on your
//! behalf. The rules are `cos_policy`'s; this module keeps their books. See
//! `docs/approvals.md`.

use cos_policy::{
    AuditRecord, ChiefDecision, ChiefRefusal, Limits, Preset, Role as PolicyRole, RunLedger, Verdict,
    render_report_section,
};
use jiff::tz::TimeZone;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{ApprovalPreset, MAX_APPROVAL_CAP, RunError, Runs, no_run, storage};
use crate::events::kind;

/// `GET /v1/runs/{id}/limits`: the run's limits on the chief and what it has used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLimits {
    pub preset: ApprovalPreset,
    /// The most requests the chief may approve in the run.
    pub approval_cap: u32,
    /// How many it has approved so far.
    pub chief_approvals: u32,
}

impl RunLimits {
    /// The limits as `cos_policy` takes them.
    pub fn policy(&self) -> Limits {
        Limits {
            preset: self.preset.into(),
            chief_cap: self.approval_cap,
        }
    }

    /// What the run has used, as `cos_policy` takes it.
    pub fn ledger(&self) -> RunLedger {
        RunLedger {
            chief_approvals: self.chief_approvals,
        }
    }
}

/// `PUT /v1/runs/{id}/limits`: what to change. Missing fields stay as they are.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitsChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<ApprovalPreset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_cap: Option<u32>,
}

impl From<ApprovalPreset> for Preset {
    fn from(preset: ApprovalPreset) -> Self {
        match preset {
            ApprovalPreset::Strict => Preset::Strict,
            ApprovalPreset::Normal => Preset::Normal,
            ApprovalPreset::Trusted => Preset::Trusted,
        }
    }
}

impl Runs {
    pub fn limits(&self, run_id: i64) -> Result<RunLimits, RunError> {
        read_limits(&self.store.lock(), run_id)?.ok_or_else(|| no_run(run_id))
    }

    /// Changes a run's preset or cap while it isn't finished. Nothing is
    /// re-checked retroactively: the change applies to requests from now on,
    /// and to chief decisions on requests still open. Logs `run.limits_changed`.
    pub fn set_limits(&self, run_id: i64, change: &LimitsChange) -> Result<RunLimits, RunError> {
        if let Some(cap) = change.approval_cap
            && cap > MAX_APPROVAL_CAP
        {
            return Err(RunError::Invalid(format!(
                "the approval cap can be at most {MAX_APPROVAL_CAP}"
            )));
        }
        let limits = {
            let conn = self.store.lock();
            let run = storage::run(&conn, run_id)?.ok_or_else(|| no_run(run_id))?;
            if run.state.is_finished() {
                return Err(RunError::Conflict(format!(
                    "run {run_id} is {}; its limits can't change any more",
                    run.state
                )));
            }
            conn.execute(
                "UPDATE runs SET preset = COALESCE(?2, preset), approval_cap = COALESCE(?3, approval_cap),
                     updated_ms = ?4
                 WHERE id = ?1",
                params![
                    run_id,
                    change.preset.map(ApprovalPreset::as_str),
                    change.approval_cap,
                    storage::now().as_millisecond(),
                ],
            )?;
            read_limits(&conn, run_id)?.ok_or_else(|| no_run(run_id))?
        };
        self.log.append(
            kind::RUN_LIMITS_CHANGED,
            Some(&run_id.to_string()),
            json!({
                "run_id": run_id,
                "preset": limits.preset,
                "approval_cap": limits.approval_cap,
                "chief_approvals": limits.chief_approvals,
            }),
        )?;
        Ok(limits)
    }

    /// Checks the chief's answer to a request from one of the run's agents
    /// against the run's limits as they are now
    /// ([`Limits::apply_chief_decision`]), and counts an accepted allow
    /// against the cap. A finished run accepts nothing. The outer error is
    /// the database's; the inner one is the limits refusing.
    pub fn apply_chief_decision(
        &self,
        run_id: i64,
        verdict: &Verdict,
        role: PolicyRole,
        decision: &ChiefDecision,
    ) -> Result<Result<RunLimits, ChiefRefusal>, RunError> {
        let mut conn = self.store.lock();
        let tx = conn.transaction()?;
        let run = storage::run(&tx, run_id)?.ok_or_else(|| no_run(run_id))?;
        if run.state.is_finished() {
            return Ok(Err(ChiefRefusal {
                why: format!("run {run_id} is {}", run.state),
            }));
        }
        let limits = read_limits(&tx, run_id)?.ok_or_else(|| no_run(run_id))?;
        let mut ledger = limits.ledger();
        if let Err(refusal) = limits
            .policy()
            .apply_chief_decision(verdict, role, decision, &mut ledger)
        {
            return Ok(Err(refusal));
        }
        if ledger != limits.ledger() {
            tx.execute(
                "UPDATE runs SET chief_approvals = ?2 WHERE id = ?1",
                params![run_id, ledger.chief_approvals],
            )?;
            tx.commit()?;
        }
        Ok(Ok(RunLimits {
            chief_approvals: ledger.chief_approvals,
            ..limits
        }))
    }

    /// Adds an answer to the run's audit trail.
    pub fn record_audit(
        &self,
        run_id: i64,
        approval_id: Option<i64>,
        record: &AuditRecord,
    ) -> Result<(), RunError> {
        let json = serde_json::to_string(record).expect("audit records serialize");
        self.store.lock().execute(
            "INSERT INTO approval_audit (run_id, approval_id, at_ms, record) VALUES (?1, ?2, ?3, ?4)",
            params![run_id, approval_id, record.at.as_millisecond(), json],
        )?;
        Ok(())
    }

    /// The run's audit trail, oldest first.
    pub fn approval_audit(&self, run_id: i64) -> Result<Vec<AuditRecord>, RunError> {
        let conn = self.store.lock();
        let records = conn
            .prepare_cached("SELECT record FROM approval_audit WHERE run_id = ?1 ORDER BY id")?
            .query_map([run_id], |row| {
                let text: String = row.get(0)?;
                serde_json::from_str(&text).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(records)
    }

    /// The report section "Approved on your behalf (N)" for the run, with
    /// times in this machine's time zone. The chief's `report_write` appends
    /// it to the report.
    pub fn approvals_report_section(&self, run_id: i64) -> Result<String, RunError> {
        self.run(run_id)?;
        let records = self.approval_audit(run_id)?;
        Ok(render_report_section(&records, &TimeZone::system()))
    }
}

fn read_limits(conn: &Connection, run_id: i64) -> rusqlite::Result<Option<RunLimits>> {
    conn.query_row(
        "SELECT preset, approval_cap, chief_approvals FROM runs WHERE id = ?1",
        [run_id],
        |row| {
            let preset: String = row.get(0)?;
            Ok(RunLimits {
                preset: ApprovalPreset::parse(&preset).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        Type::Text,
                        format!("unknown approval preset {preset:?}").into(),
                    )
                })?,
                approval_cap: row.get(1)?,
                chief_approvals: row.get(2)?,
            })
        },
    )
    .optional()
}

#[cfg(test)]
mod tests;
