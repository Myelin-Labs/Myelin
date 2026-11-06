use super::*;
use myelin_session::BlockRequest;
use myelin_session_authority::{
    network::{AuthorityClient, ClientError},
    CanonicalReceipt,
};

#[derive(Debug, thiserror::Error)]
pub enum RemoteJournalError {
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error(transparent)]
    Rpc(#[from] ClientError),
}
pub struct RemoteSubmissionReport {
    /// This attempt can be rejected while an earlier attempt already committed.
    /// Use the reconciled entries for business status, never this field alone.
    pub attempt: std::result::Result<CanonicalReceipt, ClientError>,
    pub entries: Vec<JournalEntry>,
}
impl CommandJournal {
    /// Submit an already durably bound candidate, then reconcile whether the
    /// attempt succeeded, timed out or was rejected. Dropping this future leaves
    /// Unknown durable before any bytes can reach the authority. No automatic
    /// epoch rotation, new command ID, or release occurs on a transport failure.
    pub async fn submit_fenced_remote(
        &self,
        token: u64,
        client: &mut AuthorityClient,
        request: BlockRequest,
        interpret: impl FnMut(&FinalisedBlockRecord, &CommandReceipt) -> std::result::Result<Option<BusinessOutcome>, String>,
    ) -> std::result::Result<RemoteSubmissionReport, RemoteJournalError> {
        let binding = self.remote_binding(token, client)?;
        binding
            .authorization
            .verify_candidate(&binding.policy, &binding.grant, &request)
            .map_err(|e| JournalError::Invalid(e.to_string()))?;
        self.mark_unknown(token)?;
        let attempt = client.commit(binding.authorization.clone(), request).await;
        let certificate = client.reconcile(binding.authorization, binding.commands).await?;
        let entries = self.reconcile_fenced(token, &certificate, interpret)?;
        Ok(RemoteSubmissionReport { attempt, entries })
    }

    /// Recover after reconnect/restart without resubmitting the candidate.
    pub async fn reconcile_remote(
        &self,
        token: u64,
        client: &mut AuthorityClient,
        interpret: impl FnMut(&FinalisedBlockRecord, &CommandReceipt) -> std::result::Result<Option<BusinessOutcome>, String>,
    ) -> std::result::Result<Vec<JournalEntry>, RemoteJournalError> {
        let binding = self.remote_binding(token, client)?;
        let certificate = client.reconcile(binding.authorization, binding.commands).await?;
        Ok(self.reconcile_fenced(token, &certificate, interpret)?)
    }
    fn remote_binding(&self, token: u64, client: &AuthorityClient) -> Result<FencedSubmission> {
        let reservation = self.active_reservation()?.ok_or_else(|| JournalError::Conflict("no active remote reservation".into()))?;
        if reservation.token != token {
            return Err(JournalError::Conflict("remote reservation changed".into()));
        }
        let binding = reservation.fenced_submission.ok_or_else(|| JournalError::Conflict("no durable signed submission".into()))?;
        if &binding.policy != client.policy() {
            return Err(JournalError::Invalid("remote client policy differs from pinned journal policy".into()));
        }
        Ok(binding)
    }
}
