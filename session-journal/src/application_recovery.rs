use super::*;
use myelin_session_authority::{AuthoritySigner, SessionAuthority, WriterGrant};

/// Reconcile a fenced journal against its exclusively opened local authority.
/// The administrator explicitly authorizes a new epoch only when needed to
/// revoke an unresolved submission or change workers. Missing in the same epoch
/// never releases work. This does not authorize takeover of a copied authority
/// database or replace the remote migration protocol.
pub fn recover_local_commands<E: TransitionExecutor>(
    authority: &SessionAuthority<E>,
    journal: &CommandJournal,
    administrator: &AuthoritySigner,
    worker: &AuthoritySigner,
) -> Result<WriterGrant> {
    let convert = |error: myelin_session_authority::AuthorityError| JournalError::Reconciliation(error.to_string());
    // Check the pinned journal policy before exercising administrator authority.
    {
        let state = journal.state.lock().map_err(|_| JournalError::Storage("journal mutex poisoned".into()))?;
        journal.ensure_healthy()?;
        if state.fencing_policy.as_ref() != Some(authority.policy()) {
            return Err(JournalError::Invalid("local recovery requires this authority's fenced journal".into()));
        }
    }
    if let Some(reservation) = journal.active_reservation()? {
        if let Some(binding) = &reservation.fenced_submission {
            let observed = authority.reconcile_submission(&binding.authorization, &binding.commands).map_err(convert)?;
            journal.reconcile_fenced(reservation.token, &observed, business_result)?;
        } else {
            journal.release_unsubmitted(reservation.token)?;
        }
    }
    let previous = authority.current_grant().map_err(convert)?;
    let pending = journal.active_reservation()?;
    let grant = if pending.is_some() || previous.as_ref().is_none_or(|grant| grant.owner() != worker.public_key()) {
        let request = administrator
            .authorize_grant(
                authority.policy(),
                previous.as_ref().map_or(0, |grant| grant.epoch),
                worker.public_key(),
                &authority.head().map_err(convert)?,
            )
            .map_err(convert)?;
        authority.grant(request).map_err(convert)?
    } else {
        previous.expect("existing matching grant")
    };
    if let Some(reservation) = pending {
        let binding = reservation.fenced_submission.ok_or_else(|| JournalError::Conflict("unbound submission".into()))?;
        let observed = authority.reconcile_submission(&binding.authorization, &binding.commands).map_err(convert)?;
        journal.reconcile_fenced(reservation.token, &observed, business_result)?;
    }
    Ok(grant)
}
